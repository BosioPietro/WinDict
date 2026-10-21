//! The settings window: a card in the style of the definition card, with
//! the settings grouped on it the way Windows 11 groups its own. Like the
//! card it's a composition visual tree, so all its motion runs on the system
//! compositor: the card rises in with its parts a beat after one another,
//! switches slide, the chosen theme's pill glides over, numbers roll, a new
//! shortcut's keys rise into place, and the focus ring follows Tab from
//! control to control. Changing the theme cross-fades the whole window.

mod view;

use windows::core::{Result, HSTRING};
use windows::Win32::Foundation::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::*;
use windows_numerics::{Vector2, Vector3};

use crate::config::{Config, ThemeMode};
use crate::dicts;
use crate::hotkey::{self, held_modifiers, is_modifier, Hotkey};
use crate::popup;
use crate::render::{Align, Face, Ink, Layout, Renderer};
use crate::theme::Theme;
use crate::ui::{self, scale, ts, v2, v3, Animator, Frame, Text, MARGIN};
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
/// How wide each control is, for the description beside it to stay clear.
const SHORTCUT_W: f32 = 150.0;
const COUNTER_W: f32 = 2.0 * STEP_SIZE + VALUE_W;
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
const THEME_MS: u64 = 300;
const BREATHE_MS: u64 = 1600;

// Segoe Fluent Icons and Segoe MDL2 Assets.
const CLOSE_GLYPH: &str = "\u{E8BB}";
const EDIT_GLYPH: &str = "\u{E70F}";
const MINUS_GLYPH: &str = "\u{E738}";
const PLUS_GLYPH: &str = "\u{E710}";

const SHORTCUT_DETAIL: &str = "Looks up the selected word";
const LANGUAGE_DETAIL: &str = "The dictionary words are looked up in";
const RECORDING_DETAIL: &str = "Press the new shortcut, or Esc to cancel";
const RECORDING_HINT: &str = "Press keys";
const NEEDS_MODIFIER: &str = "Hold Ctrl, Alt or Win with the key";
const UNUSABLE_KEY: &str = "That key can\u{2019}t be part of a shortcut";
pub const SHORTCUT_TAKEN: &str = "Another app already uses that shortcut";
const CONFIG_LINK: &str = "Open config.json";

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
    /// A language's code, like "en".
    Language(String),
    MaxDefinitions(usize),
    MaxSynonyms(usize),
    ShowExamples(bool),
    OnlineLookup(bool),
    UiAutomation(bool),
    Autostart(bool),
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
        if n == 0 {
            "Off".into()
        } else {
            n.to_string()
        }
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
    Language,
}

const CHOICES: [Choice; 2] = [Choice::Theme, Choice::Language];

impl Choice {
    fn names(self) -> Vec<&'static str> {
        match self {
            Choice::Theme => THEMES.iter().map(|(_, name)| *name).collect(),
            Choice::Language => languages().map(|(_, name)| name).collect(),
        }
    }
}

/// The languages there are dictionaries for: their codes, and names.
fn languages() -> impl Iterator<Item = (&'static str, &'static str)> {
    std::iter::once(("en", "English")).chain(dicts::DOWNLOADS.iter().map(|d| (d.language, d.name)))
}

/// A setting's description that can change to say what's going on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Note {
    Shortcut,
    Language,
}

const NOTES: [Note; 2] = [Note::Shortcut, Note::Language];

impl Note {
    fn usual(self) -> &'static str {
        match self {
            Note::Shortcut => SHORTCUT_DETAIL,
            Note::Language => LANGUAGE_DETAIL,
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
    Config,
}

/// What Tab stops at, in order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stop {
    Shortcut,
    Switch(Switch),
    Choice(Choice),
    Counter(Counter),
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
}

