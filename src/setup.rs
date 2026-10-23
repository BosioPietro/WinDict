//! The setup downloaded from the releases page: a window like the
//! introduction's, a picture on the left of the card and what it says on
//! the right, that installs WinDict by running the Inno Setup installer it
//! carries (installer/windict.iss) silently, and says how that went.
//!
//! Windows asks for administrator rights before the window opens
//! (assets/windict-setup.manifest), which the installer needs. Started with
//! the installer's switches, as WinGet and "Update app" do, there's no
//! window: they go to the installer as they are, see [`quietly`]. Either way
//! what an install does stays in the installer; this only runs it.
//!
//! From one stage to the next, the old text slides away and the new slides
//! in after it, line after line, as from step to step in the introduction.

use std::cell::RefCell;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{channel, Receiver, Sender};

use windows::core::{w, Interface, Result, HSTRING, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::UI::Color;
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::*;
use windows_numerics::Vector3;

use crate::config::ThemeMode;
use crate::hotkey;
use crate::i18n::{self, fill, tr, Language};
use crate::installed;
use crate::render::{Face, Ink, Renderer};
use crate::theme::{argb, Theme};
use crate::ui::{
    self, button_width, container, outline, rounded, scale, v2, v3, Animator, Button, Frame, Rect,
    Text, BUTTON_H, MARGIN,
};
use crate::win::{self, Window};

const CLASS: PCWSTR = w!("WinDict.Setup");
/// The version this setup installs: the installer is built from this crate.
const VERSION: &str = env!("CARGO_PKG_VERSION");

// Geometry, in DIPs.
const CARD_W: f32 = 640.0;
const CARD_H: f32 = 380.0;
/// The picture, on the left, as far from the card's edges all round.
const INSET: f32 = 12.0;
const PANE_W: f32 = 256.0;
const PANE_H: f32 = CARD_H - 2.0 * INSET;
const PICTURE_RADIUS: f32 = 8.0;
/// Between the picture and the text, and between the text and the right.
const PAD_X: f32 = 36.0;
const TEXT_X: f32 = INSET + PANE_W + PAD_X;
const TEXT_W: f32 = CARD_W - TEXT_X - PAD_X;
/// The strip along the top that drags the window, over the picture too.
const TOP_H: f32 = 48.0;
const CLOSE_SIZE: f32 = 32.0;
const CLOSE_INSET: f32 = 10.0;
const CLOSE_RADIUS: f32 = 5.0;
const TITLE_Y: f32 = 64.0;
/// The symbol before the title, on a square tinted with the accent, and
/// the space after it.
const BADGE: f32 = 36.0;
const BADGE_RADIUS: f32 = 8.0;
const BADGE_GAP: f32 = 14.0;
const TITLE_GAP: f32 = 14.0;
/// Between what the stage says and the progress bar, or the line saying
/// Windows didn't get permission.
const PROGRESS_GAP: f32 = 24.0;
/// The progress bar: the accent part that runs along it, over a hairline.
const BAR_H: f32 = 3.0;
const TRACK_H: f32 = 1.0;
/// How much of the bar the part that runs along it is.
const RUNNER: f32 = 0.35;
/// Between the progress bar and the line under it saying what's going on.
const STATUS_GAP: f32 = 12.0;
const PAD_BOTTOM: f32 = 28.0;
const BUTTON_GAP: f32 = 8.0;
/// How far the text slides as it goes and comes.
const SLIDE: f32 = 24.0;
/// The least room kept between the window and the screen's edges.
const SCREEN_GAP: f32 = 16.0;
/// How much larger than its size in DIPs the window opens, as the
/// introduction does.
const ENLARGE: f32 = 1.33;

// Durations, in ms.
const SHOW_MS: u64 = 360;
const LEAVE_MS: u64 = 150;
const ENTER_MS: u64 = 300;
/// Each line of the text comes in this long after the one above it.
const STAGGER_MS: u64 = 40;
const HOVER_MS: u64 = 100;
const PRESS_MS: u64 = 90;
const FOCUS_MS: u64 = 150;
/// How long the progress bar's runner takes to cross it.
const RUN_MS: u64 = 1600;
/// Once what's going has faded, it's taken away.
const GONE_MS: u32 = LEAVE_MS as u32 + 30;
const TIMER_GONE: usize = 1;
/// The next of the dots after what's going on.
const TIMER_DOTS: usize = 2;
/// The window has faded out: the setup ends.
const TIMER_QUIT: usize = 3;

// Not in the windows crate.
const WM_MOUSELEAVE: u32 = 0x02A3;
const WM_DWMCOLORIZATIONCOLORCHANGED: u32 = 0x0320;

/// The installer's thread has news, in [`Setup::news`].
const WM_APP_INSTALLER: u32 = WM_APP + 1;

/// "Cancel" in Segoe Fluent Icons and Segoe MDL2 Assets.
const CLOSE_GLYPH: &str = "\u{E8BB}";

/// Where the setup is at.
#[derive(Clone, PartialEq, Debug)]
enum Stage {
    /// Waiting for Install.
    Ready,
    /// The installer is running.
    Installing,
    Done,
    /// What went wrong.
    Failed(String),
}

impl Stage {
    fn working(&self) -> bool {
        *self == Stage::Installing
    }
}

/// What the installer's thread has to say.
enum News {
    /// It couldn't be started, for this reason.
    NotStarted(String),
    /// It ended, with this exit code: 0 when it installed WinDict.
    Ended(u32),
}

/// What's under the pointer, or has the keyboard.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Hit {
    Close,
    /// Cancel, or Close.
    Secondary,
    /// Install, Open WinDict or Try again.
    Primary,
}

