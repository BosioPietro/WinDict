//! The settings window: a card in the style of the definition card, with
//! the settings grouped on it the way Windows 11 groups its own. Like the
//! card it's a composition visual tree, so all its motion runs on the system
//! compositor: the card rises in with its parts a beat after one another,
//! switches slide, the chosen theme's pill glides over, numbers roll, a new
//! shortcut's keys rise into place, the list of languages opens out of its
//! row as the card grows to fit it, and the focus ring follows Tab from
//! control to control. Changing the theme cross-fades the whole window.
//! The one text field, for the MyMemory email, is drawn the same way, its
//! caret and all.

mod view;

use std::time::Instant;

use windows::core::{Result, HSTRING};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{CreateRectRgn, DeleteObject, SetWindowRgn};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::*;
use windows_numerics::{Vector2, Vector3};

use crate::app::edit::{Input, LineEdit, Motion};
use crate::app::picking::Picking;
use crate::config::{Config, ThemeMode};
use crate::dicts;
use crate::hotkey::{self, held_modifiers, is_modifier, Hotkey};
use crate::popup;
use crate::render::{Align, Face, Ink, Layout, Picker, Renderer, Side, Target, PICKER_ROWS};
use crate::selection;
use crate::theme::Theme;
use crate::ui::{
    self, cubic_bezier, scale, ts, v2, v3, Animator, Frame, Text, MARGIN, SCROLL_EASE,
};
use crate::win::Window;

// Geometry, in DIPs.
const CARD_W: f32 = 460.0;
/// Where the heading, the section labels and the footer start.
const LABEL_X: f32 = popup::PAD_X;
const PAD_TOP: f32 = 18.0;
const HEADER_GAP: f32 = 12.0;
const CLOSE_SIZE: f32 = 32.0;
/// The close button's distance from the card's top and right edges.
const CLOSE_INSET: f32 = 10.0;
const CLOSE_RADIUS: f32 = 5.0;
const GROUP_X: f32 = 16.0;
const GROUP_W: f32 = CARD_W - 2.0 * GROUP_X;
const GROUP_RADIUS: f32 = 8.0;
const ROW_H: f32 = 60.0;
const ROW_PAD: f32 = 16.0;
/// Between a setting's name and its description.
const DETAIL_GAP: f32 = 1.0;
/// Between a setting's description and its control.
const CONTROL_GAP: f32 = 12.0;
const SECTION_GAP: f32 = 20.0;
const LABEL_GAP: f32 = 8.0;
const FOOTER_GAP: f32 = 16.0;
const PAD_BOTTOM: f32 = 18.0;
const SCREEN_GAP: f32 = 16.0;
/// The tallest the card grows before it scrolls.
const MAX_CARD_H: f32 = 580.0;
const SWITCH_W: f32 = 40.0;
const SWITCH_H: f32 = 20.0;
const KNOB: f32 = 12.0;
/// The knob grows under the pointer, and stretches while pressed.
const KNOB_HOVER: f32 = 14.0;
const KNOB_PRESSED: f32 = 17.0;
const SEGMENT_W: f32 = 70.0;
const SEGMENT_H: f32 = 30.0;
const SEGMENT_RADIUS: f32 = 6.0;
const PILL_INSET: f32 = 3.0;
const PILL_RADIUS: f32 = 4.0;
const STEP_SIZE: f32 = 30.0;
const STEP_RADIUS: f32 = 6.0;
const VALUE_W: f32 = 40.0;
/// The text field.
const FIELD_W: f32 = 190.0;
const FIELD_H: f32 = 32.0;
const FIELD_RADIUS: f32 = 4.0;
/// Between the field's sides and its text.
const FIELD_PAD: f32 = 10.0;
/// The line along its bottom, which thickens into the accent while typing.
const FIELD_LINE: f32 = 2.0;
const CARET_W: f32 = 1.0;
/// Between a setting's name and the info symbol after it.
const INFO_GAP: f32 = 6.0;
const INFO_W: f32 = 16.0;
/// How wide the info tooltip grows before wrapping.
const TIP_W: f32 = 300.0;
const TIP_PAD: f32 = 10.0;
const TIP_RADIUS: f32 = 6.0;
/// How far the tooltip's shadow spreads, and how far it drops.
const TIP_SHADOW: f32 = 12.0;
const TIP_SHADOW_DY: f32 = 3.0;
/// Between the info symbol and its tooltip.
const TIP_GAP: f32 = 6.0;
/// How far the tooltip rises from as it appears.
const TIP_DRIFT: f32 = 4.0;
/// The default languages' row: the room for its name and description above
/// the boxes, and under them.
const TRANSLATE_PAD: f32 = 12.0;
const TRANSLATE_HEAD: f32 = 64.0;
const TRANSLATE_FOOT: f32 = 16.0;
/// Corner radii of the rings round the boxes and the swap button.
const SELECT_RING_RADIUS: f32 = 8.0;
const SWAP_RING_RADIUS: f32 = 6.0;
/// How many languages a notch of the mouse wheel scrolls their list by.
const WHEEL_ROWS: f32 = 3.0;
/// A language in the list under the languages row.
const LANGUAGE_H: f32 = 40.0;
const CHECK_SIZE: f32 = 20.0;
const CHECK_RADIUS: f32 = 4.0;
/// Between a language's box and its name.
const CHECK_GAP: f32 = 12.0;
/// The room for how a language's download is going, on its right.
const STATUS_W: f32 = 260.0;
const CHEVRON_W: f32 = 16.0;
/// How wide each control is, for the description beside it to stay clear.
const SHORTCUT_W: f32 = 150.0;
const EDIT_W: f32 = 16.0;
const CAPS_GAP: f32 = 8.0;
const FOCUS_OUTSET: f32 = 3.0;
const FOCUS_STROKE: f32 = 2.0;
/// Room kept above and below a control Tab scrolls into view.
const FOCUS_MARGIN: f32 = 12.0;
/// The ring round the shortcut while it's being recorded.
const RECORD_INSET: f32 = 3.0;
const RECORD_RADIUS: f32 = 6.0;
const RECORD_STROKE: f32 = 1.5;
/// How far parts rise from as they appear.
const DRIFT: f32 = 8.0;
/// How far a number rolls as it changes.
const ROLL: f32 = 14.0;
const WHEEL_STEP: f32 = 48.0;

// Durations, in ms.
const SHOW_MS: u64 = 360;
const FADE_IN_MS: u64 = 160;
const REVEAL_MS: u64 = 340;
const REVEAL_DELAY_MS: u64 = 40;
const REVEAL_STAGGER_MS: u64 = 30;
const HOVER_MS: u64 = 100;
const KNOB_MS: u64 = 300;
const SWITCH_FILL_MS: u64 = 160;
const PILL_MS: u64 = 340;
const LABEL_MS: u64 = 160;
const ROLL_MS: u64 = 260;
const CAPS_MS: u64 = 280;
const DETAIL_MS: u64 = 200;
const FOCUS_MS: u64 = 200;
const PRESS_MS: u64 = 90;
const SCROLL_MS: u64 = 180;
/// The pointer rests on the info symbol this long before its tooltip shows.
const TIP_DELAY_MS: u64 = 400;
const TIP_MS: u64 = 160;
const TIP_HIDE_MS: u64 = 100;
const THEME_MS: u64 = 300;
/// How long the old view takes to fade away as the languages change.
const SLIDE_FADE_MS: u64 = 180;
/// As long as the card takes to grow or shrink.
const EXPAND_MS: u64 = ui::RESIZE_MS;
/// Once the card has shrunk, the window does.
const SETTLE_MS: u32 = ui::RESIZE_MS as u32 + 60;
const TIMER_SETTLE: usize = 1;
const BREATHE_MS: u64 = 1600;

// Segoe Fluent Icons and Segoe MDL2 Assets.
const CLOSE_GLYPH: &str = "\u{E8BB}";
const EDIT_GLYPH: &str = "\u{E70F}";
const MINUS_GLYPH: &str = "\u{E738}";
const PLUS_GLYPH: &str = "\u{E710}";
const CHEVRON_GLYPH: &str = "\u{E70D}";
const CHECK_GLYPH: &str = "\u{E73E}";
const INFO_GLYPH: &str = "\u{E946}";

const SHORTCUT_DETAIL: &str = "Looks up the selected word";
const RECORDING_DETAIL: &str = "Press the new shortcut, or Esc to cancel";
const RECORDING_HINT: &str = "Press keys";
const NEEDS_MODIFIER: &str = "Hold Ctrl, Alt or Win with the key";
const UNUSABLE_KEY: &str = "That key can\u{2019}t be part of a shortcut";
pub const SHORTCUT_TAKEN: &str = "Another app already uses that shortcut";
const CONFIG_LINK: &str = "Open config.json";
const EMAIL_PLACEHOLDER: &str = "name@example.com";
/// Over the info symbol by the email field.
const EMAIL_TIP: &str = "MyMemory, the translator used when Google Translate can\u{2019}t \
    be reached, allows 5,000 characters a day, or 50,000 with an email address. \
    It\u{2019}s sent only to MyMemory, with the words translated.";