impl Control {
    fn width(self) -> f32 {
        match self {
            Control::Shortcut => SHORTCUT_W,
            Control::Switch(_) => SWITCH_W,
            Control::Choice(choice) => choice.names().len() as f32 * SEGMENT_W,
            Control::Counter(_) => COUNTER_W,
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
        "Lookup",
        &[
            row(
                "Language",
                LANGUAGE_DETAIL,
                Control::Choice(Choice::Language),
            ),
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
    /// What rises in as the window opens, and where each rests.
    reveal: Vec<(Visual, Vector3)>,
    shortcut: ShortcutView,
    /// In the order of [`SWITCHES`] and [`COUNTERS`].
    switches: Vec<SwitchView>,
    /// In the order of [`CHOICES`] and [`NOTES`].
    choices: Vec<ChoiceView>,
    notes: Vec<NoteView>,
    counters: Vec<CounterView>,
    /// Where things can be clicked, in the scroller.
    hits: Vec<(Hit, Rect)>,
    /// Where Tab stops, and the corner radius of the ring there, in the scroller.
    stops: Vec<(Stop, Rect, f32)>,
    ring: ShapeVisual,
    ring_geometry: CompositionRoundedRectangleGeometry,
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
    /// The choice whose pill is gliding over, and the option it's leaving.
    pill_from: Option<(Choice, usize)>,
    scroll: f32,
    pub visible: bool,
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
            notes: NOTES.map(|note| (note.usual().into(), false)),
            pill_from: None,
            scroll: 0.0,
            visible: false,
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
        self.notes = NOTES.map(|note| (note.usual().into(), false));
        self.frame.apply_theme(&self.renderer, &theme, s)?;
        self.replace_view(false)?;
        self.fit(work)?;

        let m = (MARGIN * s).round();
        let (w, h) = (
            (self.card.0 + 2.0 * m) as i32,
            (self.card.1 + 2.0 * m) as i32,
        );
        let left = (work.left + work.right - w) / 2;
        let top = (work.top + work.bottom - h) / 2;

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
        if let Some(stop) = hit.and_then(Hit::stop) {
            self.focus = Some(stop);
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
        let shift = hotkey::is_held(VK_SHIFT);
        let change = match (key, self.focus) {
            (VK_ESCAPE, _) => return Ok(Some(Change::Close)),
            (VK_TAB, _) => {
                self.step_focus(!shift)?;
                return Ok(None);
            }
            (VK_SPACE | VK_RETURN, Some(Stop::Shortcut)) => self.activate(Hit::Shortcut)?,
            (VK_SPACE | VK_RETURN, Some(Stop::Switch(sw))) => self.activate(Hit::Switch(sw))?,
            (VK_SPACE | VK_RETURN, Some(Stop::Config)) => self.activate(Hit::Config)?,
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

    /// A key came up: while recording, the held modifiers show as they're let go.
    pub fn key_up(&mut self, key: VIRTUAL_KEY) -> Result<()> {
        if self.recording && is_modifier(key) {
            self.set_preview(held_modifiers())?;
        }
        Ok(())
    }

    /// The window stopped being the active one: recording stops.
    pub fn deactivate(&mut self) -> Result<Option<Change>> {
        self.set_pressed(None)?;
        if !self.recording {
            return Ok(None);
        }
        self.stop_recording()?;
        Ok(Some(Change::RecordingStopped))
    }

    /// Scrolls by a mouse wheel delta.
    pub fn wheel(&mut self, delta: i32) -> Result<()> {
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

    /// Shows `language` as the one chosen, and the language's description
    /// as `note` says (how a download is going, or why it failed), or as
    /// usual.
    pub fn show_language(&mut self, language: &str, note: Option<(&str, bool)>) -> Result<()> {
        if self.values.config.language != language {
            self.pill_from = Some((Choice::Language, self.chosen(Choice::Language)));
            self.values.config.language = language.to_owned();
            self.show_choice(Choice::Language, true)?;
        }
        let (text, error) = note.unwrap_or((LANGUAGE_DETAIL, false));
        self.set_note(Note::Language, text, error)
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
        self.notes[Note::Shortcut as usize] = (SHORTCUT_DETAIL.into(), false);
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
                (self.card.1 + 2.0 * m) as i32,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
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
        })
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
        match counter {
            Counter::Definitions => self.values.config.max_definitions,
            Counter::Synonyms => self.values.config.max_synonyms,
        }
    }

    /// Which of `choice`'s options is chosen.
    fn chosen(&self, choice: Choice) -> usize {
        let config = &self.values.config;
        match choice {
            Choice::Theme => THEMES.iter().position(|(mode, _)| *mode == config.theme),
            Choice::Language => languages().position(|(code, _)| code == config.language),
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
            Choice::Language => {
                let Some((code, _)) = languages().nth(i) else {
                    return Ok(None);
                };
                self.values.config.language = code.to_owned();
                Change::Language(code.to_owned())
            }
        };
        self.show_choice(choice, true)?;
        Ok(Some(change))
    }

    /// Counts `counter` up or down one. At either end, the number shakes.
    fn step(&mut self, counter: Counter, up: bool) -> Result<Option<Change>> {
        let (lo, hi) = counter.range();
        let n = self.count(counter);
        let next = if up {
            (n + 1).min(hi)
        } else {
            n.saturating_sub(1).max(lo)
        };
        if next == n {
            if let Some(view) = &self.view {
                let host = &view.counters[counter as usize].host;
                self.anim.shake(host, host.Offset()?, self.scale * 0.5)?;
            }
            return Ok(None);
        }
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
        view.hits
            .iter()
            .find(|(_, r)| r.contains(x, y))
            .map(|&(hit, _)| hit)
    }

    fn set_hovered(&mut self, hit: Option<Hit>) -> Result<()> {
        if hit == self.hovered {
            return Ok(());
        }
        let old = std::mem::replace(&mut self.hovered, hit);
        self.show_hover(old)?;
        self.show_hover(hit)
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
        let stops: Vec<(Stop, Rect)> = view.stops.iter().map(|&(s, r, _)| (s, r)).collect();
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
        ui::max_scroll(view.content_h, self.card.1 - view.header_h, self.scale)
    }

    fn scroll_to(&mut self, to: f32) -> Result<()> {
        let to = to.clamp(0.0, self.max_scroll()).round();
        if to == self.scroll {
            return Ok(());
        }
        self.scroll = to;
        if let Some(view) = &self.view {
            let (anim, rest) = (&self.anim, v3(0.0, -to));
            anim.vector3(
                &view.scroller,
                "Offset",
                rest,
                SCROLL_MS,
                0,
                &anim.decelerate,
            )?;
        }
        self.show_scroll(true)
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
        let (top, thumb) = ui::thumb_span(viewport, view.content_h, self.scroll, max, s);
        let x = ui::size_thumb(&view.thumb, &view.thumb_clip, self.card.0, thumb, s)?;
        let to = v3(x, view.header_h + top);
        if animate {
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
        let (lo, hi) = counter.range();
        let (r, anim, t, s) = (&self.renderer, &self.anim, &self.theme, self.scale);
        let Some(view) = self.view.as_mut() else {
            return Ok(());
        };
        let v = &mut view.counters[counter as usize];
        std::mem::swap(&mut v.value, &mut v.old);
        let text = counter.text(n);
        let line = r.layout_aligned(&text, Face::Value, Ink::Primary, VALUE_W, Align::Center)?;
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
        for (button, end) in v.buttons.iter().zip([lo, hi]) {
            let opacity = if n == end { 0.35 } else { 1.0 };
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
            Some(Hit::Config) | None => Ok(()),
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
        let Some(view) = &self.view else {
            return Ok(());
        };
        let anim = &self.anim;
        let stop = self.focus.filter(|_| self.ring).and_then(|focus| {
            view.stops
                .iter()
                .find(|(stop, _, _)| *stop == focus)
                .map(|&(_, rect, radius)| (rect, radius))
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
    }
}