/// What a stage shows on the right of the card, which slides as the stage
/// changes.
struct StageView {
    /// Its title, what it says, how it's going and its buttons.
    text: ContainerVisual,
    /// Under the progress bar: what's going on.
    status: Option<Text>,
    buttons: Vec<(Hit, Button)>,
}

struct View {
    content: ContainerVisual,
    close: ContainerVisual,
    close_at: Rect,
    close_hover: SpriteVisual,
    stage: StageView,
}

struct Setup {
    hwnd: HWND,
    _target: DesktopWindowTarget,
    renderer: Renderer,
    anim: Animator,
    /// Everything on the window; fades and scales as one when showing and hiding.
    root: ContainerVisual,
    frame: Frame,
    view: Option<View>,
    /// What's on its way out, fading.
    gone: Vec<Visual>,
    stage: Stage,
    /// WinDict is installed already: this updates it.
    update: bool,
    language: Language,
    installer: &'static [u8],
    /// Where the installer is written to run it from.
    folder: PathBuf,
    news: Receiver<News>,
    sender: Sender<News>,
    /// How many steps the dots have taken.
    dots: usize,
    theme: Theme,
    scale: f32,
    hovered: Option<Hit>,
    pressed: Option<Hit>,
    focus: Option<Hit>,
    /// Whether the focus ring shows: only once the keyboard is used.
    ring: bool,
    visible: bool,
    /// Last, so it goes after everything shown on it.
    _window: Window,
}

thread_local! {
    static SETUP: RefCell<Option<Setup>> = const { RefCell::new(None) };
}

/// Runs `f` on the setup, unless it's busy already.
fn with_setup<R>(f: impl FnOnce(&mut Setup) -> R) -> Option<R> {
    SETUP.with(|setup| {
        let mut setup = setup.try_borrow_mut().ok()?;
        setup.as_mut().map(f)
    })
}

/// Shows the setup, and runs until it's closed.
pub fn run(compositor: Compositor, installer: &'static [u8]) -> Result<()> {
    let language = Language::system();
    i18n::set(language);
    ui::set_motion(ui::system_motion());
    unsafe {
        let instance = HINSTANCE(GetModuleHandleW(None)?.0);
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(setup_proc),
            hInstance: instance,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hIcon: LoadIconW(
                Some(instance),
                PCWSTR(crate::tray::ICON_ID as usize as *const u16),
            )
            .unwrap_or_default(),
            lpszClassName: CLASS,
            ..Default::default()
        });
    }
    let title = HSTRING::from(tr().setup_title);
    let window = Window::create(
        CLASS,
        PCWSTR(title.as_ptr()),
        WS_EX_NOREDIRECTIONBITMAP | WS_EX_APPWINDOW,
        WS_POPUP | WS_SYSMENU | WS_MINIMIZEBOX,
    )?;
    let mut setup = Setup::new(window, compositor, installer, language)?;
    setup.show()?;
    let (hwnd, folder) = (setup.hwnd, setup.folder.clone());
    SETUP.with(|s| *s.borrow_mut() = Some(setup));
    let _ = crate::selection::take_foreground(hwnd);
    unsafe {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    // Taken out first: the window's destroyed as it goes.
    drop(SETUP.with(|s| s.borrow_mut().take()));
    let _ = std::fs::remove_dir_all(folder);
    Ok(())
}