/// The longest an email address can be.
const MAX_EMAIL_CHARS: usize = 254;

/// What the settings window shows: the settings, and whether WinDict
/// starts with Windows, which is kept in the registry instead.
#[derive(Clone)]
pub struct Values {
    pub config: Config,
    pub autostart: bool,
}

/// Something done in the window, for the app to carry out.
#[derive(Debug, PartialEq)]
pub enum Change {
    /// Recording a new shortcut started: the current one should stop
    /// working meanwhile.
    RecordingStarted,
    /// Recording ended without a new shortcut.
    RecordingStopped,
    Hotkey(String),
    Theme(ThemeMode),
    /// A language's code, like "en", turned on or off.
    Language(String, bool),
    MaxDefinitions(usize),
    MaxSynonyms(usize),
    ShowExamples(bool),
    OnlineLookup(bool),
    UiAutomation(bool),
    Autostart(bool),
    /// The languages translated from and into when the card can't tell,
    /// as codes.
    TranslateLanguages(String, String),
    MyMemoryEmail(String),
    OpenConfig,
    Close,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Switch {
    Autostart,
    Online,
    Automation,
    Examples,
}

const SWITCHES: [Switch; 4] = [
    Switch::Autostart,
    Switch::Online,
    Switch::Automation,
    Switch::Examples,
];

/// A number stepped through with − and + buttons.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Counter {
    Definitions,
    Synonyms,
}

const COUNTERS: [Counter; 2] = [Counter::Definitions, Counter::Synonyms];

impl Counter {
    /// The lowest and highest values.
    fn range(self) -> (usize, usize) {
        match self {
            Counter::Definitions => (1, 10),
            Counter::Synonyms => (0, 12),
        }
    }

    fn text(self, n: usize) -> String {
        match n {
            0 => "Off".into(),
            _ => n.to_string(),
        }
    }

    fn width(self) -> f32 {
        2.0 * STEP_SIZE + VALUE_W
    }
}

const THEMES: [(ThemeMode, &str); 3] = [
    (ThemeMode::System, "System"),
    (ThemeMode::Light, "Light"),
    (ThemeMode::Dark, "Dark"),
];

/// A setting chosen from a few options, side by side.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Choice {
    Theme,
}

const CHOICES: [Choice; 1] = [Choice::Theme];

impl Choice {
    fn names(self) -> Vec<&'static str> {
        match self {
            Choice::Theme => THEMES.iter().map(|(_, name)| *name).collect(),
        }
    }
}

/// The names of `languages`, as the languages row sums them up: "English,
/// Italiano".
fn summary(languages: &[String]) -> String {
    let names: Vec<_> = dicts::languages()
        .filter(|(code, _)| languages.iter().any(|l| l == code))
        .map(|(_, name)| name)
        .collect();
    names.join(", ")
}

/// A setting's description that can change to say what's going on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Note {
    Shortcut,
    Language,
}

const NOTES: [Note; 2] = [Note::Shortcut, Note::Language];

impl Note {
    fn usual(self, values: &Values) -> String {
        match self {
            Note::Shortcut => SHORTCUT_DETAIL.into(),
            Note::Language => summary(&values.config.languages),
        }
    }
}

/// What's under the pointer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Hit {
    Close,
    Shortcut,
    Switch(Switch),
    /// One of a choice's options.
    Choice(Choice, usize),
    /// A counter's down (false) or up (true) button.
    Step(Counter, bool),
    /// The languages row, which opens and shuts the list under it.
    Languages,
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
    Shortcut,
    Switch(Switch),
    Choice(Choice),
    Counter(Counter),
    Email,
    Select(Side),
    Swap,
    Languages,
    Language(usize),
    Config,
}

impl Hit {
    fn stop(self) -> Option<Stop> {
        match self {
            Hit::Close => None,
            Hit::Shortcut => Some(Stop::Shortcut),
            Hit::Switch(sw) => Some(Stop::Switch(sw)),
            Hit::Choice(choice, _) => Some(Stop::Choice(choice)),
            Hit::Step(counter, _) => Some(Stop::Counter(counter)),
            Hit::Languages => Some(Stop::Languages),
            Hit::Language(i) => Some(Stop::Language(i)),
            Hit::Email => Some(Stop::Email),
            Hit::Info | Hit::Pick(_) => None,
            Hit::Select(side) => Some(Stop::Select(side)),
            Hit::Swap => Some(Stop::Swap),
            Hit::Config => Some(Stop::Config),
        }
    }
}

#[derive(Clone, Copy)]
enum Control {
    Shortcut,
    Switch(Switch),
    Choice(Choice),
    Counter(Counter),
    /// Opens the list of languages under it. It's the last row, so that
    /// only the footer moves as the list opens.
    Languages,
    /// The field for the MyMemory email, with an info symbol after the
    /// setting's name.
    Email,
    /// The default languages' boxes, with the list under the one being
    /// picked. It's taller than the other rows, and as tall as the list
    /// when that's open.
    Translate,
}

impl Control {
    fn width(self) -> f32 {
        match self {
            Control::Shortcut => SHORTCUT_W,
            Control::Switch(_) => SWITCH_W,
            Control::Choice(choice) => choice.names().len() as f32 * SEGMENT_W,
            Control::Counter(counter) => counter.width(),
            Control::Languages => CHEVRON_W,
            Control::Email => FIELD_W,
            Control::Translate => 0.0,
        }
    }
}

struct Row {
    title: &'static str,
    detail: &'static str,
    control: Control,
}

const fn row(title: &'static str, detail: &'static str, control: Control) -> Row {
    Row {
        title,
        detail,
        control,
    }
}

const SECTIONS: &[(&str, &[Row])] = &[
    (
        "General",
        &[
            row("Shortcut", SHORTCUT_DETAIL, Control::Shortcut),
            row(
                "Start with Windows",
                "Runs WinDict when you sign in",
                Control::Switch(Switch::Autostart),
            ),
        ],
    ),
    (
        "Card",
        &[
            row(
                "Theme",
                "Card and menu colors",
                Control::Choice(Choice::Theme),
            ),
            row(
                "Definitions",
                "Senses shown per part of speech",
                Control::Counter(Counter::Definitions),
            ),
            row(
                "Synonyms",
                "Similar words under each part of speech",
                Control::Counter(Counter::Synonyms),
            ),
            row(
                "Examples",
                "Sentences showing each sense in use",
                Control::Switch(Switch::Examples),
            ),
        ],
    ),
    (
        "Translation",
        &[
            row(
                "Default languages",
                "Translated from and into when the card can\u{2019}t tell",
                Control::Translate,
            ),
            row(
                "MyMemory email",
                "Raises the daily translation limit",
                Control::Email,
            ),
        ],
    ),
    (
        "Lookup",
        &[
            row(
                "Online dictionary",
                "For pronunciations, and words the built-in one lacks",
                Control::Switch(Switch::Online),
            ),
            row(
                "Read selections directly",
                "Leaves the clipboard alone in apps that allow it",
                Control::Switch(Switch::Automation),
            ),
            // Its description sums up the languages chosen.
            row("Languages", "English", Control::Languages),
        ],
    ),
];

/// A rectangle, in pixels.
#[derive(Clone, Copy, Default, Debug)]
struct Rect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl Rect {
    fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }

    fn outset(&self, d: f32) -> Rect {
        Rect {
            x: self.x - d,
            y: self.y - d,
            w: self.w + 2.0 * d,
            h: self.h + 2.0 * d,
        }
    }

    fn offset(&self, by: Vector3) -> Rect {
        Rect {
            x: self.x + by.X,
            y: self.y + by.Y,
            ..*self
        }
    }

    fn at(&self) -> Vector3 {
        v3(self.x, self.y)
    }

    fn size(&self) -> Vector2 {
        v2(self.w, self.h)
    }
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

/// The info tooltip.
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

/// The languages row, and the list that opens under it, in its group.
struct LanguagesView {
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
    /// Where the list starts in the scroller, and how tall it is.
    list_y: f32,
    list_h: f32,
    /// In the order of [`dicts::languages`].
    items: Vec<LanguageView>,
    /// What comes after, which moves down as the list opens, and where it
    /// rests with the list shut.
    footer: ContainerVisual,
    footer_at: Vector3,
    /// How much shorter the languages row is than with the list full, and
    /// where, with it full, what it moves up starts.
    slack: f32,
    slack_y: f32,
}

