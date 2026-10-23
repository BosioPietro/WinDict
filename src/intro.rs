//! The introduction: a window shown the first time WinDict starts, a few
//! steps that each have a picture on the left of the card and what they say
//! on the right, with dots for how far along they are and Back and Next
//! under them. It's built like the settings window, a composition
//! visual tree on the card's acrylic: the window rises in, and from step to
//! step the picture cross-fades as it settles, the old text slides away and
//! the new slides in after it, line after line, while the accent dot
//! stretches over to the step it's on.
//!
//! The pictures are close-ups of WinDict on the Windows desktop, in light
//! and dark: the one for the system's theme shows.

use windows::core::{Interface, Result, HSTRING};
use windows::Win32::Foundation::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::UI::Color;
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::*;
use windows_numerics::Vector3;

use crate::hotkey;
use crate::i18n::tr;
use crate::render::{Align, Face, Ink, Renderer};
use crate::theme::{argb, Theme};
use crate::ui::{
    self, container, outline, rounded, scale, v2, v3, Animator, Frame, Rect, Text, MARGIN,
};
use crate::win::{self, Window};

// Geometry, in DIPs.
const CARD_W: f32 = 720.0;
const CARD_H: f32 = 440.0;
/// The picture, on the left, as far from the card's edges all round.
const INSET: f32 = 12.0;
const PANE_W: f32 = 328.0;
const PANE_H: f32 = CARD_H - 2.0 * INSET;
const PICTURE_RADIUS: f32 = 8.0;
/// Between the picture and the text, and between the text and the right.
const PAD_X: f32 = 36.0;
/// Where the text starts.
const TEXT_X: f32 = INSET + PANE_W + PAD_X;
const TEXT_W: f32 = CARD_W - TEXT_X - PAD_X;
/// The strip along the top that drags the window, over the picture too.
const TOP_H: f32 = 48.0;
const CLOSE_SIZE: f32 = 32.0;
/// The close button's distance from the card's top and right edges.
const CLOSE_INSET: f32 = 10.0;
const CLOSE_RADIUS: f32 = 5.0;
/// Where a step's title starts, the same for every step.
const TITLE_Y: f32 = 64.0;
/// The symbol before a step's title, on a square tinted with the accent,
/// and the space after it.
const BADGE: f32 = 36.0;
const BADGE_RADIUS: f32 = 8.0;
const BADGE_GAP: f32 = 14.0;
/// Between a step's title and what it says.
const TITLE_GAP: f32 = 14.0;
/// From the buttons to the bottom of the card.
const PAD_BOTTOM: f32 = 28.0;
const BUTTON_W: f32 = 96.0;
const BUTTON_H: f32 = 32.0;
const BUTTON_GAP: f32 = 8.0;
const BUTTON_RADIUS: f32 = 4.0;
const FOCUS_OUTSET: f32 = 3.0;
/// The dots for the steps: the one for the step showing is longer.
const DOT: f32 = 6.0;
const DOT_ON: f32 = 18.0;
const DOT_GAP: f32 = 6.0;
/// How far the text slides as it goes and comes.
const SLIDE: f32 = 24.0;
/// How much larger a picture starts, as it fades in and settles.
const ZOOM: f32 = 1.04;
/// The least room kept between the window and the screen's edges.
const SCREEN_GAP: f32 = 16.0;

// Durations, in ms.
const SHOW_MS: u64 = 360;
const LEAVE_MS: u64 = 150;
const ENTER_MS: u64 = 300;
/// Each line of the text comes in this long after the one above it.
const STAGGER_MS: u64 = 40;
/// How long a picture takes to settle as it fades in.
const SETTLE_MS: u64 = 700;
const DOTS_MS: u64 = 300;
const HOVER_MS: u64 = 100;
const PRESS_MS: u64 = 90;
const FOCUS_MS: u64 = 150;
const THEME_MS: u64 = 300;
/// Once what's going has faded, it's taken away.
const GONE_MS: u32 = SETTLE_MS as u32 + 30;
const TIMER_GONE: usize = 1;

/// "Cancel" in Segoe Fluent Icons and Segoe MDL2 Assets.
const CLOSE_GLYPH: &str = "\u{E8BB}";