impl Setup {
    fn new(
        window: Window,
        compositor: Compositor,
        installer: &'static [u8],
        language: Language,
    ) -> Result<Self> {
        let hwnd = window.hwnd();
        let theme = Theme::current(ThemeMode::System);
        let target = ui::attach(hwnd, &compositor)?;
        let renderer = Renderer::new(&compositor)?;
        let root = compositor.CreateContainerVisual()?;
        target.SetRoot(&root)?;
        let frame = Frame::new(&compositor, &root, &theme)?;
        let (sender, news) = channel();
        Ok(Setup {
            hwnd,
            _target: target,
            renderer,
            anim: Animator::new(compositor)?,
            root,
            frame,
            view: None,
            gone: Vec::new(),
            stage: Stage::Ready,
            update: installed::install_dir().is_some(),
            language,
            installer,
            folder: std::env::temp_dir().join(format!("WinDict-setup-{}", std::process::id())),
            news,
            sender,
            dots: 0,
            theme,
            scale: 1.0,
            hovered: None,
            pressed: None,
            focus: Some(Hit::Primary),
            ring: false,
            visible: false,
            _window: window,
        })
    }

    /// Opens the window in the middle of the screen the pointer is on.
    fn show(&mut self) -> Result<()> {
        let (work, s) = ui::monitor_at(win::cursor_pos());
        self.scale = fitted(work, s);
        self.frame
            .apply_theme(&self.renderer, &self.theme, self.scale)?;
        self.replace_view()?;
        let (w, h) = self.fit()?;
        let left = (work.left + work.right - w) / 2;
        let top = (work.top + work.bottom - h) / 2;

        // Rises out of the middle, a little small and low, its text a beat
        // after it.
        let root = &self.root;
        root.SetCenterPoint(self.centre())?;
        root.SetOpacity(0.0)?;
        root.SetScale(scale(0.96))?;
        root.SetOffset(v3(0.0, 12.0 * self.scale))?;
        unsafe {
            let _ = SetWindowPos(self.hwnd, Some(HWND_TOP), left, top, w, h, SWP_SHOWWINDOW);
        }
        self.visible = true;
        self.anim.fade_in(root, ui::FADE_IN_MS, SHOW_MS)?;
        if let Some(view) = &self.view {
            self.bring_in(
                &view.stage.text,
                v3(0.0, SLIDE / 3.0 * self.scale),
                ui::FADE_IN_MS,
            )?;
        }
        Ok(())
    }

    /// Fades the window out, and ends the setup once it has; not while the
    /// installer runs.
    fn close(&mut self) -> Result<()> {
        if self.stage.working() || !self.visible {
            return Ok(());
        }
        self.visible = false;
        win::set_timer(self.hwnd, TIMER_QUIT, ui::HIDE_MS as u32);
        self.anim.fade_out(&self.root, 6.0 * self.scale)
    }

    // -----------------------------------------------------------------------
    // Installing

    /// Runs the installer silently, on a thread of its own that waits for it.
    fn install(&mut self) -> Result<()> {
        let exe = match write_out(&self.folder, self.installer) {
            Ok(exe) => exe,
            Err(e) => {
                let error = fill(tr().setup_not_started, &[("error", &e)]);
                return self.go(Stage::Failed(error));
            }
        };
        let language = match self.language {
            Language::Italian => "it",
            Language::English => "en",
        };
        let switches: Vec<OsString> = ["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART"]
            .into_iter()
            .map(OsString::from)
            .chain([format!("/LANG={language}").into()])
            .collect();
        let (sender, hwnd) = (self.sender.clone(), self.hwnd.0 as isize);
        std::thread::spawn(move || {
            let news = match run_installer(&exe, &switches) {
                Ok(code) => News::Ended(code),
                Err(e) => News::NotStarted(e.to_string()),
            };
            let _ = sender.send(news);
            unsafe {
                let hwnd = Some(HWND(hwnd as _));
                let _ = PostMessageW(hwnd, WM_APP_INSTALLER, WPARAM(0), LPARAM(0));
            }
        });
        self.go(Stage::Installing)
    }

    /// Takes in what the installer's thread said.
    fn hear(&mut self) -> Result<()> {
        while let Ok(news) = self.news.try_recv() {
            let stage = match news {
                News::NotStarted(error) => {
                    Stage::Failed(fill(tr().setup_not_started, &[("error", &error)]))
                }
                News::Ended(0) => Stage::Done,
                News::Ended(code) => {
                    Stage::Failed(fill(tr().setup_failed_detail, &[("code", &code)]))
                }
            };
            self.go(stage)?;
        }
        Ok(())
    }

    /// Opens the WinDict just installed through Explorer, as the installer
    /// does: as the user, not with the administrator rights this setup has.
    fn open_windict(&self) {
        if let Some(dir) = installed::install_dir() {
            let _ = Command::new(explorer())
                .arg(dir.join(installed::LAUNCHER))
                .spawn();
        }
    }