impl LanguagesView {
    /// Where `rect`, in the scroller with the list shut, is with it as
    /// `open` says, or nothing if it's in the `listed` and that's shut.
    /// What's in the list is where it is with the list open.
    fn place(&self, rect: Rect, listed: bool, open: bool) -> Option<Rect> {
        if listed && !open {
            return None;
        }
        let mut y = rect.y;
        if !listed && open && rect.y >= self.list_y {
            y += self.list_h;
        }
        if rect.y >= self.slack_y {
            y -= self.slack;
        }
        Some(Rect { y, ..rect })
    }
}

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
    scroller: ContainerVisual,
    divider: SpriteVisual,
    thumb: SpriteVisual,
    thumb_clip: CompositionRoundedRectangleGeometry,
    tip: TipView,
    /// What rises in as the window opens, and where each rests.
    reveal: Vec<(Visual, Vector3)>,
    shortcut: ShortcutView,
    /// In the order of [`SWITCHES`] and [`COUNTERS`].
    switches: Vec<SwitchView>,
    field: FieldView,
    /// In the order of [`CHOICES`] and [`NOTES`].
    choices: Vec<ChoiceView>,
    notes: Vec<NoteView>,
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
    ring: ShapeVisual,
    ring_geometry: CompositionRoundedRectangleGeometry,
    languages: LanguagesView,
}

impl View {
    /// The height of everything below the header with the list as `open` says.
    fn height(&self, open: bool) -> f32 {
        self.content_h + if open { self.languages.list_h } else { 0.0 } - self.languages.slack
    }

    /// Where things can be clicked, with the list as `open` says.
    fn hits(&self, open: bool) -> impl Iterator<Item = (Hit, Rect)> + '_ {
        self.hits.iter().filter_map(move |&(hit, rect)| {
            let listed = matches!(hit, Hit::Language(_));
            Some((hit, self.languages.place(rect, listed, open)?))
        })
    }

    /// Where Tab stops, with the list as `open` says.
    fn stops(&self, open: bool) -> impl Iterator<Item = (Stop, Rect, f32)> + '_ {
        self.stops.iter().filter_map(move |&(stop, rect, radius)| {
            let listed = matches!(stop, Stop::Language(_));
            Some((stop, self.languages.place(rect, listed, open)?, radius))
        })
    }
}

