//! The definition card: a borderless, non-activating, top-most window whose
//! content is a Windows.UI.Composition visual tree. Blur comes from the host
//! backdrop brush; every animation runs on the system compositor, so motion
//! stays smooth no matter what our UI thread is doing.

use std::time::{Duration, Instant};

use windows::core::{Interface, Result, BOOL, HSTRING};
use windows::Foundation::TimeSpan;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::WinRT::Composition::ICompositorDesktopInterop;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::*;
use windows_numerics::{Vector2, Vector3};

use crate::render::{Body, Link, Renderer};
use crate::theme::Theme;

// Geometry, in DIPs.
const CARD_W: f32 = 384.0;
const MAX_CARD_H: f32 = 460.0;
const MARGIN: f32 = 32.0; // room for the shadow
const RADIUS: f32 = 12.0;
const PAD_X: f32 = 22.0;
const PAD_TOP: f32 = 18.0;
const HEADER_GAP: f32 = 12.0;
const PAD_BOTTOM: f32 = 18.0;
const SHADOW_DY: f32 = 6.0;
const ANCHOR_GAP: f32 = 10.0;
const SCREEN_GAP: f32 = 8.0;

// Durations, in ms.
pub const SHOW_MS: u64 = 340;
pub const HIDE_MS: u64 = 120;
const RESIZE_MS: u64 = 300;
const SCROLL_MS: u64 = 140;
/// Ease-out curve for scrolling: quick to respond, soft to settle.
const SCROLL_EASE: (f32, f32, f32, f32) = (0.25, 0.8, 0.4, 1.0);

struct ScrollAnim {
    from: f32,
    started: Instant,
}

