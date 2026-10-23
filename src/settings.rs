//! The settings window: a card in the style of the definition card, with
//! the settings grouped on it the way Windows 11 groups its own. Like the
//! card it's a composition visual tree, so all its motion runs on the system
//! compositor: the card rises in with its parts a beat after one another,
//! switches slide, the chosen theme's pill glides over, numbers roll, a new
//! shortcut's keys rise into place, the shortcuts and the list of languages
//! open out of their rows as the card grows to fit them, and the focus ring
//! follows Tab from control to control. Changing the theme cross-fades the whole window.
//! The one text field, for the MyMemory email, is drawn the same way, its
//! caret and all.
//!
//! This file has the window's state and its input handling; `view` builds the
//! visuals, `metrics` has the sizes and durations, and `controls`, `picker`,
//! `scroll` and `show` split the rest of `impl Settings`.

mod controls;
mod metrics;
mod picker;
mod scroll;
mod show;
mod view;
mod widgets;

use std::cell::Cell;

use windows::core::{Result, HSTRING};
use windows::Win32::Foundation::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::*;
use windows_numerics::{Vector2, Vector3};

use crate::app::edit::{Input, LineEdit, Motion};
use crate::app::picking::Picking;
use crate::config::{self, Config, SelectionMode, Shortcut, ThemeMode, SHORTCUTS, ZOOMS};
use crate::dicts;
use crate::hotkey::{self, held_modifiers, is_modifier, Hotkey};
use crate::i18n::{self, tr, Language, Strings};
use crate::lang::LangCode;
use crate::render::{
    Align, Face, Ink, Layout, ListLayout, Picker, Renderer, Side, Target, PICKER_ROWS,
};
use crate::selection;
use crate::theme::Theme;
use crate::ui::{
    self, scale, ts, v2, v3, Animator, Bursts, Frame, Glide, List, Push, Rect, ScrollFade,
    Scrollbar, Text, Track, MARGIN,
};
use crate::win::{self, Window};

/// What the window's timer for the dots after "Waiting for Windows" is.
pub use metrics::TIMER_DOTS;
use metrics::*;

/// What the settings window shows: the settings, and whether WinDict
/// starts with Windows, and shows above Start, which are kept in the
/// registry instead.
#[derive(Clone)]
pub struct Values {
    pub config: Config,
    pub autostart: bool,
    /// Whether UI access is on, in the installed copy: only it can have it.
    pub ui_access: Option<bool>,
}

/// Something done in the window, for the app to carry out.
#[derive(Debug, PartialEq)]
pub enum Change {
    /// Recording a new shortcut started: the current one should stop
    /// working meanwhile.
    RecordingStarted,
    /// Recording ended without a new shortcut.
    RecordingStopped,
    Hotkey(Shortcut, String),
    Theme(ThemeMode),
    /// The language WinDict is shown in.
    Interface(Language),
    /// A language's code, like "en", turned on or off.
    Language(LangCode, bool),
    MaxDefinitions(usize),
    MaxSynonyms(usize),
    ShowExamples(bool),
    /// How large the card is drawn, in percent.
    Zoom(u32),
    OnlineLookup(bool),
    Selection(SelectionMode),
    Animations(bool),
    Autostart(bool),
    /// Turn UI access on or off. The switch waits, until
    /// [`Settings::settle_ui_access`].
    UiAccess(bool),
    CheckUpdates(bool),
    /// The languages translated from and into when the card can't tell,
    /// as codes.
    TranslateLanguages(LangCode, LangCode),
    MyMemoryEmail(String),
    OpenConfig,
    Close,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Switch {
    Autostart,
    /// Only in the installed copy.
    UiAccess,
    Online,
    Examples,
    Animations,
    Updates,
}

const SWITCHES: [Switch; 6] = [
    Switch::Autostart,
    Switch::UiAccess,
    Switch::Online,
    Switch::Examples,
    Switch::Animations,
    Switch::Updates,
];

/// A number stepped through with − and + buttons.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Counter {
    Definitions,
    Synonyms,
    /// Which of [`ZOOMS`] the card is drawn at.
    Zoom,
}

const COUNTERS: [Counter; 3] = [Counter::Definitions, Counter::Synonyms, Counter::Zoom];

impl Counter {
    /// The lowest and highest values.
    fn range(self) -> (usize, usize) {
        match self {
            Counter::Definitions => (1, 10),
            Counter::Synonyms => (0, 12),
            Counter::Zoom => (0, ZOOMS.len() - 1),
        }
    }

    fn text(self, n: usize) -> String {
        match (self, n) {
            (Counter::Zoom, _) => format!("{}%", ZOOMS[n]),
            (_, 0) => tr().off.into(),
            _ => n.to_string(),
        }
    }

    fn width(self) -> f32 {
        2.0 * STEP_SIZE + VALUE_W
    }
}

const THEMES: [(ThemeMode, Label); 3] = [
    (ThemeMode::System, |s| s.theme_system),
    (ThemeMode::Light, |s| s.theme_light),
    (ThemeMode::Dark, |s| s.theme_dark),
];

/// The ways of reading the selection, each with what it does.
const SELECTIONS: [(SelectionMode, Label, Label); 3] = [
    (
        SelectionMode::Automatic,
        |s| s.selection_automatic,
        |s| s.selection_automatic_detail,
    ),
    (
        SelectionMode::Direct,
        |s| s.selection_direct,
        |s| s.selection_direct_detail,
    ),
    (
        SelectionMode::Clipboard,
        |s| s.selection_clipboard,
        |s| s.selection_clipboard_detail,
    ),
];

/// What the chosen way of reading the selection does.
fn selection_detail(mode: SelectionMode) -> &'static str {
    let (_, _, detail) = SELECTIONS
        .iter()
        .find(|(m, _, _)| *m == mode)
        .unwrap_or(&SELECTIONS[0]);
    detail(tr())
}

/// A setting chosen from a few options, side by side.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Choice {
    Theme,
    Language,
    Selection,
}

const CHOICES: [Choice; 3] = [Choice::Theme, Choice::Language, Choice::Selection];

impl Choice {
    fn names(self) -> Vec<&'static str> {
        match self {
            Choice::Theme => THEMES.iter().map(|(_, name)| name(tr())).collect(),
            // Each in itself, so that it's found whichever language this is.
            Choice::Language => Language::ALL.iter().map(|l| l.name()).collect(),
            Choice::Selection => SELECTIONS.iter().map(|(_, name, _)| name(tr())).collect(),
        }
    }

    /// How wide each option is: the way of reading the selection has the
    /// row to itself, the others share it with their names.
    fn segment_w(self) -> f32 {
        match self {
            Choice::Selection => (GROUP_W - TEXT_X - ROW_PAD) / SELECTIONS.len() as f32,
            Choice::Theme | Choice::Language => SEGMENT_W,
        }
    }
}

/// The names of `languages`, as the languages row sums them up: "English,
/// Italiano".
fn summary(languages: &[LangCode]) -> String {
    let names: Vec<_> = dicts::languages()
        .filter(|(code, _)| languages.iter().any(|l| l == code))
        .map(|(_, name)| name)
        .collect();
    names.join(", ")
}

/// What the shortcuts row sums up: each shortcut's keys.
fn shortcuts_summary(config: &Config) -> String {
    let keys = |shortcut| key_names(config.hotkey(shortcut)).join("+");
    i18n::fill(
        tr().shortcuts_summary,
        &[
            ("lookup", &keys(Shortcut::Lookup)),
            ("translate", &keys(Shortcut::Translate)),
        ],
    )
}

/// A row that opens out into a list of more under it, and shuts again.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Fold {
    /// The shortcuts, each a row like the other settings.
    Shortcuts,
    /// The languages words are looked up in, each with a box to tick.
    Languages,
}

