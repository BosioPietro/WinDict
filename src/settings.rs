//! The settings window: a card in the style of the definition card, with
//! the settings grouped on it the way Windows 11 groups its own. Like the
//! card it's a composition visual tree, so all its motion runs on the system
//! compositor: the card rises in with its parts a beat after one another,
//! switches slide, the chosen theme's pill glides over, numbers roll, a new
//! shortcut's keys rise into place, and the focus ring follows Tab from
//! control to control. Changing the theme cross-fades the whole window.

use windows::core::{Interface, Result, HSTRING};
use windows::Win32::Foundation::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::UI::Color;
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::*;
use windows_numerics::{Vector2, Vector3};

use crate::config::ThemeMode;
use crate::hotkey::{self, Hotkey};
use crate::popup::{self, ts, v2, v3, Animator, Frame, Text, MARGIN};
use crate::render::{Align, Face, Ink, Layout, Renderer};
use crate::theme::{argb, Theme};

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
const THEME_W: f32 = 3.0 * SEGMENT_W;
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
const RECORDING_DETAIL: &str = "Press the new shortcut, or Esc to cancel";
const RECORDING_HINT: &str = "Press keys";
const NEEDS_MODIFIER: &str = "Hold Ctrl, Alt or Win with the key";
const UNUSABLE_KEY: &str = "That key can\u{2019}t be part of a shortcut";
pub const SHORTCUT_TAKEN: &str = "Another app already uses that shortcut";
const CONFIG_LINK: &str = "Open config.json";

/// What the settings window shows.
#[derive(Clone)]
pub struct Values {
    pub hotkey: String,
    pub theme: ThemeMode,
    pub max_definitions: usize,
    pub max_synonyms: usize,
    pub show_examples: bool,
    pub online_lookup: bool,
    pub use_ui_automation: bool,
    pub autostart: bool,
}

/// Something done in the window, for the app to carry out.
#[derive(Debug, PartialEq)]
pub enum Change {
    /// Recording a new shortcut started, so the current one should stop
    /// working meanwhile, or it was abandoned.
    Recording(bool),
    Hotkey(String),
    Theme(ThemeMode),
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

/// What's under the pointer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Hit {
    Close,
    Shortcut,
    Switch(Switch),
    Theme(usize),
    /// A counter's down (false) or up (true) button.
    Step(Counter, bool),
    Config,
}

/// What Tab stops at, in order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stop {
    Shortcut,
    Switch(Switch),
    Theme,
    Counter(Counter),
    Config,
}

impl Hit {
    fn stop(self) -> Option<Stop> {
        match self {
            Hit::Close => None,
            Hit::Shortcut => Some(Stop::Shortcut),
            Hit::Switch(sw) => Some(Stop::Switch(sw)),
            Hit::Theme(_) => Some(Stop::Theme),
            Hit::Step(counter, _) => Some(Stop::Counter(counter)),
            Hit::Config => Some(Stop::Config),
        }
    }
}

#[derive(Clone, Copy)]
enum Control {
    Shortcut,
    Switch(Switch),
    Theme,
    Counter(Counter),
}