/// The default languages' boxes in the view, and where they sit.
struct BoxesView {
    text: Text,
    caret: Option<SpriteVisual>,
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
    recording: bool,
    /// The modifiers held while recording.
    preview: HOT_KEY_MODIFIERS,
    /// What the descriptions that change say, and whether it's that
    /// something went wrong, in the order of [`NOTES`].
    notes: [(String, bool); 2],
    /// Whether the list of languages is open.
    open: bool,
    /// How each language's download is going, and whether that's that it
    /// failed, in the order of [`dicts::languages`].
    downloads: Vec<Option<(String, bool)>>,
    /// The choice whose pill is gliding over, and the option it's leaving.
    pill_from: Option<(Choice, usize)>,
    /// The list a default language is being picked from, while it's open.
    picking: Option<Picking>,
    /// What's typed in the email field.
    email: LineEdit,
    /// How far the field's text is scrolled left, in pixels.
    email_shift: f32,
    /// Whether the window is the active one, for the caret to show.
    active: bool,
    scroll: f32,
    /// Where the running scroll started, and when.
    scroll_anim: Option<(f32, Instant)>,
    pub visible: bool,
    /// Last, so it goes after everything shown on it.
    /// The tallest the card gets, in pixels. The window is made for that,
    /// so that it never resizes under the blurred background.
    room: f32,
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
            recording: false,
            preview: HOT_KEY_MODIFIERS(0),
            notes,
            open: false,
            downloads: dicts::languages().map(|_| None).collect(),
            pill_from: None,
            picking: None,
            email,
            email_shift: 0.0,
            active: true,
            scroll: 0.0,
            scroll_anim: None,
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
        (self.theme, self.scale, self.values) = (theme, s, values);
        (self.hovered, self.pressed, self.focus) = (None, None, None);
        (self.ring, self.recording, self.scroll) = (false, false, 0.0);
        self.scroll_anim = None;
        self.notes = NOTES.map(|note| (note.usual(&self.values), false));
        (self.open, self.picking) = (false, None);
        self.downloads.fill(None);
        self.email = LineEdit::new(&self.values.config.mymemory_email, MAX_EMAIL_CHARS);
        (self.email_shift, self.active) = (0.0, true);
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_SETTLE);
        }
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

        self.anim.fade_in(root, FADE_IN_MS, SHOW_MS)?;
        self.reveal()
    }

    /// Fades the window out. The caller hides it after [`ui::HIDE_MS`].
    pub fn begin_hide(&mut self) -> Result<()> {
        if !self.visible {
            return Ok(());
        }
        self.visible = false;
        self.recording = false;
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
        self.recording
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
        let hit = point.and_then(|(x, y)| self.hit_at(x, y));
        self.set_hovered(hit)
    }

    /// The left button went down at a point in window pixels.
    pub fn press(&mut self, x: i32, y: i32) -> Result<Option<Change>> {
        let hit = self.hit_at(x, y);
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
        let away = !matches!(
            hit,
            Some(Hit::Select(_) | Hit::Pick(_) | Hit::Swap)
        );
        if away {
            if let Some(side) = self.picking.take().map(|p| p.side) {
                self.focus = hit.and_then(Hit::stop).or(Some(Stop::Select(side)));
                self.relayout()?;
            }
        }
        self.show_focus(false)?;
        self.set_pressed(hit)?;
        if self.recording && hit != Some(Hit::Shortcut) {
            self.stop_recording()?;
            return Ok(Some(Change::RecordingStopped));
        }
        Ok(None)
    }

    /// The left button came up at a point in window pixels: a click, if it's
    /// on what it went down on.
    pub fn release(&mut self, x: i32, y: i32) -> Result<Option<Change>> {
        let pressed = self.pressed;
        self.set_pressed(None)?;
        match pressed {
            Some(hit) if self.hit_at(x, y) == Some(hit) => self.activate(hit),
            _ => Ok(None),
        }
    }

    /// A key went down.
    pub fn key(&mut self, key: VIRTUAL_KEY) -> Result<Option<Change>> {
        if self.recording {
            return self.record(key);
        }
        if self.picking.is_some() {
            return self.picker_key(key);
        }
        let shift = hotkey::is_held(VK_SHIFT);
        let ctrl = hotkey::is_held(VK_CONTROL);
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
            (VK_SPACE | VK_RETURN, Some(Stop::Shortcut)) => self.activate(Hit::Shortcut)?,
            (VK_SPACE | VK_RETURN, Some(Stop::Switch(sw))) => self.activate(Hit::Switch(sw))?,
            (VK_SPACE | VK_RETURN, Some(Stop::Config)) => self.activate(Hit::Config)?,
            (VK_SPACE | VK_RETURN, Some(Stop::Select(side))) => self.activate(Hit::Select(side))?,
            (VK_SPACE | VK_RETURN, Some(Stop::Swap)) => self.activate(Hit::Swap)?,
            (VK_SPACE | VK_RETURN, Some(Stop::Languages)) => self.activate(Hit::Languages)?,
            (VK_RIGHT, Some(Stop::Languages)) => {
                self.set_open(true)?;
                None
            }
            (VK_LEFT, Some(Stop::Languages | Stop::Language(_))) => {
                self.focus = Some(Stop::Languages);
                self.set_open(false)?;
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
        if self.recording {
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
        if self.recording && is_modifier(key) {
            self.set_preview(held_modifiers())?;
        }
        Ok(())
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
        if !self.recording {
            return Ok(None);
        }
        self.stop_recording()?;
        Ok(Some(Change::RecordingStopped))
    }

    /// Scrolls by a mouse wheel delta.
    pub fn wheel(&mut self, delta: i32) -> Result<()> {
        // The list of languages goes first, until it can't go any further.
        if let Some(picking) = self.picking.as_mut() {
            let rows = -(delta as f32 / 120.0 * WHEEL_ROWS).round() as isize;
            if picking.scroll(rows) {
                return self.relayout();
            }
        }
        let step = -(delta as f32 / 120.0) * WHEEL_STEP * self.scale;
        self.scroll_to(self.scroll + step)
    }

    /// The shortcut just recorded couldn't be used: it goes back to
    /// `hotkey`, saying why.
    pub fn reject_hotkey(&mut self, hotkey: &str, why: &str) -> Result<()> {
        self.values.config.hotkey = hotkey.to_owned();
        self.show_caps(true)?;
        self.shake_caps()?;
        self.set_note(Note::Shortcut, why, true)
    }

    /// Ticks `languages` (codes, like "en"), and only them.
    pub fn show_languages(&mut self, languages: &[String]) -> Result<()> {
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
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_SETTLE);
        }
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

    /// Shows `values` changed from elsewhere, cross-fading to them.
    pub fn refresh(&mut self, theme: Theme, values: Values) -> Result<()> {
        (self.values, self.recording) = (values, false);
        if self.email.text().trim() != self.values.config.mymemory_email {
            self.email = LineEdit::new(&self.values.config.mymemory_email, MAX_EMAIL_CHARS);
        }
        self.notes = NOTES.map(|note| (note.usual(&self.values), false));
        self.theme = theme;
        let (anim, renderer, s) = (&self.anim, &self.renderer, self.scale);
        self.frame.fade_theme(anim, renderer, &theme, s, THEME_MS)?;
        self.replace_view(true)
    }

    /// The window moved to a monitor with another `dpi`. `suggested` is
    /// where Windows would put it, in screen pixels.
    pub fn rescale(&mut self, dpi: u32, suggested: RECT) -> Result<()> {
        let s = dpi as f32 / 96.0;
        if (s - self.scale).abs() < 0.01 {
            return Ok(());
        }
        let scroll = self.scroll / self.scale * s;
        (self.scale, self.scroll) = (s, 0.0);
        self.scroll_anim = None;
        self.frame.apply_theme(&self.renderer, &self.theme, s)?;
        self.replace_view(false)?;
        let centre = POINT {
            x: (suggested.left + suggested.right) / 2,
            y: (suggested.top + suggested.bottom) / 2,
        };
        self.fit(ui::monitor_at(centre).0)?;
        self.scroll_to(scroll)?;
        let m = (MARGIN * s).round();
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                None,
                suggested.left,
                suggested.top,
                (self.card.0 + 2.0 * m) as i32,
                (self.room + 2.0 * m) as i32,
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
            Hit::Shortcut if self.recording => {
                self.stop_recording()?;
                Some(Change::RecordingStopped)
            }
            Hit::Shortcut => {
                self.start_recording()?;
                Some(Change::RecordingStarted)
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
                    Switch::Automation => {
                        v.config.use_ui_automation = on;
                        Change::UiAutomation(on)
                    }
                    Switch::Examples => {
                        v.config.show_examples = on;
                        Change::ShowExamples(on)
                    }
                };
                self.show_switch(sw, true)?;
                Some(change)
            }
            Hit::Choice(choice, i) => self.choose(choice, i)?,
            Hit::Step(counter, up) => self.step(counter, up)?,
            Hit::Languages => {
                self.set_open(!self.open)?;
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

    // -----------------------------------------------------------------------
    // The default languages

    /// The language on `side`, as a code.
    fn language(&self, side: Side) -> &str {
        let config = &self.values.config;
        match side {
            Side::From => &config.translate_from,
            Side::To => &config.translate_to,
        }
    }

    /// The change to make once the languages are as they are now.
    fn languages_changed(&self) -> Change {
        let config = &self.values.config;
        Change::TranslateLanguages(config.translate_from.clone(), config.translate_to.clone())
    }

    /// The boxes of the default languages, with the list under the one
    /// being picked.
    /// Returns them with how much shorter they are than with the list
    /// full, in pixels: the view is built for the full list, and what comes
    /// after slides up by that much.
    fn layout_languages(&self) -> Result<(Layout, f32)> {
        let picker = self.picking.as_ref().map(|p| Picker {
            side: p.side,
            query: p.query.text(),
            caret: p.query.caret_utf16(),
            matches: &p.matches,
            first: p.first,
            highlight: p.highlight,
        });
        let width = GROUP_W - 2.0 * ROW_PAD;
        let config = &self.values.config;
        let (from, to) = (&config.translate_from, &config.translate_to);
        let layout = self
            .renderer
            .layout_languages(from, to, picker.as_ref(), width)?;
        let spare = picker.as_ref().map_or(0.0, Picker::spare);
        let s = self.scale;
        let slack = ((layout.height + spare) * s).ceil() - (layout.height * s).ceil();
        Ok((layout, slack))
    }

    /// Builds the window's view again, for what the default languages show
    /// to have changed, which can change the height of the row too.
    fn relayout(&mut self) -> Result<()> {
        if self.refresh_boxes()? {
            return self.refit();
        }
        self.slide_view()?;
        self.refit()
    }

    /// Lists the languages `side`'s can be picked from, or shuts the list
    /// again if it's open on that side.
    fn open_picker(&mut self, side: Side) -> Result<()> {
        let open = self.picking.take().map(|p| p.side);
        if open != Some(side) {
            self.picking = Some(Picking::new(side, self.language(side)));
        }
        self.focus = Some(Stop::Select(side));
        self.relayout()?;
        let Some(view) = &self.view else {
            return Ok(());
        };
        let rect = view.translate_at;
        if self.picking.is_some() {
            self.scroll_into_view(rect)?;
        }
        Ok(())
    }

    /// Swaps the languages.
    fn swap_languages(&mut self) -> Result<Option<Change>> {
        self.picking = None;
        let config = &mut self.values.config;
        std::mem::swap(&mut config.translate_from, &mut config.translate_to);
        self.relayout()?;
        Ok(Some(self.languages_changed()))
    }

    /// Makes `code` the language of the side being picked, closing the list.
    /// The other side's language swaps them round.
    fn pick(&mut self, code: &str) -> Result<Option<Change>> {
        let Some(side) = self.picking.take().map(|p| p.side) else {
            return Ok(None);
        };
        self.focus = Some(Stop::Select(side));
        let config = &mut self.values.config;
        let (picked, other) = match side {
            Side::From => (&mut config.translate_from, &mut config.translate_to),
            Side::To => (&mut config.translate_to, &mut config.translate_from),
        };
        let changed = code != picked.as_str();
        if code == other.as_str() {
            std::mem::swap(picked, other);
        } else {
            *picked = code.to_owned();
        }
        self.relayout()?;
        Ok(changed.then(|| self.languages_changed()))
    }

    /// Narrows the list of languages down with a character typed.
    fn type_language(&mut self, unit: u16) -> Result<Option<Change>> {
        let hwnd = self.hwnd;
        let Some(side) = self.picking.as_ref().map(|p| p.side) else {
            return Ok(None);
        };
        let current = self.language(side).to_owned();
        let Some(picking) = self.picking.as_mut() else {
            return Ok(None);
        };
        match picking.query.type_unit(unit) {
            Input::Edited => {}
            Input::Paste => {
                let pasted = selection::clipboard_text(hwnd).unwrap_or_default();
                picking.query.paste(&pasted);
            }
            // The key going down picked it already.
            Input::Enter | Input::Nothing => return Ok(None),
        }
        picking.refilter(&current);
        self.relayout()?;
        Ok(None)
    }

    /// A key pressed while the list of languages is open: Up, Down, Tab,
    /// Shift+Tab, Page Up and Page Down move the highlight, Enter picks it,
    /// Esc shuts the list, and the others move the caret in the box.
    fn picker_key(&mut self, key: VIRTUAL_KEY) -> Result<Option<Change>> {
        let ctrl = hotkey::is_held(VK_CONTROL);
        let Some(side) = self.picking.as_ref().map(|p| p.side) else {
            return Ok(None);
        };
        let current = self.language(side).to_owned();
        let Some(picking) = self.picking.as_mut() else {
            return Ok(None);
        };
        match key {
            VK_ESCAPE => {
                self.picking = None;
                self.focus = Some(Stop::Select(side));
                self.relayout()?;
                return Ok(None);
            }
            VK_RETURN => {
                return match picking.highlighted() {
                    Some(code) => self.pick(code),
                    None => Ok(None),
                };
            }
            _ => {}
        }
        let page = PICKER_ROWS as isize;
        let step = match key {
            VK_DOWN => Some(1),
            VK_UP => Some(-1),
            VK_TAB if hotkey::is_held(VK_SHIFT) => Some(-1),
            VK_TAB => Some(1),
            VK_NEXT => Some(page),
            VK_PRIOR => Some(-page),
            _ => None,
        };
        if let Some(by) = step {
            picking.step(by);
            self.relayout()?;
            return Ok(None);
        }
        let edit = &mut picking.query;
        let changed = match key {
            VK_LEFT if ctrl => edit.move_caret(Motion::WordBack),
            VK_LEFT => edit.move_caret(Motion::CharBack),
            VK_RIGHT if ctrl => edit.move_caret(Motion::WordForward),
            VK_RIGHT => edit.move_caret(Motion::CharForward),
            VK_HOME => edit.move_caret(Motion::Start),
            VK_END => edit.move_caret(Motion::End),
            VK_DELETE => {
                let to = if ctrl {
                    Motion::WordForward
                } else {
                    Motion::CharForward
                };
                if edit.delete(to) {
                    picking.refilter(&current);
                }
                true
            }
            _ => false,
        };
        if changed {
            self.relayout()?;
        }
        Ok(None)
    }

    /// The email field's text changed from `before`, or may have: shows
    /// it, and saves it if it did.
    fn typed(&mut self, before: &str) -> Result<Option<Change>> {
        self.show_field()?;
        if self.email.text() == before {
            return Ok(None);
        }
        let email = self.email.text().trim().to_owned();
        self.values.config.mymemory_email = email.clone();
        Ok(Some(Change::MyMemoryEmail(email)))
    }

    /// Puts the caret where the email field was clicked, `x` window pixels
    /// across.
    fn place_caret(&mut self, x: i32) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let (x, _) = self.card_point(x, 0);
        let into = (x - view.field.at.x + self.email_shift) / self.scale;
        let at = self.renderer.field_position(self.email.text(), into)?;
        self.email.set_caret_utf16(at);
        self.show_field()
    }

    /// Whether the language at `i` in [`dicts::languages`] is ticked.
    fn ticked(&self, i: usize) -> bool {
        let Some((code, _)) = dicts::languages().nth(i) else {
            return false;
        };
        self.values.config.languages.iter().any(|l| l == code)
    }

    /// Ticks the language at `i`, or unticks it, unless it's the only one.
    fn tick(&mut self, i: usize) -> Result<Option<Change>> {
        let Some((code, _)) = dicts::languages().nth(i) else {
            return Ok(None);
        };
        let on = !self.ticked(i);
        let languages = &mut self.values.config.languages;
        if on {
            // In the order of the list, which is the order of preference.
            languages.push(code.to_owned());
            let rank = |l: &String| dicts::languages().position(|(c, _)| c == l);
            languages.sort_by_key(|l| rank(l).unwrap_or(usize::MAX));
        } else if languages.len() > 1 {
            languages.retain(|l| l != code);
        } else {
            // Words have to be looked up in something.
            if let Some(view) = &self.view {
                let item = &view.languages.items[i].on;
                self.anim.shake(item, item.Offset()?, self.scale * 0.5)?;
            }
            return Ok(None);
        }
        self.show_checks(true)?;
        let text = summary(&self.values.config.languages);
        self.set_note(Note::Language, &text, false)?;
        Ok(Some(Change::Language(code.to_owned(), on)))
    }

    /// Opens or shuts the list of languages, the card growing or shrinking
    /// with it.
    fn set_open(&mut self, open: bool) -> Result<()> {
        if open == self.open {
            return Ok(());
        }
        self.open = open;
        if !open && matches!(self.focus, Some(Stop::Language(_))) {
            self.focus = Some(Stop::Languages);
        }
        if !open && matches!(self.hovered, Some(Hit::Language(_))) {
            self.set_hovered(None)?;
        }
        self.show_open(true)?;
        self.refit()?;
        if open {
            if let Some(view) = &self.view {
                let v = &view.languages;
                let all = Rect {
                    x: 0.0,
                    y: v.list_y - (ROW_H * self.scale).round(),
                    w: self.card.0,
                    h: v.list_h + (ROW_H * self.scale).round(),
                };
                self.scroll_into_view(all)?;
            }
        }
        self.show_focus(true)
    }

    /// Limits the window's clickable area to the top `card_h` pixels of the
    /// card, and its shadow, so the transparent rest swallows no clicks.
    fn set_region(&self, card_h: f32) {
        let m = (MARGIN * self.scale).round();
        let (w, h) = (self.card.0 + 2.0 * m, card_h + 2.0 * m);
        unsafe {
            let region = CreateRectRgn(0, 0, w as i32, h.ceil() as i32);
            if SetWindowRgn(self.hwnd, Some(region), self.visible) == 0 {
                let _ = DeleteObject(region.into());
            }
        }
    }

    /// Sizes the card to what's in it now, gliding there. A window that
    /// grows does at once, moving up if it has to to stay on the screen;
    /// one that shrinks waits for the card to ([`Settings::settle`]).
    fn refit(&mut self) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let s = self.scale;
        let h = (view.header_h + view.height(self.open)).min(self.room).ceil();
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
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_SETTLE, SETTLE_MS, None);
        }
        let scroll = self.scroll.min(self.max_scroll());
        self.scroll_to(scroll)?;
        self.show_scroll(true)
    }

    fn switch(&self, sw: Switch) -> bool {
        match sw {
            Switch::Autostart => self.values.autostart,
            Switch::Online => self.values.config.online_lookup,
            Switch::Automation => self.values.config.use_ui_automation,
            Switch::Examples => self.values.config.show_examples,
        }
    }

    fn count(&self, counter: Counter) -> usize {
        let config = &self.values.config;
        match counter {
            Counter::Definitions => config.max_definitions,
            Counter::Synonyms => config.max_synonyms,
        }
    }

    /// Where `counter` goes counting up or down one, if it can.
    fn next(&self, counter: Counter, up: bool) -> Option<usize> {
        let (lo, hi) = counter.range();
        let n = self.count(counter);
        let n = if up { n + 1 } else { n.checked_sub(1)? };
        (lo..=hi).contains(&n).then_some(n)
    }

    /// Which of `choice`'s options is chosen.
    fn chosen(&self, choice: Choice) -> usize {
        let config = &self.values.config;
        match choice {
            Choice::Theme => THEMES.iter().position(|(mode, _)| *mode == config.theme),
        }
        .unwrap_or(0)
    }

    fn choose(&mut self, choice: Choice, i: usize) -> Result<Option<Change>> {
        if i == self.chosen(choice) {
            return Ok(None);
        }
        self.pill_from = Some((choice, self.chosen(choice)));
        let change = match choice {
            Choice::Theme => {
                let mode = THEMES[i].0;
                self.values.config.theme = mode;
                Change::Theme(mode)
            }
        };
        self.show_choice(choice, true)?;
        Ok(Some(change))
    }

    /// Counts `counter` up or down one. At either end, the number shakes.
    fn step(&mut self, counter: Counter, up: bool) -> Result<Option<Change>> {
        let Some(next) = self.next(counter, up) else {
            if let Some(view) = &self.view {
                let host = &view.counters[counter as usize].host;
                self.anim.shake(host, host.Offset()?, self.scale * 0.5)?;
            }
            return Ok(None);
        };
        let change = match counter {
            Counter::Definitions => {
                self.values.config.max_definitions = next;
                Change::MaxDefinitions(next)
            }
            Counter::Synonyms => {
                self.values.config.max_synonyms = next;
                Change::MaxSynonyms(next)
            }
        };
        self.show_count(counter, Some(up))?;
        Ok(Some(change))
    }

    fn start_recording(&mut self) -> Result<()> {
        self.recording = true;
        self.preview = held_modifiers();
        self.show_caps(false)?;
        self.set_note(Note::Shortcut, RECORDING_DETAIL, false)?;
        self.show_recording()
    }

    /// Stops recording, showing the shortcut there is.
    fn stop_recording(&mut self) -> Result<()> {
        self.recording = false;
        self.show_caps(true)?;
        self.set_note(Note::Shortcut, SHORTCUT_DETAIL, false)?;
        self.show_recording()
    }

    /// A key went down while recording.
    fn record(&mut self, key: VIRTUAL_KEY) -> Result<Option<Change>> {
        let modifiers = held_modifiers();
        if is_modifier(key) {
            self.set_preview(modifiers)?;
            return Ok(None);
        }
        if key == VK_ESCAPE && modifiers.0 == 0 {
            self.stop_recording()?;
            return Ok(Some(Change::RecordingStopped));
        }
        let vk = key.0 as u32;
        // Shift alone would take a capital letter away from every app.
        let strong = modifiers.0 & (MOD_CONTROL | MOD_ALT | MOD_WIN).0 != 0;
        let complaint = if hotkey::key_name(vk).is_none() {
            Some(UNUSABLE_KEY)
        } else if !strong && !hotkey::is_function_key(vk) {
            Some(NEEDS_MODIFIER)
        } else {
            None
        };
        if let Some(complaint) = complaint {
            self.shake_caps()?;
            self.set_note(Note::Shortcut, complaint, true)?;
            return Ok(None);
        }
        let text = Hotkey { modifiers, vk }.to_string();
        self.values.config.hotkey = text.clone();
        self.stop_recording()?;
        Ok(Some(Change::Hotkey(text)))
    }

    fn set_preview(&mut self, modifiers: HOT_KEY_MODIFIERS) -> Result<()> {
        if modifiers == self.preview {
            return Ok(());
        }
        self.preview = modifiers;
        self.show_caps(false)
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
        let y = y - view.header_h + self.scroll;
        view.hits(self.open)
            .find(|(_, r)| r.contains(x, y))
            .map(|(hit, _)| hit)
    }

    fn set_hovered(&mut self, hit: Option<Hit>) -> Result<()> {
        if hit == self.hovered {
            return Ok(());
        }
        let old = std::mem::replace(&mut self.hovered, hit);
        if old == Some(Hit::Info) || hit == Some(Hit::Info) {
            self.show_tip(hit == Some(Hit::Info))?;
        }
        self.show_hover(old)?;
        self.show_hover(hit)?;
        // Over the list of languages, the highlight moves along.
        if let Some(Hit::Pick(i)) = hit {
            let moved = self.picking.as_mut().is_some_and(|p| {
                let code = p.matches.get(i).copied();
                code.is_some_and(|code| p.highlight(code))
            });
            if moved {
                self.relayout()?;
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
        let stops: Vec<(Stop, Rect)> = view.stops(self.open).map(|(s, r, _)| (s, r)).collect();
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

    // -----------------------------------------------------------------------
    // Scrolling

    fn max_scroll(&self) -> f32 {
        let Some(view) = &self.view else {
            return 0.0;
        };
        ui::max_scroll(
            view.height(self.open),
            self.card.1 - view.header_h,
            self.scale,
        )
    }

    /// The scroll position on screen right now, following the running
    /// animation.
    fn visible_scroll(&self) -> f32 {
        let Some((from, started)) = self.scroll_anim else {
            return self.scroll;
        };
        let t = started.elapsed().as_secs_f32() * 1000.0 / SCROLL_MS as f32;
        if t >= 1.0 {
            return self.scroll;
        }
        from + (self.scroll - from) * cubic_bezier(SCROLL_EASE, t)
    }

    /// Scrolls to `to`, starting from where the content is on screen now,
    /// not from the last target, which would lag behind the wheel.
    fn scroll_to(&mut self, to: f32) -> Result<()> {
        let to = to.clamp(0.0, self.max_scroll()).round();
        if to == self.scroll {
            return Ok(());
        }
        let from = self.visible_scroll();
        self.scroll = to;
        self.scroll_anim = Some((from, Instant::now()));
        self.show_tip(false)?;
        if let Some(view) = &self.view {
            self.glide(&view.scroller, v3(0.0, -from), v3(0.0, -to))?;
        }
        self.show_scroll(true)
    }

    /// Moves `visual` from `from` to `to` the way scrolling does.
    fn glide(
        &self,
        visual: &impl windows::core::Interface,
        from: Vector3,
        to: Vector3,
    ) -> Result<()> {
        let a = self.anim.compositor.CreateVector3KeyFrameAnimation()?;
        a.InsertKeyFrame(0.0, from)?;
        a.InsertKeyFrameWithEasingFunction(1.0, to, &self.anim.scroll_ease)?;
        a.SetDuration(ts(SCROLL_MS))?;
        visual
            .cast::<CompositionObject>()?
            .StartAnimation(&HSTRING::from("Offset"), &a)
    }

    /// Scrolls just enough to show `rect`, in the scroller.
    fn scroll_into_view(&mut self, rect: Rect) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let margin = FOCUS_MARGIN * self.scale;
        let view_h = self.card.1 - view.header_h;
        let (top, bottom) = (rect.y - margin, rect.y + rect.h + margin);
        if top < self.scroll {
            self.scroll_to(top)
        } else if bottom > self.scroll + view_h {
            self.scroll_to(bottom - view_h)
        } else {
            Ok(())
        }
    }

    /// The divider under the header, once scrolled, and the scrollbar thumb.
    fn show_scroll(&self, animate: bool) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let (anim, s) = (&self.anim, self.scale);
        let divider = if self.scroll > 0.5 { 1.0 } else { 0.0 };
        anim.scalar(&view.divider, "Opacity", divider, 150, 0, &anim.decelerate)?;
        let max = self.max_scroll();
        if max <= 0.0 {
            Animator::stop(&view.thumb, &["Opacity"]);
            return view.thumb.SetOpacity(0.0);
        }
        let viewport = self.card.1 - view.header_h;
        let content_h = view.height(self.open);
        let (top, thumb) = ui::thumb_span(viewport, content_h, self.scroll, max, s);
        let x = ui::size_thumb(&view.thumb, &view.thumb_clip, self.card.0, thumb, s)?;
        let to = v3(x, view.header_h + top);
        let started = self
            .scroll_anim
            .filter(|(_, at)| at.elapsed().as_millis() < SCROLL_MS as u128);
        if let (true, Some((from, _))) = (animate, started) {
            let (from_top, _) = ui::thumb_span(viewport, content_h, from, max, s);
            self.glide(&view.thumb, v3(x, view.header_h + from_top), to)?;
        } else if animate {
            anim.vector3(&view.thumb, "Offset", to, SCROLL_MS, 0, &anim.decelerate)?;
        } else {
            Animator::stop(&view.thumb, &["Offset"]);
            view.thumb.SetOffset(to)?;
        }
        anim.scalar(&view.thumb, "Opacity", 1.0, 200, 0, &anim.decelerate)
    }

    // -----------------------------------------------------------------------
    // Showing the state

    /// Slides the knob over and fades the switch's fill to match its value.
    fn show_switch(&self, sw: Switch, animate: bool) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let (v, anim, on) = (&view.switches[sw as usize], &self.anim, self.switch(sw));
        let rest = v.rest[on as usize];
        let (fill, color) = (if on { 1.0 } else { 0.0 }, self.theme.knob(on));
        if animate {
            anim.vector3(&v.knob, "Offset", rest, KNOB_MS, 0, &anim.overshoot)?;
            anim.scalar(&v.on, "Opacity", fill, SWITCH_FILL_MS, 0, &anim.decelerate)?;
            let ms = SWITCH_FILL_MS;
            anim.scalar(&v.off, "Opacity", 1.0 - fill, ms, 0, &anim.decelerate)?;
            anim.color(&v.knob_brush, v.knob_brush.Color()?, color, ms)
        } else {
            Animator::stop(&v.knob, &["Offset"]);
            Animator::stop(&v.on, &["Opacity"]);
            Animator::stop(&v.off, &["Opacity"]);
            Animator::stop(&v.knob_brush, &["Color"]);
            v.knob.SetOffset(rest)?;
            v.on.SetOpacity(fill)?;
            v.off.SetOpacity(1.0 - fill)?;
            v.knob_brush.SetColor(color)
        }
    }

    /// The knob grows under the pointer, and stretches while pressed.
    fn show_knob(&self, sw: Switch) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let hit = Some(Hit::Switch(sw));
        let k = if self.pressed == hit {
            Vector3 {
                X: KNOB_PRESSED / KNOB,
                Y: KNOB_HOVER / KNOB,
                Z: 1.0,
            }
        } else if self.hovered == hit {
            scale(KNOB_HOVER / KNOB)
        } else {
            scale(1.0)
        };
        let (anim, knob) = (&self.anim, &view.switches[sw as usize].knob);
        anim.vector3(knob, "Scale", k, KNOB_MS / 2, 0, &anim.decelerate)
    }

    /// Glides `choice`'s pill to the chosen option, and brightens its name.
    fn show_choice(&self, choice: Choice, animate: bool) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let (v, anim) = (&view.choices[choice as usize], &self.anim);
        let chosen = self.chosen(choice);
        let rest = v.rest[chosen];
        match (animate, self.pill_from) {
            (true, Some((c, from))) if c == choice && from != chosen => {
                anim.vector3(&v.pill, "Offset", rest, PILL_MS, 0, &anim.overshoot)?;
                // It stretches as it goes, and settles as it arrives.
                let stretch = anim.compositor.CreateVector3KeyFrameAnimation()?;
                let wide = Vector3 {
                    X: 1.12,
                    Y: 0.92,
                    Z: 1.0,
                };
                stretch.InsertKeyFrameWithEasingFunction(0.35, wide, &anim.decelerate)?;
                stretch.InsertKeyFrameWithEasingFunction(1.0, scale(1.0), &anim.decelerate)?;
                stretch.SetDuration(ts(PILL_MS))?;
                v.pill.StartAnimation(&HSTRING::from("Scale"), &stretch)?;
            }
            _ => {
                Animator::stop(&v.pill, &["Offset", "Scale"]);
                v.pill.SetOffset(rest)?;
                v.pill.SetScale(scale(1.0))?;
            }
        }
        self.show_choice_labels(choice)
    }

    fn show_choice_labels(&self, choice: Choice) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let (anim, chosen) = (&self.anim, self.chosen(choice));
        for (i, label) in view.choices[choice as usize].labels.iter().enumerate() {
            let opacity = if i == chosen {
                1.0
            } else if self.hovered == Some(Hit::Choice(choice, i)) {
                0.85
            } else {
                0.6
            };
            anim.scalar(label, "Opacity", opacity, LABEL_MS, 0, &anim.decelerate)?;
        }
        Ok(())
    }

    /// Shows a counter's number. With `roll`, the new one rolls in from
    /// below as the old leaves upwards (or the other way, counting down).
    fn show_count(&mut self, counter: Counter, roll: Option<bool>) -> Result<()> {
        let n = self.count(counter);
        let ends = [false, true].map(|up| self.next(counter, up).is_none());
        let (r, anim, t, s) = (&self.renderer, &self.anim, &self.theme, self.scale);
        let Some(view) = self.view.as_mut() else {
            return Ok(());
        };
        let v = &mut view.counters[counter as usize];
        std::mem::swap(&mut v.value, &mut v.old);
        let text = counter.text(n);
        let w = VALUE_W;
        let line = r.layout_aligned(&text, Face::Value, Ink::Primary, w, Align::Center)?;
        let size = v.value.show(r, &line, s, t)?;
        let rest = v3(0.0, ((v.host_h - size.Y) / 2.0).round());
        let (new, old) = (&v.value.visual, &v.old.visual);
        Animator::stop(new, &["Opacity", "Offset"]);
        match roll {
            Some(up) => {
                let d = if up { ROLL * s } else { -ROLL * s };
                new.SetOpacity(0.0)?;
                new.SetOffset(v3(rest.X, rest.Y + d))?;
                anim.scalar(new, "Opacity", 1.0, ROLL_MS, 0, &anim.decelerate)?;
                anim.vector3(new, "Offset", rest, ROLL_MS, 0, &anim.overshoot)?;
                let away = v3(rest.X, rest.Y - d);
                anim.scalar(old, "Opacity", 0.0, ROLL_MS / 2, 0, &anim.accelerate)?;
                anim.vector3(old, "Offset", away, ROLL_MS, 0, &anim.decelerate)?;
            }
            None => {
                new.SetOpacity(1.0)?;
                new.SetOffset(rest)?;
                Animator::stop(old, &["Opacity", "Offset"]);
                old.SetOpacity(0.0)?;
                v.old.clear();
            }
        }
        // A button that can't count any further fades back.
        for (button, end) in v.buttons.iter().zip(ends) {
            let opacity = if end { 0.35 } else { 1.0 };
            let glyph = &button.glyph;
            anim.scalar(glyph, "Opacity", opacity, LABEL_MS, 0, &anim.decelerate)?;
        }
        Ok(())
    }

    /// Shows the shortcut's keys, or while recording, the modifiers held so
    /// far. With `rise`, the new keys rise into place as the old fade.
    fn show_caps(&mut self, rise: bool) -> Result<()> {
        let (r, anim, t, s) = (&self.renderer, &self.anim, &self.theme, self.scale);
        let layout: Layout = if self.recording {
            let held = hotkey::modifier_names(self.preview);
            if held.is_empty() {
                r.layout_aligned(RECORDING_HINT, Face::Menu, Ink::Accent, 160.0, Align::Right)?
            } else {
                r.layout_caps(&held)?
            }
        } else {
            let names = key_names(&self.values.config.hotkey);
            r.layout_caps(&names.iter().map(String::as_str).collect::<Vec<_>>())?
        };
        let Some(view) = self.view.as_mut() else {
            return Ok(());
        };
        let v = &mut view.shortcut;
        std::mem::swap(&mut v.caps, &mut v.old_caps);
        let size = v.caps.show(r, &layout, s, t)?;
        let rest = v3(
            (v.caps_right - size.X).round(),
            (v.caps_middle - size.Y / 2.0).round(),
        );
        v.caps_rest = rest;
        let (new, old) = (&v.caps.visual, &v.old_caps.visual);
        Animator::stop(new, &["Opacity", "Offset"]);
        if rise {
            new.SetOpacity(0.0)?;
            new.SetOffset(v3(rest.X, rest.Y + DRIFT * s))?;
            anim.scalar(new, "Opacity", 1.0, CAPS_MS, 0, &anim.decelerate)?;
            anim.vector3(new, "Offset", rest, CAPS_MS, 0, &anim.overshoot)?;
            anim.scalar(old, "Opacity", 0.0, CAPS_MS / 2, 0, &anim.accelerate)
        } else {
            new.SetOpacity(1.0)?;
            new.SetOffset(rest)?;
            Animator::stop(old, &["Opacity", "Offset"]);
            old.SetOpacity(0.0)?;
            v.old_caps.clear();
            Ok(())
        }
    }

    fn shake_caps(&self) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let v = &view.shortcut;
        self.anim.shake(&v.caps.visual, v.caps_rest, self.scale)
    }

    /// Changes a setting's description, cross-fading to the new one.
    fn set_note(&mut self, note: Note, text: &str, error: bool) -> Result<()> {
        let now = &self.notes[note as usize];
        if (text, error) == (now.0.as_str(), now.1) {
            return Ok(());
        }
        self.notes[note as usize] = (text.to_owned(), error);
        self.show_note(note, text, error, true)
    }

    fn show_note(&mut self, note: Note, text: &str, error: bool, animate: bool) -> Result<()> {
        let (r, anim, t, s) = (&self.renderer, &self.anim, &self.theme, self.scale);
        let Some(view) = self.view.as_mut() else {
            return Ok(());
        };
        let v = &mut view.notes[note as usize];
        std::mem::swap(&mut v.text, &mut v.old);
        let ink = if error { Ink::Critical } else { Ink::Secondary };
        let line = r.layout_line(text, Face::Small, ink, v.w)?;
        v.text.show(r, &line, s, t)?;
        let (new, old, rest) = (&v.text.visual, &v.old.visual, v.at);
        Animator::stop(new, &["Opacity", "Offset"]);
        if animate {
            new.SetOpacity(0.0)?;
            new.SetOffset(v3(rest.X, rest.Y + DRIFT * s / 2.0))?;
            anim.scalar(new, "Opacity", 1.0, DETAIL_MS, 0, &anim.decelerate)?;
            anim.vector3(new, "Offset", rest, DETAIL_MS, 0, &anim.decelerate)?;
            anim.scalar(old, "Opacity", 0.0, DETAIL_MS / 2, 0, &anim.accelerate)
        } else {
            new.SetOpacity(1.0)?;
            new.SetOffset(rest)?;
            Animator::stop(old, &["Opacity"]);
            old.SetOpacity(0.0)?;
            v.old.clear();
            Ok(())
        }
    }

    /// Shows what's typed in the email field, or its placeholder, scrolled
    /// as little as keeps the caret in view.
    fn show_field(&mut self) -> Result<()> {
        let (r, t, s) = (&self.renderer, &self.theme, self.scale);
        let (text, ink, at) = match self.email.text() {
            "" => (EMAIL_PLACEHOLDER, Ink::Tertiary, 0),
            text => (text, Ink::Primary, self.email.caret_utf16()),
        };
        let (layout, w, caret) = r.layout_field(text, ink, at)?;
        let Some(view) = &self.view else {
            return Ok(());
        };
        let v = &view.field;
        let size = v.text.show(r, &layout, s, t)?;
        let (x, w, room) = (caret.x * s, w * s, v.at.w);
        let caret_w = (CARET_W * s).round().max(1.0);
        let mut shift = self.email_shift;
        if x - shift > room - caret_w {
            shift = x - room + caret_w;
        }
        shift = shift.min(x).min(w + caret_w - room).max(0.0).round();
        self.email_shift = shift;
        let y = ((v.at.h - size.Y) / 2.0).round();
        v.text.visual.SetOffset(v3(-shift, y))?;
        v.caret.SetSize(v2(caret_w, (caret.height * s).round()))?;
        v.caret
            .SetOffset(v3((x - shift).round(), y + (caret.top * s).round()))?;
        if self.editing() {
            self.blink()?;
        }
        Ok(())
    }

    /// Whether what's typed goes in the email field.
    fn editing(&self) -> bool {
        self.visible && self.active && self.focus == Some(Stop::Email)
    }

    /// The accent line under the email field, and its caret, show while
    /// it's typed in.
    fn show_editing(&self) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let (anim, v, editing) = (&self.anim, &view.field, self.editing());
        let line = if editing { 1.0 } else { 0.0 };
        anim.scalar(&v.accent, "Opacity", line, HOVER_MS, 0, &anim.decelerate)?;
        if editing {
            self.blink()
        } else {
            Animator::stop(&v.caret, &["Opacity"]);
            v.caret.SetOpacity(0.0)
        }
    }

    /// Shows the email field's caret, solid for now: blinking restarts
    /// with every keystroke.
    fn blink(&self) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        ui::blink(&self.anim.compositor, &view.field.caret)
    }

    /// Shows the info tooltip under the info symbol, rising in a moment
    /// after the pointer settles on it, or sends it away.
    fn show_tip(&self, on: bool) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let (anim, tip, s) = (&self.anim, &view.tip, self.scale);
        Animator::stop(&tip.root, &["Opacity", "Offset"]);
        let info = view
            .hits(self.open)
            .find(|(hit, _)| *hit == Hit::Info)
            .map(|(_, rect)| rect)
            // Not while the symbol is under the header.
            .filter(|rect| rect.y >= self.scroll);
        let (true, Some(info)) = (on, info) else {
            return anim.scalar(&tip.root, "Opacity", 0.0, TIP_HIDE_MS, 0, &anim.accelerate);
        };
        let (w, h) = (tip.size.X, tip.size.Y);
        let gap = (TIP_GAP * s).round();
        let margin = (GROUP_X * s).round();
        let top = view.header_h + info.y - self.scroll;
        let below = top + info.h + gap;
        let y = if below + h <= self.card.1 - margin {
            below
        } else {
            top - gap - h
        };
        let x = (info.x + (info.w - w) / 2.0)
            .min(self.card.0 - margin - w)
            .max(margin);
        let rest = v3(x, y);
        let drift = (TIP_DRIFT * s).round();
        if tip.root.Opacity()? < 0.05 {
            tip.root.SetOffset(v3(x, y + drift))?;
        }
        anim.scalar(
            &tip.root,
            "Opacity",
            1.0,
            TIP_MS,
            TIP_DELAY_MS,
            &anim.decelerate,
        )?;
        anim.vector3(
            &tip.root,
            "Offset",
            rest,
            TIP_MS,
            TIP_DELAY_MS,
            &anim.decelerate,
        )
    }

    /// Fills the boxes of the languages used, and empties the others'.
    fn show_checks(&self, animate: bool) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let anim = &self.anim;
        for (i, item) in view.languages.items.iter().enumerate() {
            let fill = if self.ticked(i) { 1.0 } else { 0.0 };
            if animate {
                let ms = SWITCH_FILL_MS;
                anim.scalar(&item.on, "Opacity", fill, ms, 0, &anim.decelerate)?;
                anim.scalar(&item.off, "Opacity", 1.0 - fill, ms, 0, &anim.decelerate)?;
                // The tick pops in.
                if fill == 1.0 && item.on.Opacity()? < 0.5 {
                    item.on.SetScale(scale(0.7))?;
                }
                let k = scale(1.0);
                anim.vector3(&item.on, "Scale", k, KNOB_MS, 0, &anim.overshoot)?;
            } else {
                Animator::stop(&item.on, &["Opacity", "Scale"]);
                Animator::stop(&item.off, &["Opacity"]);
                item.on.SetOpacity(fill)?;
                item.on.SetScale(scale(1.0))?;
                item.off.SetOpacity(1.0 - fill)?;
            }
        }
        Ok(())
    }

    /// Shows how the download of the language at `i` is going, if it is.
    fn show_status(&self, i: usize) -> Result<()> {
        let (r, t, s) = (&self.renderer, &self.theme, self.scale);
        let Some(view) = &self.view else {
            return Ok(());
        };
        let item = &view.languages.items[i];
        let Some((text, error)) = &self.downloads[i] else {
            item.status.clear();
            return Ok(());
        };
        let ink = if *error {
            Ink::Critical
        } else {
            Ink::Secondary
        };
        let line = r.layout_aligned(text, Face::Small, ink, STATUS_W, Align::Right)?;
        let size = item.status.show(r, &line, s, t)?;
        let at = v3(
            (item.status_right - size.X).round(),
            (item.status_middle - size.Y / 2.0).round(),
        );
        item.status.visual.SetOffset(at)
    }

    /// Opens the list under the languages row, or shuts it: the group grows
    /// or shrinks with it, the footer moving down or up, and the chevron
    /// turns over.
    fn show_open(&self, animate: bool) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let (v, anim) = (&view.languages, &self.anim);
        let grow = if self.open { v.list_h } else { 0.0 };
        let h = v.group_h + grow;
        let (clip, outline) = (v2(v.group_w, h), v2(v.group_w - 1.0, h - 1.0));
        let footer = v3(v.footer_at.X, v.footer_at.Y + grow);
        let (opacity, angle) = if self.open { (1.0, 180.0) } else { (0.0, 0.0) };
        if animate {
            let ease = &anim.decelerate;
            anim.vector2(&v.clip, "Size", clip, EXPAND_MS)?;
            anim.vector2(&v.outline, "Size", outline, EXPAND_MS)?;
            anim.vector3(&v.footer, "Offset", footer, EXPAND_MS, 0, ease)?;
            anim.scalar(&v.list, "Opacity", opacity, EXPAND_MS, 0, ease)?;
            anim.scalar(
                &v.chevron,
                "RotationAngleInDegrees",
                angle,
                EXPAND_MS,
                0,
                ease,
            )
        } else {
            Animator::stop(&v.clip, &["Size"]);
            Animator::stop(&v.outline, &["Size"]);
            Animator::stop(&v.footer, &["Offset"]);
            Animator::stop(&v.list, &["Opacity"]);
            Animator::stop(&v.chevron, &["RotationAngleInDegrees"]);
            v.clip.SetSize(clip)?;
            v.outline.SetSize(outline)?;
            v.footer.SetOffset(footer)?;
            v.list.SetOpacity(opacity)?;
            v.chevron.SetRotationAngleInDegrees(angle)
        }
    }

    /// The ring round the shortcut breathes while recording.
    fn show_recording(&self) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let (anim, ring) = (&self.anim, &view.shortcut.ring);
        if !self.recording {
            return anim.scalar(ring, "Opacity", 0.0, 150, 0, &anim.accelerate);
        }
        let breathe = anim.compositor.CreateScalarKeyFrameAnimation()?;
        breathe.InsertKeyFrame(0.0, 1.0)?;
        breathe.InsertKeyFrameWithEasingFunction(0.5, 0.35, &anim.fade)?;
        breathe.InsertKeyFrameWithEasingFunction(1.0, 1.0, &anim.fade)?;
        breathe.SetDuration(ts(BREATHE_MS))?;
        breathe.SetIterationBehavior(AnimationIterationBehavior::Forever)?;
        ring.SetOpacity(1.0)?;
        ring.StartAnimation(&HSTRING::from("Opacity"), &breathe)
    }

    /// Lights up what's under the pointer, or puts out what no longer is.
    fn show_hover(&self, hit: Option<Hit>) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let anim = &self.anim;
        let on = hit.is_some() && hit == self.hovered;
        let fade = |visual: &SpriteVisual| {
            let opacity = if on { 1.0 } else { 0.0 };
            anim.scalar(visual, "Opacity", opacity, HOVER_MS, 0, &anim.decelerate)
        };
        match hit {
            Some(Hit::Close) => fade(&view.close_hover),
            Some(Hit::Shortcut) => fade(&view.shortcut.hover),
            Some(Hit::Switch(sw)) => {
                fade(&view.switches[sw as usize].hover)?;
                self.show_knob(sw)
            }
            Some(Hit::Choice(choice, _)) => self.show_choice_labels(choice),
            Some(Hit::Step(counter, up)) => {
                fade(&view.counters[counter as usize].buttons[up as usize].hover)
            }
            Some(Hit::Languages) => fade(&view.languages.hover),
            Some(Hit::Language(i)) => fade(&view.languages.items[i].hover),
            Some(Hit::Email) => fade(&view.field.hover),
            Some(Hit::Info | Hit::Select(_) | Hit::Swap | Hit::Pick(_) | Hit::Config) | None => {
                Ok(())
            }
        }
    }

    /// Presses in, or lets go of, what the button went down on.
    fn show_pressed(&self, hit: Option<Hit>) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let (anim, down) = (&self.anim, hit.is_some() && hit == self.pressed);
        match hit {
            Some(Hit::Switch(sw)) => self.show_knob(sw),
            Some(Hit::Step(counter, up)) => {
                let button = &view.counters[counter as usize].buttons[up as usize].visual;
                let (k, ease) = if down {
                    (0.9, &anim.decelerate)
                } else {
                    (1.0, &anim.overshoot)
                };
                let ms = if down { PRESS_MS } else { PRESS_MS * 3 };
                anim.vector3(button, "Scale", scale(k), ms, 0, ease)
            }
            _ => Ok(()),
        }
    }

    /// Moves the focus ring to the focused control, gliding there when it
    /// was showing already, or hides it.
    fn show_focus(&mut self, animate: bool) -> Result<()> {
        self.show_editing()?;
        let Some(view) = &self.view else {
            return Ok(());
        };
        let anim = &self.anim;
        let stop = self.focus.filter(|_| self.ring).and_then(|focus| {
            view.stops(self.open)
                .find(|(stop, _, _)| *stop == focus)
                .map(|(_, rect, radius)| (rect, radius))
        });
        let Some((rect, radius)) = stop else {
            self.ring_shown = false;
            Animator::stop(&view.ring, &["Opacity"]);
            return view.ring.SetOpacity(0.0);
        };
        let geometry = &view.ring_geometry;
        let (at, size) = (v2(rect.x, rect.y), rect.size());
        if animate && self.ring_shown {
            anim.vector2(geometry, "Offset", at, FOCUS_MS)?;
            anim.vector2(geometry, "Size", size, FOCUS_MS)?;
        } else {
            Animator::stop(geometry, &["Offset", "Size"]);
            geometry.SetOffset(at)?;
            geometry.SetSize(size)?;
        }
        geometry.SetCornerRadius(v2(radius, radius))?;
        self.ring_shown = true;
        anim.scalar(&view.ring, "Opacity", 1.0, HOVER_MS, 0, &anim.decelerate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages_row_comes_last() {
        // Only the footer moves as the list of languages opens.
        let (_, rows) = SECTIONS.last().unwrap();
        assert!(matches!(rows.last().unwrap().control, Control::Languages));
        let count = SECTIONS
            .iter()
            .flat_map(|(_, rows)| rows.iter())
            .filter(|row| matches!(row.control, Control::Languages))
            .count();
        assert_eq!(count, 1);
        let names = |l: &[&str]| summary(&l.iter().map(|l| l.to_string()).collect::<Vec<_>>());
        assert_eq!(names(&["it", "en"]), "English, Italiano");
        assert_eq!(names(&["it", "xx"]), "Italiano");
    }

    #[test]
    fn rows_fit_the_card() {
        for (_, rows) in SECTIONS {
            for row in *rows {
                // Room for the name and description beside each control.
                let room = GROUP_W - 2.0 * ROW_PAD - row.control.width() - CONTROL_GAP;
                assert!(room >= 120.0, "{}", row.title);
            }
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
    }
}