/// Progress of a CSS-style cubic-bezier easing at time `t` (0..=1).
fn cubic_bezier((x1, y1, x2, y2): (f32, f32, f32, f32), t: f32) -> f32 {
    let bezier = |a: f32, b: f32, u: f32| {
        3.0 * a * u * (1.0 - u).powi(2) + 3.0 * b * u * u * (1.0 - u) + u.powi(3)
    };
    // Solve x(u) = t by bisection, then evaluate y(u).
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..24 {
        let mid = (lo + hi) / 2.0;
        if bezier(x1, x2, mid) < t {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    bezier(y1, y2, (lo + hi) / 2.0)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Transition {
    /// No animation (the card is appearing).
    Instant,
    /// Different content: the body fades in, the card resizes.
    Replace,
    /// The same entry, refined: no fade, scroll position kept.
    Refine,
}

pub struct Popup {
    pub hwnd: HWND,
    compositor: Compositor,
    _target: DesktopWindowTarget,
    renderer: Renderer,
    decelerate: CompositionEasingFunction,
    accelerate: CompositionEasingFunction,
    scroll_ease: CompositionEasingFunction,

    root: ContainerVisual,
    frame: ContainerVisual,
    shadow: SpriteVisual,
    shadow_surface: CompositionSurfaceBrush,
    shadow_brush: CompositionNineGridBrush,
    card: ContainerVisual,
    clip: CompositionRoundedRectangleGeometry,
    tint: CompositionColorBrush,
    header: SpriteVisual,
    header_brush: CompositionSurfaceBrush,
    header_back: SpriteVisual,
    header_back_brush: CompositionSurfaceBrush,
    header_surface: Option<CompositionDrawingSurface>,
    links: Vec<Link>,
    divider: SpriteVisual,
    divider_brush: CompositionColorBrush,
    body_host: ContainerVisual,
    body: SpriteVisual,
    body_brush: CompositionSurfaceBrush,
    loader: ContainerVisual,
    dots: Vec<SpriteVisual>,
    dot_brush: CompositionColorBrush,
    scrollbar: SpriteVisual,
    scrollbar_clip: CompositionRoundedRectangleGeometry,
    scrollbar_brush: CompositionColorBrush,
    border: ShapeVisual,
    border_geometry: CompositionRoundedRectangleGeometry,
    border_brush: CompositionColorBrush,

    theme: Theme,
    shadow_key: Option<(u32, bool)>,
    scale: f32,
    grow_up: bool,
    /// Window position and size, physical pixels.
    window: RECT,
    /// Card height and header area height, physical pixels.
    card_h: f32,
    header_h: f32,
    body_h: f32,
    scroll: f32,
    scroll_anim: Option<ScrollAnim>,
    loading: bool,
    pub visible: bool,
}

fn ts(ms: u64) -> TimeSpan {
    TimeSpan::from(Duration::from_millis(ms))
}

fn v2(x: f32, y: f32) -> Vector2 {
    Vector2 { X: x, Y: y }
}

fn v3(x: f32, y: f32) -> Vector3 {
    Vector3 { X: x, Y: y, Z: 0.0 }
}

impl Popup {
    pub fn new(hwnd: HWND, compositor: Compositor, theme: Theme) -> Result<Popup> {
        unsafe {
            // Lets the host backdrop brush sample what's behind the window.
            let on = BOOL(1);
            let size = std::mem::size_of::<BOOL>() as u32;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_HOSTBACKDROPBRUSH,
                &on as *const _ as _,
                size,
            );
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_TRANSITIONS_FORCEDISABLED,
                &on as *const _ as _,
                size,
            );
        }
        let interop: ICompositorDesktopInterop = compositor.cast()?;
        let target = unsafe { interop.CreateDesktopWindowTarget(hwnd, true)? };
        let renderer = Renderer::new(&compositor)?;
        let c = &compositor;

        let decelerate = c
            .CreateCubicBezierEasingFunction(v2(0.1, 0.9), v2(0.2, 1.0))?
            .cast()?;
        let (x1, y1, x2, y2) = SCROLL_EASE;
        let scroll_ease = c
            .CreateCubicBezierEasingFunction(v2(x1, y1), v2(x2, y2))?
            .cast()?;
        let accelerate = c
            .CreateCubicBezierEasingFunction(v2(0.7, 0.0), v2(1.0, 0.5))?
            .cast()?;

        let root = c.CreateContainerVisual()?;
        target.SetRoot(&root)?;
        let frame = c.CreateContainerVisual()?;
        root.Children()?.InsertAtTop(&frame)?;

        let shadow = c.CreateSpriteVisual()?;
        let shadow_surface = c.CreateSurfaceBrush()?;
        let shadow_brush = c.CreateNineGridBrush()?;
        shadow_brush.SetSource(&shadow_surface)?;
        shadow.SetBrush(&shadow_brush)?;
        frame.Children()?.InsertAtTop(&shadow)?;

        let card = c.CreateContainerVisual()?;
        let clip = c.CreateRoundedRectangleGeometry()?;
        card.SetClip(&c.CreateGeometricClipWithGeometry(&clip)?)?;
        frame.Children()?.InsertAtTop(&card)?;

        // Acrylic: the blurred desktop, then a translucent tint on top.
        let backdrop = c.CreateSpriteVisual()?;
        backdrop.SetBrush(&c.CreateHostBackdropBrush()?)?;
        backdrop.SetRelativeSizeAdjustment(v2(1.0, 1.0))?;
        card.Children()?.InsertAtTop(&backdrop)?;
        let tint = c.CreateColorBrushWithColor(theme.tint())?;
        let tint_visual = c.CreateSpriteVisual()?;
        tint_visual.SetBrush(&tint)?;
        tint_visual.SetRelativeSizeAdjustment(v2(1.0, 1.0))?;
        card.Children()?.InsertAtTop(&tint_visual)?;

        let header_back = c.CreateSpriteVisual()?;
        let header_back_brush = c.CreateSurfaceBrush()?;
        header_back_brush.SetStretch(CompositionStretch::None)?;
        header_back_brush.SetHorizontalAlignmentRatio(0.0)?;
        header_back_brush.SetVerticalAlignmentRatio(0.0)?;
        header_back.SetBrush(&header_back_brush)?;
        header_back.SetOpacity(0.0)?;
        card.Children()?.InsertAtTop(&header_back)?;

        let header = c.CreateSpriteVisual()?;
        let header_brush = c.CreateSurfaceBrush()?;
        header_brush.SetStretch(CompositionStretch::None)?;
        header_brush.SetHorizontalAlignmentRatio(0.0)?;
        header_brush.SetVerticalAlignmentRatio(0.0)?;
        header.SetBrush(&header_brush)?;
        card.Children()?.InsertAtTop(&header)?;

        let body_host = c.CreateContainerVisual()?;
        body_host.SetClip(&c.CreateInsetClip()?)?;
        card.Children()?.InsertAtTop(&body_host)?;
        let body = c.CreateSpriteVisual()?;
        let body_brush = c.CreateSurfaceBrush()?;
        body_brush.SetStretch(CompositionStretch::None)?;
        body_brush.SetHorizontalAlignmentRatio(0.0)?;
        body_brush.SetVerticalAlignmentRatio(0.0)?;
        body.SetBrush(&body_brush)?;
        body_host.Children()?.InsertAtTop(&body)?;

        let loader = c.CreateContainerVisual()?;
        body_host.Children()?.InsertAtTop(&loader)?;
        let dot_brush = c.CreateColorBrushWithColor(theme.accent)?;
        let mut dots = Vec::new();
        for _ in 0..3 {
            let dot = c.CreateSpriteVisual()?;
            dot.SetBrush(&dot_brush)?;
            let ellipse = c.CreateEllipseGeometry()?;
            dot.SetClip(&c.CreateGeometricClipWithGeometry(&ellipse)?)?;
            loader.Children()?.InsertAtTop(&dot)?;
            dots.push(dot);
        }

        let divider = c.CreateSpriteVisual()?;
        let divider_brush = c.CreateColorBrush()?;
        divider.SetBrush(&divider_brush)?;
        divider.SetOpacity(0.0)?;
        card.Children()?.InsertAtTop(&divider)?;

        let scrollbar = c.CreateSpriteVisual()?;
        let scrollbar_clip = c.CreateRoundedRectangleGeometry()?;
        scrollbar.SetClip(&c.CreateGeometricClipWithGeometry(&scrollbar_clip)?)?;
        let scrollbar_brush = c.CreateColorBrush()?;
        scrollbar.SetBrush(&scrollbar_brush)?;
        scrollbar.SetOpacity(0.0)?;
        card.Children()?.InsertAtTop(&scrollbar)?;

        // A hairline edge, as on Windows 11 flyouts.
        let border = c.CreateShapeVisual()?;
        let border_geometry = c.CreateRoundedRectangleGeometry()?;
        let border_shape = c.CreateSpriteShapeWithGeometry(&border_geometry)?;
        let border_brush = c.CreateColorBrush()?;
        border_shape.SetStrokeBrush(&border_brush)?;
        border_shape.SetStrokeThickness(1.0)?;
        border.Shapes()?.Append(&border_shape)?;
        frame.Children()?.InsertAtTop(&border)?;

        Ok(Popup {
            hwnd,
            compositor,
            _target: target,
            renderer,
            decelerate,
            accelerate,
            scroll_ease,
            root,
            frame,
            shadow,
            shadow_surface,
            shadow_brush,
            card,
            clip,
            tint,
            header,
            header_brush,
            header_back,
            header_back_brush,
            header_surface: None,
            links: Vec::new(),
            divider,
            divider_brush,
            body_host,
            body,
            body_brush,
            loader,
            dots,
            dot_brush,
            scrollbar,
            scrollbar_clip,
            scrollbar_brush,
            border,
            border_geometry,
            border_brush,
            theme,
            shadow_key: None,
            scale: 1.0,
            grow_up: false,
            window: RECT::default(),
            card_h: 0.0,
            header_h: 0.0,
            body_h: 0.0,
            scroll: 0.0,
            scroll_anim: None,
            loading: false,
            visible: false,
        })
    }

    // -----------------------------------------------------------------------
    // Animation helpers

    fn scalar(
        &self,
        target: &impl Interface,
        property: &str,
        to: f32,
        ms: u64,
        delay: u64,
        ease: &CompositionEasingFunction,
    ) -> Result<()> {
        let anim = self.compositor.CreateScalarKeyFrameAnimation()?;
        anim.InsertKeyFrameWithEasingFunction(1.0, to, ease)?;
        anim.SetDuration(ts(ms))?;
        if delay > 0 {
            anim.SetDelayTime(ts(delay))?;
        }
        target
            .cast::<CompositionObject>()?
            .StartAnimation(&HSTRING::from(property), &anim)
    }

    fn vector2(&self, target: &impl Interface, property: &str, to: Vector2, ms: u64) -> Result<()> {
        let anim = self.compositor.CreateVector2KeyFrameAnimation()?;
        anim.InsertKeyFrameWithEasingFunction(1.0, to, &self.decelerate)?;
        anim.SetDuration(ts(ms))?;
        target
            .cast::<CompositionObject>()?
            .StartAnimation(&HSTRING::from(property), &anim)
    }

    fn vector3(
        &self,
        target: &impl Interface,
        property: &str,
        to: Vector3,
        ms: u64,
        delay: u64,
        ease: &CompositionEasingFunction,
    ) -> Result<()> {
        let anim = self.compositor.CreateVector3KeyFrameAnimation()?;
        anim.InsertKeyFrameWithEasingFunction(1.0, to, ease)?;
        anim.SetDuration(ts(ms))?;
        if delay > 0 {
            anim.SetDelayTime(ts(delay))?;
        }
        target
            .cast::<CompositionObject>()?
            .StartAnimation(&HSTRING::from(property), &anim)
    }

    fn stop(target: &impl Interface, properties: &[&str]) {
        if let Ok(object) = target.cast::<CompositionObject>() {
            for p in properties {
                let _ = object.StopAnimation(&HSTRING::from(*p));
            }
        }
    }

    // -----------------------------------------------------------------------
    // Public API

    /// Positions the card next to `anchor` (screen pixels) and animates it in.
    pub fn show(
        &mut self,
        anchor: RECT,
        theme: Theme,
        word: &str,
        phonetic: Option<&str>,
        body: Body,
    ) -> Result<()> {
        self.theme = theme;
        self.place(anchor)?;
        self.apply_theme()?;

        // Reset everything to its resting state before animating in.
        for v in [
            &self.root.cast::<Visual>()?,
            &self.frame.cast()?,
            &self.body.cast()?,
        ] {
            Self::stop(v, &["Opacity", "Offset", "Scale"]);
        }
        self.card_h = 0.0;
        self.set_content(word, phonetic, body, Transition::Instant)?;

        let s = self.scale;
        let card_w = CARD_W * s;
        let pivot_y = if self.grow_up {
            self.frame_y() + self.card_h
        } else {
            self.frame_y()
        };
        self.root.SetCenterPoint(Vector3 {
            X: MARGIN * s + card_w / 2.0,
            Y: pivot_y,
            Z: 0.0,
        })?;
        self.root.SetOpacity(0.0)?;
        self.root.SetScale(Vector3 {
            X: 0.96,
            Y: 0.96,
            Z: 1.0,
        })?;
        self.root
            .SetOffset(v3(0.0, if self.grow_up { 10.0 } else { -10.0 } * s))?;

        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                self.window.left,
                self.window.top,
                self.window.right - self.window.left,
                self.window.bottom - self.window.top,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }
        self.visible = true;

        self.scalar(&self.root, "Opacity", 1.0, 160, 0, &self.decelerate)?;
        self.vector3(
            &self.root,
            "Scale",
            Vector3 {
                X: 1.0,
                Y: 1.0,
                Z: 1.0,
            },
            SHOW_MS,
            0,
            &self.decelerate,
        )?;
        self.vector3(
            &self.root,
            "Offset",
            v3(0.0, 0.0),
            SHOW_MS,
            0,
            &self.decelerate,
        )?;
        Ok(())
    }

    /// Fades the card out. The caller hides the window after [`HIDE_MS`].
    pub fn begin_hide(&mut self) -> Result<()> {
        if !self.visible {
            return Ok(());
        }
        self.visible = false;
        let dy = if self.grow_up { 6.0 } else { -6.0 } * self.scale;
        self.scalar(&self.root, "Opacity", 0.0, HIDE_MS, 0, &self.accelerate)?;
        self.vector3(
            &self.root,
            "Offset",
            v3(0.0, dy),
            HIDE_MS,
            0,
            &self.accelerate,
        )?;
        self.vector3(
            &self.root,
            "Scale",
            Vector3 {
                X: 0.98,
                Y: 0.98,
                Z: 1.0,
            },
            HIDE_MS,
            0,
            &self.accelerate,
        )
    }

    pub fn finish_hide(&mut self) {
        if self.visible {
            return;
        }
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        // Release the text surfaces while idle.
        let _ = self.header_brush.SetSurface(None::<&ICompositionSurface>);
        let _ = self.body_brush.SetSurface(None::<&ICompositionSurface>);
        let _ = self
            .header_back_brush
            .SetSurface(None::<&ICompositionSurface>);
        self.header_surface = None;
        self.stop_loader();
        self.renderer.trim();
    }

    /// Replaces the card's content.
    pub fn set_content(
        &mut self,
        word: &str,
        phonetic: Option<&str>,
        body: Body,
        transition: Transition,
    ) -> Result<()> {
        let s = self.scale;
        let content_w = CARD_W - 2.0 * PAD_X;
        let header = self.renderer.layout_header(word, phonetic, content_w)?;
        let body_layout = self.renderer.layout_body(&body, content_w, PAD_BOTTOM)?;
        let header_surface = self.renderer.draw(&header, s, &self.theme)?;
        let body_surface = self.renderer.draw(&body_layout, s, &self.theme)?;

        let header_h = (PAD_TOP + header.height + HEADER_GAP) * s;
        let body_h = body_layout.height * s;
        let max_h = MAX_CARD_H * s;
        let card_h = (header_h + body_h.min(max_h - header_h)).ceil();
        let transition = if self.card_h > 0.0 {
            transition
        } else {
            Transition::Instant
        };
        let previous_header_h = self.header_h;
        self.header_h = header_h;
        self.body_h = body_h;
        self.links = body_layout.links.clone();

        // Header: on a refinement, cross-fade from the old rendering so only
        // what changed (typically the pronunciation) appears to fade in.
        let old_header = self.header_surface.replace(header_surface.clone());
        Self::stop(&self.header, &["Opacity"]);
        Self::stop(&self.header_back, &["Opacity"]);
        self.header_back.SetOpacity(0.0)?;
        if let (Transition::Refine, Some(old)) = (transition, old_header) {
            self.header_back_brush.SetSurface(&old)?;
            self.header_back.SetSize(self.header.Size()?)?;
            self.header_back.SetOffset(v3(PAD_X * s, PAD_TOP * s))?;
            self.header_back.SetOpacity(1.0)?;
            self.header.SetOpacity(0.0)?;
            self.scalar(&self.header_back, "Opacity", 0.0, 240, 0, &self.decelerate)?;
            self.scalar(&self.header, "Opacity", 1.0, 240, 0, &self.decelerate)?;
        } else {
            self.header.SetOpacity(1.0)?;
        }
        self.header_brush.SetSurface(&header_surface)?;
        self.header.SetSize(v2(content_w * s, header.height * s))?;
        self.header.SetOffset(v3(PAD_X * s, PAD_TOP * s))?;
        self.body_brush.SetSurface(&body_surface)?;
        self.body.SetSize(v2(content_w * s, body_h))?;
        Self::stop(&self.body, &["Offset", "Opacity"]);
        Self::stop(&self.body_host, &["Offset"]);

        let was_loading = self.loading;
        self.loading = matches!(body, Body::Loading);
        if self.loading {
            self.start_loader()?;
        } else if was_loading {
            self.scalar(&self.loader, "Opacity", 0.0, 120, 0, &self.accelerate)?;
        }

        match transition {
            Transition::Instant => {
                self.scroll = 0.0;
                self.scroll_anim = None;
                self.body.SetOpacity(1.0)?;
                self.body.SetOffset(v3(PAD_X * s, 0.0))?;
                self.body_host.SetOffset(v3(0.0, header_h))?;
                self.resize(card_h, false)?;
            }
            Transition::Replace => {
                // New body slides up a touch and fades in while the card resizes.
                self.scroll = 0.0;
                self.scroll_anim = None;
                self.body.SetOpacity(0.0)?;
                self.body.SetOffset(v3(PAD_X * s, 8.0 * s))?;
                self.scalar(&self.body, "Opacity", 1.0, 220, 80, &self.decelerate)?;
                let rest = v3(PAD_X * s, 0.0);
                self.vector3(&self.body, "Offset", rest, RESIZE_MS, 80, &self.decelerate)?;
                self.move_body_host(previous_header_h)?;
                self.resize(card_h, true)?;
            }
            Transition::Refine => {
                // Same entry with more detail: keep the reader's place.
                self.body.SetOpacity(1.0)?;
                self.resize(card_h, true)?;
                self.scroll = self.scroll.min(self.max_scroll());
                self.scroll_anim = None;
                self.body.SetOffset(v3(PAD_X * s, -self.scroll.round()))?;
                self.move_body_host(previous_header_h)?;
            }
        }
        self.update_scroll_chrome(false)?;
        Ok(())
    }

    fn move_body_host(&self, previous_header_h: f32) -> Result<()> {
        let to = v3(0.0, self.header_h);
        if (previous_header_h - self.header_h).abs() > 0.5 {
            self.vector3(
                &self.body_host,
                "Offset",
                to,
                RESIZE_MS,
                0,
                &self.decelerate,
            )
        } else {
            self.body_host.SetOffset(to)
        }
    }

    /// Scrolls the body by a mouse wheel delta.
    pub fn scroll_by(&mut self, wheel_delta: i32) -> Result<()> {
        let max = self.max_scroll();
        if max <= 0.0 {
            return Ok(());
        }
        // Start from where the content is on screen right now, not from the
        // previous target: restarting from a stale value is what made
        // scrolling feel like it lagged behind the wheel.
        let from = self.visible_scroll();
        let step = -(wheel_delta as f32 / 120.0) * 56.0 * self.scale;
        self.scroll = (self.scroll + step).clamp(0.0, max);
        self.scroll_anim = Some(ScrollAnim {
            from,
            started: Instant::now(),
        });

        let x = PAD_X * self.scale;
        let anim = self.compositor.CreateVector3KeyFrameAnimation()?;
        anim.InsertKeyFrame(0.0, v3(x, -from))?;
        anim.InsertKeyFrameWithEasingFunction(1.0, v3(x, -self.scroll.round()), &self.scroll_ease)?;
        anim.SetDuration(ts(SCROLL_MS))?;
        self.body.StartAnimation(&HSTRING::from("Offset"), &anim)?;
        self.update_scroll_chrome(true)
    }

    /// The scroll position currently on screen, following the running animation.
    fn visible_scroll(&self) -> f32 {
        let Some(anim) = &self.scroll_anim else {
            return self.scroll;
        };
        let t = anim.started.elapsed().as_secs_f32() * 1000.0 / SCROLL_MS as f32;
        if t >= 1.0 {
            return self.scroll;
        }
        anim.from + (self.scroll - anim.from) * cubic_bezier(SCROLL_EASE, t)
    }

    /// The link under a point in window coordinates, if any.
    pub fn link_at(&self, x: i32, y: i32) -> Option<&str> {
        let s = self.scale;
        let (x, y) = (x as f32 - MARGIN * s, y as f32 - self.frame_y());
        // Only the visible part of the body is clickable.
        if y < self.header_h || y >= self.card_h || x < 0.0 || x >= CARD_W * s {
            return None;
        }
        let (bx, by) = ((x - PAD_X * s) / s, (y - self.header_h + self.scroll) / s);
        self.links
            .iter()
            .find(|l| bx >= l.x && bx < l.x + l.width && by >= l.y && by < l.y + l.height)
            .map(|l| l.url.as_str())
    }

    /// The card's bounds in screen pixels.
    pub fn card_rect(&self) -> RECT {
        let left = self.window.left + (MARGIN * self.scale) as i32;
        let top = self.window.top + self.frame_y() as i32;
        RECT {
            left,
            top,
            right: left + (CARD_W * self.scale) as i32,
            bottom: top + self.card_h as i32,
        }
    }

    /// Shrinks the window's hit-test region to the card once a resize settles.
    pub fn settle(&mut self) {
        self.set_region(self.card_h, self.card_h);
    }

    // -----------------------------------------------------------------------
    // Internals

    fn frame_y(&self) -> f32 {
        let m = MARGIN * self.scale;
        if self.grow_up {
            (self.window.bottom - self.window.top) as f32 - m - self.card_h
        } else {
            m
        }
    }

    fn max_scroll(&self) -> f32 {
        let overflow = self.body_h - (self.card_h - self.header_h);
        // Ignore rounding slack: no scrollbar for a pixel or two.
        if overflow > 2.0 * self.scale {
            overflow
        } else {
            0.0
        }
    }

    fn place(&mut self, anchor: RECT) -> Result<()> {
        unsafe {
            let center = POINT {
                x: (anchor.left + anchor.right) / 2,
                y: (anchor.top + anchor.bottom) / 2,
            };
            let monitor = MonitorFromPoint(center, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            let _ = GetMonitorInfoW(monitor, &mut info);
            let work = info.rcWork;
            let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
            let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
            let s = dpi_x as f32 / 96.0;
            self.scale = s;

            let px = |dip: f32| (dip * s).round() as i32;
            let (card_w, max_h, m) = (px(CARD_W), px(MAX_CARD_H), px(MARGIN));
            let (gap, edge) = (px(ANCHOR_GAP), px(SCREEN_GAP));

            let card_x = (anchor.left - px(PAD_X)).clamp(
                work.left + edge,
                (work.right - edge - card_w).max(work.left + edge),
            );
            let room_below = work.bottom - edge - (anchor.bottom + gap);
            let room_above = (anchor.top - gap) - (work.top + edge);
            self.grow_up = room_below < max_h && room_above > room_below;

            let window_h = max_h + 2 * m;
            let window_y = if self.grow_up {
                let card_bottom = (anchor.top - gap).max(work.top + edge + max_h);
                card_bottom + m - window_h
            } else {
                let card_top = (anchor.bottom + gap)
                    .min(work.bottom - edge - max_h)
                    .max(work.top + edge);
                card_top - m
            };
            self.window = RECT {
                left: card_x - m,
                top: window_y,
                right: card_x + card_w + m,
                bottom: window_y + window_h,
            };
        }
        Ok(())
    }

    fn apply_theme(&mut self) -> Result<()> {
        let t = self.theme;
        self.tint.SetColor(t.tint())?;
        self.border_brush.SetColor(t.border())?;
        self.dot_brush.SetColor(t.accent)?;
        let ink = if t.dark { 0xFF } else { 0x00 };
        self.divider_brush.SetColor(crate::theme::argb(
            if t.dark { 0x1F } else { 0x14 },
            ink,
            ink,
            ink,
        ))?;
        self.scrollbar_brush.SetColor(crate::theme::argb(
            if t.dark { 0x60 } else { 0x50 },
            ink,
            ink,
            ink,
        ))?;

        let s = self.scale;
        let key = ((s * 100.0) as u32, t.dark);
        if self.shadow_key != Some(key) {
            let (surface, inset) =
                self.renderer
                    .draw_shadow(MARGIN * s * 0.75, RADIUS * s, t.shadow_opacity())?;
            self.shadow_surface.SetSurface(&surface)?;
            self.shadow_brush.SetInsets(inset)?;
            self.shadow_key = Some(key);
        }
        let m = MARGIN * s;
        let shadow_margin = m * 0.75;
        self.shadow
            .SetOffset(v3(-shadow_margin, -shadow_margin + SHADOW_DY * s))?;
        self.clip.SetCornerRadius(v2(RADIUS * s, RADIUS * s))?;
        self.border_geometry
            .SetCornerRadius(v2(RADIUS * s - 0.5, RADIUS * s - 0.5))?;
        self.border_geometry.SetOffset(v2(0.5, 0.5))?;
        Ok(())
    }

    fn resize(&mut self, card_h: f32, animate: bool) -> Result<()> {
        let s = self.scale;
        let w = (CARD_W * s).round();
        let old_h = self.card_h;
        self.card_h = card_h;
        let shadow_margin = MARGIN * s * 0.75;
        let window_h = (self.window.bottom - self.window.top) as f32;
        let m = MARGIN * s;
        let frame_y = if self.grow_up {
            window_h - m - card_h
        } else {
            m
        };

        let sizes: [(&dyn Fn() -> Result<CompositionObject>, Vector2); 5] = [
            (&|| self.card.cast(), v2(w, card_h)),
            (&|| self.clip.cast(), v2(w, card_h)),
            (&|| self.border.cast(), v2(w, card_h)),
            (&|| self.border_geometry.cast(), v2(w - 1.0, card_h - 1.0)),
            (
                &|| self.shadow.cast(),
                v2(w + 2.0 * shadow_margin, card_h + 2.0 * shadow_margin),
            ),
        ];
        let body_host_size = v2(w, (card_h - self.header_h).max(0.0));
        if animate {
            self.set_region(old_h, card_h);
            for (target, size) in sizes {
                self.vector2(&target()?, "Size", size, RESIZE_MS)?;
            }
            self.vector2(&self.body_host, "Size", body_host_size, RESIZE_MS)?;
            self.vector3(
                &self.frame,
                "Offset",
                v3(m, frame_y),
                RESIZE_MS,
                0,
                &self.decelerate,
            )?;
        } else {
            for (target, size) in sizes {
                let object = target()?;
                Self::stop(&object, &["Size"]);
                match object.cast::<Visual>() {
                    Ok(visual) => visual.SetSize(size)?,
                    Err(_) => object
                        .cast::<CompositionRoundedRectangleGeometry>()?
                        .SetSize(size)?,
                }
            }
            Self::stop(&self.body_host, &["Size"]);
            self.body_host.SetSize(body_host_size)?;
            Self::stop(&self.frame, &["Offset"]);
            self.frame.SetOffset(v3(m, frame_y))?;
            self.set_region(card_h, card_h);
        }
        self.divider.SetSize(v2(w, 1.0))?;
        Ok(())
    }

    /// Limits the window's clickable area to the card plus its shadow, so the
    /// transparent rest of the window doesn't swallow clicks.
    fn set_region(&self, h1: f32, h2: f32) {
        let s = self.scale;
        let m = MARGIN * s;
        let window_h = (self.window.bottom - self.window.top) as f32;
        let w = (self.window.right - self.window.left) as f32;
        let h = h1.max(h2);
        let (top, bottom) = if self.grow_up {
            (window_h - 2.0 * m - h, window_h)
        } else {
            (0.0, h + 2.0 * m)
        };
        unsafe {
            let region = CreateRectRgn(0, top as i32, w as i32, bottom.ceil() as i32);
            if SetWindowRgn(self.hwnd, Some(region), self.visible) == 0 {
                let _ = DeleteObject(region.into());
            }
        }
    }

    fn update_scroll_chrome(&self, animate: bool) -> Result<()> {
        let s = self.scale;
        let max = self.max_scroll();
        self.divider.SetOffset(v3(0.0, self.header_h - 1.0))?;
        let divider_opacity = if self.scroll > 0.5 { 1.0 } else { 0.0 };
        self.scalar(
            &self.divider,
            "Opacity",
            divider_opacity,
            150,
            0,
            &self.decelerate,
        )?;

        if max <= 0.0 {
            Self::stop(&self.scrollbar, &["Opacity"]);
            self.scrollbar.SetOpacity(0.0)?;
            return Ok(());
        }
        let viewport = self.card_h - self.header_h;
        let track = viewport - 8.0 * s;
        let thumb = (track * viewport / self.body_h).max(28.0 * s);
        let thumb_y = |scroll: f32| {
            self.header_h + 4.0 * s + (track - thumb) * (scroll / max).clamp(0.0, 1.0)
        };
        let y = thumb_y(self.scroll);
        let x = CARD_W * s - 7.0 * s;
        self.scrollbar.SetSize(v2(3.0 * s, thumb))?;
        self.scrollbar_clip.SetSize(v2(3.0 * s, thumb))?;
        self.scrollbar_clip.SetCornerRadius(v2(1.5 * s, 1.5 * s))?;
        if animate {
            // Track the content exactly, starting from where the thumb is now.
            let anim = self.compositor.CreateVector3KeyFrameAnimation()?;
            anim.InsertKeyFrame(0.0, v3(x, thumb_y(self.visible_scroll())))?;
            anim.InsertKeyFrameWithEasingFunction(1.0, v3(x, y), &self.scroll_ease)?;
            anim.SetDuration(ts(SCROLL_MS))?;
            self.scrollbar
                .StartAnimation(&HSTRING::from("Offset"), &anim)?;
        } else {
            Self::stop(&self.scrollbar, &["Offset"]);
            self.scrollbar.SetOffset(v3(x, y))?;
        }
        self.scalar(&self.scrollbar, "Opacity", 1.0, 200, 150, &self.decelerate)
    }

    fn start_loader(&self) -> Result<()> {
        let s = self.scale;
        let size = 7.0 * s;
        Self::stop(&self.loader, &["Opacity"]);
        self.loader.SetOpacity(1.0)?;
        self.loader.SetOffset(v3(PAD_X * s, 6.0 * s))?;
        for (i, dot) in self.dots.iter().enumerate() {
            dot.SetSize(v2(size, size))?;
            dot.SetOffset(v3(i as f32 * size * 1.9, 0.0))?;
            dot.SetCenterPoint(Vector3 {
                X: size / 2.0,
                Y: size / 2.0,
                Z: 0.0,
            })?;
            if let Ok(clip) = dot.Clip()?.cast::<CompositionGeometricClip>() {
                if let Ok(ellipse) = clip.Geometry()?.cast::<CompositionEllipseGeometry>() {
                    ellipse.SetCenter(v2(size / 2.0, size / 2.0))?;
                    ellipse.SetRadius(v2(size / 2.0, size / 2.0))?;
                }
            }
            let pulse = self.compositor.CreateScalarKeyFrameAnimation()?;
            pulse.InsertKeyFrame(0.0, 0.25)?;
            pulse.InsertKeyFrameWithEasingFunction(0.3, 1.0, &self.decelerate)?;
            pulse.InsertKeyFrameWithEasingFunction(0.7, 0.25, &self.decelerate)?;
            pulse.InsertKeyFrame(1.0, 0.25)?;
            pulse.SetDuration(ts(1100))?;
            pulse.SetDelayTime(ts(i as u64 * 160))?;
            pulse.SetIterationBehavior(AnimationIterationBehavior::Forever)?;
            dot.SetOpacity(0.25)?;
            dot.StartAnimation(&HSTRING::from("Opacity"), &pulse)?;

            let bounce = self.compositor.CreateVector3KeyFrameAnimation()?;
            let small = Vector3 {
                X: 0.8,
                Y: 0.8,
                Z: 1.0,
            };
            bounce.InsertKeyFrame(0.0, small)?;
            bounce.InsertKeyFrameWithEasingFunction(
                0.3,
                Vector3 {
                    X: 1.0,
                    Y: 1.0,
                    Z: 1.0,
                },
                &self.decelerate,
            )?;
            bounce.InsertKeyFrameWithEasingFunction(0.7, small, &self.decelerate)?;
            bounce.InsertKeyFrame(1.0, small)?;
            bounce.SetDuration(ts(1100))?;
            bounce.SetDelayTime(ts(i as u64 * 160))?;
            bounce.SetIterationBehavior(AnimationIterationBehavior::Forever)?;
            dot.SetScale(small)?;
            dot.StartAnimation(&HSTRING::from("Scale"), &bounce)?;
        }
        Ok(())
    }

    fn stop_loader(&self) {
        for dot in &self.dots {
            Self::stop(dot, &["Opacity", "Scale"]);
        }
        let _ = self.loader.SetOpacity(0.0);
    }
}