impl Control {
    fn width(self) -> f32 {
        match self {
            Control::Shortcut => SHORTCUT_W,
            Control::Switch(_) => SWITCH_W,
            Control::Theme => THEME_W,
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
            row("Theme", "Card and menu colors", Control::Theme),
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

fn scale(k: f32) -> Vector3 {
    Vector3 { X: k, Y: k, Z: 1.0 }
}

fn container(c: &Compositor, parent: &ContainerVisual, at: Vector3) -> Result<ContainerVisual> {
    let visual = c.CreateContainerVisual()?;
    visual.SetOffset(at)?;
    parent.Children()?.InsertAtTop(&visual)?;
    Ok(visual)
}

/// A rectangle of `color` with corners of `radius` pixels, in `parent`.
fn rounded(
    c: &Compositor,
    parent: &ContainerVisual,
    rect: Rect,
    radius: f32,
    color: Color,
) -> Result<(SpriteVisual, CompositionColorBrush)> {
    let visual = c.CreateSpriteVisual()?;
    let brush = c.CreateColorBrushWithColor(color)?;
    visual.SetBrush(&brush)?;
    visual.SetOffset(rect.at())?;
    visual.SetSize(rect.size())?;
    if radius > 0.0 {
        let clip = c.CreateRoundedRectangleGeometry()?;
        clip.SetSize(rect.size())?;
        clip.SetCornerRadius(v2(radius, radius))?;
        visual.SetClip(&c.CreateGeometricClipWithGeometry(&clip)?)?;
    }
    parent.Children()?.InsertAtTop(&visual)?;
    Ok((visual, brush))
}

/// A hairline just inside a rounded rectangle, in `parent`.
fn outline(
    c: &Compositor,
    parent: &ContainerVisual,
    rect: Rect,
    radius: f32,
    color: Color,
) -> Result<ShapeVisual> {
    let visual = c.CreateShapeVisual()?;
    visual.SetOffset(rect.at())?;
    visual.SetSize(rect.size())?;
    let geometry = c.CreateRoundedRectangleGeometry()?;
    geometry.SetOffset(v2(0.5, 0.5))?;
    geometry.SetSize(v2(rect.w - 1.0, rect.h - 1.0))?;
    geometry.SetCornerRadius(v2(radius - 0.5, radius - 0.5))?;
    let shape = c.CreateSpriteShapeWithGeometry(&geometry)?;
    shape.SetStrokeBrush(&c.CreateColorBrushWithColor(color)?)?;
    shape.SetStrokeThickness(1.0)?;
    visual.Shapes()?.Append(&shape)?;
    parent.Children()?.InsertAtTop(&visual)?;
    Ok(visual)
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

/// The modifiers held down right now, as of the message being handled.
fn held_modifiers() -> HOT_KEY_MODIFIERS {
    let down = |vk: VIRTUAL_KEY| unsafe { GetKeyState(vk.0 as i32) } < 0;
    let mut modifiers = HOT_KEY_MODIFIERS(0);
    for (vks, modifier) in [
        (&[VK_CONTROL][..], MOD_CONTROL),
        (&[VK_MENU], MOD_ALT),
        (&[VK_SHIFT], MOD_SHIFT),
        (&[VK_LWIN, VK_RWIN], MOD_WIN),
    ] {
        if vks.iter().any(|&vk| down(vk)) {
            modifiers |= modifier;
        }
    }
    modifiers
}

fn is_modifier(vk: VIRTUAL_KEY) -> bool {
    matches!(
        vk,
        VK_CONTROL
            | VK_LCONTROL
            | VK_RCONTROL
            | VK_MENU
            | VK_LMENU
            | VK_RMENU
            | VK_SHIFT
            | VK_LSHIFT
            | VK_RSHIFT
            | VK_LWIN
            | VK_RWIN
    )
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

struct ThemeView {
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
    detail: Text,
    old_detail: Text,
    detail_at: Vector3,
    detail_w: f32,
    ring: ShapeVisual,
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
    theme: ThemeView,
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
    /// The shortcut's description, and whether it says something went wrong.
    detail: (String, bool),
    /// The option the theme's pill is leaving, while it glides over.
    pill_from: Option<usize>,
    scroll: f32,
    pub visible: bool,
}

impl Settings {
    pub fn new(hwnd: HWND, compositor: Compositor, theme: Theme, values: Values) -> Result<Self> {
        let target = popup::attach(hwnd, &compositor)?;
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
            detail: (SHORTCUT_DETAIL.into(), false),
            pill_from: None,
            scroll: 0.0,
            visible: false,
        })
    }

    // -----------------------------------------------------------------------
    // Public API

    /// Opens the window in the middle of the screen `near` (screen pixels)
    /// is on, showing `values`.
    pub fn show(&mut self, near: POINT, theme: Theme, values: Values) -> Result<()> {
        let (work, s) = popup::monitor_at(near);
        (self.theme, self.scale, self.values) = (theme, s, values);
        (self.hovered, self.pressed, self.focus) = (None, None, None);
        (self.ring, self.recording, self.scroll) = (false, false, 0.0);
        self.detail = (SHORTCUT_DETAIL.into(), false);
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

        let anim = &self.anim;
        anim.scalar(root, "Opacity", 1.0, FADE_IN_MS, 0, &anim.decelerate)?;
        anim.vector3(root, "Scale", scale(1.0), SHOW_MS, 0, &anim.decelerate)?;
        let rest = v3(0.0, 0.0);
        anim.vector3(root, "Offset", rest, SHOW_MS, 0, &anim.decelerate)?;
        self.reveal()
    }

    /// Fades the window out. The caller hides it after [`popup::HIDE_MS`].
    pub fn begin_hide(&mut self) -> Result<()> {
        if !self.visible {
            return Ok(());
        }
        self.visible = false;
        self.recording = false;
        let (anim, root) = (&self.anim, &self.root);
        let ms = popup::HIDE_MS;
        anim.scalar(root, "Opacity", 0.0, ms, 0, &anim.accelerate)?;
        anim.vector3(
            root,
            "Offset",
            v3(0.0, 6.0 * self.scale),
            ms,
            0,
            &anim.accelerate,
        )?;
        anim.vector3(root, "Scale", scale(0.98), ms, 0, &anim.accelerate)
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
            return Ok(Some(Change::Recording(false)));
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
        let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
        let change = match (key, self.focus) {
            (VK_ESCAPE, _) => return Ok(Some(Change::Close)),
            (VK_TAB, _) => {
                self.step_focus(!shift)?;
                return Ok(None);
            }
            (VK_SPACE | VK_RETURN, Some(Stop::Shortcut)) => self.activate(Hit::Shortcut)?,
            (VK_SPACE | VK_RETURN, Some(Stop::Switch(sw))) => self.activate(Hit::Switch(sw))?,
            (VK_SPACE | VK_RETURN, Some(Stop::Config)) => self.activate(Hit::Config)?,
            (VK_LEFT | VK_RIGHT, Some(Stop::Theme)) => {
                let at = self.theme_index();
                let next = if key == VK_RIGHT {
                    (at + 1).min(THEMES.len() - 1)
                } else {
                    at.saturating_sub(1)
                };
                self.choose_theme(next)?
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
        Ok(Some(Change::Recording(false)))
    }

    /// Scrolls by a mouse wheel delta.
    pub fn wheel(&mut self, delta: i32) -> Result<()> {
        let step = -(delta as f32 / 120.0) * WHEEL_STEP * self.scale;
        self.scroll_to(self.scroll + step)
    }

    /// The shortcut just recorded couldn't be used: it goes back to
    /// `hotkey`, saying why.
    pub fn reject_hotkey(&mut self, hotkey: &str, why: &str) -> Result<()> {
        self.values.hotkey = hotkey.to_owned();
        self.show_caps(true)?;
        self.shake_caps()?;
        self.set_detail(why, true)
    }

    /// Switches to `theme`, cross-fading everything.
    pub fn retheme(&mut self, theme: Theme) -> Result<()> {
        let same = (theme.dark, theme.transparency, theme.accent)
            == (self.theme.dark, self.theme.transparency, self.theme.accent);
        if same {
            self.pill_from = None;
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
        self.detail = (SHORTCUT_DETAIL.into(), false);
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
        self.fit(popup::monitor_at(centre).0)?;
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
                Some(Change::Recording(false))
            }
            Hit::Shortcut => {
                self.start_recording()?;
                Some(Change::Recording(true))
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
                        v.online_lookup = on;
                        Change::OnlineLookup(on)
                    }
                    Switch::Automation => {
                        v.use_ui_automation = on;
                        Change::UiAutomation(on)
                    }
                    Switch::Examples => {
                        v.show_examples = on;
                        Change::ShowExamples(on)
                    }
                };
                self.show_switch(sw, true)?;
                Some(change)
            }
            Hit::Theme(i) => self.choose_theme(i)?,
            Hit::Step(counter, up) => self.step(counter, up)?,
        })
    }

    fn switch(&self, sw: Switch) -> bool {
        match sw {
            Switch::Autostart => self.values.autostart,
            Switch::Online => self.values.online_lookup,
            Switch::Automation => self.values.use_ui_automation,
            Switch::Examples => self.values.show_examples,
        }
    }

    fn count(&self, counter: Counter) -> usize {
        match counter {
            Counter::Definitions => self.values.max_definitions,
            Counter::Synonyms => self.values.max_synonyms,
        }
    }

    fn theme_index(&self) -> usize {
        THEMES
            .iter()
            .position(|(mode, _)| *mode == self.values.theme)
            .unwrap_or(0)
    }

    fn choose_theme(&mut self, i: usize) -> Result<Option<Change>> {
        let mode = THEMES[i].0;
        if mode == self.values.theme {
            return Ok(None);
        }
        self.pill_from = Some(self.theme_index());
        self.values.theme = mode;
        self.show_theme(true)?;
        Ok(Some(Change::Theme(mode)))
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
                self.values.max_definitions = next;
                Change::MaxDefinitions(next)
            }
            Counter::Synonyms => {
                self.values.max_synonyms = next;
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
        self.set_detail(RECORDING_DETAIL, false)?;
        self.show_recording()
    }

    /// Stops recording, showing the shortcut there is.
    fn stop_recording(&mut self) -> Result<()> {
        self.recording = false;
        self.show_caps(true)?;
        self.set_detail(SHORTCUT_DETAIL, false)?;
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
            return Ok(Some(Change::Recording(false)));
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
            self.set_detail(complaint, true)?;
            return Ok(None);
        }
        let text = Hotkey { modifiers, vk }.to_string();
        self.values.hotkey = text.clone();
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
        let overflow = view.content_h - (self.card.1 - view.header_h);
        // Ignore rounding slack: no scrolling for a pixel or two.
        if overflow > 2.0 * self.scale {
            overflow
        } else {
            0.0
        }
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
        let track = viewport - 8.0 * s;
        let thumb = (track * viewport / view.content_h).max(28.0 * s);
        let y = view.header_h + 4.0 * s + (track - thumb) * (self.scroll / max);
        let to = v3(self.card.0 - 7.0 * s, y);
        view.thumb.SetSize(v2(3.0 * s, thumb))?;
        view.thumb_clip.SetSize(v2(3.0 * s, thumb))?;
        view.thumb_clip.SetCornerRadius(v2(1.5 * s, 1.5 * s))?;
        if animate {
            anim.vector3(&view.thumb, "Offset", to, SCROLL_MS, 0, &anim.decelerate)?;
        } else {
            Animator::stop(&view.thumb, &["Offset"]);
            view.thumb.SetOffset(to)?;
        }
        anim.scalar(&view.thumb, "Opacity", 1.0, 200, 0, &anim.decelerate)
    }