/// In the order they're in on the card.
const FOLDS: [Fold; 2] = [Fold::Shortcuts, Fold::Languages];

/// The rows in the shortcuts' list.
const SHORTCUT_ROWS: [Row; SHORTCUTS.len()] = [
    row(
        "\u{E721}",
        |s| s.lookup_shortcut,
        |s| s.shortcut_detail,
        Control::Shortcut(Shortcut::Lookup),
    ),
    row(
        "\u{E8C1}",
        |s| s.translate_shortcut,
        |s| s.translate_shortcut_detail,
        Control::Shortcut(Shortcut::Translate),
    ),
];

/// A setting's description that can change to say what's going on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Note {
    Shortcuts,
    Shortcut,
    TranslateShortcut,
    Language,
    Selection,
    /// How turning UI access on or off is going.
    UiAccess,
}

const NOTES: [Note; 6] = [
    Note::Shortcuts,
    Note::Shortcut,
    Note::TranslateShortcut,
    Note::Language,
    Note::Selection,
    Note::UiAccess,
];

impl Note {
    /// The description under `shortcut`.
    fn of(shortcut: Shortcut) -> Note {
        match shortcut {
            Shortcut::Lookup => Note::Shortcut,
            Shortcut::Translate => Note::TranslateShortcut,
        }
    }

    fn usual(self, values: &Values) -> String {
        match self {
            Note::Shortcuts => shortcuts_summary(&values.config),
            Note::Shortcut => tr().shortcut_detail.into(),
            Note::TranslateShortcut => tr().translate_shortcut_detail.into(),
            Note::Language => summary(&values.config.languages),
            Note::Selection => selection_detail(values.config.selection).into(),
            Note::UiAccess => tr().above_start_detail.into(),
        }
    }
}

/// What's under the pointer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Hit {
    Close,
    Shortcut(Shortcut),
    Switch(Switch),
    /// One of a choice's options.
    Choice(Choice, usize),
    /// A counter's down (false) or up (true) button.
    Step(Counter, bool),
    /// A row that opens and shuts the list under it.
    Fold(Fold),
    /// A language in that list, by its place in [`dicts::languages`].
    Language(usize),
    /// The email field.
    Email,
    /// The info symbol beside it, with its tooltip.
    Info,
    /// A default language's box, which lists the others to pick from.
    Select(Side),
    /// The button between them, which swaps them.
    Swap,
    /// A language in that list, by its place among the matches.
    Pick(usize),
    Config,
}

/// What Tab stops at, in order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stop {
    Shortcut(Shortcut),
    Switch(Switch),
    Choice(Choice),
    Counter(Counter),
    Email,
    Select(Side),
    Swap,
    Fold(Fold),
    Language(usize),
    Config,
}

impl Hit {
    fn stop(self) -> Option<Stop> {
        match self {
            Hit::Close => None,
            Hit::Shortcut(shortcut) => Some(Stop::Shortcut(shortcut)),
            Hit::Switch(sw) => Some(Stop::Switch(sw)),
            Hit::Choice(choice, _) => Some(Stop::Choice(choice)),
            Hit::Step(counter, _) => Some(Stop::Counter(counter)),
            Hit::Fold(fold) => Some(Stop::Fold(fold)),
            Hit::Language(i) => Some(Stop::Language(i)),
            Hit::Email => Some(Stop::Email),
            Hit::Info | Hit::Pick(_) => None,
            Hit::Select(side) => Some(Stop::Select(side)),
            Hit::Swap => Some(Stop::Swap),
            Hit::Config => Some(Stop::Config),
        }
    }

    /// The fold whose list it's in, if it's in one.
    fn folded(self) -> Option<Fold> {
        self.stop().and_then(Stop::folded)
    }
}