    // -----------------------------------------------------------------------
    // What clicks and keys do

    fn activate(&mut self, hit: Hit) -> Result<()> {
        match (&self.stage, hit) {
            (_, Hit::Close) | (_, Hit::Secondary) => self.close(),
            (Stage::Ready | Stage::Failed(_), Hit::Primary) => self.install(),
            (Stage::Done, Hit::Primary) => {
                self.open_windict();
                self.close()
            }
            (Stage::Installing, Hit::Primary) => Ok(()),
        }
    }

    fn hover(&mut self, point: Option<(i32, i32)>) -> Result<()> {
        let hit = point.and_then(|(x, y)| self.hit_at(x, y));
        if hit == self.hovered {
            return Ok(());
        }
        let old = std::mem::replace(&mut self.hovered, hit);
        self.show_hover(old)?;
        self.show_hover(hit)
    }

    fn press(&mut self, x: i32, y: i32) -> Result<()> {
        let hit = self.hit_at(x, y);
        // The pointer's in charge now: the ring goes until Tab brings it back.
        self.ring = false;
        if let Some(hit) = hit.filter(|&hit| hit != Hit::Close) {
            self.focus = Some(hit);
        }
        self.show_focus()?;
        self.set_pressed(hit)
    }

    fn release(&mut self, x: i32, y: i32) -> Result<()> {
        let pressed = self.pressed;
        self.set_pressed(None)?;
        match pressed {
            Some(hit) if self.hit_at(x, y) == Some(hit) => self.activate(hit),
            _ => Ok(()),
        }
    }

    fn key(&mut self, key: VIRTUAL_KEY) -> Result<()> {
        match key {
            VK_ESCAPE => self.close(),
            VK_RETURN | VK_SPACE => match self.focus.filter(|&f| self.button(f).is_some()) {
                Some(hit) => self.activate(hit),
                None => Ok(()),
            },
            VK_TAB | VK_LEFT | VK_RIGHT => {
                let forward = match key {
                    VK_TAB => !hotkey::is_held(VK_SHIFT),
                    key => key == VK_RIGHT,
                };
                self.step_focus(forward)
            }
            _ => Ok(()),
        }
    }