/// A step of the introduction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Step {
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
    /// The steps, in order: the one about UI access only when it can be
    /// turned on.
    pub fn all(ui_access: bool) -> Vec<Step> {
        [Step::Lookup, Step::Translate, Step::AboveStart, Step::Ready]
            .into_iter()
            .filter(|&step| step != Step::AboveStart || ui_access)
            .collect()
    }

    fn title(self) -> &'static str {
        match self {
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
    /// Assets: as on the shortcuts' rows in the settings for those, layers
    /// for the card above Start, and a tick to finish.
    fn glyph(self) -> &'static str {
        match self {
            Step::Lookup => "\u{E721}",
            Step::Translate => "\u{E8C1}",
            Step::AboveStart => "\u{E81E}",
            Step::Ready => "\u{E73E}",
        }
    }

    /// The picture, in light or `dark`.
    fn picture(self, dark: bool) -> &'static [u8] {
        match (self, dark) {
            (Step::Lookup, false) => include_bytes!("../assets/intro/lookup-light.jpg"),
            (Step::Lookup, true) => include_bytes!("../assets/intro/lookup-dark.jpg"),
            (Step::Translate, false) => include_bytes!("../assets/intro/translate-light.jpg"),
            (Step::Translate, true) => include_bytes!("../assets/intro/translate-dark.jpg"),
            (Step::AboveStart, false) => include_bytes!("../assets/intro/start-light.jpg"),
            (Step::AboveStart, true) => include_bytes!("../assets/intro/start-dark.jpg"),
            (Step::Ready, false) => include_bytes!("../assets/intro/tray-light.jpg"),
            (Step::Ready, true) => include_bytes!("../assets/intro/tray-dark.jpg"),
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

/// Something done in the window, for the app to carry out.
#[derive(Debug, PartialEq)]
pub enum Action {
    /// Done, or closed: either way, it's been seen.
    Close,
}

/// What's under the pointer, or has the keyboard.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Hit {
    Close,
    Back,
    Next,
}

struct ButtonView {
    root: ContainerVisual,
    hover: SpriteVisual,
    /// Round it while it has the keyboard.
    ring: SpriteVisual,
    at: Rect,
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
    /// The step's title, what it says and its keys, which slide as the step
    /// changes.
    text: ContainerVisual,
    close_at: Rect,
    close_hover: SpriteVisual,
    dots: Vec<DotView>,
    /// The buttons, which change at once.
    footer: ContainerVisual,
    /// Back, when there's a step before, and Next.
    buttons: Vec<(Hit, ButtonView)>,
}

