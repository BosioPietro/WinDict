//! The introduction: a window shown the first time WinDict starts, a few
//! steps that each have a picture on the left of the card and what they say
//! on the right, with dots for how far along they are and Back and Next
//! under them. It's built like the settings window, a composition
//! visual tree on the card's acrylic: the window rises in, and from step to
//! step the picture cross-fades as it settles, the old text slides away and
//! the new slides in after it, line after line, while the accent dot
//! stretches over to the step it's on.
//!
//! Under what a step says is what can be done there: a box to pick the
//! language WinDict is shown in from, a list of dictionaries to tick, a
//! card opened on an example, or a switch for UI access, or for starting
//! with Windows.
//!
//! The pictures are close-ups of WinDict on the Windows desktop, in light
//! and dark: the one for the system's theme shows.

use std::rc::Rc;

use windows::core::{Interface, Result, HSTRING};
use windows::Win32::Foundation::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::*;
use windows_numerics::Vector3;

use crate::app::edit::{Input, Motion};
use crate::app::picking::Picking;
use crate::dicts;
use crate::hotkey;
use crate::i18n::{fill, tr, Language};
use crate::lang::LangCode;
use crate::render::{Face, Ink, Layout, Moving, Picker, Renderer, Tick, SELECT_H};
use crate::selection;
use crate::theme::Theme;
use crate::ui::picture_card::{
    self, Close, BUTTON_GAP, ENTER_MS, FOCUS_MS, HOVER_MS, INSET, LEAVE_MS, PAD_BOTTOM, PAD_X,
    SHOW_MS, SLIDE, TOP_H,
};
use crate::ui::{
    self, button_width, container, outline, ring, rounded, scale, v2, v3, Animator, Button, Frame,
    List, Morph, Rect, Text, BUTTON_H, FOCUS_OUTSET, MARGIN,
};
use crate::win::{self, Window};

pub(crate) mod scene;

// Geometry, in DIPs.
const CARD_W: f32 = 720.0;
const CARD_H: f32 = 440.0;
const PANE_W: f32 = 328.0;
const PANE_H: f32 = CARD_H - 2.0 * INSET;
/// Where the text starts.
const TEXT_X: f32 = INSET + PANE_W + PAD_X;
const TEXT_W: f32 = CARD_W - TEXT_X - PAD_X;
/// Between what a step says and what can be done there, and under the
/// line saying the shortcut is taken.
const ACTIONS_GAP: f32 = 20.0;
const WARNING_GAP: f32 = 10.0;
/// A step's switch, on a row like the settings window's, and the line
/// under it saying how turning it on went.
const ROW_H: f32 = 56.0;
const ROW_PAD: f32 = 16.0;
const ROW_RADIUS: f32 = 8.0;
const SWITCH_W: f32 = 40.0;
const SWITCH_H: f32 = 20.0;
const KNOB: f32 = 12.0;
const KNOB_HOVER: f32 = 14.0;
const NOTE_GAP: f32 = 10.0;
/// How dim the switch is while Windows asks.
const WAITING_OPACITY: f32 = 0.5;
/// Between what a step says and the box or list to choose from, and the
/// corners of the box and of the list, as the renderer draws them.
const PICKER_GAP: f32 = 12.0;
const SELECT_RADIUS: f32 = 5.0;
const LIST_RADIUS: f32 = 8.0;
/// Under the dictionaries, room for why one didn't download, over the
/// buttons.
const NOTE_ROOM: f32 = 34.0;
/// The dots for the steps: the one for the step showing is longer.
const DOT: f32 = 6.0;
const DOT_ON: f32 = 18.0;
const DOT_GAP: f32 = 6.0;
/// How much larger a picture starts, as it fades in and settles.
const ZOOM: f32 = 1.04;

// Durations, in ms.
/// How long a picture takes to settle as it fades in.
const SETTLE_MS: u64 = 700;
const DOTS_MS: u64 = 300;
const THEME_MS: u64 = 300;
const KNOB_MS: u64 = 300;
const SWITCH_FILL_MS: u64 = 160;
/// Once what's going has faded, it's taken away.
const GONE_MS: u32 = SETTLE_MS as u32 + 30;
const TIMER_GONE: usize = 1;
/// The next of the dots after "Waiting for Windows".
const TIMER_DOTS: usize = 2;

/// A step of the introduction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Step {
    /// The language WinDict is shown in, which it's shown in at once.
    Languages,
    /// The dictionaries to download, which are downloaded once Next says
    /// so.
    Dictionaries,
    /// The lookup shortcut, and the box it opens with nothing selected.
    Lookup,
    /// The translation shortcut, and Ctrl+T on a card.
    Translate,
    /// UI access, which shows the card above Start and Search.
    AboveStart,
    /// Where WinDict is once the window's gone.
    Ready,
}

impl Step {
    /// The steps, in order: the language first, so the rest reads in the
    /// one chosen, the dictionaries once it's clear what they're for, and
    /// the one about UI access only when it can be turned on.
    pub fn all(ui_access: bool) -> Vec<Step> {
        use Step::*;
        [
            Languages,
            Lookup,
            Translate,
            Dictionaries,
            AboveStart,
            Ready,
        ]
        .into_iter()
        .filter(|&step| step != Step::AboveStart || ui_access)
        .collect()
    }

    fn title(self) -> &'static str {
        match self {
            Step::Languages => tr().intro_languages,
            Step::Dictionaries => tr().intro_dictionaries,
            Step::Lookup => tr().intro_lookup,
            Step::Translate => tr().intro_translate,
            Step::AboveStart => tr().intro_above_start,
            Step::Ready => tr().intro_ready,
        }
    }

    /// What the step says, and the keys of each shortcut named in it, which
    /// show as keys.
    fn detail(self, shortcuts: &Shortcuts) -> (&'static str, Vec<(&'static str, Vec<String>)>) {
        match self {
            Step::Languages => (tr().intro_languages_detail, Vec::new()),
            Step::Dictionaries => (tr().intro_dictionaries_detail, Vec::new()),
            Step::Lookup => (
                tr().intro_lookup_detail,
                vec![("hotkey", key_names(&shortcuts.lookup))],
            ),
            Step::Translate => (
                tr().intro_translate_detail,
                vec![
                    ("hotkey", key_names(&shortcuts.translate)),
                    ("card_hotkey", vec!["Ctrl".into(), "T".into()]),
                ],
            ),
            Step::AboveStart => (tr().intro_above_start_detail, Vec::new()),
            Step::Ready => (tr().intro_ready_detail, Vec::new()),
        }
    }

    /// The symbol before the title, in Segoe Fluent Icons and Segoe MDL2
    /// Assets: a globe for the languages, a book for the dictionaries, as
    /// on the shortcuts' rows in the settings for those, layers for the
    /// card above Start, and a tick to finish.
    fn glyph(self) -> &'static str {
        match self {
            Step::Languages => "\u{E774}",
            Step::Dictionaries => "\u{E82D}",
            Step::Lookup => "\u{E721}",
            Step::Translate => "\u{E8C1}",
            Step::AboveStart => "\u{E81E}",
            Step::Ready => "\u{E73E}",
        }
    }

    /// The picture drawn live, in the language WinDict's shown in: the
    /// languages have a translation's, and the dictionaries a definition's.
    fn scene(self) -> Option<scene::Shot> {
        match self {
            Step::Lookup | Step::Dictionaries => Some(scene::Shot::Lookup),
            Step::Translate | Step::Languages => Some(scene::Shot::Translate),
            Step::AboveStart | Step::Ready => None,
        }
    }

    /// The picture, in light or `dark`, of a step that isn't drawn live.
    fn picture(self, dark: bool) -> &'static [u8] {
        match (self, dark) {
            (Step::AboveStart, false) => include_bytes!("../assets/intro/start-light.jpg"),
            (Step::AboveStart, true) => include_bytes!("../assets/intro/start-dark.jpg"),
            (_, false) => include_bytes!("../assets/intro/tray-light.jpg"),
            (_, true) => include_bytes!("../assets/intro/tray-dark.jpg"),
        }
    }
}

/// The keys of the shortcut `text`, as written when it can't be read.
fn key_names(text: &str) -> Vec<String> {
    match hotkey::parse(text) {
        Ok(hotkey) => hotkey.names(),
        Err(_) => vec![text.to_owned()],
    }
}

/// The shortcuts the steps name, as the settings have them.
#[derive(Clone, Default)]
pub struct Shortcuts {
    pub lookup: String,
    pub translate: String,
}

/// What the steps offer, as things stand.
#[derive(Clone, Copy, Default)]
pub struct Options {
    /// Another app has the lookup shortcut.
    pub shortcut_taken: bool,
    /// Translations can be had: they come from the internet, like the
    /// online answers.
    pub online: bool,
    /// The card shows above Start and Search.
    pub ui_access: bool,
    pub autostart: bool,
    /// The language WinDict is shown in.
    pub interface: Language,
}