impl Stop {
    /// The fold whose list it's in, if it's in one.
    fn folded(self) -> Option<Fold> {
        match self {
            Stop::Shortcut(_) => Some(Fold::Shortcuts),
            Stop::Language(_) => Some(Fold::Languages),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
enum Control {
    Shortcut(Shortcut),
    Switch(Switch),
    Choice(Choice),
    Counter(Counter),
    /// Opens the list under it. It's the last row of its group, so that
    /// only what's below the group moves as the list opens.
    Fold(Fold),
    /// The field for the MyMemory email, with an info symbol after the
    /// setting's name.
    Email,
    /// The default languages' boxes, with the list under the one being
    /// picked. It's taller than the other rows, and as tall as the list
    /// when that's open.
    Translate,
    /// The ways of reading the selection, under the setting's name and over
    /// what the chosen one does.
    Selection,
}

impl Control {
    fn width(self) -> f32 {
        match self {
            Control::Shortcut(_) => SHORTCUT_W,
            Control::Switch(_) => SWITCH_W,
            Control::Choice(choice) => choice.names().len() as f32 * choice.segment_w(),
            Control::Counter(counter) => counter.width(),
            Control::Fold(_) => CHEVRON_W,
            Control::Email => FIELD_W,
            Control::Translate | Control::Selection => 0.0,
        }
    }
}

/// A string of the current language, picked out of its [`Strings`]: the rows
/// are consts, written before the language is known.
type Label = fn(&Strings) -> &'static str;

struct Row {
    /// A Segoe Fluent Icons glyph shown before the name.
    icon: &'static str,
    title: Label,
    detail: Label,
    control: Control,
}

impl Row {
    fn title(&self) -> &'static str {
        (self.title)(tr())
    }

    fn detail(&self) -> &'static str {
        (self.detail)(tr())
    }

    /// Whether the row shows, with `values`: the one for UI access only in
    /// the installed copy.
    fn shown(&self, values: &Values) -> bool {
        !matches!(self.control, Control::Switch(Switch::UiAccess)) || values.ui_access.is_some()
    }
}

const fn row(icon: &'static str, title: Label, detail: Label, control: Control) -> Row {
    Row {
        icon,
        title,
        detail,
        control,
    }
}

const SECTIONS: &[(Label, &[Row])] = &[
    (
        |s| s.general,
        &[
            row(
                "\u{E7E8}",
                |s| s.start_with_windows,
                |s| s.start_with_windows_detail,
                Control::Switch(Switch::Autostart),
            ),
            row(
                "\u{E81E}",
                |s| s.above_start,
                |s| s.above_start_detail,
                Control::Switch(Switch::UiAccess),
            ),
            row(
                "\u{E895}",
                |s| s.check_updates,
                |s| s.check_updates_detail,
                Control::Switch(Switch::Updates),
            ),
            row(
                "\u{F2B7}",
                |s| s.language,
                |s| s.language_detail,
                Control::Choice(Choice::Language),
            ),
            // Its description sums up the shortcuts' keys.
            row(
                "\u{E765}",
                |s| s.shortcuts,
                |_| "",
                Control::Fold(Fold::Shortcuts),
            ),
        ],
    ),
    (
        |s| s.card,
        &[
            row(
                "\u{E790}",
                |s| s.theme,
                |s| s.theme_detail,
                Control::Choice(Choice::Theme),
            ),
            row(
                "\u{E8E9}",
                |s| s.text_size,
                |s| s.text_size_detail,
                Control::Counter(Counter::Zoom),
            ),
            row(
                "\u{E736}",
                |s| s.definitions,
                |s| s.definitions_detail,
                Control::Counter(Counter::Definitions),
            ),
            row(
                "\u{E8FD}",
                |s| s.synonyms,
                |s| s.synonyms_detail,
                Control::Counter(Counter::Synonyms),
            ),
            row(
                "\u{E8BD}",
                |s| s.examples,
                |s| s.examples_detail,
                Control::Switch(Switch::Examples),
            ),
            row(
                "\u{E7F8}",
                |s| s.animations,
                |s| s.animations_detail,
                Control::Switch(Switch::Animations),
            ),
        ],
    ),
    (
        |s| s.lookup,
        &[
            row(
                "\u{E753}",
                |s| s.online_dictionary,
                |s| s.online_dictionary_detail,
                Control::Switch(Switch::Online),
            ),
            // Its description is what the chosen way does.
            row("\u{E77F}", |s| s.read_selection, |_| "", Control::Selection),
            // Its description sums up the languages chosen, each in itself.
            row(
                "\u{E774}",
                |s| s.languages,
                |_| "English",
                Control::Fold(Fold::Languages),
            ),
        ],
    ),
    (
        |s| s.translation,
        &[
            row(
                "\u{E8C1}",
                |s| s.default_languages,
                |s| s.default_languages_detail,
                Control::Translate,
            ),
            row(
                "\u{E715}",
                |s| s.mymemory_email,
                |s| s.mymemory_email_detail,
                Control::Email,
            ),
        ],
    ),
];

/// The scale of the window zoomed to `zoom` on a monitor of `scale` pixels
/// per DIP: no wider than fits in the `work` area, unless even the unzoomed
/// one isn't. It scrolls to fit in height.
fn zoomed(work: RECT, scale: f32, zoom: f32) -> f32 {
    let room = (work.right - work.left) as f32 / (CARD_W + 2.0 * SCREEN_GAP);
    (scale * zoom).min(room.max(scale))
}

/// The keys of the shortcut `text`, as written when it can't be read.
fn key_names(text: &str) -> Vec<String> {
    hotkey::parse(text).map(|h| h.names()).unwrap_or_else(|_| {
        text.split('+')
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .map(String::from)
            .collect()
    })
}

// ---------------------------------------------------------------------------
// The visuals, rebuilt whenever the scale or the theme changes

struct SwitchView {
    /// The accent fill of a switch that's on, over the outline of one that's off.
    on: SpriteVisual,
    off: ContainerVisual,
    knob: SpriteVisual,
    knob_brush: CompositionColorBrush,
    /// The knob's place when off and when on.
    rest: [Vector3; 2],
    /// Behind the whole row, which is what's clicked.
    hover: SpriteVisual,
}

struct ChoiceView {
    pill: ContainerVisual,
    labels: Vec<SpriteVisual>,
    /// Where the pill sits over each option.
    rest: Vec<Vector3>,
}

struct StepButton {
    visual: ContainerVisual,
    hover: SpriteVisual,
    glyph: SpriteVisual,
}

struct CounterView {
    /// Down, then up.
    buttons: Vec<StepButton>,
    /// Clips the number, which rolls in and out of it.
    host: ContainerVisual,
    value: Text,
    old: Text,
    host_h: f32,
}

struct ShortcutView {
    hover: SpriteVisual,
    caps: Text,
    old_caps: Text,
    /// Where the caps end on the right, and their middle, in the group.
    caps_right: f32,
    caps_middle: f32,
    caps_rest: Vector3,
    ring: ShapeVisual,
}

struct NoteView {
    text: Text,
    old: Text,
    at: Vector3,
    w: f32,
}

/// A language in the list, with a box ticked when it's used.
struct LanguageView {
    /// The accent fill and tick of a box that's ticked, over the outline
    /// of one that isn't.
    on: ContainerVisual,
    off: ContainerVisual,
    hover: SpriteVisual,
    /// How its download is going, ending at `status_right` and centred on
    /// `status_middle`, in the list.
    status: Text,
    status_right: f32,
    status_middle: f32,
}

/// A tooltip, shown over what it's for.
struct TipView {
    root: ContainerVisual,
    size: Vector2,
}

/// The email field.
struct FieldView {
    /// Lightens the field under the pointer.
    hover: SpriteVisual,
    /// The accent line along the bottom, shown while typing.
    accent: SpriteVisual,
    /// What's typed, scrolling inside the field to keep the caret in view.
    text: Text,
    caret: SpriteVisual,
    /// Where the host is in the scroller.
    at: Rect,
}

/// A row that folds, and the list that opens under it, in its group.
struct FoldView {
    hover: SpriteVisual,
    /// Turns over as the list opens.
    chevron: SpriteVisual,
    /// The group's clip and outline, which grow with the list (the
    /// background is as tall as it is open already).
    clip: CompositionRoundedRectangleGeometry,
    outline: CompositionRoundedRectangleGeometry,
    group_w: f32,
    /// The group's height with the list shut.
    group_h: f32,
    /// The list, at the bottom of the group.
    list: ContainerVisual,
    /// Where the list starts in the scroller with every list shut, and
    /// how tall it is.
    list_y: f32,
    list_h: f32,
    /// What comes after, which moves down as the list opens: the groups
    /// below and the footer. It rests at the top with the list shut.
    after: ContainerVisual,
}

/// Which folds have their list open, in the order of [`FOLDS`].
type Open = [bool; FOLDS.len()];

struct View {
    /// Everything; fades as one when the theme changes.
    content: ContainerVisual,
    header_h: f32,
    /// The height of everything below the header, in pixels.
    content_h: f32,
    close_at: Rect,
    close_hover: SpriteVisual,
    /// Clips the scrolling part below the header.
    body: ContainerVisual,
    /// Shows `body`, faded at the ends it scrolls towards.
    fade: ScrollFade,
    scroller: ContainerVisual,
    bar: Scrollbar,
    /// The tooltips, and what each shows over.
    tips: Vec<(Hit, TipView)>,
    /// What rises in as the window opens, and where each rests.
    reveal: Vec<(Visual, Vector3)>,
    /// In the order of [`SHORTCUTS`].
    shortcuts: Vec<ShortcutView>,
    /// In the order of [`SWITCHES`] and [`COUNTERS`].
    /// A row that isn't shown has none.
    switches: Vec<Option<SwitchView>>,
    field: FieldView,
    /// In the order of [`CHOICES`] and [`NOTES`].
    choices: Vec<ChoiceView>,
    notes: Vec<Option<NoteView>>,
    counters: Vec<CounterView>,
    /// The default languages' row, in the scroller.
    translate_at: Rect,
    spare: SpareView,
    /// The boxes of the default languages, to draw again in place.
    boxes: BoxesView,
    /// Where things can be clicked, in the scroller.
    hits: Vec<(Hit, Rect)>,
    /// Where Tab stops, and the corner radius of the ring there, in the scroller.
    stops: Vec<(Stop, Rect, f32)>,
    ring: SpriteVisual,
    /// The mask of the ring, and the corner radius it was drawn for.
    ring_source: CompositionSurfaceBrush,
    ring_grid: CompositionNineGridBrush,
    ring_radius: Cell<f32>,
    /// In the order of [`FOLDS`].
    folds: Vec<FoldView>,
    /// In the order of [`dicts::languages`].
    languages: Vec<LanguageView>,
    /// How much shorter the default languages' row is than with the list
    /// of matches full, and where, with it full, what it moves up starts.
    slack: f32,
    slack_y: f32,
}

impl View {
    /// The height of everything below the header with the lists as `open` says.
    fn height(&self, open: &Open) -> f32 {
        let lists: f32 = self
            .folds
            .iter()
            .zip(open)
            .filter(|(_, &open)| open)
            .map(|(fold, _)| fold.list_h)
            .sum();
        self.content_h + lists - self.slack
    }

    /// Where `rect`, in the scroller with every list shut, is with them as
    /// `open` says, or nothing if it's in the list of `folded` and that's
    /// shut. What's in a list is where it is with that list open.
    fn place(&self, rect: Rect, folded: Option<Fold>, open: &Open) -> Option<Rect> {
        let mut y = rect.y;
        for ((fold, view), &open) in FOLDS.iter().zip(&self.folds).zip(open) {
            let listed = folded == Some(*fold);
            if listed && !open {
                return None;
            }
            if !listed && open && rect.y >= view.list_y {
                y += view.list_h;
            }
        }
        if rect.y >= self.slack_y {
            y -= self.slack;
        }
        Some(Rect { y, ..rect })
    }