    /// Moves the keyboard to the other button: there are two at most.
    fn step_focus(&mut self, forward: bool) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        // From the left.
        let mut stops: Vec<&(Hit, Button)> = view.stage.buttons.iter().collect();
        stops.sort_by(|a, b| a.1.at.x.total_cmp(&b.1.at.x));
        let stops: Vec<Hit> = stops.into_iter().map(|(hit, _)| *hit).collect();
        if stops.is_empty() {
            return Ok(());
        }
        let n = stops.len();
        let at = self.focus.and_then(|f| stops.iter().position(|&s| s == f));
        // The first Tab shows where the focus is.
        let next = match (at, forward, self.ring) {
            (Some(i), _, false) => i,
            (None, _, _) => n - 1,
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

    fn on_timer(&mut self, id: usize) -> Result<()> {
        match id {
            TIMER_DOTS => {
                self.dots += 1;
                self.show_status()
            }
            TIMER_QUIT => {
                win::kill_timer(self.hwnd, TIMER_QUIT);
                unsafe { PostQuitMessage(0) };
                Ok(())
            }
            _ => {
                win::kill_timer(self.hwnd, TIMER_GONE);
                for gone in self.gone.drain(..) {
                    if let Ok(parent) = gone.Parent() {
                        parent.Children()?.Remove(&gone)?;
                    }
                }
                Ok(())
            }
        }
    }

    fn retheme(&mut self) -> Result<()> {
        let theme = Theme::current(ThemeMode::System);
        let same = (theme.dark, theme.transparency, theme.accent)
            == (self.theme.dark, self.theme.transparency, self.theme.accent);
        if same {
            return Ok(());
        }
        self.theme = theme;
        self.frame.apply_theme(&self.renderer, &theme, self.scale)?;
        self.replace_view()
    }

    /// The window moved to a monitor with another `dpi`. `suggested` is
    /// where Windows would put it, in screen pixels.
    fn rescale(&mut self, dpi: u32, suggested: RECT) -> Result<()> {
        let centre = POINT {
            x: (suggested.left + suggested.right) / 2,
            y: (suggested.top + suggested.bottom) / 2,
        };
        let s = fitted(ui::monitor_at(centre).0, dpi as f32 / 96.0);
        if (s - self.scale).abs() < 0.01 {
            return Ok(());
        }
        self.scale = s;
        self.frame.apply_theme(&self.renderer, &self.theme, s)?;
        self.replace_view()?;
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
    // Showing the state

    /// Moves on to `stage`: the old text slides away and fades, and the new
    /// slides in after it.
    fn go(&mut self, stage: Stage) -> Result<()> {
        let was_working = self.stage.working();
        (self.stage, self.dots) = (stage, 0);
        if self.stage.working() {
            win::set_timer(self.hwnd, TIMER_DOTS, ui::DOTS_MS);
        } else {
            win::kill_timer(self.hwnd, TIMER_DOTS);
        }
        let Some(view) = self.view.take() else {
            return Ok(());
        };
        let (anim, dx) = (&self.anim, SLIDE * self.scale);
        let old = view.stage.text.clone();
        Animator::stop(&old, &["Opacity", "Offset"]);
        anim.scalar(&old, "Opacity", 0.0, LEAVE_MS, 0, &anim.accelerate)?;
        anim.vector3(&old, "Offset", v3(-dx, 0.0), LEAVE_MS, 0, &anim.accelerate)?;
        self.gone.push(old.cast()?);
        win::set_timer(self.hwnd, TIMER_GONE, GONE_MS);

        let stage = self.build_stage(&view.content)?;
        self.bring_in(&stage.text, v3(dx, 0.0), LEAVE_MS)?;
        self.view = Some(View { stage, ..view });
        if was_working != self.stage.working() {
            self.show_close()?;
        }
        // What goes on has the keyboard.
        self.focus = Some(Hit::Primary);
        self.show_status()?;
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

    fn show_hover(&self, hit: Option<Hit>) -> Result<()> {
        let (Some(view), Some(hit)) = (&self.view, hit) else {
            return Ok(());
        };
        let hover = match hit {
            Hit::Close if !self.stage.working() => &view.close_hover,
            Hit::Close => return Ok(()),
            _ => match self.button(hit) {
                Some(button) => &button.hover,
                None => return Ok(()),
            },
        };
        let on = f32::from(u8::from(self.hovered == Some(hit)));
        let anim = &self.anim;
        anim.scalar(hover, "Opacity", on, HOVER_MS, 0, &anim.decelerate)
    }

    fn show_focus(&self) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let anim = &self.anim;
        for (hit, button) in &view.stage.buttons {
            let to = f32::from(u8::from(self.ring && self.focus == Some(*hit)));
            anim.scalar(&button.ring, "Opacity", to, FOCUS_MS, 0, &anim.decelerate)?;
        }
        Ok(())
    }

    /// Dims the close button while the installer runs, when it can't be
    /// closed.
    fn show_close(&self) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let to = if self.stage.working() { 0.4 } else { 1.0 };
        let anim = &self.anim;
        if self.stage.working() {
            anim.scalar(
                &view.close_hover,
                "Opacity",
                0.0,
                HOVER_MS,
                0,
                &anim.decelerate,
            )?;
        }
        anim.scalar(&view.close, "Opacity", to, HOVER_MS, 0, &anim.decelerate)
    }

    /// Says under the progress bar what's going on, with dots.
    fn show_status(&self) -> Result<()> {
        let Some(text) = self
            .view
            .as_ref()
            .and_then(|view| view.stage.status.as_ref())
        else {
            return Ok(());
        };
        let line = ui::waiting(tr().setup_installing, self.dots);
        let line = self
            .renderer
            .layout_line(&line, Face::Small, Ink::Secondary, TEXT_W)?;
        text.show(&self.renderer, &line, self.scale, &self.theme)?;
        Ok(())
    }

    fn button(&self, hit: Hit) -> Option<&Button> {
        let view = self.view.as_ref()?;
        view.stage
            .buttons
            .iter()
            .find(|(h, _)| *h == hit)
            .map(|(_, b)| b)
    }

    // -----------------------------------------------------------------------
    // Building the visuals

    /// Builds the view afresh for the current scale and theme.
    fn replace_view(&mut self) -> Result<()> {
        let view = self.build()?;
        if let Some(old) = self.view.replace(view) {
            self.frame.card.Children()?.Remove(&old.content)?;
        }
        self.show_close()?;
        self.show_status()?;
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
        let content = c.CreateContainerVisual()?;
        self.frame.card.Children()?.InsertAtTop(&content)?;

        // The picture, its corners rounded, with a hairline round it: the
        // card on a word, as the introduction opens on.
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
        let picture = if t.dark {
            include_bytes!("../assets/intro/lookup-dark.jpg").as_slice()
        } else {
            include_bytes!("../assets/intro/lookup-light.jpg").as_slice()
        };
        let surface = r.draw_picture(picture, at.w, at.h)?;
        let sprite = c.CreateSpriteVisual()?;
        sprite.SetBrush(&c.CreateSurfaceBrushWithSurface(&surface)?)?;
        sprite.SetSize(at.size())?;
        pane.Children()?.InsertAtTop(&sprite)?;
        let edge = if t.dark {
            argb(0x24, 0xFF, 0xFF, 0xFF)
        } else {
            argb(0x24, 0x00, 0x00, 0x00)
        };
        outline(c, r, &content, at, px(PICTURE_RADIUS), edge)?;

        let size = px(CLOSE_SIZE);
        let close_at = Rect {
            x: px(CARD_W) - px(CLOSE_INSET) - size,
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

        let stage = self.build_stage(&content)?;
        Ok(View {
            content,
            close,
            close_at,
            close_hover,
            stage,
        })
    }

    /// The stage's symbol, its title, what it says, how it's going and its
    /// buttons, each a visual of its own in the text returned, in `content`.
    fn build_stage(&self, content: &ContainerVisual) -> Result<StageView> {
        let (c, r, t, s) = (
            &self.anim.compositor,
            &self.renderer,
            &self.theme,
            self.scale,
        );
        let px = |dip: f32| (dip * s).round();
        let text = container(c, content, v3(0.0, 0.0))?;
        let x = px(TEXT_X);
        let mut y = px(TITLE_Y);
        let version: &dyn std::fmt::Display = &VERSION;
        let (glyph, title, said) = match (&self.stage, self.update) {
            (Stage::Ready | Stage::Installing, false) => (
                "\u{E896}",
                tr().setup_install,
                fill(tr().setup_install_detail, &[("version", version)]),
            ),
            (Stage::Ready | Stage::Installing, true) => (
                "\u{E896}",
                tr().setup_update,
                fill(tr().setup_update_detail, &[("version", version)]),
            ),
            (Stage::Done, false) => (
                "\u{E73E}",
                tr().setup_installed,
                tr().setup_installed_detail.to_owned(),
            ),
            (Stage::Done, true) => (
                "\u{E73E}",
                tr().setup_updated,
                fill(tr().setup_updated_detail, &[("version", version)]),
            ),
            (Stage::Failed(error), _) => ("\u{E783}", tr().setup_failed, error.clone()),
        };
        let failed = matches!(self.stage, Stage::Failed(_));

        // The symbol, centred on the capitals of the title's first line.
        let indent = BADGE + BADGE_GAP;
        let line = r.layout_line(title, Face::Display, Ink::Primary, TEXT_W - indent)?;
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
        let (hue, ink) = if failed {
            let red = t.critical();
            let channel = |v: f32| (v * 255.0).round() as u8;
            let red = Color {
                A: 0xFF,
                R: channel(red.r),
                G: channel(red.g),
                B: channel(red.b),
            };
            (red, Ink::Critical)
        } else {
            (t.accent, Ink::Accent)
        };
        let tint = argb(if t.dark { 0x33 } else { 0x24 }, hue.R, hue.G, hue.B);
        rounded(c, r, &badge, face, px(BADGE_RADIUS), tint)?;
        let symbol = Text::new(c, &badge)?;
        let shape = r.layout_line(glyph, Face::LargeButton, ink, BADGE)?;
        let size = symbol.show(r, &shape, s, t)?;
        symbol
            .visual
            .SetOffset(v3(0.0, ((b - size.Y) / 2.0).round()))?;

        let heading = Text::new(c, &text)?;
        let size = heading.show(r, &line, s, t)?;
        heading.visual.SetOffset(v3(px(TEXT_X + indent), y))?;
        y += size.Y.max(b) + px(TITLE_GAP);

        let detail = Text::new(c, &text)?;
        let paragraph = r.layout_paragraph(&said, &[], Ink::Secondary, TEXT_W)?;
        let size = detail.show(r, &paragraph, s, t)?;
        detail.visual.SetOffset(v3(x, y))?;
        y += size.Y + px(PROGRESS_GAP);

        let mut status = None;
        if self.stage == Stage::Installing {
            self.build_progress(&text, v3(x, y))?;
            let line = Text::new(c, &text)?;
            line.visual
                .SetOffset(v3(x, y + px(BAR_H) + px(STATUS_GAP)))?;
            status = Some(line);
        }

        // The buttons, at the bottom right, the one to go on with last.
        let footer = container(c, &text, v3(0.0, 0.0))?;
        let labels = match (&self.stage, self.update) {
            (Stage::Ready, false) => Some((tr().setup_cancel, tr().setup_install_button)),
            (Stage::Ready, true) => Some((tr().setup_cancel, tr().setup_update_button)),
            (Stage::Done, _) => Some((tr().setup_close, tr().setup_open)),
            (Stage::Failed(_), _) => Some((tr().setup_close, tr().setup_try_again)),
            (Stage::Installing, _) => None,
        };
        let mut buttons = Vec::new();
        if let Some((secondary, primary)) = labels {
            let h = px(BUTTON_H);
            let top = px(CARD_H - PAD_BOTTOM) - h;
            let mut right = px(CARD_W - PAD_X);
            for (hit, label) in [(Hit::Primary, primary), (Hit::Secondary, secondary)] {
                let w = px(button_width(r, label)?);
                let at = Rect {
                    x: right - w,
                    y: top,
                    w,
                    h,
                };
                right = at.x - px(BUTTON_GAP);
                let primary = hit == Hit::Primary;
                buttons.push((hit, ui::button(c, r, t, s, &footer, at, label, primary)?));
            }
        }
        Ok(StageView {
            text,
            status,
            buttons,
        })
    }

    /// A progress bar at `at` in `parent`, as wide as the text, that says
    /// something's going on but not how far along: the accent runs along a
    /// hairline, again and again.
    fn build_progress(&self, parent: &ContainerVisual, at: Vector3) -> Result<()> {
        let (c, r, t, s) = (
            &self.anim.compositor,
            &self.renderer,
            &self.theme,
            self.scale,
        );
        let (w, h) = ((TEXT_W * s).round(), (BAR_H * s).round());
        let bar = container(c, parent, at)?;
        bar.SetSize(v2(w, h))?;
        bar.SetClip(&c.CreateInsetClip()?)?;
        let track_h = (TRACK_H * s).round().max(1.0);
        let track = Rect {
            y: ((h - track_h) / 2.0).round(),
            w,
            h: track_h,
            ..Rect::default()
        };
        rounded(c, r, &bar, track, track_h / 2.0, t.strong_stroke())?;
        let runner_w = (w * RUNNER).round();
        let runner = Rect {
            w: runner_w,
            h,
            ..Rect::default()
        };
        let (runner, _) = rounded(c, r, &bar, runner, h / 2.0, t.accent)?;
        if !ui::motion() {
            // Still, in the middle.
            return runner.SetOffset(v3(((w - runner_w) / 2.0).round(), 0.0));
        }
        let run = c.CreateVector3KeyFrameAnimation()?;
        run.InsertKeyFrame(0.0, v3(-runner_w, 0.0))?;
        run.InsertKeyFrameWithEasingFunction(1.0, v3(w, 0.0), &self.anim.fade)?;
        run.SetDuration(ui::ts(RUN_MS))?;
        run.SetIterationBehavior(AnimationIterationBehavior::Forever)?;
        runner.StartAnimation(&HSTRING::from("Offset"), &run)
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

    /// Whether a point in window pixels is on the strip along the top, which
    /// drags the window.
    fn caption_at(&self, x: i32, y: i32) -> bool {
        let Some(view) = &self.view else {
            return false;
        };
        let (x, y) = self.card_point(x, y);
        let px = |dip: f32| (dip * self.scale).round();
        x >= 0.0 && x < px(CARD_W) && y >= 0.0 && y < px(TOP_H) && !view.close_at.contains(x, y)
    }

    fn hit_at(&self, x: i32, y: i32) -> Option<Hit> {
        let view = self.view.as_ref()?;
        let (x, y) = self.card_point(x, y);
        if view.close_at.contains(x, y) {
            return Some(Hit::Close);
        }
        view.stage
            .buttons
            .iter()
            .find(|(_, b)| b.at.contains(x, y))
            .map(|(hit, _)| *hit)
    }
}

/// Runs the installer silently, as WinGet and "Update app" do, passing it
/// `switches` as they are, and returns its exit code. There's no window.
pub fn quietly(installer: &[u8], switches: &[OsString]) -> i32 {
    let folder = std::env::temp_dir().join(format!("WinDict-setup-{}", std::process::id()));
    let code = write_out(&folder, installer)
        .and_then(|exe| run_installer(&exe, switches))
        .map_or(1, |code| code as i32);
    let _ = std::fs::remove_dir_all(folder);
    code
}

/// Writes `installer` out into `folder`, and returns where.
fn write_out(folder: &Path, installer: &[u8]) -> std::io::Result<PathBuf> {
    let exe = folder.join(format!("WinDict-v{VERSION}-inno.exe"));
    std::fs::create_dir_all(folder)?;
    std::fs::write(&exe, installer)?;
    Ok(exe)
}

/// Runs the installer `exe` with `switches` and waits for it: it has the
/// administrator rights this setup was started with. Returns its exit code.
fn run_installer(exe: &Path, switches: &[OsString]) -> std::io::Result<u32> {
    let status = Command::new(exe).args(switches).status()?;
    // Inno Setup's exit codes are small and positive.
    Ok(status.code().map_or(1, |code| code as u32))
}

/// Explorer, which starts what it's given as the user signed in.
fn explorer() -> PathBuf {
    let windows = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    Path::new(&windows).join("explorer.exe")
}

/// The scale of the window on a monitor of `scale` pixels per DIP, enlarged
/// by `ENLARGE`: smaller, should it not fit in the `work` area.
fn fitted(work: RECT, scale: f32) -> f32 {
    let w = (work.right - work.left) as f32 / (CARD_W + 2.0 * SCREEN_GAP);
    let h = (work.bottom - work.top) as f32 / (CARD_H + 2.0 * SCREEN_GAP);
    (scale * ENLARGE).min(w).min(h)
}

unsafe extern "system" fn setup_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // Mouse messages carry client coordinates (screen ones for WM_NCHITTEST).
    let (x, y) = win::lparam_point(lparam);
    let default = || unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    match msg {
        // The top drags the window.
        WM_NCHITTEST => {
            let hit = default();
            let mut pt = POINT { x, y };
            let caption = hit.0 == HTCLIENT as isize
                && unsafe { ScreenToClient(hwnd, &mut pt).as_bool() }
                && with_setup(|s| s.caption_at(pt.x, pt.y)).unwrap_or(false);
            if caption {
                LRESULT(HTCAPTION as isize)
            } else {
                hit
            }
        }
        WM_MOUSEMOVE => {
            // Ask for WM_MOUSELEAVE, to put out the hover when the pointer goes.
            win::track_mouse_leave(hwnd);
            with_setup(|s| s.hover(Some((x, y))));
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            with_setup(|s| s.hover(None));
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            unsafe { SetCapture(hwnd) };
            with_setup(|s| s.press(x, y));
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            unsafe {
                let _ = ReleaseCapture();
            }
            with_setup(|s| s.release(x, y));
            LRESULT(0)
        }
        WM_KEYDOWN => {
            let key = VIRTUAL_KEY(wparam.0 as u16);
            with_setup(|s| s.key(key));
            LRESULT(0)
        }
        // Alt on its own: there's no menu bar to go to.
        WM_SYSCOMMAND if (wparam.0 & 0xFFF0) as u32 == SC_KEYMENU && lparam.0 == 0 => LRESULT(0),
        WM_ACTIVATE => {
            if (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE {
                with_setup(|s| s.set_pressed(None));
            }
            default()
        }
        WM_TIMER => {
            let id = wparam.0;
            with_setup(|s| s.on_timer(id));
            LRESULT(0)
        }
        WM_APP_INSTALLER => {
            with_setup(|s| s.hear());
            LRESULT(0)
        }
        // Alt+F4, or Close on the taskbar: the window fades out, and only
        // then goes; not while the installer runs.
        WM_CLOSE => {
            with_setup(|s| s.close());
            LRESULT(0)
        }
        // Dragged to a monitor with another scale: redrawn for it.
        WM_DPICHANGED => {
            let suggested = unsafe { *(lparam.0 as *const RECT) };
            let dpi = (wparam.0 & 0xFFFF) as u32;
            with_setup(|s| s.rescale(dpi, suggested));
            LRESULT(0)
        }
        // Light or dark mode, or the accent color, changed.
        WM_SETTINGCHANGE | WM_DWMCOLORIZATIONCOLORCHANGED => {
            with_setup(|s| s.retheme());
            default()
        }
        _ => default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buttons_fit_beside_the_picture() {
        // Both buttons of every stage, side by side, in the text's width.
        for language in Language::ALL {
            i18n::set(*language);
            let pairs = [
                (tr().setup_cancel, tr().setup_install_button),
                (tr().setup_cancel, tr().setup_update_button),
                (tr().setup_close, tr().setup_open),
                (tr().setup_close, tr().setup_try_again),
            ];
            for (a, b) in pairs {
                // At most 8 DIPs a letter, and the padding either side.
                let width = |label: &str| (label.chars().count() as f32 * 8.0 + 32.0).max(96.0);
                assert!(width(a) + width(b) + BUTTON_GAP <= TEXT_W, "{a} {b}");
            }
        }
        i18n::set(Language::English);
    }
}