pub struct Intro {
    pub hwnd: HWND,
    _target: DesktopWindowTarget,
    renderer: Renderer,
    anim: Animator,
    /// Everything on the window; fades and scales as one when showing and hiding.
    root: ContainerVisual,
    frame: Frame,
    view: Option<View>,
    /// What's on its way out, fading: steps, and views of an older theme.
    gone: Vec<Visual>,
    steps: Vec<Step>,
    /// Which of them shows.
    at: usize,
    shortcuts: Shortcuts,
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
    pub fn new(window: Window, compositor: Compositor, theme: Theme) -> Result<Self> {
        let hwnd = window.hwnd();
        let target = ui::attach(hwnd, &compositor)?;
        let renderer = Renderer::new(&compositor)?;
        let root = compositor.CreateContainerVisual()?;
        target.SetRoot(&root)?;
        let frame = Frame::new(&compositor, &root, &theme)?;
        Ok(Intro {
            hwnd,
            _target: target,
            renderer,
            anim: Animator::new(compositor)?,
            root,
            frame,
            view: None,
            gone: Vec::new(),
            steps: Step::all(false),
            at: 0,
            shortcuts: Shortcuts::default(),
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

    /// Opens the window on its first step, in the middle of the screen
    /// `near` (screen pixels) is on.
    pub fn show(
        &mut self,
        near: POINT,
        theme: Theme,
        steps: Vec<Step>,
        shortcuts: Shortcuts,
    ) -> Result<()> {
        let (work, s) = ui::monitor_at(near);
        let s = fitted(work, s);
        (self.theme, self.scale, self.steps, self.shortcuts) = (theme, s, steps, shortcuts);
        (self.at, self.hovered, self.pressed, self.focus) = (0, None, None, None);
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
            self.bring_in(&view.text, v3(0.0, SLIDE / 3.0 * s), ui::FADE_IN_MS)?;
        }
        Ok(())
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
        x >= 0.0 && x < px(CARD_W) && y >= 0.0 && y < px(TOP_H) && !view.close_at.contains(x, y)
    }

    /// The pointer moved to a point in window pixels, or left.
    pub fn hover(&mut self, point: Option<(i32, i32)>) -> Result<()> {
        let hit = point.and_then(|(x, y)| self.hit_at(x, y));
        if hit == self.hovered {
            return Ok(());
        }
        let old = std::mem::replace(&mut self.hovered, hit);
        self.show_hover(old)?;
        self.show_hover(hit)
    }

    /// The left button went down at a point in window pixels.
    pub fn press(&mut self, x: i32, y: i32) -> Result<()> {
        let hit = self.hit_at(x, y);
        // The pointer's in charge now: the ring goes until Tab brings it back.
        self.ring = false;
        if let Some(hit) = hit.filter(|&hit| hit != Hit::Close) {
            self.focus = Some(hit);
        }
        self.show_focus()?;
        self.set_pressed(hit)
    }

    /// The left button came up at a point in window pixels: a click, if it's
    /// on what it went down on.
    pub fn release(&mut self, x: i32, y: i32) -> Result<Option<Action>> {
        let pressed = self.pressed;
        self.set_pressed(None)?;
        match pressed {
            Some(hit) if self.hit_at(x, y) == Some(hit) => self.activate(hit),
            _ => Ok(None),
        }
    }

    /// A key went down.
    pub fn key(&mut self, key: VIRTUAL_KEY) -> Result<Option<Action>> {
        let shift = hotkey::is_held(VK_SHIFT);
        match key {
            VK_ESCAPE => Ok(Some(Action::Close)),
            VK_LEFT if self.at > 0 => self.activate(Hit::Back),
            VK_RIGHT if self.at + 1 < self.steps.len() => self.activate(Hit::Next),
            VK_RETURN | VK_SPACE => self.activate(self.focus.unwrap_or(Hit::Next)),
            VK_TAB => {
                self.step_focus(!shift)?;
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    /// The window stopped being the active one.
    pub fn deactivate(&mut self) -> Result<()> {
        self.set_pressed(None)
    }

    /// What was fading out has, and goes.
    pub fn on_timer(&mut self) -> Result<()> {
        win::kill_timer(self.hwnd, TIMER_GONE);
        for gone in self.gone.drain(..) {
            if let Ok(parent) = gone.Parent() {
                parent.Children()?.Remove(&gone)?;
            }
        }
        Ok(())
    }

    /// Switches to `theme`, cross-fading everything.
    pub fn retheme(&mut self, theme: Theme) -> Result<()> {
        let same = (theme.dark, theme.transparency, theme.accent)
            == (self.theme.dark, self.theme.transparency, self.theme.accent);
        if same {
            return Ok(());
        }
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
            Hit::Next if self.at + 1 < self.steps.len() => self.go(self.at + 1)?,
            Hit::Next => return Ok(Some(Action::Close)),
            Hit::Back => {}
        }
        Ok(None)
    }

    /// Shows step `to`: its picture fades in over the old one as it settles,
    /// the old text slides away and fades, and the new slides in after it
    /// from the side `to` is on.
    fn go(&mut self, to: usize) -> Result<()> {
        let forward = to > self.at;
        self.at = to;
        let Some(view) = self.view.take() else {
            return Ok(());
        };
        let (anim, s) = (&self.anim, self.scale);
        let dx = if forward { SLIDE * s } else { -SLIDE * s };

        let old = view.text.clone();
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
        let text = self.build_text(&view.content)?;
        self.bring_in(&text, v3(dx, 0.0), LEAVE_MS)?;
        self.show_dots(&view.dots, true)?;

        // Back comes and goes, and Next turns to Done.
        view.content.Children()?.Remove(&view.footer)?;
        let (footer, buttons) = self.build_footer(&view.content)?;
        // What had the keyboard may be gone: Next has it then.
        if self
            .focus
            .is_some_and(|f| !buttons.iter().any(|(hit, _)| *hit == f))
        {
            self.focus = Some(Hit::Next);
        }
        self.view = Some(View {
            picture,
            text,
            footer,
            buttons,
            ..view
        });
        self.show_hover(self.hovered)?;
        self.show_focus()
    }

    /// Brings each line of `text` in from `from` away, a beat after the one
    /// above it, starting after `delay`.
    fn bring_in(&self, text: &ContainerVisual, from: Vector3, delay: u64) -> Result<()> {
        let anim = &self.anim;
        for (i, line) in text.Children()?.into_iter().enumerate() {
            let rest = line.Offset()?;
            let delay = delay + i as u64 * STAGGER_MS;
            Animator::stop(&line, &["Opacity", "Offset"]);
            line.SetOpacity(0.0)?;
            line.SetOffset(rest + from)?;
            anim.scalar(&line, "Opacity", 1.0, ENTER_MS, delay, &anim.decelerate)?;
            anim.vector3(&line, "Offset", rest, ENTER_MS, delay, &anim.decelerate)?;
        }
        Ok(())
    }

    /// Moves the keyboard to the next (or previous) button, going round.
    fn step_focus(&mut self, forward: bool) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let stops: Vec<Hit> = view.buttons.iter().map(|(hit, _)| *hit).collect();
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
        self.show_focus()
    }

    fn set_pressed(&mut self, hit: Option<Hit>) -> Result<()> {
        if hit == self.pressed {
            return Ok(());
        }
        let old = std::mem::replace(&mut self.pressed, hit);
        for (hit, opacity) in [(old, 1.0), (hit, 0.8)] {
            if let Some(button) = hit.and_then(|hit| self.button(hit)) {
                let anim = &self.anim;
                anim.scalar(
                    &button.root,
                    "Opacity",
                    opacity,
                    PRESS_MS,
                    0,
                    &anim.decelerate,
                )?;
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
            Hit::Close => &view.close_hover,
            _ => match self.button(hit) {
                Some(button) => &button.hover,
                None => return Ok(()),
            },
        };
        let to = if self.hovered == Some(hit) { 1.0 } else { 0.0 };
        let anim = &self.anim;
        anim.scalar(hover, "Opacity", to, HOVER_MS, 0, &anim.decelerate)
    }

    fn show_focus(&self) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let anim = &self.anim;
        for (hit, button) in &view.buttons {
            let on = self.ring && self.focus == Some(*hit);
            let to = if on { 1.0 } else { 0.0 };
            anim.scalar(&button.ring, "Opacity", to, FOCUS_MS, 0, &anim.decelerate)?;
        }
        Ok(())
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

    fn button(&self, hit: Hit) -> Option<&ButtonView> {
        let view = self.view.as_ref()?;
        view.buttons.iter().find(|(h, _)| *h == hit).map(|(_, b)| b)
    }

    // -----------------------------------------------------------------------
    // Building the visuals

    /// Builds the view afresh for the current scale and theme. With
    /// `crossfade`, the old view fades out as the new one fades in;
    /// otherwise it goes at once.
    fn replace_view(&mut self, crossfade: bool) -> Result<()> {
        let view = self.build()?;
        let anim = &self.anim;
        if let Some(old) = self.view.replace(view) {
            if crossfade {
                anim.scalar(&old.content, "Opacity", 0.0, THEME_MS, 0, &anim.fade)?;
                self.gone.push(old.content.cast()?);
                win::set_timer(self.hwnd, TIMER_GONE, GONE_MS);
            } else {
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
        self.show_hover(self.hovered)?;
        self.show_focus()
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

        // The picture, its corners rounded, with a hairline round it.
        let at = Rect {
            x: px(INSET),
            y: px(INSET),
            w: px(PANE_W),
            h: px(PANE_H),
        };
        let pane = container(c, &content, at.at())?;
        pane.SetSize(at.size())?;
        let corners = c.CreateRoundedRectangleGeometry()?;
        corners.SetSize(at.size())?;
        corners.SetCornerRadius(v2(px(PICTURE_RADIUS), px(PICTURE_RADIUS)))?;
        pane.SetClip(&c.CreateGeometricClipWithGeometry(&corners)?)?;
        let picture = self.build_picture(&pane)?;
        // A shade darker or lighter than the picture, whatever it shows.
        let edge = if t.dark {
            argb(0x24, 0xFF, 0xFF, 0xFF)
        } else {
            argb(0x24, 0x00, 0x00, 0x00)
        };
        outline(c, r, &content, at, px(PICTURE_RADIUS), edge)?;

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
        let (close_hover, _) = rounded(c, r, &close, face, px(CLOSE_RADIUS), t.highlight())?;
        close_hover.SetOpacity(0.0)?;
        let glyph = Text::new(c, &close)?;
        let cross = r.layout_line(CLOSE_GLYPH, Face::Chrome, Ink::Primary, CLOSE_SIZE)?;
        let glyph_size = glyph.show(r, &cross, s, t)?;
        glyph
            .visual
            .SetOffset(v3(0.0, ((size - glyph_size.Y) / 2.0).round()))?;

        let text = self.build_text(&content)?;
        let dots = self.build_dots(&content)?;
        let (footer, buttons) = self.build_footer(&content)?;
        Ok(View {
            content,
            pane,
            picture,
            text,
            close_at,
            close_hover,
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
        let surface = r.draw_picture(step.picture(self.theme.dark), size.X, size.Y)?;
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

    /// The step's symbol, its title and what it says, each a visual of its
    /// own in what's returned, in `content`.
    fn build_text(&self, content: &ContainerVisual) -> Result<ContainerVisual> {
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
        let mut y = px(TITLE_Y);

        // The symbol, centred on the capitals of the title's first line.
        let indent = BADGE + BADGE_GAP;
        let line = r.layout_line(step.title(), Face::Display, Ink::Primary, TEXT_W - indent)?;
        let b = px(BADGE);
        let badge = container(
            c,
            &text,
            v3(x, y + (line.cap_middle() * s - b / 2.0).round()),
        )?;
        let face = Rect {
            w: b,
            h: b,
            ..Rect::default()
        };
        let a = t.accent;
        let tint = argb(if t.dark { 0x33 } else { 0x24 }, a.R, a.G, a.B);
        rounded(c, r, &badge, face, px(BADGE_RADIUS), tint)?;
        let glyph = Text::new(c, &badge)?;
        let symbol = r.layout_line(step.glyph(), Face::LargeButton, Ink::Accent, BADGE)?;
        let size = glyph.show(r, &symbol, s, t)?;
        glyph
            .visual
            .SetOffset(v3(0.0, ((b - size.Y) / 2.0).round()))?;

        let title = Text::new(c, &text)?;
        let size = title.show(r, &line, s, t)?;
        title.visual.SetOffset(v3(px(TEXT_X + indent), y))?;
        y += size.Y.max(b) + px(TITLE_GAP);

        let detail = Text::new(c, &text)?;
        let (said, keys) = step.detail(&self.shortcuts);
        let paragraph = r.layout_paragraph(said, &keys, Ink::Secondary, TEXT_W)?;
        detail.show(r, &paragraph, s, t)?;
        detail.visual.SetOffset(v3(x, y))?;
        Ok(text)
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
    ) -> Result<(ContainerVisual, Vec<(Hit, ButtonView)>)> {
        let s = self.scale;
        let px = |dip: f32| (dip * s).round();
        let footer = container(&self.anim.compositor, content, v3(0.0, 0.0))?;
        let last = self.at + 1 == self.steps.len();
        let (w, h) = (px(BUTTON_W), px(BUTTON_H));
        let y = px(CARD_H - PAD_BOTTOM) - h;
        let mut right = px(CARD_W - PAD_X);
        let next = if last {
            tr().intro_done
        } else {
            tr().intro_next
        };
        let specs = [(Hit::Next, next, true), (Hit::Back, tr().intro_back, false)];
        let mut buttons = Vec::new();
        for (hit, label, primary) in specs {
            if hit == Hit::Back && self.at == 0 {
                continue;
            }
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
    /// it's the `primary` one, as Windows draws the one to go on with.
    fn build_button(
        &self,
        content: &ContainerVisual,
        at: Rect,
        label: &str,
        primary: bool,
    ) -> Result<ButtonView> {
        let (c, r, t, s) = (
            &self.anim.compositor,
            &self.renderer,
            &self.theme,
            self.scale,
        );
        let px = |dip: f32| (dip * s).round();
        let root = container(c, content, at.at())?;
        let face = Rect {
            w: at.w,
            h: at.h,
            ..Rect::default()
        };
        let radius = px(BUTTON_RADIUS);
        // The label stands out on the accent in the other theme's color.
        let inverse = Theme {
            dark: !t.dark,
            ..*t
        };
        let (fill, stroke, ink) = if primary {
            (t.accent, t.accent, inverse)
        } else {
            (t.control(), t.control_stroke(), *t)
        };
        rounded(c, r, &root, face, radius, fill)?;
        outline(c, r, &root, face, radius, stroke)?;
        let (hover, _) = rounded(c, r, &root, face, radius, t.highlight())?;
        hover.SetOpacity(0.0)?;
        let text = Text::new(c, &root)?;
        let line = r.layout_aligned(label, Face::Menu, Ink::Primary, BUTTON_W, Align::Center)?;
        let size = text.show(r, &line, s, &ink)?;
        text.visual
            .SetOffset(v3(0.0, ((at.h - size.Y) / 2.0).round()))?;
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
        Ok(ButtonView {
            root,
            hover,
            ring,
            at,
        })
    }

    // -----------------------------------------------------------------------
    // Geometry

    /// Sizes the card for the current scale. Returns the window's size in
    /// pixels: the card, and room for its shadow.
    fn fit(&self) -> Result<(i32, i32)> {
        let s = self.scale;
        let m = (MARGIN * s).round();
        let (w, h) = ((CARD_W * s).round(), (CARD_H * s).round());
        self.frame.resize(&self.anim, w, h, v3(m, m), s, false)?;
        Ok(((w + 2.0 * m) as i32, (h + 2.0 * m) as i32))
    }

    /// The middle of the card, in the window: what it grows from.
    fn centre(&self) -> Vector3 {
        let s = self.scale;
        let m = (MARGIN * s).round();
        Vector3 {
            X: m + (CARD_W * s).round() / 2.0,
            Y: m + (CARD_H * s).round() / 2.0,
            Z: 0.0,
        }
    }

    /// A point in window pixels, relative to the card's top left.
    fn card_point(&self, x: i32, y: i32) -> (f32, f32) {
        let m = (MARGIN * self.scale).round();
        (x as f32 - m, y as f32 - m)
    }

    fn hit_at(&self, x: i32, y: i32) -> Option<Hit> {
        let view = self.view.as_ref()?;
        let (x, y) = self.card_point(x, y);
        if view.close_at.contains(x, y) {
            return Some(Hit::Close);
        }
        view.buttons
            .iter()
            .find(|(_, b)| b.at.contains(x, y))
            .map(|(hit, _)| *hit)
    }

    /// Names the window in the language now current, as the taskbar shows it.
    fn retitle(&self) {
        unsafe {
            let _ = SetWindowTextW(self.hwnd, &HSTRING::from(tr().intro_title));
        }
    }
}

/// The scale of the window on a monitor of `scale` pixels per DIP: smaller,
/// should it not fit in the `work` area.
fn fitted(work: RECT, scale: f32) -> f32 {
    let w = (work.right - work.left) as f32 / (CARD_W + 2.0 * SCREEN_GAP);
    let h = (work.bottom - work.top) as f32 / (CARD_H + 2.0 * SCREEN_GAP);
    scale.min(w).min(h)
}

/// A ring `stroke` pixels thick just inside `rect`, in `parent`, as round
/// what the keyboard is on.
fn ring(
    c: &Compositor,
    r: &Renderer,
    parent: &ContainerVisual,
    rect: Rect,
    radius: f32,
    stroke: f32,
    color: Color,
) -> Result<SpriteVisual> {
    let shape =
        c.CreateSurfaceBrushWithSurface(&r.draw_rounded(rect.w, rect.h, radius, Some(stroke))?)?;
    shape.SetStretch(CompositionStretch::Fill)?;
    let masked = c.CreateMaskBrush()?;
    masked.SetSource(&c.CreateColorBrushWithColor(color)?)?;
    masked.SetMask(&shape)?;
    let visual = c.CreateSpriteVisual()?;
    visual.SetBrush(&masked)?;
    visual.SetOffset(rect.at())?;
    visual.SetSize(rect.size())?;
    parent.Children()?.InsertAtTop(&visual)?;
    Ok(visual)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shows_ui_access_only_where_it_can_be_had() {
        use Step::*;
        assert_eq!(Step::all(true), [Lookup, Translate, AboveStart, Ready]);
        assert_eq!(Step::all(false), [Lookup, Translate, Ready]);
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
}
