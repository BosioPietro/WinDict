//! What the card, the tray menu and the settings window are built from:
//! windows that host a composition visual tree, the outline they share,
//! text sprites, and the animations they move with. The introduction and
//! the setup share more still, in [`picture_card`].

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use windows::core::{Interface, Result, BOOL, HSTRING};
use windows::Foundation::TimeSpan;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::WinRT::Composition::ICompositorDesktopInterop;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    SystemParametersInfoW, SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
};
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::*;
use windows_numerics::{Vector2, Vector3};

use crate::render::{Layout, Renderer};
use crate::theme::Theme;

mod button;
mod list;
mod scroll;
mod shapes;

pub mod picture_card;

pub use button::{button, button_width, ring, Button, BUTTON_H, BUTTON_W, FOCUS_OUTSET};
pub use list::List;
pub use shapes::{container, outline, rounded, Rect};

pub use scroll::{
    max_scroll, stretch, stretch_limit, thumb_span, Bursts, Glide, Push, ScrollFade, Scrollbar,
    Track, CARD_ENDS,
};

// Geometry, in DIPs.
/// Room around a card in its window, for the shadow.
pub const MARGIN: f32 = 32.0;
/// The stroke of the ring around what Tab, or typing, has focused, as on
/// Windows 11 controls.
pub const FOCUS_STROKE: f32 = 2.0;
/// Room kept above and below what Tab scrolls into view, clear of the fades
/// at the ends of what scrolls.
pub const FOCUS_MARGIN: f32 = 20.0;
/// How many rows a notch of the mouse wheel scrolls a list by.
pub const WHEEL_ROWS: f32 = 3.0;
const RADIUS: f32 = 12.0;
const SHADOW_DY: f32 = 6.0;
/// How far the shadow extends past the card; it must fit in `MARGIN`.
const SHADOW_SPREAD: f32 = MARGIN * 0.75;

// Durations, in ms.
/// How long a window takes to fade out; it's hidden once that's played.
pub const HIDE_MS: u64 = 120;
/// How long a card takes to fade in.
pub const FADE_IN_MS: u64 = 160;
/// How long a card takes to change size.
pub const RESIZE_MS: u64 = 300;
const SHAKE_MS: u64 = 400;
/// Ease-out curve for scrolling: quick to respond, soft to settle.
pub const SCROLL_EASE: (f32, f32, f32, f32) = (0.25, 0.8, 0.4, 1.0);
/// Ease-out that runs a touch past the end and settles back, like a spring.
const OVERSHOOT_EASE: (f32, f32, f32, f32) = (0.3, 1.35, 0.55, 1.0);