    // -----------------------------------------------------------------------
    // Building the view

    /// Builds the view afresh for the current scale and theme, showing the
    /// current state at rest. With `crossfade`, the old view fades out as
    /// the new one fades in; otherwise it goes at once.
    fn replace_view(&mut self, crossfade: bool) -> Result<()> {
        let children = self.frame.card.Children()?;
        if let Some(old) = self.fading.take() {
            children.Remove(&old)?;
        }
        let view = self.build()?;
        if let Some(old) = self.view.replace(view) {
            if crossfade {
                let anim = &self.anim;
                anim.scalar(&old.content, "Opacity", 0.0, THEME_MS, 0, &anim.fade)?;
                self.fading = Some(old.content);
            } else {
                children.Remove(&old.content)?;
            }
        }
        self.restore()?;
        if crossfade {
            if let Some(view) = &self.view {
                let anim = &self.anim;
                view.content.SetOpacity(0.0)?;
                anim.scalar(&view.content, "Opacity", 1.0, THEME_MS, 0, &anim.fade)?;
            }
        }
        Ok(())
    }

    /// Sizes the card to its content, as far as the `work` area allows.
    fn fit(&mut self, work: RECT) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let s = self.scale;
        let room = (work.bottom - work.top) as f32 - 2.0 * (SCREEN_GAP * s).round();
        let w = (CARD_W * s).round();
        let h = (view.header_h + view.content_h).min(room).ceil();
        self.card = (w, h);
        let m = (MARGIN * s).round();
        self.frame.resize(&self.anim, w, h, v3(m, m), s, false)?;
        view.body.SetSize(v2(w, h - view.header_h))?;
        self.scroll = self.scroll.min(self.max_scroll());
        if let Some(view) = &self.view {
            Animator::stop(&view.scroller, &["Offset"]);
            view.scroller.SetOffset(v3(0.0, -self.scroll))?;
        }
        self.show_scroll(false)
    }

    /// Shows the state on a freshly built view, everything at rest.
    fn restore(&mut self) -> Result<()> {
        for sw in SWITCHES {
            self.show_switch(sw, false)?;
            self.show_knob(sw)?;
        }
        let from = self.pill_from.take();
        self.show_theme(false)?;
        // The theme changed with a click on the pill's way over: it carries on here.
        if let (Some(from), Some(view)) = (from, &self.view) {
            let pill = &view.theme.pill;
            pill.SetOffset(view.theme.rest[from])?;
            self.pill_from = Some(from);
            self.show_theme(true)?;
            self.pill_from = None;
        }
        for counter in COUNTERS {
            self.show_count(counter, None)?;
        }
        self.show_caps(false)?;
        let (text, error) = self.detail.clone();
        self.show_detail(&text, error, false)?;
        self.show_recording()?;
        self.show_hover(self.hovered)?;
        self.ring_shown = false;
        self.show_focus(false)?;
        if let Some(view) = &self.view {
            view.scroller.SetOffset(v3(0.0, -self.scroll))?;
        }
        self.show_scroll(false)
    }

    /// Each part rises into place a beat after the one above it.
    fn reveal(&self) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let (anim, drift) = (&self.anim, DRIFT * self.scale);
        for (i, (part, rest)) in view.reveal.iter().enumerate() {
            let delay = REVEAL_DELAY_MS + i as u64 * REVEAL_STAGGER_MS;
            Animator::stop(part, &["Opacity", "Offset"]);
            part.SetOpacity(0.0)?;
            part.SetOffset(v3(rest.X, rest.Y + drift))?;
            anim.scalar(part, "Opacity", 1.0, REVEAL_MS, delay, &anim.decelerate)?;
            anim.vector3(part, "Offset", *rest, REVEAL_MS, delay, &anim.decelerate)?;
        }
        Ok(())
    }

    fn build(&self) -> Result<View> {
        let (c, r, t, s) = (
            &self.anim.compositor,
            &self.renderer,
            &self.theme,
            self.scale,
        );
        let px = |dip: f32| (dip * s).round();
        let card_w = px(CARD_W);
        let content = c.CreateContainerVisual()?;
        self.frame.card.Children()?.InsertAtTop(&content)?;
        let mut reveal: Vec<(Visual, Vector3)> = Vec::new();

        // The header: the heading, the version under it, and the close button.
        let title = Text::new(c, &content)?;
        let heading = r.layout_line("Settings", Face::Heading, Ink::Primary, 300.0)?;
        let title_size = title.show(r, &heading, s, t)?;
        let title_at = v3(px(LABEL_X), px(PAD_TOP));
        title.visual.SetOffset(title_at)?;
        reveal.push((title.visual.cast()?, title_at));
        let version = Text::new(c, &content)?;
        let name = format!("WinDict {}", env!("CARGO_PKG_VERSION"));
        let line = r.layout_line(&name, Face::Small, Ink::Tertiary, 300.0)?;
        let version_size = version.show(r, &line, s, t)?;
        let version_at = v3(title_at.X, title_at.Y + title_size.Y);
        version.visual.SetOffset(version_at)?;
        reveal.push((version.visual.cast()?, version_at));
        let header_h = version_at.Y + version_size.Y + px(HEADER_GAP);

        let size = px(CLOSE_SIZE);
        let close_at = Rect {
            x: card_w - px(CLOSE_INSET) - size,
            y: px(CLOSE_INSET),
            w: size,
            h: size,
        };
        let close = container(c, &content, close_at.at())?;
        let face = Rect {
            w: size,
            h: size,
            ..Rect::default()
        };
        let (close_hover, _) = rounded(c, &close, face, px(CLOSE_RADIUS), t.highlight())?;
        close_hover.SetOpacity(0.0)?;
        let glyph = Text::new(c, &close)?;
        let cross = r.layout_line(CLOSE_GLYPH, Face::Chrome, Ink::Primary, CLOSE_SIZE)?;
        let glyph_size = glyph.show(r, &cross, s, t)?;
        glyph
            .visual
            .SetOffset(v3(0.0, ((size - glyph_size.Y) / 2.0).round()))?;
        reveal.push((close.cast()?, close_at.at()));

        // The groups of settings scroll under the header, should they not fit.
        let body = c.CreateContainerVisual()?;
        body.SetClip(&c.CreateInsetClip()?)?;
        body.SetOffset(v3(0.0, header_h))?;
        // As tall as the card allows; `fit` sizes it again when that changes.
        body.SetSize(v2(card_w, (self.card.1 - header_h).max(0.0)))?;
        content.Children()?.InsertAtTop(&body)?;
        let scroller = container(c, &body, v3(0.0, 0.0))?;

        let (group_x, group_w, row_h) = (px(GROUP_X), card_w - 2.0 * px(GROUP_X), px(ROW_H));
        let mut parts = Parts::default();
        let mut y = 0.0;
        for (i, (name, rows)) in SECTIONS.iter().enumerate() {
            y += if i == 0 { px(2.0) } else { px(SECTION_GAP) };
            let label = Text::new(c, &scroller)?;
            let caps = r.layout_label(name, Ink::Accent, 300.0)?;
            let label_size = label.show(r, &caps, s, t)?;
            let at = v3(px(LABEL_X), y);
            label.visual.SetOffset(at)?;
            reveal.push((label.visual.cast()?, at));
            y += label_size.Y + px(LABEL_GAP);

            let group_at = v3(group_x, y);
            let group_h = rows.len() as f32 * row_h;
            let group = container(c, &scroller, group_at)?;
            let bounds = Rect {
                w: group_w,
                h: group_h,
                ..Rect::default()
            };
            group.SetSize(bounds.size())?;
            let clip = c.CreateRoundedRectangleGeometry()?;
            clip.SetSize(bounds.size())?;
            clip.SetCornerRadius(v2(px(GROUP_RADIUS), px(GROUP_RADIUS)))?;
            group.SetClip(&c.CreateGeometricClipWithGeometry(&clip)?)?;
            rounded(c, &group, bounds, 0.0, t.group())?;
            for (j, spec) in rows.iter().enumerate() {
                let row = Rect {
                    x: 0.0,
                    y: j as f32 * row_h,
                    w: group_w,
                    h: row_h,
                };
                self.build_row(&mut parts, &group, group_at, spec, row, j > 0)?;
            }
            outline(c, &group, bounds, px(GROUP_RADIUS), t.group_stroke())?;
            reveal.push((group.cast()?, group_at));
            y += group_h;
        }

        // The footer: a way to the file behind it all.
        y += px(FOOTER_GAP);
        let footer_at = v3(0.0, y);
        let footer = container(c, &scroller, footer_at)?;
        let link = Text::new(c, &footer)?;
        let link_size = link.show(r, &r.layout_link(CONFIG_LINK)?, s, t)?;
        let link_at = Rect {
            x: px(LABEL_X),
            y: 0.0,
            w: link_size.X,
            h: link_size.Y,
        };
        link.visual.SetOffset(link_at.at())?;
        let link_at = link_at.offset(footer_at);
        parts.hits.push((Hit::Config, link_at.outset(px(4.0))));
        parts
            .stops
            .push((Stop::Config, link_at.outset(px(FOCUS_OUTSET)), px(4.0)));
        reveal.push((footer.cast()?, footer_at));
        y += link_size.Y + px(PAD_BOTTOM);
        let content_h = y;

        // The ring around what the keyboard is on, gliding from stop to stop.
        let ring = c.CreateShapeVisual()?;
        ring.SetSize(v2(card_w, content_h))?;
        ring.SetOpacity(0.0)?;
        let ring_geometry = c.CreateRoundedRectangleGeometry()?;
        let shape = c.CreateSpriteShapeWithGeometry(&ring_geometry)?;
        shape.SetStrokeBrush(&c.CreateColorBrushWithColor(t.focus())?)?;
        shape.SetStrokeThickness(px(FOCUS_STROKE).max(1.0))?;
        ring.Shapes()?.Append(&shape)?;
        scroller.Children()?.InsertAtTop(&ring)?;

        // The divider under the header and the thumb, shown when it scrolls.
        let line = Rect {
            x: 0.0,
            y: header_h - 1.0,
            w: card_w,
            h: 1.0,
        };
        let (divider, _) = rounded(c, &content, line, 0.0, t.separator())?;
        divider.SetOpacity(0.0)?;
        let ink = if t.dark { 0xFF } else { 0x00 };
        let alpha = if t.dark { 0x60 } else { 0x50 };
        let thumb = c.CreateSpriteVisual()?;
        thumb.SetBrush(&c.CreateColorBrushWithColor(argb(alpha, ink, ink, ink))?)?;
        let thumb_clip = c.CreateRoundedRectangleGeometry()?;
        thumb.SetClip(&c.CreateGeometricClipWithGeometry(&thumb_clip)?)?;
        thumb.SetOpacity(0.0)?;
        content.Children()?.InsertAtTop(&thumb)?;

        let missing = || windows::core::Error::from(E_UNEXPECTED);
        Ok(View {
            content,
            header_h,
            content_h,
            close_at,
            close_hover,
            body,
            scroller,
            divider,
            thumb,
            thumb_clip,
            reveal,
            shortcut: parts.shortcut.ok_or_else(missing)?,
            switches: parts
                .switches
                .into_iter()
                .collect::<Option<_>>()
                .ok_or_else(missing)?,
            theme: parts.theme.ok_or_else(missing)?,
            counters: parts
                .counters
                .into_iter()
                .collect::<Option<_>>()
                .ok_or_else(missing)?,
            hits: parts.hits,
            stops: parts.stops,
            ring,
            ring_geometry,
        })
    }

    /// One setting: its name and description on the left, its control on
    /// the right, and a line above unless it comes first. `row` is its place
    /// in the `group`, which sits at `group_at` in the scroller.
    fn build_row(
        &self,
        parts: &mut Parts,
        group: &ContainerVisual,
        group_at: Vector3,
        spec: &Row,
        row: Rect,
        divided: bool,
    ) -> Result<()> {
        let (c, r, t, s) = (
            &self.anim.compositor,
            &self.renderer,
            &self.theme,
            self.scale,
        );
        let px = |dip: f32| (dip * s).round();
        if divided {
            let line = Rect {
                h: s.round().max(1.0),
                ..row
            };
            rounded(c, group, line, 0.0, t.separator())?;
        }
        // Rows that are clicked as a whole light up under the pointer.
        let hover = match spec.control {
            Control::Shortcut | Control::Switch(_) => {
                let (hover, _) = rounded(c, group, row, 0.0, t.highlight())?;
                hover.SetOpacity(0.0)?;
                Some(hover)
            }
            Control::Theme | Control::Counter(_) => None,
        };

        let room = GROUP_W - 2.0 * ROW_PAD - spec.control.width() - CONTROL_GAP;
        let title = Text::new(c, group)?;
        let name = r.layout_line(spec.title, Face::Setting, Ink::Primary, room)?;
        let title_size = title.show(r, &name, s, t)?;
        let detail = Text::new(c, group)?;
        let line = r.layout_line(spec.detail, Face::Small, Ink::Secondary, room)?;
        let detail_size = detail.show(r, &line, s, t)?;
        let block = title_size.Y + px(DETAIL_GAP) + detail_size.Y;
        let text_y = row.y + ((row.h - block) / 2.0).round();
        title.visual.SetOffset(v3(px(ROW_PAD), text_y))?;
        let detail_at = v3(px(ROW_PAD), text_y + title_size.Y + px(DETAIL_GAP));
        detail.visual.SetOffset(detail_at)?;

        let right = row.w - px(ROW_PAD);
        let middle = row.y + row.h / 2.0;
        let place = |rect: Rect| rect.offset(group_at);
        let outset = px(FOCUS_OUTSET);
        match spec.control {
            Control::Shortcut => {
                let edit = Text::new(c, group)?;
                let pen = r.layout_line(EDIT_GLYPH, Face::Icon, Ink::Secondary, EDIT_W)?;
                let size = edit.show(r, &pen, s, t)?;
                let x = right - px(EDIT_W);
                edit.visual
                    .SetOffset(v3(x, (middle - size.Y / 2.0).round()))?;
                let old_detail = Text::new(c, group)?;
                let old_caps = Text::new(c, group)?;
                let caps = Text::new(c, group)?;
                // Rings the row, breathing, while a new shortcut is awaited.
                let ring = c.CreateShapeVisual()?;
                ring.SetOffset(row.at())?;
                ring.SetSize(row.size())?;
                ring.SetOpacity(0.0)?;
                let inset = px(RECORD_INSET);
                let geometry = c.CreateRoundedRectangleGeometry()?;
                geometry.SetOffset(v2(inset, inset))?;
                geometry.SetSize(v2(row.w - 2.0 * inset, row.h - 2.0 * inset))?;
                geometry.SetCornerRadius(v2(px(RECORD_RADIUS), px(RECORD_RADIUS)))?;
                let shape = c.CreateSpriteShapeWithGeometry(&geometry)?;
                shape.SetStrokeBrush(&c.CreateColorBrushWithColor(t.accent)?)?;
                shape.SetStrokeThickness((RECORD_STROKE * s).max(1.0))?;
                ring.Shapes()?.Append(&shape)?;
                group.Children()?.InsertAtTop(&ring)?;

                parts.hits.push((Hit::Shortcut, place(row)));
                let ring_at = place(row).outset(-inset);
                parts
                    .stops
                    .push((Stop::Shortcut, ring_at, px(RECORD_RADIUS)));
                parts.shortcut = Some(ShortcutView {
                    hover: hover.ok_or_else(|| windows::core::Error::from(E_UNEXPECTED))?,
                    caps,
                    old_caps,
                    caps_right: x - px(CAPS_GAP),
                    caps_middle: middle,
                    caps_rest: v3(0.0, 0.0),
                    detail,
                    old_detail,
                    detail_at,
                    detail_w: room,
                    ring,
                });
            }
            Control::Switch(sw) => {
                let (view, track) = self.build_switch(group, right, middle)?;
                parts.hits.push((Hit::Switch(sw), place(row)));
                let radius = track.h / 2.0 + outset;
                parts
                    .stops
                    .push((Stop::Switch(sw), place(track).outset(outset), radius));
                parts.switches[sw as usize] = Some(SwitchView {
                    hover: hover.ok_or_else(|| windows::core::Error::from(E_UNEXPECTED))?,
                    ..view
                });
            }
            Control::Theme => {
                let (view, at, segments) = self.build_theme(group, right, middle)?;
                for (i, segment) in segments.into_iter().enumerate() {
                    parts.hits.push((Hit::Theme(i), place(segment)));
                }
                let radius = px(SEGMENT_RADIUS) + outset;
                parts
                    .stops
                    .push((Stop::Theme, place(at).outset(outset), radius));
                parts.theme = Some(view);
            }
            Control::Counter(counter) => {
                let (view, at, buttons) = self.build_counter(group, right, middle)?;
                for (up, button) in [false, true].into_iter().zip(buttons) {
                    parts.hits.push((Hit::Step(counter, up), place(button)));
                }
                let radius = px(STEP_RADIUS) + outset;
                parts
                    .stops
                    .push((Stop::Counter(counter), place(at).outset(outset), radius));
                parts.counters[counter as usize] = Some(view);
            }
        }
        Ok(())
    }

    /// A switch ending at `right` and centred on `middle`, in `group`.
    /// Returns it (without its row's hover) and its track.
    fn build_switch(
        &self,
        group: &ContainerVisual,
        right: f32,
        middle: f32,
    ) -> Result<(SwitchView, Rect)> {
        let (c, t, s) = (&self.anim.compositor, &self.theme, self.scale);
        let px = |dip: f32| (dip * s).round();
        let (w, h) = (px(SWITCH_W), px(SWITCH_H));
        let track = Rect {
            x: right - w,
            y: (middle - h / 2.0).round(),
            w,
            h,
        };
        let root = container(c, group, track.at())?;
        let face = Rect {
            w,
            h,
            ..Rect::default()
        };
        let off = container(c, &root, v3(0.0, 0.0))?;
        rounded(c, &off, face, h / 2.0, t.sunken())?;
        outline(c, &off, face, h / 2.0, t.strong_stroke())?;
        let (on, _) = rounded(c, &root, face, h / 2.0, t.accent)?;
        let knob = px(KNOB);
        let dot = Rect {
            w: knob,
            h: knob,
            ..Rect::default()
        };
        let (knob_visual, knob_brush) = rounded(c, &root, dot, knob / 2.0, t.knob(false))?;
        knob_visual.SetCenterPoint(Vector3 {
            X: knob / 2.0,
            Y: knob / 2.0,
            Z: 0.0,
        })?;
        let inset = ((h - knob) / 2.0).round();
        let view = SwitchView {
            on,
            off,
            knob: knob_visual,
            knob_brush,
            rest: [v3(inset, inset), v3(w - inset - knob, inset)],
            hover: c.CreateSpriteVisual()?,
        };
        Ok((view, track))
    }

    /// The choice of themes ending at `right` and centred on `middle`, in
    /// `group`. Returns it, its bounds and each option's.
    fn build_theme(
        &self,
        group: &ContainerVisual,
        right: f32,
        middle: f32,
    ) -> Result<(ThemeView, Rect, Vec<Rect>)> {
        let (c, r, t, s) = (
            &self.anim.compositor,
            &self.renderer,
            &self.theme,
            self.scale,
        );
        let px = |dip: f32| (dip * s).round();
        let (segment, h) = (px(SEGMENT_W), px(SEGMENT_H));
        let n = THEMES.len() as f32;
        let at = Rect {
            x: right - n * segment,
            y: (middle - h / 2.0).round(),
            w: n * segment,
            h,
        };
        let root = container(c, group, at.at())?;
        let face = Rect {
            w: at.w,
            h,
            ..Rect::default()
        };
        rounded(c, &root, face, px(SEGMENT_RADIUS), t.sunken())?;
        outline(c, &root, face, px(SEGMENT_RADIUS), t.control_stroke())?;

        let inset = px(PILL_INSET);
        let pill_face = Rect {
            w: segment - 2.0 * inset,
            h: h - 2.0 * inset,
            ..Rect::default()
        };
        let pill = container(c, &root, v3(inset, inset))?;
        pill.SetSize(pill_face.size())?;
        pill.SetCenterPoint(Vector3 {
            X: pill_face.w / 2.0,
            Y: pill_face.h / 2.0,
            Z: 0.0,
        })?;
        rounded(c, &pill, pill_face, px(PILL_RADIUS), t.raised())?;
        outline(c, &pill, pill_face, px(PILL_RADIUS), t.control_stroke())?;

        let mut labels = Vec::new();
        let mut segments = Vec::new();
        let mut rest = Vec::new();
        for (i, (_, name)) in THEMES.iter().enumerate() {
            let x = i as f32 * segment;
            let label = Text::new(c, &root)?;
            let line =
                r.layout_aligned(name, Face::Menu, Ink::Primary, SEGMENT_W, Align::Center)?;
            let size = label.show(r, &line, s, t)?;
            label
                .visual
                .SetOffset(v3(x, ((h - size.Y) / 2.0).round()))?;
            labels.push(label.visual);
            segments.push(Rect {
                x: at.x + x,
                y: at.y,
                w: segment,
                h,
            });
            rest.push(v3(x + inset, inset));
        }
        Ok((ThemeView { pill, labels, rest }, at, segments))
    }

    /// A number between − and + buttons, ending at `right` and centred on
    /// `middle`, in `group`. Returns it, its bounds and its buttons'.
    fn build_counter(
        &self,
        group: &ContainerVisual,
        right: f32,
        middle: f32,
    ) -> Result<(CounterView, Rect, Vec<Rect>)> {
        let (c, r, t, s) = (
            &self.anim.compositor,
            &self.renderer,
            &self.theme,
            self.scale,
        );
        let px = |dip: f32| (dip * s).round();
        let (b, value_w) = (px(STEP_SIZE), px(VALUE_W));
        let at = Rect {
            x: right - 2.0 * b - value_w,
            y: (middle - b / 2.0).round(),
            w: 2.0 * b + value_w,
            h: b,
        };
        let root = container(c, group, at.at())?;
        let face = Rect {
            w: b,
            h: b,
            ..Rect::default()
        };
        let mut buttons = Vec::new();
        let mut rects = Vec::new();
        for (x, glyph) in [(0.0, MINUS_GLYPH), (b + value_w, PLUS_GLYPH)] {
            let visual = container(c, &root, v3(x, 0.0))?;
            visual.SetSize(face.size())?;
            visual.SetCenterPoint(Vector3 {
                X: b / 2.0,
                Y: b / 2.0,
                Z: 0.0,
            })?;
            let radius = px(STEP_RADIUS);
            rounded(c, &visual, face, radius, t.control())?;
            outline(c, &visual, face, radius, t.control_stroke())?;
            let (hover, _) = rounded(c, &visual, face, radius, t.highlight())?;
            hover.SetOpacity(0.0)?;
            let text = Text::new(c, &visual)?;
            let line = r.layout_line(glyph, Face::Chrome, Ink::Primary, STEP_SIZE)?;
            let size = text.show(r, &line, s, t)?;
            text.visual
                .SetOffset(v3(0.0, ((b - size.Y) / 2.0).round()))?;
            buttons.push(StepButton {
                visual,
                hover,
                glyph: text.visual,
            });
            rects.push(Rect {
                x: at.x + x,
                y: at.y,
                w: b,
                h: b,
            });
        }
        let host = container(c, &root, v3(b, 0.0))?;
        host.SetSize(v2(value_w, b))?;
        host.SetClip(&c.CreateInsetClip()?)?;
        let old = Text::new(c, &host)?;
        let value = Text::new(c, &host)?;
        let view = CounterView {
            buttons,
            host,
            value,
            old,
            host_h: b,
        };
        Ok((view, at, rects))
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

    /// Glides the pill to the chosen theme, and brightens its name.
    fn show_theme(&self, animate: bool) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let (v, anim) = (&view.theme, &self.anim);
        let chosen = self.theme_index();
        let rest = v.rest[chosen];
        match (animate, self.pill_from) {
            (true, Some(from)) if from != chosen => {
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
        self.show_theme_labels()
    }

    fn show_theme_labels(&self) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let (anim, chosen) = (&self.anim, self.theme_index());
        for (i, label) in view.theme.labels.iter().enumerate() {
            let opacity = if i == chosen {
                1.0
            } else if self.hovered == Some(Hit::Theme(i)) {
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
            let names = key_names(&self.values.hotkey);
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

    /// Changes the shortcut's description, cross-fading to the new one.
    fn set_detail(&mut self, text: &str, error: bool) -> Result<()> {
        if (text, error) == (self.detail.0.as_str(), self.detail.1) {
            return Ok(());
        }
        self.detail = (text.to_owned(), error);
        self.show_detail(text, error, true)
    }

    fn show_detail(&mut self, text: &str, error: bool, animate: bool) -> Result<()> {
        let (r, anim, t, s) = (&self.renderer, &self.anim, &self.theme, self.scale);
        let Some(view) = self.view.as_mut() else {
            return Ok(());
        };
        let v = &mut view.shortcut;
        std::mem::swap(&mut v.detail, &mut v.old_detail);
        let ink = if error { Ink::Critical } else { Ink::Secondary };
        let line = r.layout_line(text, Face::Small, ink, v.detail_w)?;
        v.detail.show(r, &line, s, t)?;
        let (new, old, rest) = (&v.detail.visual, &v.old_detail.visual, v.detail_at);
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
            v.old_detail.clear();
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
            Some(Hit::Theme(_)) => self.show_theme_labels(),
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

/// The parts of a view as its rows are built.
#[derive(Default)]
struct Parts {
    shortcut: Option<ShortcutView>,
    switches: [Option<SwitchView>; 4],
    theme: Option<ThemeView>,
    counters: [Option<CounterView>; 2],
    hits: Vec<(Hit, Rect)>,
    stops: Vec<(Stop, Rect, f32)>,
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