    /// Where things can be clicked, with the lists as `open` says.
    fn hits<'a>(&'a self, open: &'a Open) -> impl Iterator<Item = (Hit, Rect)> + 'a {
        self.hits
            .iter()
            .filter_map(move |&(hit, rect)| Some((hit, self.place(rect, hit.folded(), open)?)))
    }

    /// Where Tab stops, with the lists as `open` says.
    fn stops<'a>(&'a self, open: &'a Open) -> impl Iterator<Item = (Stop, Rect, f32)> + 'a {
        self.stops.iter().filter_map(move |&(stop, rect, radius)| {
            Some((stop, self.place(rect, stop.folded(), open)?, radius))
        })
    }
}

/// The default languages' boxes in the view, and where they sit.
struct BoxesView {
    text: Text,
    caret: Option<SpriteVisual>,
    /// Lightens the swap button under the pointer.
    swap_hover: SpriteVisual,
    /// In the group, and the group in the scroller.
    at: Vector3,
    group_at: Vector3,
}

/// What slides up as the list of matches gets shorter: the rows after the
/// languages in their group, the groups after it, and the group's own
/// background and outline.
struct SpareView {
    tail: ContainerVisual,
    below: ContainerVisual,
    bg: SpriteVisual,
    clip: CompositionRoundedRectangleGeometry,
    border: CompositionRoundedRectangleGeometry,
    /// The group's size with the list full.
    size: Vector2,
}

pub struct Settings {
    pub hwnd: HWND,
    _target: DesktopWindowTarget,
    renderer: Renderer,
    anim: Animator,
    /// Everything on the window; fades and scales as one when showing and hiding.
    root: ContainerVisual,
    frame: Frame,
    view: Option<View>,
    /// The view before the theme changed, fading out.
    fading: Option<ContainerVisual>,
    values: Values,
    theme: Theme,
    scale: f32,
    /// Card size, physical pixels.
    card: (f32, f32),
    hovered: Option<Hit>,
    pressed: Option<Hit>,
    focus: Option<Stop>,
    /// Whether the focus ring shows: only once the keyboard is used.
    ring: bool,
    ring_shown: bool,
    /// The shortcut being recorded, if one is.
    recording: Option<Shortcut>,
    /// The modifiers held while recording.
    preview: HOT_KEY_MODIFIERS,
    /// What the descriptions that change say, and whether it's that
    /// something went wrong, in the order of [`NOTES`].
    notes: [(String, bool); NOTES.len()],
    /// Which folds have their list open.
    open: Open,
    /// How each language's download is going, and whether that's that it
    /// failed, in the order of [`dicts::languages`].
    downloads: Vec<Option<(String, bool)>>,
    /// The choice whose pill is gliding over, and the option it's leaving.
    pill_from: Option<(Choice, usize)>,
    /// The list a default language is being picked from, while it's open,
    /// and what shows it, in the boxes.
    picking: Option<Picking>,
    list: List,
    /// What's typed in the email field.
    email: LineEdit,
    /// How far the field's text is scrolled left, in pixels.
    email_shift: f32,
    /// Whether the window is the active one, for the caret to show.
    active: bool,
    scroll: f32,
    /// The scroll last started, which may still be running.
    scroll_anim: Option<Glide>,
    wheel_bursts: Bursts,
    /// Windows is asking whether UI access may change, and how many steps
    /// the dots after saying so have taken.
    waiting: bool,
    dots: usize,
    pub visible: bool,
    /// The tallest the card gets, in pixels. The window is made for that,
    /// so that it never resizes under the blurred background.
    room: f32,
    /// Last, so it goes after everything shown on it.
    _window: Window,
}

impl Settings {
    pub fn new(
        window: Window,
        compositor: Compositor,
        theme: Theme,
        values: Values,
    ) -> Result<Self> {
        let hwnd = window.hwnd();
        let target = ui::attach(hwnd, &compositor)?;
        let renderer = Renderer::new(&compositor)?;
        let root = compositor.CreateContainerVisual()?;
        target.SetRoot(&root)?;
        let frame = Frame::new(&compositor, &root, &theme)?;
        let notes = NOTES.map(|note| (note.usual(&values), false));
        let email = LineEdit::new(&values.config.mymemory_email, MAX_EMAIL_CHARS);
        let list = List::new(&compositor)?;
        Ok(Settings {
            hwnd,
            _target: target,
            renderer,
            anim: Animator::new(compositor)?,
            root,
            frame,
            view: None,
            fading: None,
            values,
            theme,
            scale: 1.0,
            card: (0.0, 0.0),
            hovered: None,
            pressed: None,
            focus: None,
            ring: false,
            ring_shown: false,
            recording: None,
            preview: HOT_KEY_MODIFIERS(0),
            notes,
            open: [false; FOLDS.len()],
            downloads: dicts::languages().map(|_| None).collect(),
            pill_from: None,
            picking: None,
            list,
            email,
            email_shift: 0.0,
            active: true,
            scroll: 0.0,
            scroll_anim: None,
            wheel_bursts: Bursts::default(),
            waiting: false,
            dots: 0,
            visible: false,
            room: 0.0,
            _window: window,
        })
    }

    // -----------------------------------------------------------------------
    // Public API

    /// Opens the window in the middle of the screen `near` (screen pixels)
    /// is on, showing `values`.
    pub fn show(&mut self, near: POINT, theme: Theme, values: Values) -> Result<()> {
        let (work, s) = ui::monitor_at(near);
        let s = zoomed(work, s, values.config.zoom_factor());
        (self.theme, self.scale, self.values) = (theme, s, values);
        (self.hovered, self.pressed, self.focus) = (None, None, None);
        (self.ring, self.recording, self.scroll) = (false, None, 0.0);
        self.scroll_anim = None;
        self.notes = NOTES.map(|note| (note.usual(&self.values), false));
        self.retitle();
        (self.open, self.picking) = ([false; FOLDS.len()], None);
        self.downloads.fill(None);
        self.email = LineEdit::new(&self.values.config.mymemory_email, MAX_EMAIL_CHARS);
        (self.email_shift, self.active) = (0.0, true);
        win::kill_timer(self.hwnd, TIMER_SETTLE);
        self.frame.apply_theme(&self.renderer, &theme, s)?;
        self.replace_view(false)?;
        self.fit(work)?;

        let m = (MARGIN * s).round();
        let gap = (SCREEN_GAP * s).round();
        let w = (self.card.0 + 2.0 * m) as i32;
        let h = (self.room + 2.0 * m) as i32;
        let left = (work.left + work.right - w) / 2;
        let centred = (work.top + work.bottom - (self.card.1 + 2.0 * m) as i32) / 2;
        let top = centred
            .min(work.bottom - gap as i32 - h + m as i32)
            .max(work.top + gap as i32 - m as i32);

        // Rises out of the middle, a little small and low.
        let root = &self.root;
        Animator::stop(root, &["Opacity", "Offset", "Scale"]);
        root.SetCenterPoint(Vector3 {
            X: m + self.card.0 / 2.0,
            Y: m + self.card.1 / 2.0,
            Z: 0.0,
        })?;
        root.SetOpacity(0.0)?;
        root.SetScale(scale(0.96))?;
        root.SetOffset(v3(0.0, 12.0 * s))?;
        unsafe {
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOP), left, top, w, h, SWP_SHOWWINDOW);
        }
        self.set_region(self.card.1);
        self.visible = true;