/// Something done in the window, for the app to carry out.
#[derive(Debug, PartialEq)]
pub enum Action {
    /// Done, or closed: either way, it's been seen.
    Close,
    /// A card on an example, as the step's shortcut would open it.
    ShowMe(Step),
    /// The settings, to choose another lookup shortcut.
    ChooseShortcut,
    /// Turn UI access on or off. The switch waits, until
    /// [`Intro::settle_ui_access`].
    UiAccess(bool),
    Autostart(bool),
    /// Show WinDict in this language. The window follows, with
    /// [`Intro::update`].
    Interface(Language),
    /// Look words up in these languages, besides English: Next was pressed
    /// on the dictionaries' step, which downloads those that aren't there.
    /// How that goes is told with [`Intro::show_download`].
    Dictionaries(Vec<&'static str>),
}

/// What's under the pointer, or has the keyboard.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Hit {
    Close,
    Back,
    Next,
    ShowMe,
    ChooseShortcut,
    /// The step's switch, and its row.
    Switch,
    /// The box the language WinDict is shown in is picked from.
    Select,
    /// The list of dictionaries to tick, which has the keyboard as one.
    List,
    /// A row of the list showing: a language that matches what's typed
    /// in the box, or a dictionary, English first.
    Row(usize),
}

/// A step's switch.
struct SwitchView {
    /// Dimmed while Windows asks.
    root: ContainerVisual,
    /// The accent fill of a switch that's on, over the outline of one that's off.
    on: SpriteVisual,
    off: ContainerVisual,
    knob: SpriteVisual,
    knob_brush: CompositionColorBrush,
    /// The knob's place when off and when on.
    rest: [Vector3; 2],
}

/// What a step shows on the right of the card, which slides as the step
/// changes.
struct StepView {
    /// Its title, what it says and what can be done there.
    text: ContainerVisual,
    /// What can be clicked in it: buttons, or the switch's row.
    controls: Vec<(Hit, Button)>,
    switch: Option<SwitchView>,
    /// Under the switch for UI access: how turning it on is going. Under
    /// the dictionaries: why one didn't download.
    note: Option<Text>,
    /// The box to pick a language in, or the list of dictionaries.
    picker: Option<PickerView>,
}

/// The box to pick the language WinDict is shown in, with the list under
/// it while it's open, or the list of dictionaries to tick: the list's
/// rows are the intro's [`List`], shown over it.
struct PickerView {
    text: Text,
    /// What the box showed before it opened or shut.
    morph: Morph,
    /// In the box, while what's typed narrows the list.
    caret: SpriteVisual,
    /// Where it is on the card, in pixels.
    at: Vector3,
    /// How many of the list's rows show at once.
    rows: usize,
}

/// A dot for a step.
struct DotView {
    shape: CompositionRoundedRectangleGeometry,
    brush: CompositionColorBrush,
}

struct View {
    /// Everything; fades as one when the theme changes.
    content: ContainerVisual,
    /// Holds the picture, which fills it.
    pane: ContainerVisual,
    picture: SpriteVisual,
    step: StepView,
    close: Close,
    dots: Vec<DotView>,
    /// The buttons, which change at once.
    footer: ContainerVisual,
    /// Back, when there's a step before, and Next.
    buttons: Vec<(Hit, Button)>,
}

pub struct Intro {
    pub hwnd: HWND,
    _target: DesktopWindowTarget,
    renderer: Rc<Renderer>,
    anim: Animator,
    /// Everything on the window; fades and scales as one when showing and hiding.
    root: ContainerVisual,
    frame: Frame,
    view: Option<View>,
    /// What's on its way out, fading: steps, and views of an older theme.
    gone: Vec<Visual>,
    /// The language the view says what it says in.
    built_in: Language,
    /// While the box picking the language moves over the view, shown in
    /// another, to where it goes in it: what's in its place there.
    landing: Option<Visual>,
    steps: Vec<Step>,
    /// Which of them shows.
    at: usize,
    shortcuts: Shortcuts,
    options: Options,
    /// Windows is asking whether UI access may change.
    waiting: bool,
    /// How turning UI access on went, and whether it failed.
    note: Option<(String, bool)>,
    /// How many steps the dots have taken while Windows asks.
    dots: usize,
    /// The list of languages open under the box, and what's typed in it.
    picking: Option<Picking<()>>,
    /// The rows of the list open under the box, or of the dictionaries.
    list: List,
    /// The dictionary's row the keyboard is on, or the pointer was last.
    tick_at: Option<usize>,
    /// The dictionaries ticked, and those Next has already said to have:
    /// when there are more of the first, Next says Download.
    chosen: Vec<&'static str>,
    wanted: Vec<&'static str>,
    /// How the dictionaries' downloads are going, and whether they failed.
    downloads: Vec<(&'static str, String, bool)>,
    theme: Theme,
    scale: f32,
    hovered: Option<Hit>,
    pressed: Option<Hit>,
    focus: Option<Hit>,
    /// Whether the focus ring shows: only once the keyboard is used.
    ring: bool,
    pub visible: bool,
    /// Last, so it goes after everything shown on it.
    _window: Window,
}

impl Intro {
    pub fn new(
        window: Window,
        compositor: Compositor,
        renderer: Rc<Renderer>,
        theme: Theme,
    ) -> Result<Self> {
        let hwnd = window.hwnd();
        let target = ui::attach(hwnd, &compositor)?;
        let root = compositor.CreateContainerVisual()?;
        target.SetRoot(&root)?;
        let frame = Frame::new(&compositor, &root, &theme)?;
        let list = List::new(&compositor)?;
        list.apply_theme(&theme)?;
        Ok(Intro {
            hwnd,
            _target: target,
            renderer,
            anim: Animator::new(compositor)?,
            root,
            frame,
            view: None,
            gone: Vec::new(),
            built_in: crate::i18n::current(),
            landing: None,
            steps: Step::all(false),
            at: 0,
            shortcuts: Shortcuts::default(),
            options: Options::default(),
            waiting: false,
            note: None,
            dots: 0,
            picking: None,
            list,
            tick_at: None,
            chosen: Vec::new(),
            wanted: Vec::new(),
            downloads: Vec::new(),
            theme,
            scale: 1.0,
            hovered: None,
            pressed: None,
            focus: None,
            ring: false,
            visible: false,
            _window: window,
        })
    }

    // -----------------------------------------------------------------------
    // Public API