/// Progress of a CSS-style cubic-bezier easing at time `t` (0..=1).
pub fn cubic_bezier((x1, y1, x2, y2): (f32, f32, f32, f32), t: f32) -> f32 {
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

/// The work area and scale (pixels per DIP) of the monitor nearest `point`.
pub fn monitor_at(point: POINT) -> (RECT, f32) {
    let monitor = unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
    unsafe {
        let _ = GetMonitorInfoW(monitor, &mut info);
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
    }
    (info.rcWork, dpi_x as f32 / 96.0)
}

/// Makes `hwnd` a composition surface: the host backdrop brush can sample
/// what's behind it, DWM's own show/hide animations are off, and the
/// returned target (kept alive by the caller) displays the visual tree.
pub fn attach(hwnd: HWND, compositor: &Compositor) -> Result<DesktopWindowTarget> {
    unsafe {
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
        let interop: ICompositorDesktopInterop = compositor.cast()?;
        interop.CreateDesktopWindowTarget(hwnd, true)
    }
}

pub fn ts(ms: u64) -> TimeSpan {
    TimeSpan::from(Duration::from_millis(ms))
}

/// Whether things move into place; if not, they jump there.
static MOTION: AtomicBool = AtomicBool::new(true);

pub fn set_motion(on: bool) {
    MOTION.store(on, Ordering::Relaxed);
}

pub fn motion() -> bool {
    MOTION.load(Ordering::Relaxed)
}

/// Whether Windows plays animations (Accessibility > Visual effects).
pub fn system_motion() -> bool {
    let mut on = BOOL(1);
    let read = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some(&mut on as *mut BOOL as *mut _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    read.is_err() || on.as_bool()
}

/// How long a movement of `ms` takes: as short as the compositor allows
/// when there's to be no motion.
pub fn span(ms: u64) -> TimeSpan {
    ts(if motion() { ms } else { 1 })
}

pub fn v2(x: f32, y: f32) -> Vector2 {
    Vector2 { X: x, Y: y }
}

pub fn v3(x: f32, y: f32) -> Vector3 {
    Vector3 { X: x, Y: y, Z: 0.0 }
}

/// A visual's scale, `k` times its size both ways.
pub fn scale(k: f32) -> Vector3 {
    Vector3 { X: k, Y: k, Z: 1.0 }
}

/// A visual showing a text surface at its natural size, from the top left.
pub fn text_sprite(c: &Compositor) -> Result<(SpriteVisual, CompositionSurfaceBrush)> {
    let brush = c.CreateSurfaceBrush()?;
    brush.SetStretch(CompositionStretch::None)?;
    brush.SetHorizontalAlignmentRatio(0.0)?;
    brush.SetVerticalAlignmentRatio(0.0)?;
    let sprite = c.CreateSpriteVisual()?;
    sprite.SetBrush(&brush)?;
    Ok((sprite, brush))
}

/// How long a caret takes to blink once.
pub const BLINK_MS: u64 = 1060;

/// Makes `caret` blink: solid for the first half of each cycle, hidden for
/// the rest, starting solid now.
pub fn blink(c: &Compositor, caret: &SpriteVisual) -> Result<()> {
    let a = c.CreateScalarKeyFrameAnimation()?;
    a.InsertKeyFrame(0.0, 1.0)?;
    a.InsertKeyFrame(0.49, 1.0)?;
    a.InsertKeyFrame(0.5, 0.0)?;
    a.InsertKeyFrame(0.99, 0.0)?;
    a.InsertKeyFrame(1.0, 1.0)?;
    a.SetDuration(ts(BLINK_MS))?;
    a.SetIterationBehavior(AnimationIterationBehavior::Forever)?;
    caret.SetOpacity(1.0)?;
    caret.StartAnimation(&HSTRING::from("Opacity"), &a)
}

/// A line of text and the brush that shows it.
/// How long each step of [`waiting`]'s dots shows, in ms.
pub const DOTS_MS: u32 = 400;

/// `text` with `dots` dots after it, from one to three, as what's waiting
/// shows: they come one by one, step after step.
pub fn waiting(text: &str, dots: usize) -> String {
    format!("{text}{}", ".".repeat(dots % 3 + 1))
}

pub struct Text {
    pub visual: SpriteVisual,
    pub brush: CompositionSurfaceBrush,
}

impl Text {
    pub fn new(c: &Compositor, parent: &ContainerVisual) -> Result<Text> {
        let (visual, brush) = text_sprite(c)?;
        parent.Children()?.InsertAtTop(&visual)?;
        Ok(Text { visual, brush })
    }

    /// Shows `layout` drawn at `s` pixels per DIP; returns its size in pixels.
    pub fn show(
        &self,
        renderer: &Renderer,
        layout: &Layout,
        s: f32,
        theme: &Theme,
    ) -> Result<Vector2> {
        self.brush.SetSurface(&renderer.draw(layout, s, theme)?)?;
        let size = v2((layout.width * s).ceil(), (layout.height * s).ceil());
        self.visual.SetSize(size)?;
        Ok(size)
    }

    pub fn clear(&self) {
        let _ = self.brush.SetSurface(None::<&ICompositionSurface>);
    }
}

/// Starts compositor animations with the card's easing curves.
pub struct Animator {
    pub compositor: Compositor,
    pub decelerate: CompositionEasingFunction,
    pub accelerate: CompositionEasingFunction,
    /// Ease-out for scrolling, matching [`SCROLL_EASE`].
    pub scroll_ease: CompositionEasingFunction,
    /// Straight from one sample to the next, for motion played as samples.
    linear: CompositionEasingFunction,
    /// Ease-in-out, for fades that should be seen happening.
    pub fade: CompositionEasingFunction,
    /// For things that move into place with a little bounce.
    pub overshoot: CompositionEasingFunction,
}

impl Animator {
    pub fn new(compositor: Compositor) -> Result<Animator> {
        let c = &compositor;
        let decelerate: CompositionEasingFunction = c
            .CreateCubicBezierEasingFunction(v2(0.1, 0.9), v2(0.2, 1.0))?
            .cast()?;
        let (x1, y1, x2, y2) = SCROLL_EASE;
        let scroll_ease = c
            .CreateCubicBezierEasingFunction(v2(x1, y1), v2(x2, y2))?
            .cast()?;
        let linear = c.CreateLinearEasingFunction()?.cast()?;
        let accelerate = c
            .CreateCubicBezierEasingFunction(v2(0.7, 0.0), v2(1.0, 0.5))?
            .cast()?;
        let fade = c
            .CreateCubicBezierEasingFunction(v2(0.4, 0.0), v2(0.6, 1.0))?
            .cast()?;
        let (x1, y1, x2, y2) = OVERSHOOT_EASE;
        // Should the system refuse a curve that leaves 0..1, settle softly instead.
        let overshoot: CompositionEasingFunction =
            match c.CreateCubicBezierEasingFunction(v2(x1, y1), v2(x2, y2)) {
                Ok(curve) => curve.cast()?,
                Err(_) => decelerate.clone(),
            };
        Ok(Animator {
            compositor,
            decelerate,
            accelerate,
            scroll_ease,
            linear,
            fade,
            overshoot,
        })
    }

    pub fn scalar(
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
        anim.SetDuration(span(ms))?;
        if delay > 0 && motion() {
            anim.SetDelayTime(ts(delay))?;
        }
        target
            .cast::<CompositionObject>()?
            .StartAnimation(&HSTRING::from(property), &anim)
    }

    pub fn vector2(
        &self,
        target: &impl Interface,
        property: &str,
        to: Vector2,
        ms: u64,
    ) -> Result<()> {
        let anim = self.compositor.CreateVector2KeyFrameAnimation()?;
        anim.InsertKeyFrameWithEasingFunction(1.0, to, &self.decelerate)?;
        anim.SetDuration(span(ms))?;
        target
            .cast::<CompositionObject>()?
            .StartAnimation(&HSTRING::from(property), &anim)
    }

    pub fn vector3(
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
        anim.SetDuration(span(ms))?;
        if delay > 0 && motion() {
            anim.SetDelayTime(ts(delay))?;
        }
        target
            .cast::<CompositionObject>()?
            .StartAnimation(&HSTRING::from(property), &anim)
    }

    /// Brings a window's `root` visual in: it fades in over `fade_ms`, and
    /// settles at full size where it belongs over `ms`.
    pub fn fade_in(&self, root: &ContainerVisual, fade_ms: u64, ms: u64) -> Result<()> {
        let ease = &self.decelerate;
        self.scalar(root, "Opacity", 1.0, fade_ms, 0, ease)?;
        self.vector3(root, "Scale", scale(1.0), ms, 0, ease)?;
        self.vector3(root, "Offset", v3(0.0, 0.0), ms, 0, ease)
    }

    /// Fades a window's `root` visual out over [`HIDE_MS`], as it shrinks a
    /// little and drifts `dy` pixels down (or up, when negative).
    pub fn fade_out(&self, root: &ContainerVisual, dy: f32) -> Result<()> {
        let ease = &self.accelerate;
        self.scalar(root, "Opacity", 0.0, HIDE_MS, 0, ease)?;
        self.vector3(root, "Offset", v3(0.0, dy), HIDE_MS, 0, ease)?;
        self.vector3(root, "Scale", scale(0.98), HIDE_MS, 0, ease)
    }

    /// Fades a brush from `from` to `to`.
    pub fn color(
        &self,
        brush: &CompositionColorBrush,
        from: windows::UI::Color,
        to: windows::UI::Color,
        ms: u64,
    ) -> Result<()> {
        let anim = self.compositor.CreateColorKeyFrameAnimation()?;
        anim.SetInterpolationColorSpace(CompositionColorSpace::Rgb)?;
        anim.InsertKeyFrame(0.0, from)?;
        anim.InsertKeyFrameWithEasingFunction(1.0, to, &self.fade)?;
        anim.SetDuration(span(ms))?;
        brush.SetColor(to)?;
        brush.StartAnimation(&HSTRING::from("Color"), &anim)
    }

    /// Shakes `visual`, which rests at `at`, from side to side: a sign that
    /// what was asked for can't be done. `s` is pixels per DIP.
    pub fn shake(&self, visual: &impl Interface, at: Vector3, s: f32) -> Result<()> {
        if !motion() {
            return Ok(());
        }
        let a = self.compositor.CreateVector3KeyFrameAnimation()?;
        let swings = [
            (0.0, 0.0),
            (0.12, -8.0),
            (0.28, 7.0),
            (0.44, -5.0),
            (0.6, 3.0),
            (0.78, -1.0),
            (1.0, 0.0),
        ];
        for (t, dx) in swings {
            a.InsertKeyFrame(t, v3(at.X + dx * s, at.Y))?;
        }
        a.SetDuration(ts(SHAKE_MS))?;
        visual
            .cast::<CompositionObject>()?
            .StartAnimation(&HSTRING::from("Offset"), &a)
    }

    pub fn stop(target: &impl Interface, properties: &[&str]) {
        if let Ok(object) = target.cast::<CompositionObject>() {
            for p in properties {
                let _ = object.StopAnimation(&HSTRING::from(*p));
            }
        }
    }
}

/// The card's outline: its shadow, the clipped acrylic background that
/// holds the content, and a hairline border on top.
pub struct Frame {
    pub visual: ContainerVisual,
    shadow: SpriteVisual,
    shadow_surface: CompositionSurfaceBrush,
    shadow_brush: CompositionNineGridBrush,
    /// Scale, darkness and the renderer's generation the shadow was last
    /// drawn for.
    shadow_key: Option<(u32, bool, u32)>,
    /// The content goes in here.
    pub card: ContainerVisual,
    clip: CompositionRoundedRectangleGeometry,
    tint: CompositionColorBrush,
    border: ShapeVisual,
    border_geometry: CompositionRoundedRectangleGeometry,
    border_brush: CompositionColorBrush,
}

impl Frame {
    pub fn new(c: &Compositor, parent: &ContainerVisual, theme: &Theme) -> Result<Frame> {
        let visual = c.CreateContainerVisual()?;
        parent.Children()?.InsertAtTop(&visual)?;

        let shadow = c.CreateSpriteVisual()?;
        let shadow_surface = c.CreateSurfaceBrush()?;
        // The nine-grid needs the whole surface; the default, Uniform, keeps
        // its aspect ratio and leaves the edges unstretched.
        shadow_surface.SetStretch(CompositionStretch::Fill)?;
        let shadow_brush = c.CreateNineGridBrush()?;
        shadow_brush.SetSource(&shadow_surface)?;
        shadow.SetBrush(&shadow_brush)?;
        visual.Children()?.InsertAtTop(&shadow)?;

        let card = c.CreateContainerVisual()?;
        let clip = c.CreateRoundedRectangleGeometry()?;
        card.SetClip(&c.CreateGeometricClipWithGeometry(&clip)?)?;
        visual.Children()?.InsertAtTop(&card)?;

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

        // A hairline edge, as on Windows 11 flyouts.
        let border = c.CreateShapeVisual()?;
        let border_geometry = c.CreateRoundedRectangleGeometry()?;
        let border_shape = c.CreateSpriteShapeWithGeometry(&border_geometry)?;
        let border_brush = c.CreateColorBrush()?;
        border_shape.SetStrokeBrush(&border_brush)?;
        border_shape.SetStrokeThickness(1.0)?;
        border.Shapes()?.Append(&border_shape)?;
        visual.Children()?.InsertAtTop(&border)?;

        Ok(Frame {
            visual,
            shadow,
            shadow_surface,
            shadow_brush,
            shadow_key: None,
            card,
            clip,
            tint,
            border,
            border_geometry,
            border_brush,
        })
    }

    pub fn apply_theme(&mut self, renderer: &Renderer, t: &Theme, s: f32) -> Result<()> {
        self.tint.SetColor(t.tint())?;
        self.border_brush.SetColor(t.border())?;
        let key = ((s * 100.0) as u32, t.dark, renderer.generation());
        if self.shadow_key != Some(key) {
            let (surface, inset) =
                renderer.draw_shadow(SHADOW_SPREAD * s, RADIUS * s, t.shadow_opacity())?;
            self.shadow_surface.SetSurface(&surface)?;
            self.shadow_brush.SetInsets(inset)?;
            self.shadow_key = Some(key);
        }
        let shadow_margin = SHADOW_SPREAD * s;
        self.shadow
            .SetOffset(v3(-shadow_margin, -shadow_margin + SHADOW_DY * s))?;
        self.clip.SetCornerRadius(v2(RADIUS * s, RADIUS * s))?;
        self.border_geometry
            .SetCornerRadius(v2(RADIUS * s - 0.5, RADIUS * s - 0.5))?;
        self.border_geometry.SetOffset(v2(0.5, 0.5))?;
        Ok(())
    }

    /// Like [`Frame::apply_theme`], with the card's colors fading over `ms`.
    pub fn fade_theme(
        &mut self,
        anim: &Animator,
        renderer: &Renderer,
        t: &Theme,
        s: f32,
        ms: u64,
    ) -> Result<()> {
        let (tint, border) = (self.tint.Color()?, self.border_brush.Color()?);
        self.apply_theme(renderer, t, s)?;
        anim.color(&self.tint, tint, t.tint(), ms)?;
        anim.color(&self.border_brush, border, t.border(), ms)
    }

    /// Sizes the card to `w` × `h` pixels with its top left at `offset`.
    pub fn resize(
        &self,
        anim: &Animator,
        w: f32,
        h: f32,
        offset: Vector3,
        s: f32,
        animate: bool,
    ) -> Result<()> {
        let shadow_margin = SHADOW_SPREAD * s;
        let sizes: [(&dyn Fn() -> Result<CompositionObject>, Vector2); 5] = [
            (&|| self.card.cast(), v2(w, h)),
            (&|| self.clip.cast(), v2(w, h)),
            (&|| self.border.cast(), v2(w, h)),
            (&|| self.border_geometry.cast(), v2(w - 1.0, h - 1.0)),
            (
                &|| self.shadow.cast(),
                v2(w + 2.0 * shadow_margin, h + 2.0 * shadow_margin),
            ),
        ];
        if animate {
            for (target, size) in sizes {
                anim.vector2(&target()?, "Size", size, RESIZE_MS)?;
            }
            anim.vector3(
                &self.visual,
                "Offset",
                offset,
                RESIZE_MS,
                0,
                &anim.decelerate,
            )
        } else {
            for (target, size) in sizes {
                let object = target()?;
                Animator::stop(&object, &["Size"]);
                match object.cast::<Visual>() {
                    Ok(visual) => visual.SetSize(size)?,
                    Err(_) => object
                        .cast::<CompositionRoundedRectangleGeometry>()?
                        .SetSize(size)?,
                }
            }
            Animator::stop(&self.visual, &["Offset"]);
            self.visual.SetOffset(offset)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dots_come_one_by_one() {
        let steps: Vec<String> = (0..4).map(|i| waiting("Waiting", i)).collect();
        assert_eq!(steps, ["Waiting.", "Waiting..", "Waiting...", "Waiting."]);
    }

    #[test]
    fn eases_like_css() {
        assert!(cubic_bezier(SCROLL_EASE, 0.0).abs() < 1e-3);
        assert!((cubic_bezier(SCROLL_EASE, 1.0) - 1.0).abs() < 1e-3);
        // An ease-out is ahead of linear halfway through, and never goes back.
        assert!(cubic_bezier(SCROLL_EASE, 0.5) > 0.5);
        let samples: Vec<f32> = (0..=20)
            .map(|i| cubic_bezier(SCROLL_EASE, i as f32 / 20.0))
            .collect();
        assert!(samples.windows(2).all(|w| w[1] >= w[0]));
    }
}