        self.anim.fade_in(root, ui::FADE_IN_MS, SHOW_MS)?;
        self.reveal()
    }

    /// Fades the window out. The caller hides it after [`ui::HIDE_MS`].
    pub fn begin_hide(&mut self) -> Result<()> {
        if !self.visible {
            return Ok(());
        }
        self.visible = false;
        self.recording = None;
        self.anim.fade_out(&self.root, 6.0 * self.scale)
    }

    pub fn finish_hide(&mut self) {
        if self.visible {
            return;
        }
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    /// Whether a new shortcut is being recorded.
    pub fn recording(&self) -> bool {
        self.recording.is_some()
    }

    /// Whether a point in window pixels is on the header, which drags the window.
    pub fn caption_at(&self, x: i32, y: i32) -> bool {
        let Some(view) = &self.view else {
            return false;
        };
        let (x, y) = self.card_point(x, y);
        x >= 0.0
            && x < self.card.0
            && y >= 0.0
            && y < view.header_h
            && !view.close_at.contains(x, y)
    }

    /// Whether a point in window pixels is on the link to the config file.
    pub fn link_at(&self, x: i32, y: i32) -> bool {
        self.hit_at(x, y) == Some(Hit::Config)
    }

    /// Whether a point in window pixels is on the email field.
    pub fn field_at(&self, x: i32, y: i32) -> bool {
        self.hit_at(x, y) == Some(Hit::Email)
    }

    /// The pointer moved to a point in window pixels, or left.
    pub fn hover(&mut self, point: Option<(i32, i32)>) -> Result<()> {
        // On a scrollbar, or dragging its thumb, nothing else is hovered.
        let in_boxes = point.and_then(|(x, y)| self.boxes_point(x, y));
        let on_list = self.list.hover_bar(&self.anim, in_boxes)?;
        let point = if on_list { None } else { point };
        let point = if self.hover_bar(point)? { None } else { point };
        let hit = point.and_then(|(x, y)| self.hit_at(x, y));
        self.set_hovered(hit)
    }

    /// The left button went down at a point in window pixels.
    pub fn press(&mut self, x: i32, y: i32) -> Result<Option<Change>> {
        let hit = self.hit_at(x, y);
        // A click sends a tooltip away, as the system's do.
        self.show_tip(None)?;
        if let Some((bx, by)) = self.boxes_point(x, y) {
            if self.list.press(&self.anim, bx, by)? {
                return Ok(None);
            }
        }
        if self.press_bar(x, y)? {
            return Ok(None);
        }
        // The pointer's in charge now: the ring goes until Tab brings it back.
        self.ring = false;
        self.active = true;
        if let Some(stop) = hit.and_then(Hit::stop) {
            self.focus = Some(stop);
        } else if self.focus == Some(Stop::Email) {
            // A click anywhere else leaves the field.
            self.focus = None;
        }
        if hit == Some(Hit::Email) {
            self.place_caret(x)?;
        }
        // A click away from the open box shuts its list.
        let away = !matches!(hit, Some(Hit::Select(_) | Hit::Pick(_) | Hit::Swap));
        if away {
            if let Some(side) = self.picking.take().map(|p| p.side) {
                self.focus = hit.and_then(Hit::stop).or(Some(Stop::Select(side)));
                self.relayout()?;
            }
        }
        self.show_focus(false)?;
        self.set_pressed(hit)?;
        let on_it = |shortcut| hit == Some(Hit::Shortcut(shortcut));
        if self.recording.is_some_and(|shortcut| !on_it(shortcut)) {
            self.stop_recording()?;
            return Ok(Some(Change::RecordingStopped));
        }
        Ok(None)
    }

    /// The left button came up at a point in window pixels: a click, if it's
    /// on what it went down on.
    pub fn release(&mut self, x: i32, y: i32) -> Result<Option<Change>> {
        let on_list = self.list.release(&self.anim)?;
        if on_list | self.view.as_mut().is_some_and(|view| view.bar.release()) {
            self.hover(Some((x, y)))?;
            return Ok(None);
        }
        let pressed = self.pressed;
        self.set_pressed(None)?;
        match pressed {
            Some(hit) if self.hit_at(x, y) == Some(hit) => self.activate(hit),
            _ => Ok(None),
        }
    }

    /// A key went down.
    pub fn key(&mut self, key: VIRTUAL_KEY) -> Result<Option<Change>> {
        if self.recording() {
            return self.record(key, held_modifiers());
        }
        if self.picking.is_some() {
            return self.picker_key(key);
        }
        let shift = hotkey::is_held(VK_SHIFT);
        let ctrl = hotkey::is_held(VK_CONTROL);
        // Ctrl+plus, Ctrl+minus and Ctrl+0 zoom the window, as on the card.
        let zoom = self.values.config.zoom;
        let zoom = match key {
            VK_OEM_PLUS | VK_ADD if ctrl => Some(config::step_zoom(zoom, 1)),
            VK_OEM_MINUS | VK_SUBTRACT if ctrl => Some(config::step_zoom(zoom, -1)),
            k if ctrl && (k.0 == b'0' as u16 || k == VK_NUMPAD0) => Some(100),
            _ => None,
        };
        if let Some(zoom) = zoom {
            return Ok((zoom != self.values.config.zoom).then_some(Change::Zoom(zoom)));
        }
        let change = match (key, self.focus) {
            (VK_ESCAPE, _) => return Ok(Some(Change::Close)),
            (VK_TAB, _) => {
                self.step_focus(!shift)?;
                return Ok(None);
            }
            (VK_LEFT | VK_RIGHT | VK_HOME | VK_END, Some(Stop::Email)) => {
                let motion = match key {
                    VK_LEFT if ctrl => Motion::WordBack,
                    VK_LEFT => Motion::CharBack,
                    VK_RIGHT if ctrl => Motion::WordForward,
                    VK_RIGHT => Motion::CharForward,
                    VK_HOME => Motion::Start,
                    _ => Motion::End,
                };
                if self.email.move_caret(motion) {
                    self.show_field()?;
                }
                None
            }
            (VK_DELETE, Some(Stop::Email)) => {
                let before = self.email.text().to_owned();
                let motion = if ctrl {
                    Motion::WordForward
                } else {
                    Motion::CharForward
                };
                self.email.delete(motion);
                self.typed(&before)?
            }
            (VK_SPACE | VK_RETURN, Some(Stop::Shortcut(shortcut))) => {
                self.activate(Hit::Shortcut(shortcut))?
            }
            (VK_SPACE | VK_RETURN, Some(Stop::Switch(sw))) => self.activate(Hit::Switch(sw))?,
            (VK_SPACE | VK_RETURN, Some(Stop::Config)) => self.activate(Hit::Config)?,
            (VK_SPACE | VK_RETURN, Some(Stop::Select(side))) => self.activate(Hit::Select(side))?,
            (VK_SPACE | VK_RETURN, Some(Stop::Swap)) => self.activate(Hit::Swap)?,
            (VK_SPACE | VK_RETURN, Some(Stop::Fold(fold))) => self.activate(Hit::Fold(fold))?,
            (VK_RIGHT, Some(Stop::Fold(fold))) => {
                self.set_open(fold, true)?;
                None
            }
            (VK_LEFT, Some(Stop::Fold(fold))) => {
                self.set_open(fold, false)?;
                None
            }
            // From what's in a list back to the row it folds into, shutting it.
            (VK_LEFT, Some(stop @ (Stop::Language(_) | Stop::Shortcut(_)))) => {
                if let Some(fold) = stop.folded() {
                    self.focus = Some(Stop::Fold(fold));
                    self.set_open(fold, false)?;
                }
                None
            }
            (VK_SPACE | VK_RETURN, Some(Stop::Language(i))) => self.activate(Hit::Language(i))?,
            (VK_LEFT | VK_RIGHT, Some(Stop::Choice(choice))) => {
                let at = self.chosen(choice);
                let next = if key == VK_RIGHT {
                    (at + 1).min(choice.names().len() - 1)
                } else {
                    at.saturating_sub(1)
                };
                self.choose(choice, next)?
            }
            (VK_LEFT | VK_DOWN | VK_SUBTRACT | VK_OEM_MINUS, Some(Stop::Counter(c))) => {
                self.step(c, false)?
            }
            (VK_RIGHT | VK_UP | VK_ADD | VK_OEM_PLUS, Some(Stop::Counter(c))) => {
                self.step(c, true)?
            }
            _ => None,
        };
        Ok(change)
    }

    /// A character was typed (one UTF-16 unit of WM_CHAR): it goes in the
    /// box of the language being picked, or else in the email field, when
    /// that has the focus.
    pub fn char(&mut self, unit: u16) -> Result<Option<Change>> {
        if self.recording() {
            return Ok(None);
        }
        if self.picking.is_some() {
            return self.type_language(unit);
        }
        if self.focus != Some(Stop::Email) {
            return Ok(None);
        }
        let before = self.email.text().to_owned();
        match self.email.type_unit(unit) {
            Input::Edited => {}
            Input::Paste => {
                if let Some(text) = selection::clipboard_text(self.hwnd) {
                    self.email.paste(&text);
                }
            }
            Input::Enter | Input::Nothing => return Ok(None),
        }
        self.typed(&before)
    }

    /// A key came up: while recording, the held modifiers show as they're let go.
    pub fn key_up(&mut self, key: VIRTUAL_KEY) -> Result<()> {
        if self.recording() && is_modifier(key) {
            self.set_preview(held_modifiers())?;
        }
        Ok(())
    }

    /// A key went `down`, or up, while recording, caught before Windows
    /// could act on it, leaving `modifiers` held.
    pub fn record_key(
        &mut self,
        key: VIRTUAL_KEY,
        modifiers: HOT_KEY_MODIFIERS,
        down: bool,
    ) -> Result<Option<Change>> {
        if !self.recording() {
            return Ok(None);
        }
        if down {
            return self.record(key, modifiers);
        }
        if is_modifier(key) {
            self.set_preview(modifiers)?;
        }
        Ok(None)
    }

    /// The window became the active one again: the caret's back.
    pub fn activated(&mut self) -> Result<()> {
        self.active = true;
        self.show_editing()
    }

    /// The window stopped being the active one: recording stops.
    pub fn deactivate(&mut self) -> Result<Option<Change>> {
        self.active = false;
        self.show_editing()?;
        self.set_pressed(None)?;
        self.lose_capture()?;
        if !self.recording() {
            return Ok(None);
        }
        self.stop_recording()?;
        Ok(Some(Change::RecordingStopped))
    }

    /// Scrolls by a mouse wheel delta, the pointer at a point in window
    /// pixels: the list of languages, however far it can go, when it's on
    /// it, or else the window.
    pub fn wheel(&mut self, delta: i32, (x, y): (i32, i32)) -> Result<()> {
        let in_boxes = self.boxes_point(x, y);
        if in_boxes.is_some_and(|(x, y)| self.list.contains(x, y)) {
            self.list.wheel(&self.anim, delta)?;
            // The highlight moves to the row the pointer's on now.
            return self.hover(Some((x, y)));
        }
        let mut step = -(delta as f32 / 120.0) * WHEEL_STEP * self.scale;
        // Past the end, it gives a little, and springs back.
        let max = self.max_scroll();
        if max <= 0.0 {
            return Ok(());
        }
        // A touchpad's scroll is smoothed already, so it follows at once.
        let precise = self.wheel_bursts.precise(delta);
        if precise {
            step = self.wheel_bursts.precise_step(step, self.scale);
        }
        if !self.wheel_bursts.take(self.scroll + step, max) {
            return Ok(());
        }
        let visible = self.visible_scroll();
        let (to, push) = ui::stretch(visible, self.scroll, step, max, self.scale);
        let ms = if precise {
            self.scroll_anim.map_or(0, |glide| glide.easing_left())
        } else {
            SCROLL_MS
        };
        self.glide_to(to, push, ms)
    }

    /// The shortcut just recorded couldn't be used: `shortcut` goes back
    /// to `hotkey`, saying why.
    pub fn reject_hotkey(&mut self, shortcut: Shortcut, hotkey: &str, why: &str) -> Result<()> {
        *self.values.config.hotkey_mut(shortcut) = hotkey.to_owned();
        self.show_caps(shortcut, true)?;
        self.shake_caps(shortcut)?;
        let summary = shortcuts_summary(&self.values.config);
        self.set_note(Note::Shortcuts, &summary, false)?;
        self.set_note(Note::of(shortcut), why, true)
    }

    /// Ticks `languages` (codes, like "en"), and only them.
    pub fn show_languages(&mut self, languages: &[LangCode]) -> Result<()> {
        self.values.config.languages = languages.to_vec();
        self.show_checks(true)?;
        let text = summary(languages);
        self.set_note(Note::Language, &text, false)
    }

    /// Says how `language`'s download is going, or why it failed, beside it
    /// in the list; or nothing, with no `status`.
    pub fn show_download(&mut self, language: &str, status: Option<(&str, bool)>) -> Result<()> {
        let Some(i) = dicts::languages().position(|(code, _)| code == language) else {
            return Ok(());
        };
        let status = status.map(|(text, error)| (text.to_owned(), error));
        if self.downloads[i] == status {
            return Ok(());
        }
        self.downloads[i] = status;
        self.show_status(i)
    }

    /// The card has shrunk with the list: the window does too.
    pub fn settle(&mut self) -> Result<()> {
        win::kill_timer(self.hwnd, TIMER_SETTLE);
        let Some(view) = &self.view else {
            return Ok(());
        };
        view.body
            .SetSize(v2(self.card.0, self.card.1 - view.header_h))?;
        self.set_region(self.card.1);
        Ok(())
    }

    /// Switches to `theme`, cross-fading everything.
    pub fn retheme(&mut self, theme: Theme) -> Result<()> {
        let same = (theme.dark, theme.transparency, theme.accent)
            == (self.theme.dark, self.theme.transparency, self.theme.accent);
        if same {
            self.pill_from = self
                .pill_from
                .filter(|(choice, _)| *choice != Choice::Theme);
            return Ok(());
        }
        self.theme = theme;
        let (anim, renderer, s) = (&self.anim, &self.renderer, self.scale);
        self.frame.fade_theme(anim, renderer, &theme, s, THEME_MS)?;
        self.replace_view(true)
    }

    /// Shows the window in the language now current, cross-fading to it.
    pub fn relanguage(&mut self) -> Result<()> {
        self.notes = NOTES.map(|note| (note.usual(&self.values), false));
        self.retitle();
        self.replace_view(true)
    }

    /// Names the window in the language now current, as the taskbar shows it.
    fn retitle(&self) {
        unsafe {
            let _ = SetWindowTextW(self.hwnd, &HSTRING::from(tr().settings_window_title));
        }
    }

    /// UI access is `on`, as turning it on or off left it, and `note` says
    /// how that went, and whether it `failed`.
    pub fn settle_ui_access(&mut self, on: bool, note: &str, failed: bool) -> Result<()> {
        (self.values.ui_access, self.waiting) = (Some(on), false);
        win::kill_timer(self.hwnd, TIMER_DOTS);
        self.show_switch(Switch::UiAccess, true)?;
        self.set_note(Note::UiAccess, note, failed)
    }

    /// Whether Windows is asking whether UI access may change: the window
    /// stays while it does.
    pub fn waiting(&self) -> bool {
        self.waiting
    }

    /// The dots after "Waiting for Windows" take a step, in place.
    pub fn step_dots(&mut self) -> Result<()> {
        if !self.waiting {
            return Ok(());
        }
        self.dots += 1;
        let text = ui::waiting(tr().intro_ui_access_waiting, self.dots);
        self.notes[Note::UiAccess as usize] = (text.clone(), false);
        self.show_note(Note::UiAccess, &text, false, false)
    }

    /// Shows `values` changed from elsewhere, cross-fading to them.
    pub fn refresh(&mut self, theme: Theme, values: Values) -> Result<()> {
        (self.values, self.recording) = (values, None);
        if self.email.text().trim() != self.values.config.mymemory_email {
            self.email = LineEdit::new(&self.values.config.mymemory_email, MAX_EMAIL_CHARS);
        }
        self.notes = NOTES.map(|note| (note.usual(&self.values), false));
        self.retitle();
        self.theme = theme;
        let (anim, renderer, s) = (&self.anim, &self.renderer, self.scale);
        self.frame.fade_theme(anim, renderer, &theme, s, THEME_MS)?;
        self.replace_view(true)
    }

    /// The window moved to a monitor with another `dpi`. `suggested` is
    /// where Windows would put it, in screen pixels.
    pub fn rescale(&mut self, dpi: u32, suggested: RECT) -> Result<()> {
        let centre = POINT {
            x: (suggested.left + suggested.right) / 2,
            y: (suggested.top + suggested.bottom) / 2,
        };
        let work = ui::monitor_at(centre).0;
        let s = zoomed(work, dpi as f32 / 96.0, self.values.config.zoom_factor());
        if (s - self.scale).abs() < 0.01 {
            return Ok(());
        }
        self.redraw_at(s, work, |_, _| (suggested.left, suggested.top))
    }

    /// Draws the window `zoom` percent as large as Windows has it, its
    /// setting showing that. It stays where it is, as far as it fits.
    pub fn set_zoom(&mut self, zoom: u32) -> Result<()> {
        // Stepped here, the number has rolled to it already.
        let (up, changed) = (
            zoom > self.values.config.zoom,
            zoom != self.values.config.zoom,
        );
        self.values.config.zoom = zoom;
        let mut r = RECT::default();
        unsafe { GetWindowRect(self.hwnd, &mut r)? };
        let centre = POINT {
            x: (r.left + r.right) / 2,
            y: (r.top + r.bottom) / 2,
        };
        let (work, dpi_scale) = ui::monitor_at(centre);
        let s = zoomed(work, dpi_scale, self.values.config.zoom_factor());
        if (s - self.scale).abs() < 0.01 {
            return match changed {
                true => self.show_count(Counter::Zoom, Some(up)),
                false => Ok(()),
            };
        }
        self.redraw_at(s, work, |w, h| {
            let m = (MARGIN * s).round() as i32;
            let gap = (SCREEN_GAP * s).round() as i32;
            let left = (centre.x - w / 2)
                .min(work.right - gap - w + m)
                .max(work.left + gap - m);
            let top = r.top.min(work.bottom - gap - h + m).max(work.top + gap - m);
            (left, top)
        })
    }

    /// Draws the window afresh at `s` pixels per DIP, keeping the place
    /// scrolled to, in the `work` area, and moves it where `at` says for
    /// its new width and height.
    fn redraw_at(
        &mut self,
        s: f32,
        work: RECT,
        at: impl FnOnce(i32, i32) -> (i32, i32),
    ) -> Result<()> {
        let scroll = self.scroll / self.scale * s;
        (self.scale, self.scroll) = (s, 0.0);
        self.scroll_anim = None;
        self.frame.apply_theme(&self.renderer, &self.theme, s)?;
        self.replace_view(false)?;
        self.fit(work)?;
        self.scroll_to(scroll)?;
        let m = (MARGIN * s).round();
        let (w, h) = ((self.card.0 + 2.0 * m) as i32, (self.room + 2.0 * m) as i32);
        let (left, top) = at(w, h);
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                None,
                left,
                top,
                w,
                h,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        self.set_region(self.card.1);
        let center = Vector3 {
            X: m + self.card.0 / 2.0,
            Y: m + self.card.1 / 2.0,
            Z: 0.0,
        };
        self.root.SetCenterPoint(center)
    }

    // -----------------------------------------------------------------------
    // What clicks and keys do

    fn activate(&mut self, hit: Hit) -> Result<Option<Change>> {
        Ok(match hit {
            Hit::Close => Some(Change::Close),
            Hit::Config => Some(Change::OpenConfig),
            Hit::Shortcut(shortcut) if self.recording == Some(shortcut) => {
                self.stop_recording()?;
                Some(Change::RecordingStopped)
            }
            Hit::Shortcut(shortcut) => {
                // Recording the other shortcut stops, and this one starts.
                self.stop_recording()?;
                self.start_recording(shortcut)?;
                Some(Change::RecordingStarted)
            }
            // It waits for Windows to ask, and can't be turned again until
            // it has.
            Hit::Switch(Switch::UiAccess) if self.waiting => None,
            Hit::Switch(Switch::UiAccess) => {
                let on = !self.switch(Switch::UiAccess);
                (self.values.ui_access, self.waiting, self.dots) = (Some(on), true, 0);
                self.show_switch(Switch::UiAccess, true)?;
                let text = ui::waiting(tr().intro_ui_access_waiting, 0);
                self.set_note(Note::UiAccess, &text, false)?;
                win::set_timer(self.hwnd, TIMER_DOTS, ui::DOTS_MS);
                Some(Change::UiAccess(on))
            }
            Hit::Switch(sw) => {
                let on = !self.switch(sw);
                let v = &mut self.values;
                let change = match sw {
                    Switch::Autostart => {
                        v.autostart = on;
                        Change::Autostart(on)
                    }
                    Switch::Online => {
                        v.config.online_lookup = on;
                        Change::OnlineLookup(on)
                    }
                    Switch::Examples => {
                        v.config.show_examples = on;
                        Change::ShowExamples(on)
                    }
                    Switch::Animations => {
                        v.config.animations = Some(on);
                        Change::Animations(on)
                    }
                    Switch::Updates => {
                        v.config.check_updates = on;
                        Change::CheckUpdates(on)
                    }
                    // Turned above.
                    Switch::UiAccess => return Ok(None),
                };
                self.show_switch(sw, true)?;
                Some(change)
            }
            Hit::Choice(choice, i) => self.choose(choice, i)?,
            Hit::Step(counter, up) => self.step(counter, up)?,
            Hit::Fold(fold) => {
                self.set_open(fold, !self.open[fold as usize])?;
                None
            }
            Hit::Language(i) => self.tick(i)?,
            Hit::Select(side) => {
                self.open_picker(side)?;
                None
            }
            Hit::Swap => self.swap_languages()?,
            Hit::Pick(i) => {
                let code = self
                    .picking
                    .as_ref()
                    .and_then(|p| p.matches.get(i).copied());
                match code {
                    Some(code) => self.pick(code)?,
                    None => None,
                }
            }
            Hit::Email | Hit::Info => None,
        })
    }

    /// Opens or shuts `fold`'s list, the card growing or shrinking with it.
    fn set_open(&mut self, fold: Fold, open: bool) -> Result<()> {
        if open == self.open[fold as usize] {
            return Ok(());
        }
        self.open[fold as usize] = open;
        if !open && self.focus.and_then(Stop::folded) == Some(fold) {
            self.focus = Some(Stop::Fold(fold));
        }
        if !open && self.hovered.and_then(Hit::folded) == Some(fold) {
            self.set_hovered(None)?;
        }
        self.show_open(true)?;
        self.refit()?;
        if open {
            if let Some(view) = &self.view {
                let v = &view.folds[fold as usize];
                let row_h = (ROW_H * self.scale).round();
                let all = Rect {
                    x: 0.0,
                    y: v.list_y - row_h,
                    w: self.card.0,
                    h: v.list_h + row_h,
                };
                if let Some(all) = view.place(all, Some(fold), &self.open) {
                    self.scroll_into_view(all)?;
                }
            }
        }
        self.show_focus(true)
    }

    /// Limits the window's clickable area to the top `card_h` pixels of the
    /// card, and its shadow, so the transparent rest swallows no clicks.
    fn set_region(&self, card_h: f32) {
        let m = (MARGIN * self.scale).round();
        let (w, h) = (self.card.0 + 2.0 * m, card_h + 2.0 * m);
        let region = RECT {
            left: 0,
            top: 0,
            right: w as i32,
            bottom: h.ceil() as i32,
        };
        win::set_region(self.hwnd, region, self.visible);
    }

    /// Sizes the card to what's in it now, gliding there. A window that
    /// grows does at once, moving up if it has to to stay on the screen;
    /// one that shrinks waits for the card to ([`Settings::settle`]).
    fn refit(&mut self) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let s = self.scale;
        let h = (view.header_h + view.height(&self.open))
            .min(self.room)
            .ceil();
        let old = self.card.1;
        if h == old {
            return Ok(());
        }
        self.card.1 = h;
        let m = (MARGIN * s).round();
        self.frame
            .resize(&self.anim, self.card.0, h, v3(m, m), s, true)?;
        self.root.SetCenterPoint(Vector3 {
            X: m + self.card.0 / 2.0,
            Y: m + h / 2.0,
            Z: 0.0,
        })?;
        if h > old {
            view.body.SetSize(v2(self.card.0, h - view.header_h))?;
        }
        // The window is already as tall as the card gets: only what is
        // clickable follows, once a smaller card has finished shrinking.
        self.set_region(old.max(h));
        win::set_timer(self.hwnd, TIMER_SETTLE, SETTLE_MS);
        let scroll = self.scroll.min(self.max_scroll());
        self.scroll_to(scroll)?;
        self.show_scroll(true)
    }

    // -----------------------------------------------------------------------
    // Pointer and keyboard focus

    /// A point in window pixels, relative to the card's top left.
    fn card_point(&self, x: i32, y: i32) -> (f32, f32) {
        let m = (MARGIN * self.scale).round();
        (x as f32 - m, y as f32 - m)
    }

    fn hit_at(&self, x: i32, y: i32) -> Option<Hit> {
        let view = self.view.as_ref()?;
        let in_boxes = self.boxes_point(x, y);
        let (x, y) = self.card_point(x, y);
        if x < 0.0 || x >= self.card.0 || y < 0.0 || y >= self.card.1 {
            return None;
        }
        if view.close_at.contains(x, y) {
            return Some(Hit::Close);
        }
        if y < view.header_h {
            return None;
        }
        if let Some((bx, by)) = in_boxes.filter(|&(bx, by)| self.list.contains(bx, by)) {
            return self.list.row_at(bx, by).map(Hit::Pick);
        }
        let y = y - view.header_h + self.scroll;
        view.hits(&self.open)
            .find(|(_, r)| r.contains(x, y))
            .map(|(hit, _)| hit)
    }

    /// A point in window pixels in the default languages' boxes, in pixels
    /// from their top left, if it's on the part of the window that scrolls.
    fn boxes_point(&self, x: i32, y: i32) -> Option<(f32, f32)> {
        let view = self.view.as_ref()?;
        let (x, y) = self.card_point(x, y);
        if x < 0.0 || x >= self.card.0 || y < view.header_h || y >= self.card.1 {
            return None;
        }
        let at = view.boxes.at + view.boxes.group_at;
        Some((x - at.X, y - view.header_h + self.scroll - at.Y))
    }

    fn set_hovered(&mut self, hit: Option<Hit>) -> Result<()> {
        if hit == self.hovered {
            return Ok(());
        }
        let old = std::mem::replace(&mut self.hovered, hit);
        if self.has_tip(old) || self.has_tip(hit) {
            self.show_tip(hit)?;
        }
        self.show_hover(old)?;
        self.show_hover(hit)?;
        // Over the list of languages, the highlight moves along.
        if let Some(Hit::Pick(i)) = hit {
            let moved = self.picking.as_mut().and_then(|p| {
                let code = p.matches.get(i).copied()?;
                p.highlight(code).then_some(p.highlight)
            });
            if let Some(row) = moved {
                self.list.set_highlight(&self.anim, row)?;
            }
        }
        Ok(())
    }

    fn set_pressed(&mut self, hit: Option<Hit>) -> Result<()> {
        if hit == self.pressed {
            return Ok(());
        }
        let old = std::mem::replace(&mut self.pressed, hit);
        self.show_pressed(old)?;
        self.show_pressed(hit)
    }

    /// Moves the focus ring to the next (or previous) control, going round.
    fn step_focus(&mut self, forward: bool) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let stops: Vec<(Stop, Rect)> = view.stops(&self.open).map(|(s, r, _)| (s, r)).collect();
        let n = stops.len();
        let at = self
            .focus
            .and_then(|f| stops.iter().position(|&(s, _)| s == f));
        // The first Tab shows where the focus is, if the pointer put it somewhere.
        let next = match (at, forward, self.ring) {
            (Some(i), _, false) => i,
            (None, true, _) => 0,
            (None, false, _) => n - 1,
            (Some(i), true, true) => (i + 1) % n,
            (Some(i), false, true) => (i + n - 1) % n,
        };
        let (stop, rect) = stops[next];
        (self.focus, self.ring) = (Some(stop), true);
        self.show_focus(true)?;
        self.scroll_into_view(rect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_come_last() {
        // Only what's below its group moves as a fold's list opens.
        let folds: Vec<Fold> = SECTIONS
            .iter()
            .flat_map(|(_, rows)| rows.iter())
            .filter_map(|row| match row.control {
                Control::Fold(fold) => Some(fold),
                _ => None,
            })
            .collect();
        assert_eq!(folds, FOLDS);
        for (_, rows) in SECTIONS {
            let last = rows.len() - 1;
            assert!(rows[..last]
                .iter()
                .all(|row| !matches!(row.control, Control::Fold(_))));
        }
        let names = |l: &[&str]| summary(&l.iter().map(|&l| l.into()).collect::<Vec<_>>());
        assert_eq!(names(&["it", "en"]), "English, Italiano");
        assert_eq!(names(&["it", "xx"]), "Italiano");
    }

    #[test]
    fn rows_fit_the_card() {
        let rows = SECTIONS.iter().flat_map(|(_, rows)| rows.iter());
        for row in rows.chain(&SHORTCUT_ROWS) {
            // Room for the name and description beside each control.
            let room = GROUP_W - ROW_PAD - TEXT_X - row.control.width() - CONTROL_GAP;
            assert!(room >= 120.0, "{}", row.title());
        }
    }

    #[test]
    fn reads_shortcuts_it_cannot_parse() {
        assert_eq!(key_names("ctrl+alt+d"), ["Ctrl", "Alt", "D"]);
        assert_eq!(key_names("Ctrl+Foo"), ["Ctrl", "Foo"]);
    }

    #[test]
    fn counts_within_range() {
        for counter in COUNTERS {
            let (lo, hi) = counter.range();
            assert!(lo < hi);
        }
        assert_eq!(Counter::Synonyms.text(0), "Off");
        assert_eq!(Counter::Definitions.text(4), "4");
        assert_eq!(Counter::Definitions.text(1), "1");
        assert_eq!(Counter::Zoom.text(0), "80%");
        assert_eq!(Counter::Zoom.text(config::zoom_index(100)), "100%");
    }
}