    /// Opens the window on `start`, or else its first step, in the middle
    /// of the screen `near` (screen pixels) is on.
    pub fn show(
        &mut self,
        near: POINT,
        theme: Theme,
        steps: Vec<Step>,
        start: Step,
        shortcuts: Shortcuts,
        options: Options,
    ) -> Result<()> {
        let (work, s) = ui::monitor_at(near);
        let s = fitted(work, s);
        self.at = steps.iter().position(|&step| step == start).unwrap_or(0);
        (self.theme, self.scale, self.steps, self.shortcuts) = (theme, s, steps, shortcuts);
        (self.options, self.waiting, self.note) = (options, false, None);
        self.downloads.clear();
        (self.picking, self.tick_at) = (None, None);
        (self.hovered, self.pressed, self.focus) = (None, None, None);
        self.ring = false;
        self.retitle();
        self.frame.apply_theme(&self.renderer, &theme, s)?;
        self.replace_view(false)?;
        let (w, h) = self.fit()?;
        let left = (work.left + work.right - w) / 2;
        let top = (work.top + work.bottom - h) / 2;

        // Rises out of the middle, a little small and low, its text a beat
        // after it.
        let root = &self.root;
        Animator::stop(root, &["Opacity", "Offset", "Scale"]);
        root.SetCenterPoint(self.centre())?;
        root.SetOpacity(0.0)?;
        root.SetScale(scale(0.96))?;
        root.SetOffset(v3(0.0, 12.0 * s))?;
        unsafe {
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOP), left, top, w, h, SWP_SHOWWINDOW);
        }
        self.visible = true;
        self.anim.fade_in(root, ui::FADE_IN_MS, SHOW_MS)?;
        if let Some(view) = &self.view {
            picture_card::bring_in(
                &self.anim,
                &view.step.text,
                v3(0.0, SLIDE / 3.0 * s),
                ui::FADE_IN_MS,
            )?;
        }
        Ok(())
    }

    /// The shortcuts or what the steps offer changed, while the window's
    /// open: it shows them, as it does the language now current.
    pub fn update(&mut self, shortcuts: Shortcuts, options: Options) -> Result<()> {
        // What the switch was turned to holds while Windows asks.
        let ui_access = if self.waiting {
            self.options.ui_access
        } else {
            options.ui_access
        };
        self.shortcuts = shortcuts;
        self.options = Options {
            ui_access,
            ..options
        };
        self.retitle();
        self.replace_view(false)
    }

    /// UI access is `on`, as turning it on or off left it, and `note` says
    /// how that went, and whether it failed.
    pub fn settle_ui_access(&mut self, on: bool, note: Option<(String, bool)>) -> Result<()> {
        (self.options.ui_access, self.waiting, self.note) = (on, false, note);
        win::kill_timer(self.hwnd, TIMER_DOTS);
        self.show_switch(true)?;
        self.show_note()
    }

    /// Words are looked up in `languages`, or will be once they're
    /// downloaded: their dictionaries are ticked, and only those.
    pub fn show_languages(&mut self, languages: &[LangCode]) -> Result<()> {
        let used = |code: &&str| languages.iter().any(|l| l.as_str() == *code);
        self.wanted = dicts::DOWNLOADS
            .iter()
            .map(|d| d.language)
            .filter(used)
            .collect();
        self.chosen = self.wanted.clone();
        if self.steps.get(self.at) == Some(&Step::Dictionaries) {
            self.show_footer()?;
            self.show_picker()?;
        }
        Ok(())
    }

    /// Says how `language`'s download is going, beside it, or, when it
    /// failed, why, under the dictionaries. `None` once it's over.
    pub fn show_download(&mut self, language: &str, status: Option<(&str, bool)>) -> Result<()> {
        self.downloads.retain(|(code, ..)| *code != language);
        if let (Some(download), Some((note, failed))) = (dicts::find(language), status) {
            self.downloads
                .push((download.language, note.to_owned(), failed));
        }
        self.show_picker()?;
        self.show_note()
    }

    /// The card, in screen pixels.
    pub fn card_rect(&self) -> RECT {
        let mut rect = RECT::default();
        unsafe {
            let _ = GetWindowRect(self.hwnd, &mut rect);
        }
        let m = (MARGIN * self.scale).round() as i32;
        RECT {
            left: rect.left + m,
            top: rect.top + m,
            right: rect.right - m,
            bottom: rect.bottom - m,
        }
    }

    /// Fades the window out. The caller hides it after [`ui::HIDE_MS`].
    pub fn begin_hide(&mut self) -> Result<()> {
        if !self.visible {
            return Ok(());
        }
        self.visible = false;
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

    /// Whether a point in window pixels is on the strip along the top, which
    /// drags the window.
    pub fn caption_at(&self, x: i32, y: i32) -> bool {
        let Some(view) = &self.view else {
            return false;
        };
        let (x, y) = self.card_point(x, y);
        let px = |dip: f32| (dip * self.scale).round();
        x >= 0.0 && x < px(CARD_W) && y >= 0.0 && y < px(TOP_H) && !view.close.at.contains(x, y)
    }

    /// The pointer moved to a point in window pixels, or left.
    pub fn hover(&mut self, point: Option<(i32, i32)>) -> Result<()> {
        // On the list's scrollbar, or dragging its thumb, nothing else is
        // hovered.
        let in_list = point.and_then(|(x, y)| self.list_point(x, y));
        let on_bar = self.list.hover_bar(&self.anim, in_list)?;
        let point = if on_bar { None } else { point };
        let hit = point.and_then(|(x, y)| self.hit_at(x, y));
        if hit == self.hovered {
            return Ok(());
        }
        let old = std::mem::replace(&mut self.hovered, hit);
        self.show_hover(old)?;
        self.show_hover(hit)?;
        // Over a list, the highlight follows the pointer.
        if let Some(picking) = self.picking.as_mut() {
            let code = match hit {
                Some(Hit::Row(i)) => picking.matches.get(i).copied(),
                _ => None,
            };
            if code.is_some_and(|code| picking.highlight(code)) {
                let row = picking.highlight;
                self.list.set_highlight(&self.anim, row)?;
            }
            return Ok(());
        }
        if let Some(Hit::Row(i)) = hit {
            self.tick_at = Some(i);
        }
        self.show_tick_highlight()
    }

    /// The left button went down at a point in window pixels.
    pub fn press(&mut self, x: i32, y: i32) -> Result<()> {
        if let Some((lx, ly)) = self.list_point(x, y) {
            if self.list.press(&self.anim, lx, ly)? {
                return Ok(());
            }
        }
        let hit = self.hit_at(x, y);
        // The pointer's in charge now: the ring goes until Tab brings it back.
        self.ring = false;
        if let Some(stop) = hit.and_then(|hit| self.stop(hit)) {
            self.focus = Some(stop);
        }
        // A click away from the open box shuts its list.
        let away = !matches!(hit, Some(Hit::Select | Hit::List | Hit::Row(_)));
        if away && self.picking.take().is_some() {
            self.show_picker()?;
        }
        self.show_focus()?;
        self.show_tick_highlight()?;
        self.set_pressed(hit)
    }

    /// The left button came up at a point in window pixels: a click, if it's
    /// on what it went down on.
    pub fn release(&mut self, x: i32, y: i32) -> Result<Option<Action>> {
        if self.list.release(&self.anim)? {
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
    pub fn key(&mut self, key: VIRTUAL_KEY) -> Result<Option<Action>> {
        if self.picking.is_some() {
            return self.picker_key(key);
        }
        let shift = hotkey::is_held(VK_SHIFT);
        match (key, self.focus) {
            (VK_ESCAPE, _) => Ok(Some(Action::Close)),
            // The box opens as a combo box does.
            (VK_RETURN | VK_SPACE | VK_F4 | VK_DOWN | VK_UP, Some(Hit::Select)) => {
                self.ring = true;
                self.activate(Hit::Select)
            }
            // In the dictionaries, the arrows go from row to row, Space
            // ticks the one they're on, and Enter goes on, as elsewhere.
            (VK_DOWN | VK_UP, Some(Hit::List)) => {
                let last = dicts::DOWNLOADS.len();
                let at = self.tick_at.unwrap_or(0);
                self.tick_at = Some(match key {
                    VK_DOWN => (at + 1).min(last),
                    _ => at.saturating_sub(1),
                });
                self.ring = true;
                self.show_focus()?;
                self.show_picker()?;
                Ok(None)
            }
            (VK_SPACE, Some(Hit::List)) => {
                let at = *self.tick_at.get_or_insert(0);
                self.ring = true;
                self.activate(Hit::Row(at))
            }
            (VK_RETURN, Some(Hit::List)) => self.activate(Hit::Next),
            (VK_LEFT, _) if self.at > 0 => self.activate(Hit::Back),
            (VK_RIGHT, _) if self.at + 1 < self.steps.len() => self.activate(Hit::Next),
            (VK_RETURN | VK_SPACE, focus) => self.activate(focus.unwrap_or(Hit::Next)),
            (VK_TAB, _) => {
                self.step_focus(!shift)?;
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    /// A character was typed: while the box has the keyboard, it narrows
    /// the list of languages, opening it first.
    pub fn char(&mut self, unit: u16) -> Result<Option<Action>> {
        if self.steps[self.at] != Step::Languages {
            return Ok(None);
        }
        if self.picking.is_none() {
            // Space opened it already, and control characters don't type.
            if self.focus != Some(Hit::Select) || unit <= 0x20 || unit == 0x7F {
                return Ok(None);
            }
            self.open_picker();
        }
        let hwnd = self.hwnd;
        let current = self.options.interface.code();
        let Some(picking) = self.picking.as_mut() else {
            return Ok(None);
        };
        // What Space opened the box with isn't looked for.
        if unit == 0x20 && picking.query.text().is_empty() {
            return Ok(None);
        }
        match picking.query.type_unit(unit) {
            Input::Edited => {}
            Input::Paste => {
                let pasted = selection::clipboard_text(hwnd).unwrap_or_default();
                picking.query.paste(&pasted);
            }
            // The key going down picked it already.
            Input::Enter | Input::Nothing => return Ok(None),
        }
        picking.refilter(current);
        self.show_picker()?;
        Ok(None)
    }

    /// Scrolls the list the pointer's on, at a point in window pixels, by
    /// a mouse wheel delta.
    pub fn wheel(&mut self, delta: i32, (x, y): (i32, i32)) -> Result<()> {
        let in_list = self.list_point(x, y);
        if !in_list.is_some_and(|(x, y)| self.list.contains(x, y)) {
            return Ok(());
        }
        self.list.wheel(&self.anim, delta)?;
        // The highlight moves to the row the pointer's on now.
        self.hover(Some((x, y)))
    }

    /// The window stopped being the active one.
    pub fn deactivate(&mut self) -> Result<()> {
        self.list.release(&self.anim)?;
        self.set_pressed(None)
    }

    /// Timer `id` went off: what was fading out has, and goes, or the dots
    /// take a step.
    pub fn on_timer(&mut self, id: usize) -> Result<()> {
        if id == TIMER_DOTS {
            self.dots += 1;
            let waiting = ui::waiting(tr().intro_ui_access_waiting, self.dots);
            self.note = Some((waiting, false));
            return self.show_note();
        }
        win::kill_timer(self.hwnd, TIMER_GONE);
        self.land()?;
        self.release_list()?;
        for gone in self.gone.drain(..) {
            if let Ok(parent) = gone.Parent() {
                parent.Children()?.Remove(&gone)?;
            }
        }
        Ok(())
    }

    /// Switches to `theme`, cross-fading everything.
    pub fn retheme(&mut self, theme: Theme) -> Result<()> {
        if theme == self.theme {
            return Ok(());
        }
        self.theme = theme;
        self.list.apply_theme(&theme)?;
        let (anim, renderer, s) = (&self.anim, &self.renderer, self.scale);
        self.frame.fade_theme(anim, renderer, &theme, s, THEME_MS)?;
        self.replace_view(true)
    }

    /// The GPU went away, and the renderer draws with another: draws
    /// everything again.
    pub fn recover(&mut self) -> Result<()> {
        self.frame
            .apply_theme(&self.renderer, &self.theme, self.scale)?;
        self.replace_view(false)
    }

    /// The window moved to a monitor with another `dpi`. `suggested` is
    /// where Windows would put it, in screen pixels.
    pub fn rescale(&mut self, dpi: u32, suggested: RECT) -> Result<()> {
        let centre = POINT {
            x: (suggested.left + suggested.right) / 2,
            y: (suggested.top + suggested.bottom) / 2,
        };
        let work = ui::monitor_at(centre).0;
        let s = fitted(work, dpi as f32 / 96.0);
        if (s - self.scale).abs() < 0.01 {
            return Ok(());
        }
        self.scale = s;
        self.frame.apply_theme(&self.renderer, &self.theme, s)?;
        self.replace_view(false)?;
        self.root.SetCenterPoint(self.centre())?;
        let (w, h) = self.fit()?;
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                None,
                suggested.left,
                suggested.top,
                w,
                h,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // What clicks and keys do

    fn activate(&mut self, hit: Hit) -> Result<Option<Action>> {
        match hit {
            Hit::Close => return Ok(Some(Action::Close)),
            Hit::Back if self.at > 0 => self.go(self.at - 1)?,
            Hit::Next if self.at + 1 < self.steps.len() => {
                let confirmed = self.confirm();
                self.go(self.at + 1)?;
                return Ok(confirmed);
            }
            Hit::Next => return Ok(Some(Action::Close)),
            Hit::Back => {}
            Hit::ShowMe => return Ok(Some(Action::ShowMe(self.steps[self.at]))),
            Hit::ChooseShortcut => return Ok(Some(Action::ChooseShortcut)),
            Hit::Switch => return self.flip(),
            Hit::Select => {
                if self.picking.take().is_none() {
                    self.open_picker();
                }
                self.focus = Some(Hit::Select);
                self.show_picker()?;
            }
            Hit::List => {}
            Hit::Row(i) => return self.choose(i),
        }
        Ok(None)
    }

    /// Opens the list of languages WinDict can be shown in under the box,
    /// the current one highlighted.
    fn open_picker(&mut self) {
        let current = self.options.interface.code();
        let shown = |code: &str| Language::from_code(code).is_some();
        self.picking = Some(Picking::among((), current, shown));
    }

    /// Picks the language on row `i` of the list open under the box, or
    /// ticks the dictionary on it, or unticks it: English, on the first,
    /// comes with WinDict.
    fn choose(&mut self, i: usize) -> Result<Option<Action>> {
        if let Some(picking) = self.picking.take() {
            let language = (picking.matches.get(i).copied())
                .and_then(Language::from_code)
                .filter(|&l| l != self.options.interface);
            self.focus = Some(Hit::Select);
            self.show_picker()?;
            // The window follows, once the app's shown WinDict in it.
            return Ok(language.map(|l| {
                self.options.interface = l;
                Action::Interface(l)
            }));
        }
        let download = i.checked_sub(1).and_then(|i| dicts::DOWNLOADS.get(i));
        if self.steps[self.at] != Step::Dictionaries {
            return Ok(None);
        }
        let Some(language) = download.map(|d| d.language) else {
            return Ok(None);
        };
        self.tick_at = Some(i);
        if self.chosen.contains(&language) {
            self.chosen.retain(|&l| l != language);
        } else {
            self.chosen.push(language);
        }
        // The tick, and Next turning to Download, or back.
        self.show_picker()?;
        self.show_footer()?;
        Ok(None)
    }

    /// A key pressed while the list of languages is open under the box:
    /// Up, Down, Page Up and Page Down move the highlight, Enter picks it,
    /// Esc and Tab shut the list, and the others move the caret in the box.
    fn picker_key(&mut self, key: VIRTUAL_KEY) -> Result<Option<Action>> {
        let ctrl = hotkey::is_held(VK_CONTROL);
        let current = self.options.interface.code();
        let rows = self.picker_rows() as isize;
        let Some(picking) = self.picking.as_mut() else {
            return Ok(None);
        };
        match key {
            VK_ESCAPE | VK_F4 => {
                self.picking = None;
                self.show_picker()?;
                return Ok(None);
            }
            VK_TAB => {
                self.picking = None;
                self.show_picker()?;
                self.step_focus(!hotkey::is_held(VK_SHIFT))?;
                return Ok(None);
            }
            VK_RETURN => {
                let row = picking.highlight;
                return match row {
                    Some(row) => self.choose(row),
                    None => Ok(None),
                };
            }
            _ => {}
        }
        let step = match key {
            VK_DOWN => Some(1),
            VK_UP => Some(-1),
            VK_NEXT => Some(rows),
            VK_PRIOR => Some(-rows),
            _ => None,
        };
        if let Some(by) = step {
            picking.step(by);
            self.show_picker()?;
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
                    picking.refilter(current);
                }
                true
            }
            _ => false,
        };
        if changed {
            self.show_picker()?;
        }
        Ok(None)
    }

    /// Leaving the dictionaries' step with Next: the ones ticked are the
    /// ones to have, if that's changed.
    fn confirm(&mut self) -> Option<Action> {
        let same = self.chosen.len() == self.wanted.len()
            && self.chosen.iter().all(|l| self.wanted.contains(l));
        if self.steps[self.at] != Step::Dictionaries || same {
            return None;
        }
        self.wanted = self.chosen.clone();
        Some(Action::Dictionaries(self.chosen.clone()))
    }

    /// Next would download a dictionary that's been ticked.
    fn to_download(&self) -> bool {
        self.steps[self.at] == Step::Dictionaries
            && self.chosen.iter().any(|l| !self.wanted.contains(l))
    }

    /// Turns the step's switch over. The one for UI access waits for
    /// Windows to ask, and can't be turned again until it has.
    fn flip(&mut self) -> Result<Option<Action>> {
        let action = match self.steps[self.at] {
            Step::AboveStart if !self.waiting => {
                self.options.ui_access = !self.options.ui_access;
                (self.waiting, self.dots) = (true, 0);
                self.note = Some((ui::waiting(tr().intro_ui_access_waiting, 0), false));
                win::set_timer(self.hwnd, TIMER_DOTS, ui::DOTS_MS);
                Action::UiAccess(self.options.ui_access)
            }
            Step::Ready => {
                self.options.autostart = !self.options.autostart;
                Action::Autostart(self.options.autostart)
            }
            _ => return Ok(None),
        };
        self.show_switch(true)?;
        self.show_note()?;
        Ok(Some(action))
    }

    /// Whether the step's switch is on.
    fn switch_on(&self) -> bool {
        match self.steps[self.at] {
            Step::AboveStart => self.options.ui_access,
            Step::Ready => self.options.autostart,
            Step::Languages | Step::Dictionaries | Step::Lookup | Step::Translate => false,
        }
    }

    /// Shows step `to`: its picture fades in over the old one as it settles,
    /// the old text slides away and fades, and the new slides in after it
    /// from the side `to` is on.
    fn go(&mut self, to: usize) -> Result<()> {
        self.land()?;
        let forward = to > self.at;
        self.at = to;
        (self.picking, self.tick_at) = (None, None);
        let Some(view) = self.view.take() else {
            return Ok(());
        };
        let (anim, s) = (&self.anim, self.scale);
        let dx = if forward { SLIDE * s } else { -SLIDE * s };

        let old = view.step.text.clone();
        Animator::stop(&old, &["Opacity", "Offset"]);
        anim.scalar(&old, "Opacity", 0.0, LEAVE_MS, 0, &anim.accelerate)?;
        anim.vector3(&old, "Offset", v3(-dx, 0.0), LEAVE_MS, 0, &anim.accelerate)?;
        self.gone.push(old.cast()?);
        self.gone.push(view.picture.cast()?);
        win::set_timer(self.hwnd, TIMER_GONE, GONE_MS);

        let picture = self.build_picture(&view.pane)?;
        picture.SetOpacity(0.0)?;
        picture.SetScale(scale(ZOOM))?;
        let anim = &self.anim;
        anim.scalar(&picture, "Opacity", 1.0, ENTER_MS, 0, &anim.fade)?;
        anim.vector3(
            &picture,
            "Scale",
            scale(1.0),
            SETTLE_MS,
            0,
            &anim.decelerate,
        )?;
        // Back comes and goes, and Next turns to Done. The step goes over
        // the buttons, for a list open over them.
        view.content.Children()?.Remove(&view.footer)?;
        let (footer, buttons) = self.build_footer(&view.content)?;
        let step = self.build_step(&view.content)?;
        picture_card::bring_in(&self.anim, &step.text, v3(dx, 0.0), LEAVE_MS)?;
        self.show_dots(&view.dots, true)?;
        self.view = Some(View {
            picture,
            step,
            footer,
            buttons,
            ..view
        });
        self.keep_focus();
        self.show_switch(false)?;
        self.show_note()?;
        self.show_picker()?;
        self.show_hover(self.hovered)?;
        self.show_focus()
    }

    /// Builds the buttons that go through the steps again, for what Next
    /// says, where they were.
    fn show_footer(&mut self) -> Result<()> {
        let Some(view) = self.view.take() else {
            return Ok(());
        };
        let children = view.content.Children()?;
        let (footer, buttons) = self.build_footer(&view.content)?;
        children.Remove(&footer)?;
        children.InsertAbove(&footer, &view.footer)?;
        children.Remove(&view.footer)?;
        self.view = Some(View {
            footer,
            buttons,
            ..view
        });
        self.keep_focus();
        self.show_hover(self.hovered)?;
        self.show_focus()
    }

    /// What has the keyboard once `hit` is clicked: the box for its list,
    /// and the dictionaries as one for a row of them.
    fn stop(&self, hit: Hit) -> Option<Hit> {
        match hit {
            Hit::Close => None,
            Hit::Row(_) | Hit::List if self.picking.is_some() => Some(Hit::Select),
            Hit::Row(_) => Some(Hit::List),
            hit => Some(hit),
        }
    }

    /// What had the keyboard may be gone: Next has it then.
    fn keep_focus(&mut self) {
        if self
            .focus
            .is_some_and(|f| !self.targets().any(|(hit, _)| *hit == f))
        {
            self.focus = Some(Hit::Next);
        }
    }

    /// Moves the keyboard to the next (or previous) button, going round.
    fn step_focus(&mut self, forward: bool) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        // What the step offers, then the buttons from the left.
        let mut footer: Vec<&(Hit, Button)> = view.buttons.iter().collect();
        footer.sort_by(|a, b| a.1.at.x.total_cmp(&b.1.at.x));
        let stops: Vec<Hit> = (view.step.controls.iter().chain(footer))
            .map(|(hit, _)| *hit)
            .collect();
        let n = stops.len();
        let at = self.focus.and_then(|f| stops.iter().position(|&s| s == f));
        // The first Tab shows where the focus is, if the pointer put it somewhere.
        let next = match (at, forward, self.ring) {
            (Some(i), _, false) => i,
            (None, true, _) => 0,
            (None, false, _) => n - 1,
            (Some(i), true, true) => (i + 1) % n,
            (Some(i), false, true) => (i + n - 1) % n,
        };
        (self.focus, self.ring) = (Some(stops[next]), true);
        self.show_focus()?;
        self.show_tick_highlight()
    }

    fn set_pressed(&mut self, hit: Option<Hit>) -> Result<()> {
        if hit == self.pressed {
            return Ok(());
        }
        let old = std::mem::replace(&mut self.pressed, hit);
        for (hit, down) in [(old, false), (hit, true)] {
            if let Some(button) = hit.and_then(|hit| self.button(hit)) {
                picture_card::show_pressed(&self.anim, button, down)?;
            }
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Showing the state

    /// Lights `hit` up if it's under the pointer, or puts it out.
    fn show_hover(&self, hit: Option<Hit>) -> Result<()> {
        let (Some(view), Some(hit)) = (&self.view, hit) else {
            return Ok(());
        };
        let hover = match hit {
            Hit::Close => &view.close.hover,
            // The list lights its rows itself.
            Hit::List | Hit::Row(_) => return Ok(()),
            _ => match self.button(hit) {
                Some(button) => &button.hover,
                None => return Ok(()),
            },
        };
        let on = self.hovered == Some(hit);
        let anim = &self.anim;
        anim.scalar(
            hover,
            "Opacity",
            f32::from(u8::from(on)),
            HOVER_MS,
            0,
            &anim.decelerate,
        )?;
        // The knob grows under the pointer.
        if let Some(switch) = view.step.switch.as_ref().filter(|_| hit == Hit::Switch) {
            let k = if on { KNOB_HOVER / KNOB } else { 1.0 };
            anim.vector3(
                &switch.knob,
                "Scale",
                scale(k),
                KNOB_MS / 2,
                0,
                &anim.decelerate,
            )?;
        }
        Ok(())
    }

    fn show_focus(&self) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let anim = &self.anim;
        for (hit, button) in view.step.controls.iter().chain(&view.buttons) {
            let on = self.ring && self.focus == Some(*hit);
            let to = if on { 1.0 } else { 0.0 };
            anim.scalar(&button.ring, "Opacity", to, FOCUS_MS, 0, &anim.decelerate)?;
        }
        Ok(())
    }

    /// Turns the step's switch on or off, as it is, dimmed while Windows
    /// asks.
    fn show_switch(&self, animate: bool) -> Result<()> {
        let Some(v) = self
            .view
            .as_ref()
            .and_then(|view| view.step.switch.as_ref())
        else {
            return Ok(());
        };
        let on = self.switch_on();
        let anim = &self.anim;
        let rest = v.rest[usize::from(on)];
        let (fill, color) = (f32::from(u8::from(on)), self.theme.knob(on));
        let waiting = self.waiting && self.steps[self.at] == Step::AboveStart;
        let opacity = if waiting { WAITING_OPACITY } else { 1.0 };
        if animate {
            let ms = SWITCH_FILL_MS;
            anim.vector3(&v.knob, "Offset", rest, KNOB_MS, 0, &anim.overshoot)?;
            anim.scalar(&v.on, "Opacity", fill, ms, 0, &anim.decelerate)?;
            anim.scalar(&v.off, "Opacity", 1.0 - fill, ms, 0, &anim.decelerate)?;
            anim.scalar(&v.root, "Opacity", opacity, ms, 0, &anim.decelerate)?;
            anim.color(&v.knob_brush, v.knob_brush.Color()?, color, ms)
        } else {
            v.knob.SetOffset(rest)?;
            v.on.SetOpacity(fill)?;
            v.off.SetOpacity(1.0 - fill)?;
            v.root.SetOpacity(opacity)?;
            v.knob_brush.SetColor(color)
        }
    }

    /// Says under the switch for UI access how turning it on is going, or
    /// under the dictionaries why one didn't download.
    fn show_note(&self) -> Result<()> {
        let Some(text) = self.view.as_ref().and_then(|view| view.step.note.as_ref()) else {
            return Ok(());
        };
        let note = match self.steps[self.at] {
            Step::Dictionaries => (self.downloads.iter())
                .find(|(.., failed)| *failed)
                .map(|(_, note, failed)| (note, failed)),
            _ => self.note.as_ref().map(|(note, failed)| (note, failed)),
        };
        let Some((note, failed)) = note else {
            text.clear();
            return Ok(());
        };
        let ink = if *failed {
            Ink::Critical
        } else {
            Ink::Secondary
        };
        let line = self.renderer.layout_line(note, Face::Small, ink, TEXT_W)?;
        text.show(&self.renderer, &line, self.scale, &self.theme)?;
        Ok(())
    }

    /// Takes the list out of what's on its way out, before that's let go
    /// of, keeping it only in the box the current step shows.
    fn release_list(&self) -> Result<()> {
        let keep = match self.view.as_ref().and_then(|v| v.step.picker.as_ref()) {
            Some(picker) => vec![picker.text.visual.cast()?],
            None => Vec::new(),
        };
        self.list.detach_unless(&keep)
    }

    /// Shows the box as it is, with the list of languages under it while
    /// it's open, or the dictionaries as they're ticked, or takes the list
    /// away when the step has neither.
    fn show_picker(&mut self) -> Result<()> {
        let step = self.steps[self.at];
        let Some(view) = self.view.as_ref().and_then(|v| v.step.picker.as_ref()) else {
            return self.list.hide(&self.anim, LEAVE_MS);
        };
        let (layout, moving) = self.layout_picker(step, view.rows)?;
        let (anim, r, t, s) = (&self.anim, &self.renderer, &self.theme, self.scale);
        (view.morph).show(anim, r, &view.text, &layout, moving.as_ref(), s, t)?;
        match layout.caret {
            Some(m) => {
                view.caret
                    .SetOffset(v3((m.x * s).round(), (m.y * s).round()))?;
                view.caret
                    .SetSize(v2((m.width * s).round().max(1.0), (m.height * s).round()))?;
                ui::blink(&anim.compositor, &view.caret)?;
            }
            None => {
                Animator::stop(&view.caret, &["Opacity"]);
                view.caret.SetOpacity(0.0)?;
            }
        }
        match layout.list.as_deref() {
            Some(list) => {
                self.list.attach(&view.text.visual.cast()?)?;
                self.list.show(anim, r, t, list, s)
            }
            None => self.list.hide(anim, LEAVE_MS),
        }
    }

    /// The box of the language WinDict is shown in, with `rows` of the list
    /// under it while it's open, or the dictionaries, `rows` of them at
    /// once; and what moves in the box as it opens or shuts.
    fn layout_picker(&self, step: Step, rows: usize) -> Result<(Layout, Option<Moving>)> {
        let r = &self.renderer;
        if step == Step::Languages {
            let picker = self.picking.as_ref().map(|p| Picker {
                query: p.query.text(),
                caret: p.query.caret_utf16(),
                matches: &p.matches,
                highlight: p.highlight,
                rows,
            });
            let language = self.options.interface.code();
            let (layout, moving) = r.layout_interface(language, picker.as_ref(), TEXT_W)?;
            return Ok((layout, Some(moving)));
        }
        // English, which comes with WinDict and can't be unticked here,
        // then those to download.
        let asides: Vec<(&str, String)> = std::iter::once("en")
            .chain(dicts::DOWNLOADS.iter().map(|d| d.language))
            .map(|code| (code, self.aside(code)))
            .collect();
        let ticks: Vec<Tick> = asides
            .iter()
            .map(|(code, aside)| Tick {
                code,
                on: *code == "en" || self.chosen.contains(code),
                fixed: *code == "en",
                aside,
            })
            .collect();
        Ok((
            r.layout_ticks(&ticks, self.tick_highlight(), rows, TEXT_W)?,
            None,
        ))
    }

    /// What's said at the right of a dictionary's row: how big it is, or
    /// how far its download has got.
    fn aside(&self, language: &str) -> String {
        let downloading = (self.downloads.iter())
            .find(|(code, _, failed)| *code == language && !failed)
            .map(|(_, status, _)| status.clone());
        match dicts::find(language) {
            Some(download) => downloading
                .unwrap_or_else(|| fill(tr().intro_size, &[("size", &download.megabytes())])),
            None => tr().intro_built_in.to_owned(),
        }
    }

    /// The dictionary's row lit: the one under the pointer, or else the
    /// keyboard's, while it's on them.
    fn tick_highlight(&self) -> Option<usize> {
        match self.hovered {
            Some(Hit::Row(i)) => Some(i),
            _ if self.ring && self.focus == Some(Hit::List) => self.tick_at.or(Some(0)),
            _ => None,
        }
    }

    fn show_tick_highlight(&mut self) -> Result<()> {
        if self.steps[self.at] != Step::Dictionaries || self.view.is_none() {
            return Ok(());
        }
        let row = self.tick_highlight();
        self.list.set_highlight(&self.anim, row)
    }

    /// How many rows of the list under the box show at once.
    fn picker_rows(&self) -> usize {
        (self.view.as_ref())
            .and_then(|view| view.step.picker.as_ref())
            .map_or(1, |picker| picker.rows)
    }

    /// Lengthens the dot of the step showing, in the accent, and shortens
    /// the others, the ones after it moving along to make room.
    fn show_dots(&self, dots: &[DotView], animate: bool) -> Result<()> {
        let s = self.scale;
        let (dot, gap) = (DOT * s, DOT_GAP * s);
        let mut x = 0.0;
        for (i, view) in dots.iter().enumerate() {
            let on = i == self.at;
            let size = v2(if on { DOT_ON * s } else { dot }, dot);
            let color = if on {
                self.theme.accent
            } else {
                self.theme.strong_stroke()
            };
            let at = v2(x, 0.0);
            x += size.X + gap;
            if animate {
                let anim = &self.anim;
                anim.vector2(&view.shape, "Size", size, DOTS_MS)?;
                anim.vector2(&view.shape, "Offset", at, DOTS_MS)?;
                anim.color(&view.brush, view.brush.Color()?, color, DOTS_MS)?;
            } else {
                view.shape.SetSize(size)?;
                view.shape.SetOffset(at)?;
                view.brush.SetColor(color)?;
            }
        }
        Ok(())
    }

    fn button(&self, hit: Hit) -> Option<&Button> {
        self.targets().find(|(h, _)| *h == hit).map(|(_, b)| b)
    }

    /// What can be clicked, but for the close button: what the step offers,
    /// and the buttons that go through the steps.
    fn targets(&self) -> impl Iterator<Item = &(Hit, Button)> {
        self.view
            .iter()
            .flat_map(|view| view.step.controls.iter().chain(&view.buttons))
    }

    // -----------------------------------------------------------------------
    // Building the visuals

    /// Builds the view afresh for the current scale, theme and language.
    /// With `crossfade`, or in another language, the old view fades out as
    /// the new one fades in; otherwise it goes at once.
    fn replace_view(&mut self, crossfade: bool) -> Result<()> {
        self.land()?;
        let language = crate::i18n::current();
        let relanguage = self.view.is_some() && language != self.built_in;
        self.built_in = language;
        let crossfade = crossfade || relanguage;
        let mut view = self.build()?;
        if relanguage {
            self.keep_box(&mut view)?;
        }
        let anim = &self.anim;
        if let Some(old) = self.view.replace(view) {
            if crossfade {
                anim.scalar(&old.content, "Opacity", 0.0, THEME_MS, 0, &anim.fade)?;
                self.gone.push(old.content.cast()?);
                win::set_timer(self.hwnd, TIMER_GONE, GONE_MS);
            } else {
                self.release_list()?;
                self.frame.card.Children()?.Remove(&old.content)?;
            }
        }
        if let Some(view) = &self.view {
            if crossfade {
                view.content.SetOpacity(0.0)?;
                anim.scalar(&view.content, "Opacity", 1.0, THEME_MS, 0, &anim.fade)?;
            }
            self.show_dots(&view.dots, false)?;
        }
        self.keep_focus();
        self.show_switch(false)?;
        self.show_note()?;
        self.show_picker()?;
        self.show_hover(self.hovered)?;
        self.show_focus()
    }

    /// The language it's shown in was just picked in the box: rather than
    /// fade with the rest, the box stays as it is, still turning shut, and
    /// moves over `view` to where it goes there, as what's said around it
    /// changes.
    fn keep_box(&mut self, view: &mut View) -> Result<()> {
        if self.steps[self.at] != Step::Languages {
            return Ok(());
        }
        let Some(was) = self.view.as_mut().and_then(|v| v.step.picker.as_mut()) else {
            return Ok(());
        };
        let Some(new) = view.step.picker.as_mut() else {
            return Ok(());
        };
        let kept: Visual = was.text.visual.cast()?;
        let landing: Visual = new.text.visual.cast()?;
        landing.SetOpacity(0.0)?;
        if let Ok(parent) = kept.Parent() {
            parent.Children()?.Remove(&kept)?;
        }
        self.frame.card.Children()?.InsertAtTop(&kept)?;
        std::mem::swap(&mut was.text, &mut new.text);
        std::mem::swap(&mut was.morph, &mut new.morph);
        std::mem::swap(&mut was.caret, &mut new.caret);
        let anim = &self.anim;
        anim.vector3(&kept, "Offset", new.at, THEME_MS, 0, &anim.fade)?;
        self.landing = Some(landing);
        Ok(())
    }

    /// The box [`Self::keep_box`] moved is where it goes: it goes into the
    /// view, in what was in its place.
    fn land(&mut self) -> Result<()> {
        let Some(landing) = self.landing.take() else {
            return Ok(());
        };
        let Ok(parent) = landing.Parent() else {
            return Ok(());
        };
        if let Some(picker) = self.view.as_ref().and_then(|v| v.step.picker.as_ref()) {
            let kept: Visual = picker.text.visual.cast()?;
            Animator::stop(&kept, &["Offset"]);
            if let Ok(card) = kept.Parent() {
                card.Children()?.Remove(&kept)?;
            }
            kept.SetOffset(picker.at)?;
            parent.Children()?.InsertAbove(&kept, &landing)?;
        }
        parent.Children()?.Remove(&landing)
    }

    fn build(&self) -> Result<View> {
        let (c, r, t, s) = (
            &self.anim.compositor,
            &self.renderer,
            &self.theme,
            self.scale,
        );
        let px = |dip: f32| (dip * s).round();
        let content = c.CreateContainerVisual()?;
        self.frame.card.Children()?.InsertAtTop(&content)?;

        // The picture, its corners rounded, with a hairline round it.
        let at = Rect {
            x: px(INSET),
            y: px(INSET),
            w: px(PANE_W),
            h: px(PANE_H),
        };
        let pane = picture_card::picture_pane(c, r, t, s, &content, at)?;
        let picture = self.build_picture(&pane)?;
        let close = picture_card::close_button(c, r, t, s, &content, CARD_W)?;

        // The step goes over the dots and the buttons, for a list open over
        // them.
        let dots = self.build_dots(&content)?;
        let (footer, buttons) = self.build_footer(&content)?;
        let step = self.build_step(&content)?;
        Ok(View {
            content,
            pane,
            picture,
            step,
            close,
            dots,
            footer,
            buttons,
        })
    }

    /// The step's picture, filling the `pane`, on top of what's there.
    fn build_picture(&self, pane: &ContainerVisual) -> Result<SpriteVisual> {
        let (c, r, s) = (&self.anim.compositor, &self.renderer, self.scale);
        let size = v2((PANE_W * s).round(), (PANE_H * s).round());
        let step = self.steps[self.at];
        let surface = match step.scene() {
            Some(shot) => {
                scene::draw(r, shot, crate::i18n::current(), &self.theme, size.X, size.Y)?
            }
            None => r.draw_picture(step.picture(self.theme.dark), size.X, size.Y)?,
        };
        let picture = c.CreateSpriteVisual()?;
        picture.SetBrush(&c.CreateSurfaceBrushWithSurface(&surface)?)?;
        picture.SetSize(size)?;
        picture.SetCenterPoint(Vector3 {
            X: size.X / 2.0,
            Y: size.Y / 2.0,
            Z: 0.0,
        })?;
        pane.Children()?.InsertAtTop(&picture)?;
        Ok(picture)
    }

    /// The step's symbol, its title, what it says and what can be done
    /// there, each a visual of its own in the text returned, in `content`.
    fn build_step(&self, content: &ContainerVisual) -> Result<StepView> {
        let (c, r, t, s) = (
            &self.anim.compositor,
            &self.renderer,
            &self.theme,
            self.scale,
        );
        let px = |dip: f32| (dip * s).round();
        let step = self.steps[self.at];
        let text = container(c, content, v3(0.0, 0.0))?;
        let x = px(TEXT_X);

        let (title, glyph) = (step.title(), step.glyph());
        let mut y = picture_card::heading(c, r, t, s, &text, TEXT_X, TEXT_W, title, glyph, false)?;

        let detail = Text::new(c, &text)?;
        let (said, keys) = step.detail(&self.shortcuts);
        let paragraph = r.layout_paragraph(said, &keys, Ink::Secondary, TEXT_W)?;
        let size = detail.show(r, &paragraph, s, t)?;
        detail.visual.SetOffset(v3(x, y))?;
        let picks = matches!(step, Step::Languages | Step::Dictionaries);
        y += size.Y + px(if picks { PICKER_GAP } else { ACTIONS_GAP });

        let mut view = StepView {
            text,
            controls: Vec::new(),
            switch: None,
            note: None,
            picker: None,
        };
        match step {
            // A combo box, its list as long as fits down to the buttons, as
            // there'll be more languages; and the dictionaries, a list as
            // long as leaves room for a note, the rest of it scrolling.
            Step::Languages | Step::Dictionaries => {
                let top = y / s;
                let rows = if step == Step::Languages {
                    Picker::rows_in(CARD_H - PAD_BOTTOM - top - SELECT_H)
                } else {
                    Tick::rows_in(CARD_H - PAD_BOTTOM - BUTTON_H - NOTE_ROOM - top)
                };
                let (layout, moving) = self.layout_picker(step, rows)?;
                let picker = Text::new(c, &view.text)?;
                let at = v3(x, y);
                picker.visual.SetOffset(at)?;
                let morph = Morph::new(c, &picker, layout.caret.is_some())?;
                morph.show(&self.anim, r, &picker, &layout, moving.as_ref(), s, t)?;
                let parent = picker.visual.cast::<ContainerVisual>()?;
                let (caret, _) = rounded(c, r, &parent, Rect::default(), 0.0, t.accent)?;
                caret.SetOpacity(0.0)?;
                let face = Rect {
                    x,
                    y,
                    w: px(TEXT_W),
                    h: px(SELECT_H),
                };
                let (hit, face, radius) = match step {
                    Step::Languages => (Hit::Select, face, SELECT_RADIUS),
                    _ => {
                        let h = (layout.height * s).round();
                        (Hit::List, Rect { h, ..face }, LIST_RADIUS)
                    }
                };
                let button = self.build_stop(&view.text, face, radius)?;
                view.controls.push((hit, button));
                if step == Step::Dictionaries {
                    let note = Text::new(c, &view.text)?;
                    note.visual.SetOffset(v3(x, y + face.h + px(NOTE_GAP)))?;
                    view.note = Some(note);
                }
                view.picker = Some(PickerView {
                    text: picker,
                    morph,
                    caret,
                    at,
                    rows,
                });
            }
            Step::Lookup | Step::Translate => {
                let taken = step == Step::Lookup && self.options.shortcut_taken;
                if taken {
                    let warning = Text::new(c, &view.text)?;
                    let line = r.layout_line(
                        tr().intro_shortcut_taken,
                        Face::Menu,
                        Ink::Critical,
                        TEXT_W,
                    )?;
                    let size = warning.show(r, &line, s, t)?;
                    warning.visual.SetOffset(v3(x, y))?;
                    y += size.Y + px(WARNING_GAP);
                }
                let mut offers = Vec::new();
                if taken {
                    offers.push((Hit::ChooseShortcut, tr().intro_choose_shortcut));
                }
                // Translations need the internet.
                if step == Step::Lookup || self.options.online {
                    offers.push((Hit::ShowMe, tr().intro_show_me));
                }
                let mut left = x;
                for (hit, label) in offers {
                    let w = button_width(r, label)?;
                    let at = Rect {
                        x: left,
                        y,
                        w: px(w),
                        h: px(BUTTON_H),
                    };
                    left += at.w + px(BUTTON_GAP);
                    let button = self.build_button(&view.text, at, label, false)?;
                    view.controls.push((hit, button));
                }
            }
            Step::AboveStart | Step::Ready => {
                let label = if step == Step::AboveStart {
                    tr().intro_ui_access
                } else {
                    tr().start_with_windows
                };
                let row = Rect {
                    x,
                    y,
                    w: px(TEXT_W),
                    h: px(ROW_H),
                };
                let (button, switch) = self.build_switch_row(&view.text, row, label)?;
                view.controls.push((Hit::Switch, button));
                view.switch = Some(switch);
                if step == Step::AboveStart {
                    let note = Text::new(c, &view.text)?;
                    note.visual.SetOffset(v3(x, y + row.h + px(NOTE_GAP)))?;
                    view.note = Some(note);
                }
            }
        }
        Ok(view)
    }

    /// A row like a setting's at `at` in `parent`, saying `label`, with a
    /// switch at its right. Returns the row, which is what's clicked, and
    /// the switch.
    fn build_switch_row(
        &self,
        parent: &ContainerVisual,
        at: Rect,
        label: &str,
    ) -> Result<(Button, SwitchView)> {
        let (c, r, t, s) = (
            &self.anim.compositor,
            &self.renderer,
            &self.theme,
            self.scale,
        );
        let px = |dip: f32| (dip * s).round();
        let root = container(c, parent, at.at())?;
        let face = Rect {
            w: at.w,
            h: at.h,
            ..Rect::default()
        };
        let radius = px(ROW_RADIUS);
        rounded(c, r, &root, face, radius, t.group())?;
        let (hover, _) = rounded(c, r, &root, face, radius, t.highlight())?;
        hover.SetOpacity(0.0)?;
        outline(c, r, &root, face, radius, t.group_stroke())?;

        let (w, h) = (px(SWITCH_W), px(SWITCH_H));
        let track_at = v3(at.w - px(ROW_PAD) - w, ((at.h - h) / 2.0).round());
        let switch = container(c, &root, track_at)?;
        let track = Rect {
            w,
            h,
            ..Rect::default()
        };
        let off = container(c, &switch, v3(0.0, 0.0))?;
        rounded(c, r, &off, track, h / 2.0, t.sunken())?;
        outline(c, r, &off, track, h / 2.0, t.strong_stroke())?;
        let (on, _) = rounded(c, r, &switch, track, h / 2.0, t.accent)?;
        let knob = px(KNOB);
        let dot = Rect {
            w: knob,
            h: knob,
            ..Rect::default()
        };
        let (knob_visual, knob_brush) = rounded(c, r, &switch, dot, knob / 2.0, t.knob(false))?;
        knob_visual.SetCenterPoint(Vector3 {
            X: knob / 2.0,
            Y: knob / 2.0,
            Z: 0.0,
        })?;
        let inset = ((h - knob) / 2.0).round();

        let text = Text::new(c, &root)?;
        let room = (TEXT_W - SWITCH_W - 3.0 * ROW_PAD).max(1.0);
        let line = r.layout_line(label, Face::Setting, Ink::Primary, room)?;
        let size = text.show(r, &line, s, t)?;
        text.visual
            .SetOffset(v3(px(ROW_PAD), ((at.h - size.Y) / 2.0).round()))?;

        let outset = px(FOCUS_OUTSET);
        let ring = ring(
            c,
            r,
            &root,
            face.outset(outset),
            radius + outset,
            ui::FOCUS_STROKE * s,
            t.focus(),
        )?;
        ring.SetOpacity(0.0)?;
        let row = Button {
            root,
            hover,
            ring,
            at,
        };
        let switch = SwitchView {
            root: switch,
            on,
            off,
            knob: knob_visual,
            knob_brush,
            rest: [v3(inset, inset), v3(w - inset - knob, inset)],
        };
        Ok((row, switch))
    }

    /// What has the keyboard at `at` in `parent`, drawn by the renderer
    /// with corners of `radius` DIPs: the ring round it then, and the fill
    /// that lights it under the pointer.
    fn build_stop(&self, parent: &ContainerVisual, at: Rect, radius: f32) -> Result<Button> {
        let (c, r, t, s) = (
            &self.anim.compositor,
            &self.renderer,
            &self.theme,
            self.scale,
        );
        let px = |dip: f32| (dip * s).round();
        let root = container(c, parent, at.at())?;
        let face = Rect {
            w: at.w,
            h: at.h,
            ..Rect::default()
        };
        let radius = px(radius);
        let (hover, _) = rounded(c, r, &root, face, radius, t.highlight())?;
        hover.SetOpacity(0.0)?;
        let outset = px(FOCUS_OUTSET);
        let ring = ring(
            c,
            r,
            &root,
            face.outset(outset),
            radius + outset,
            ui::FOCUS_STROKE * s,
            t.focus(),
        )?;
        ring.SetOpacity(0.0)?;
        Ok(Button {
            root,
            hover,
            ring,
            at,
        })
    }

    /// A dot for each step, along the bottom of `content`, level with the
    /// buttons.
    fn build_dots(&self, content: &ContainerVisual) -> Result<Vec<DotView>> {
        let (c, s) = (&self.anim.compositor, self.scale);
        let px = |dip: f32| (dip * s).round();
        let middle = px(CARD_H - PAD_BOTTOM - BUTTON_H / 2.0);
        let n = self.steps.len() as f32;
        let w = px(DOT_ON + (n - 1.0) * (DOT + DOT_GAP));
        let visual = c.CreateShapeVisual()?;
        visual.SetOffset(v3(px(TEXT_X), (middle - px(DOT) / 2.0).round()))?;
        visual.SetSize(v2(w, px(DOT)))?;
        let mut dots = Vec::new();
        for _ in &self.steps {
            let shape = c.CreateRoundedRectangleGeometry()?;
            shape.SetCornerRadius(v2(DOT * s / 2.0, DOT * s / 2.0))?;
            let brush = c.CreateColorBrush()?;
            let sprite = c.CreateSpriteShapeWithGeometry(&shape)?;
            sprite.SetFillBrush(&brush)?;
            visual.Shapes()?.Append(&sprite)?;
            dots.push(DotView { shape, brush });
        }
        content.Children()?.InsertAtTop(&visual)?;
        Ok(dots)
    }

    /// The buttons that go through the steps, at the bottom right of
    /// `content`. Returns what holds them, and the buttons.
    fn build_footer(
        &self,
        content: &ContainerVisual,
    ) -> Result<(ContainerVisual, Vec<(Hit, Button)>)> {
        let s = self.scale;
        let px = |dip: f32| (dip * s).round();
        let footer = container(&self.anim.compositor, content, v3(0.0, 0.0))?;
        let last = self.at + 1 == self.steps.len();
        let h = px(BUTTON_H);
        let y = px(CARD_H - PAD_BOTTOM) - h;
        let mut right = px(CARD_W - PAD_X);
        let next = if last {
            tr().intro_done
        } else if self.to_download() {
            tr().intro_download
        } else {
            tr().intro_next
        };
        let specs = [(Hit::Next, next, true), (Hit::Back, tr().intro_back, false)];
        let mut buttons = Vec::new();
        for (hit, label, primary) in specs {
            if hit == Hit::Back && self.at == 0 {
                continue;
            }
            // Download may say more than fits the others' width.
            let w = px(button_width(&self.renderer, label)?);
            let at = Rect {
                x: right - w,
                y,
                w,
                h,
            };
            right = at.x - px(BUTTON_GAP);
            buttons.push((hit, self.build_button(&footer, at, label, primary)?));
        }
        Ok((footer, buttons))
    }

    /// A button saying `label` at `at` in `content`: in the accent when
    /// it's the `primary` one.
    fn build_button(
        &self,
        content: &ContainerVisual,
        at: Rect,
        label: &str,
        primary: bool,
    ) -> Result<Button> {
        let (c, r, t, s) = (
            &self.anim.compositor,
            &self.renderer,
            &self.theme,
            self.scale,
        );
        ui::button(c, r, t, s, content, at, label, primary)
    }

    // -----------------------------------------------------------------------
    // Geometry

    /// Sizes the card for the current scale. Returns the window's size in
    /// pixels: the card, and room for its shadow.
    fn fit(&self) -> Result<(i32, i32)> {
        picture_card::fit(&self.frame, &self.anim, self.scale, CARD_W, CARD_H)
    }

    /// The middle of the card, in the window: what it grows from.
    fn centre(&self) -> Vector3 {
        picture_card::centre(self.scale, CARD_W, CARD_H)
    }

    /// A point in window pixels, relative to the card's top left.
    fn card_point(&self, x: i32, y: i32) -> (f32, f32) {
        picture_card::card_point(self.scale, x, y)
    }

    fn hit_at(&self, wx: i32, wy: i32) -> Option<Hit> {
        let view = self.view.as_ref()?;
        let (x, y) = self.card_point(wx, wy);
        if view.close.at.contains(x, y) {
            return Some(Hit::Close);
        }
        let in_list = self.list_point(wx, wy);
        if let Some((lx, ly)) = in_list.filter(|&(lx, ly)| self.list.contains(lx, ly)) {
            return Some(self.list.row_at(lx, ly).map_or(Hit::List, Hit::Row));
        }
        self.targets()
            .find(|(_, b)| b.at.contains(x, y))
            .map(|(hit, _)| *hit)
    }

    /// A point in window pixels, from the top left of what the list is
    /// shown in: the box, or the dictionaries.
    fn list_point(&self, x: i32, y: i32) -> Option<(f32, f32)> {
        let at = self.view.as_ref()?.step.picker.as_ref()?.at;
        let (x, y) = self.card_point(x, y);
        Some((x - at.X, y - at.Y))
    }

    /// Names the window in the language now current, as the taskbar shows it.
    fn retitle(&self) {
        unsafe {
            let _ = SetWindowTextW(self.hwnd, &HSTRING::from(tr().intro_title));
        }
    }
}

/// The scale of the window on a monitor of `scale` pixels per DIP: see
/// [`picture_card::fitted`].
fn fitted(work: RECT, scale: f32) -> f32 {
    picture_card::fitted(work, scale, CARD_W, CARD_H)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::BUTTON_W;

    #[test]
    fn shows_ui_access_only_where_it_can_be_had() {
        use Step::*;
        assert_eq!(
            Step::all(true),
            [
                Languages,
                Lookup,
                Translate,
                AboveStart,
                Dictionaries,
                Ready
            ]
        );
        assert_eq!(
            Step::all(false),
            [Languages, Lookup, Translate, Dictionaries, Ready]
        );
    }

    #[test]
    fn names_the_shortcuts() {
        let shortcuts = Shortcuts {
            lookup: "alt+d".into(),
            translate: "Alt+T".into(),
        };
        let (said, keys) = Step::Lookup.detail(&shortcuts);
        assert!(said.contains("{hotkey}"));
        assert_eq!(keys, [("hotkey", vec!["Alt".to_owned(), "D".to_owned()])]);
        let (said, keys) = Step::Translate.detail(&shortcuts);
        for (name, _) in keys {
            assert!(said.contains(&format!("{{{name}}}")), "{name}");
        }
    }

    #[test]
    fn dots_fit_beside_the_buttons() {
        let n = Step::all(true).len() as f32;
        let dots = DOT_ON + (n - 1.0) * (DOT + DOT_GAP);
        assert!(dots + 2.0 * BUTTON_W + BUTTON_GAP + 24.0 < TEXT_W);
    }

    #[test]
    fn opens_enlarged_unless_the_screen_is_too_small() {
        let work = |w, h| RECT {
            left: 0,
            top: 0,
            right: w,
            bottom: h,
        };
        assert_eq!(fitted(work(1920, 1040), 1.0), 1.25);
        // As high as 440 DIPs and 16 either side fit, 600 pixels.
        assert_eq!(fitted(work(1024, 600), 1.5), 600.0 / 472.0);
        // As wide as 720 DIPs and 16 either side fit, 700 pixels.
        assert_eq!(fitted(work(700, 2000), 2.0), 700.0 / 752.0);
    }
}
