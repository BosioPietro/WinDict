//! What the card, the tray menu and the settings window are built from:
//! windows that host a composition visual tree, the outline they share,
//! text sprites, and the animations they move with.

use std::time::{Duration, Instant};

use windows::core::{Interface, Result, BOOL, HSTRING};
use windows::Foundation::TimeSpan;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::WinRT::Composition::ICompositorDesktopInterop;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::*;
use windows_numerics::{Vector2, Vector3};

use crate::render::{Layout, Renderer};
use crate::theme::Theme;

// Geometry, in DIPs.
/// Room around a card in its window, for the shadow.
pub const MARGIN: f32 = 32.0;
const RADIUS: f32 = 12.0;
const SHADOW_DY: f32 = 6.0;
/// How far the shadow extends past the card; it must fit in `MARGIN`.
const SHADOW_SPREAD: f32 = MARGIN * 0.75;

/// The scrollbar thumb's width, and how far its left edge is from the
/// card's right edge.
const THUMB_W: f32 = 3.0;
const THUMB_RIGHT: f32 = 7.0;
/// Room above the thumb, and its shortest.
const THUMB_PAD: f32 = 4.0;
const THUMB_MIN_H: f32 = 28.0;
/// Room below the thumb: it ends where the card's corner starts to curve.
const THUMB_END: f32 = RADIUS;

/// How far content fades out towards an edge more of it is past.
const FADE: f32 = 20.0;
/// The farthest content can be pulled past its end, and how soon pulling
/// gets hard.
const STRETCH_MAX: f32 = 16.0;
const STRETCH_SOFT: f32 = 48.0;

// Durations, in ms.
/// How long a window takes to fade out; it's hidden once that's played.
pub const HIDE_MS: u64 = 120;
/// How long a card takes to change size.
pub const RESIZE_MS: u64 = 300;
const SHAKE_MS: u64 = 400;
/// Ease-out curve for scrolling: quick to respond, soft to settle.
pub const SCROLL_EASE: (f32, f32, f32, f32) = (0.25, 0.8, 0.4, 1.0);
/// Ease-out that runs a touch past the end and settles back, like a spring.
const OVERSHOOT_EASE: (f32, f32, f32, f32) = (0.3, 1.35, 0.55, 1.0);
/// How soon content pushed past its end gets furthest, before it springs
/// back; it's come back after `BOUNCE_SPAN` times that.
const BOUNCE_MS: f32 = 60.0;
const BOUNCE_SPAN: f32 = 8.0;
/// How often a bounce is sampled for the compositor to play.
const BOUNCE_FRAME_MS: f32 = 16.0;
/// How long a scrolling viewport's fades take to change.
const FADE_MS: u64 = 180;

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

pub fn v2(x: f32, y: f32) -> Vector2 {
    Vector2 { X: x, Y: y }
}

pub fn v3(x: f32, y: f32) -> Vector3 {
    Vector3 { X: x, Y: y, Z: 0.0 }
}

/// How far content `content_h` tall scrolls in a `viewport` that tall, in
/// pixels at `s` per DIP. A pixel or two of rounding slack doesn't count.
pub fn max_scroll(content_h: f32, viewport: f32, s: f32) -> f32 {
    let overflow = content_h - viewport;
    if overflow > 2.0 * s {
        overflow
    } else {
        0.0
    }
}

/// The scrollbar thumb for content `content_h` tall in a `viewport` that
/// tall, scrolled `scroll` of `max`: its top, below the viewport's, and its
/// height, in pixels at `s` per DIP.
pub fn thumb_span(viewport: f32, content_h: f32, scroll: f32, max: f32, s: f32) -> (f32, f32) {
    let track = viewport - (THUMB_PAD + THUMB_END) * s;
    let h = (track * viewport / content_h).max(THUMB_MIN_H * s);
    let top = THUMB_PAD * s + (track - h) * (scroll / max).clamp(0.0, 1.0);
    (top, h)
}

/// How long the wheel goes quiet between one spin and the next.
const BURST_GAP_MS: u128 = 120;
/// How long a touchpad's stream of steps goes on being told apart from a
/// wheel's, after one that couldn't be a wheel's: now and then it sends a
/// whole notch's worth too.
const PRECISE_MS: u128 = 250;
/// How far, in DIPs, the first step of a touchpad's swipe goes at most:
/// Windows holds back the start of a swipe until it's sure of it, then
/// sends it all at once, which would jump.
const OPENING_STEP: f32 = 16.0;

/// The wheel's steps, told apart into spins, or a touchpad's into swipes
/// and the steps Windows sends as each coasts to a stop. Only the first
/// step of one that runs into the end throws the content past it; the rest
/// would only hold it out there, for as long as the coasting goes on.
#[derive(Default)]
pub struct Bursts {
    last: Option<Instant>,
    pushed: bool,
    precise: Option<Instant>,
}

impl Bursts {
    /// Whether a wheel `delta` comes from a touchpad (or a wheel without
    /// notches), which sends a stream of small ones that Windows already
    /// smooths and coasts, rather than whole notches.
    pub fn precise(&mut self, delta: i32) -> bool {
        let now = Instant::now();
        if delta % 120 != 0 {
            self.precise = Some(now);
        }
        self.precise
            .is_some_and(|at| now.duration_since(at).as_millis() <= PRECISE_MS)
    }

    fn quiet(&self, now: Instant) -> bool {
        self.last
            .is_none_or(|at| now.duration_since(at).as_millis() > BURST_GAP_MS)
    }

    /// A touchpad's `step`, in pixels at `s` per DIP, as far as it should
    /// go: the first of a swipe only as far as one of the rest might.
    pub fn precise_step(&self, step: f32, s: f32) -> f32 {
        if self.quiet(Instant::now()) {
            step.clamp(-OPENING_STEP * s, OPENING_STEP * s)
        } else {
            step
        }
    }

    /// Whether a step to `want` should play, in content that scrolls
    /// `0..=max`.
    pub fn take(&mut self, want: f32, max: f32) -> bool {
        let now = Instant::now();
        if self.quiet(now) {
            self.pushed = false;
        }
        self.last = Some(now);
        if want == want.clamp(0.0, max) {
            // Away from the end, it may run into it again.
            self.pushed = false;
            return true;
        }
        !std::mem::replace(&mut self.pushed, true)
    }
}

/// How a scroll pushes the content past its end, in pixels, signed.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Push {
    None,
    /// Thrown as far as this from still.
    Strike(f32),
}

/// The farthest content goes past its end, in pixels at `s` per DIP.
pub fn stretch_limit(s: f32) -> f32 {
    STRETCH_MAX * s
}

/// Where a scroll `step` on from `at` stops in `0..=max`, and how it pushes
/// past that, all in pixels at `s` per DIP; the further past the content
/// already is (`from`, where it is on screen), the less it gives.
///
/// A step that reaches the end throws the content with all of it, so it
/// gives however little was left, a bigger one harder but less and less
/// so.
pub fn stretch(from: f32, at: f32, step: f32, max: f32, s: f32) -> (f32, Push) {
    let want = at + step;
    let to = want.clamp(0.0, max);
    if want == to {
        return (to, Push::None);
    }
    let (most, soft) = (STRETCH_MAX * s, STRETCH_SOFT * s);
    let already = from - from.clamp(0.0, max);
    let already = if already * step > 0.0 {
        already.abs()
    } else {
        0.0
    };
    let give = (1.0 - already / most).max(0.0);
    let push = most * (1.0 - (-step.abs() / soft).exp()) * give;
    (to, Push::Strike(push.copysign(step)))
}

/// How far past the end content gets on the spring, starting `past` it
/// and going `kick` pixels per ms, out or back.
fn spring_peak(past: f32, kick: f32) -> f32 {
    // Where (past + (kick + past / T) t) e^(-t / T) stops changing.
    let (x, v, tau) = (past, kick, BOUNCE_MS);
    let speed = v + x / tau;
    let t = if speed == 0.0 { 0.0 } else { v * tau / speed };
    if t <= 0.0 {
        return x.abs();
    }
    ((x + speed * t) * (-t / tau).exp()).abs().max(x.abs())
}

/// A scroll under way, in pixels, to `to`: eased, or at once (`ms` 0) for
/// a touchpad, which Windows smooths itself. Whatever of it is past the end
/// is on a critically damped spring: struck, it's off at once, quickest at
/// the start, and back with no pause at the furthest point; pulled, it
/// comes back as soon as the pulling stops, or slackens. A push adds to
/// how it's moving, never starting it afresh.
#[derive(Clone, Copy)]
pub struct Glide {
    pub to: f32,
    /// Where the part within the ends starts.
    base: f32,
    /// How far past the end it starts, and how fast it's going there, in
    /// pixels per ms.
    past: f32,
    kick: f32,
    /// How long it takes to get there, bounce aside.
    ms: u64,
    started: Instant,
}

impl Glide {
    /// A scroll to `to`, in `0..=max`, pushed past it (see [`stretch`]) but
    /// never more than `most`, taking over from `previous` where it's got
    /// to and as fast as it's going, or starting still at `at`.
    pub fn new(
        previous: Option<Glide>,
        at: f32,
        to: f32,
        push: Push,
        max: f32,
        most: f32,
        ms: u64,
    ) -> Glide {
        let started = Instant::now();
        let (from, velocity) = match previous.filter(Glide::running) {
            Some(g) => {
                let t = g.ms_at(started);
                let h = t.min(4.0);
                let v = if h > 0.0 {
                    (g.at_ms(t) - g.at_ms(t - h)) / h
                } else {
                    0.0
                };
                (g.at_ms(t), v)
            }
            None => (at, 0.0),
        };
        let base = from.clamp(0.0, max);
        let past = from - base;
        // Already past the end, it carries on as it was going; a strike
        // adds what throws it that far from still.
        let going = if past != 0.0 { velocity } else { 0.0 };
        let mut kick = match push {
            Push::None => going,
            Push::Strike(far) => going + far * std::f32::consts::E / BOUNCE_MS,
        };
        // However hard, it goes no further than `most`.
        if spring_peak(past, kick) > most {
            let (mut lo, mut hi) = (0.0, 1.0);
            for _ in 0..24 {
                let mid = (lo + hi) / 2.0;
                if spring_peak(past, kick * mid) > most {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            kick *= lo;
        }
        Glide {
            to,
            base,
            past,
            kick,
            ms,
            started,
        }
    }

    fn springs(&self) -> bool {
        self.past != 0.0 || self.kick != 0.0
    }

    fn duration(&self) -> u64 {
        if self.springs() {
            self.ms.max((BOUNCE_MS * BOUNCE_SPAN) as u64)
        } else {
            self.ms
        }
    }

    fn ms_at(&self, now: Instant) -> f32 {
        now.saturating_duration_since(self.started).as_secs_f32() * 1000.0
    }

    /// How much longer, in ms, the part within the ends is easing: a
    /// touchpad's step, coming while it does, carries on easing in the
    /// time left rather than cutting it short.
    pub fn easing_left(&self) -> u64 {
        self.ms
            .saturating_sub(self.started.elapsed().as_millis() as u64)
    }

    pub fn running(&self) -> bool {
        self.started.elapsed().as_millis() < self.duration() as u128
    }

    /// Where it is on screen now, as the compositor plays it.
    pub fn at(&self) -> f32 {
        self.at_ms(self.ms_at(Instant::now()))
    }

    fn at_ms(&self, t: f32) -> f32 {
        if t >= self.duration() as f32 {
            return self.to;
        }
        let glide = match self.ms {
            0 => 1.0,
            ms => cubic_bezier(SCROLL_EASE, (t / ms as f32).min(1.0)),
        };
        let spring = (self.past + (self.kick + self.past / BOUNCE_MS) * t) * (-t / BOUNCE_MS).exp();
        self.base + (self.to - self.base) * glide + spring
    }

    /// Plays it on `visual`'s offset, `place` saying where a scroll
    /// position puts it.
    pub fn play(
        &self,
        anim: &Animator,
        visual: &impl Interface,
        place: impl Fn(f32) -> Vector3,
    ) -> Result<()> {
        if self.duration() == 0 {
            Animator::stop(visual, &["Offset"]);
            return visual.cast::<Visual>()?.SetOffset(place(self.to));
        }
        let a = anim.compositor.CreateVector3KeyFrameAnimation()?;
        // From where it is on screen as the compositor starts it, which can
        // be a frame after where this worked out it would be: a jump that
        // shows, at the speed of a bounce.
        a.InsertExpressionKeyFrame(0.0, &HSTRING::from("this.StartingValue"))?;
        if !self.springs() {
            a.InsertKeyFrameWithEasingFunction(1.0, place(self.to), &anim.scroll_ease)?;
        } else {
            // No curve the compositor has bounces like that: it plays samples.
            let duration = self.duration() as f32;
            let frames = (duration / BOUNCE_FRAME_MS).ceil() as u32;
            for i in 1..=frames {
                let progress = i as f32 / frames as f32;
                let at = place(self.at_ms(duration * progress));
                a.InsertKeyFrameWithEasingFunction(progress, at, &anim.linear)?;
            }
        }
        a.SetDuration(ts(self.duration()))?;
        visual
            .cast::<CompositionObject>()?
            .StartAnimation(&HSTRING::from("Offset"), &a)
    }
}

/// Shows a scrolling viewport faded out towards whichever of its edges more
/// is past, so that it's plain it scrolls.
pub struct ScrollFade {
    /// Takes the viewport's place on screen, while it scrolls.
    visual: SpriteVisual,
    parent: ContainerVisual,
    /// The viewport's own tree, drawn only through `visual`, while it
    /// scrolls; otherwise the viewport's in `parent`, drawn as it is.
    root: ContainerVisual,
    faded: std::cell::Cell<bool>,
    _surface: CompositionVisualSurface,
    /// The ends of the mask, and the stops the fades run to from them.
    top: CompositionColorGradientStop,
    bottom: CompositionColorGradientStop,
    top_in: CompositionColorGradientStop,
    bottom_in: CompositionColorGradientStop,
    host: ContainerVisual,
}

impl ScrollFade {
    /// Shows `host`, the clipped viewport, in `parent`. Moving and sizing
    /// `host` moves and sizes what's shown; it must not be in a tree itself.
    pub fn new(
        c: &Compositor,
        parent: &ContainerVisual,
        host: &ContainerVisual,
    ) -> Result<ScrollFade> {
        let root = c.CreateContainerVisual()?;
        let surface = c.CreateVisualSurface()?;
        surface.SetSourceVisual(&root)?;
        let source = c.CreateSurfaceBrushWithSurface(&surface)?;
        source.SetStretch(CompositionStretch::None)?;
        source.SetHorizontalAlignmentRatio(0.0)?;
        source.SetVerticalAlignmentRatio(0.0)?;
        // Drawn pixel for pixel, as sharp as the viewport itself.
        source.SetSnapToPixels(true)?;

        let mask = c.CreateLinearGradientBrush()?;
        mask.SetStartPoint(v2(0.0, 0.0))?;
        mask.SetEndPoint(v2(0.0, 1.0))?;
        let opaque = windows::UI::Color {
            A: 255,
            R: 255,
            G: 255,
            B: 255,
        };
        let stops = mask.ColorStops()?;
        let stop = |at: f32| -> Result<CompositionColorGradientStop> {
            let stop = c.CreateColorGradientStopWithOffsetAndColor(at, opaque)?;
            stops.Append(&stop)?;
            Ok(stop)
        };
        let (top, top_in, bottom_in, bottom) = (stop(0.0)?, stop(0.0)?, stop(1.0)?, stop(1.0)?);

        let brush = c.CreateMaskBrush()?;
        brush.SetSource(&source)?;
        brush.SetMask(&mask)?;
        let visual = c.CreateSpriteVisual()?;
        visual.SetBrush(&brush)?;
        parent.Children()?.InsertAtTop(&visual)?;
        // Until it scrolls, there's nothing to fade.
        parent.Children()?.InsertAbove(host, &visual)?;

        // Where the viewport is, and how big, even as it animates.
        let follow = |target: &CompositionObject, property: &str, expression: &str| {
            let e = c.CreateExpressionAnimationWithExpression(&HSTRING::from(expression))?;
            e.SetReferenceParameter(&HSTRING::from("h"), host)?;
            target.StartAnimation(&HSTRING::from(property), &e)
        };
        follow(&visual.cast()?, "Offset", "h.Offset")?;
        follow(&visual.cast()?, "Size", "h.Size")?;
        follow(
            &surface.cast()?,
            "SourceOffset",
            "Vector2(h.Offset.X, h.Offset.Y)",
        )?;
        follow(&surface.cast()?, "SourceSize", "h.Size")?;

        Ok(ScrollFade {
            visual,
            parent: parent.clone(),
            root,
            faded: Default::default(),
            _surface: surface,
            top,
            bottom,
            top_in,
            bottom_in,
            host: host.clone(),
        })
    }

    /// Makes the fades as long as they are at `s` pixels per DIP, however
    /// tall the viewport.
    pub fn set_scale(&self, c: &Compositor, s: f32) -> Result<()> {
        let length = format!("Min({} / Max(h.Size.Y, 1), 0.5)", FADE * s);
        for (stop, expression) in [
            (&self.top_in, length.clone()),
            (&self.bottom_in, format!("1 - {length}")),
        ] {
            let e = c.CreateExpressionAnimationWithExpression(&HSTRING::from(expression))?;
            e.SetReferenceParameter(&HSTRING::from("h"), &self.host)?;
            stop.StartAnimation(&HSTRING::from("Offset"), &e)?;
        }
        Ok(())
    }

    /// Fades the content towards the top once it's `scroll` from it, and
    /// towards the bottom unless it's at `max`; the further from an edge,
    /// the more, up to all the way.
    pub fn show(
        &self,
        anim: &Animator,
        scroll: f32,
        max: f32,
        s: f32,
        animate: bool,
    ) -> Result<()> {
        // Drawn through the mask only while there's something to fade.
        let faded = max > 0.0;
        if faded != self.faded.replace(faded) {
            let (from, to) = (self.parent.Children()?, self.root.Children()?);
            if faded {
                from.Remove(&self.host)?;
                to.InsertAtTop(&self.host)?;
            } else {
                to.Remove(&self.host)?;
                from.InsertAbove(&self.host, &self.visual)?;
            }
        }
        let alpha = |past: f32| {
            let past = if max > 0.0 { past.max(0.0) } else { 0.0 };
            (255.0 * (1.0 - (past / (FADE * s)).min(1.0))).round() as u8
        };
        for (stop, a) in [
            (&self.top, alpha(scroll)),
            (&self.bottom, alpha(max - scroll)),
        ] {
            let to = windows::UI::Color {
                A: a,
                R: 255,
                G: 255,
                B: 255,
            };
            if !animate {
                Animator::stop(stop, &["Color"]);
                stop.SetColor(to)?;
                continue;
            }
            if stop.Color()? == to {
                continue;
            }
            // From wherever it is now.
            let a = anim.compositor.CreateColorKeyFrameAnimation()?;
            a.InsertKeyFrameWithEasingFunction(1.0, to, &anim.decelerate)?;
            a.SetDuration(ts(FADE_MS))?;
            stop.SetColor(to)?;
            stop.StartAnimation(&HSTRING::from("Color"), &a)?;
        }
        Ok(())
    }
}

/// Sizes a scrollbar `thumb`, clipped by `clip`, `h` pixels tall. Returns
/// its left edge in a card `card_w` pixels wide.
pub fn size_thumb(
    thumb: &SpriteVisual,
    clip: &CompositionRoundedRectangleGeometry,
    card_w: f32,
    h: f32,
    s: f32,
) -> Result<f32> {
    let w = THUMB_W * s;
    thumb.SetSize(v2(w, h))?;
    clip.SetSize(v2(w, h))?;
    clip.SetCornerRadius(v2(w / 2.0, w / 2.0))?;
    Ok(card_w - THUMB_RIGHT * s)
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
        anim.SetDuration(ts(ms))?;
        if delay > 0 {
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
        anim.SetDuration(ts(ms))?;
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
        anim.SetDuration(ts(ms))?;
        if delay > 0 {
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
        anim.SetDuration(ts(ms))?;
        brush.SetColor(to)?;
        brush.StartAnimation(&HSTRING::from("Color"), &anim)
    }

    /// Shakes `visual`, which rests at `at`, from side to side: a sign that
    /// what was asked for can't be done. `s` is pixels per DIP.
    pub fn shake(&self, visual: &impl Interface, at: Vector3, s: f32) -> Result<()> {
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
    /// Scale and darkness the shadow was last drawn for.
    shadow_key: Option<(u32, bool)>,
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
        let key = ((s * 100.0) as u32, t.dark);
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
    fn scrolls_only_past_rounding_slack() {
        assert_eq!(max_scroll(100.0, 99.0, 1.0), 0.0);
        assert_eq!(max_scroll(100.0, 80.0, 1.0), 20.0);
        assert_eq!(max_scroll(100.0, 97.0, 2.0), 0.0);
    }

    #[test]
    fn thumb_runs_the_length_of_the_track() {
        let (viewport, content, s) = (200.0, 800.0, 1.5);
        let max = max_scroll(content, viewport, s);
        let (top, h) = thumb_span(viewport, content, 0.0, max, s);
        assert_eq!(top, THUMB_PAD * s);
        let (end, _) = thumb_span(viewport, content, max, max, s);
        // It stops short of the corner.
        assert!((end + h - (viewport - THUMB_END * s)).abs() < 1e-3);
        // Proportional, but never too small to grab.
        assert!((h - (viewport - (THUMB_PAD + THUMB_END) * s) / 4.0).abs() < 1e-3);
        let (_, tiny) = thumb_span(viewport, 100_000.0, 0.0, 1.0, s);
        assert_eq!(tiny, THUMB_MIN_H * s);
    }

    #[test]
    fn stretches_past_the_end_less_and_less() {
        let (max, s) = (500.0, 1.0);
        let strike = |from, at, step| match stretch(from, at, step, max, s) {
            (to, Push::Strike(by)) => (to, by),
            (to, Push::None) => (to, 0.0),
        };
        assert_eq!(strike(100.0, 100.0, 100.0), (200.0, 0.0));
        let (to, over) = strike(480.0, 480.0, 40.0);
        assert_eq!(to, max);
        assert!(over > 0.0 && over < 20.0);
        // However little was left, it gives as much.
        assert_eq!(strike(499.0, 499.0, 40.0), (max, over));
        // Pushed again from there, it gives less.
        let (_, again) = strike(max + over, max, 40.0);
        assert!(again > 0.0 && again < over);
        // Never past the most, at the top too.
        let (to, up) = strike(0.0, 0.0, -10_000.0);
        assert_eq!(to, 0.0);
        assert!(up < 0.0 && up >= -STRETCH_MAX * s);
    }

    #[test]
    fn glides_past_and_settles_back() {
        let g = Glide::new(None, 0.0, 100.0, Push::Strike(10.0), 100.0, 16.0, 140);
        assert!(g.at_ms(0.0).abs() < 0.1);
        // It goes past the end, and doesn't stop there before coming back.
        let path: Vec<f32> = (0..=g.duration()).map(|t| g.at_ms(t as f32)).collect();
        let furthest = path.iter().cloned().fold(0.0, f32::max);
        assert!(furthest > 105.0 && furthest < 110.0);
        let held = path.iter().filter(|&&at| at > furthest - 0.5).count();
        assert!(held < 40);
        assert_eq!(g.at_ms(g.duration() as f32), 100.0);
        assert_eq!(g.at_ms(10_000.0), 100.0);
        // Pushed at the end, it's furthest soon after.
        let push = Glide::new(None, 100.0, 100.0, Push::Strike(10.0), 100.0, 16.0, 140);
        assert!((push.at_ms(BOUNCE_MS) - 110.0).abs() < 0.5);
        let plain = Glide::new(None, 0.0, 100.0, Push::None, 100.0, 16.0, 140);
        assert_eq!(plain.at_ms(140.0), 100.0);
        // A touchpad's scroll is there at once.
        let at_once = Glide::new(None, 0.0, 100.0, Push::None, 100.0, 16.0, 0);
        assert_eq!((at_once.at_ms(0.0), at_once.duration()), (100.0, 0));
    }

    /// Where content scrolled from `start` in `0..=max` is, every ms, as
    /// the wheel scrolls it by each `step` in turn, `gap` ms apart, then
    /// lets it settle; `precise` as a touchpad does it, at once.
    fn wheel(start: f32, max: f32, step: f32, steps: usize, gap: u64, precise: bool) -> Vec<f32> {
        let (mut scroll, mut glide, mut path) = (start, None::<Glide>, vec![start]);
        let mut bursts = Bursts::default();
        let pause = std::time::Duration::from_millis(gap);
        for _ in 0..steps {
            if !bursts.take(scroll + step, max) {
                // Held back: what's under way carries on.
                let g = glide.as_mut().unwrap();
                let t = g.ms_at(Instant::now());
                path.extend((1..=gap).map(|d| g.at_ms(t + d as f32)));
                g.started -= pause;
                continue;
            }
            let visible = glide.map_or(scroll, |g| g.at());
            let (to, push) = stretch(visible, scroll, step, max, 1.0);
            let ms = if precise {
                glide.map_or(0, |g| g.easing_left())
            } else {
                140
            };
            let mut g = Glide::new(glide, scroll, to.round(), push, max, 16.0, ms);
            scroll = to.round();
            path.extend((1..=gap).map(|t| g.at_ms(t as f32)));
            // As though `gap` went by before the next.
            g.started -= pause;
            glide = Some(g);
        }
        let g = glide.unwrap();
        let t = g.ms_at(Instant::now()) as u64;
        path.extend((t + 1..=g.duration()).map(|t| g.at_ms(t as f32)));
        path
    }

    /// How many times `path` turns round, at any speed.
    fn turns(path: &[f32]) -> usize {
        let moves: Vec<f32> = path
            .windows(2)
            .map(|w| w[1] - w[0])
            .filter(|d| d.abs() > 1e-5)
            .collect();
        moves.windows(2).filter(|d| d[0] * d[1] < 0.0).count()
    }

    #[test]
    fn runs_into_the_end_and_back_once() {
        let max = 300.0;
        // A notched wheel, spun on past the end.
        let notched = wheel(0.0, max, 56.0, 10, 40, false);
        // A touchpad: many small steps, close together, coasting on into
        // the end.
        let smooth = wheel(0.0, max, 8.0, 60, 8, true);
        for path in [notched, smooth] {
            assert_eq!(turns(&path), 1);
            assert!(path.iter().all(|&at| at <= max + STRETCH_MAX + 0.5));
            assert!(path.iter().any(|&at| at > max + 2.0));
            assert_eq!(*path.last().unwrap(), max);
        }
    }

    #[test]
    fn eases_into_a_touchpad_swipe() {
        // Windows sends the start of a swipe all at once: only a step's
        // worth of it goes.
        let mut bursts = Bursts::default();
        assert_eq!(bursts.precise_step(-150.0, 1.5), -OPENING_STEP * 1.5);
        assert_eq!(bursts.precise_step(10.0, 1.5), 10.0);
        // The rest of the swipe goes as far as it says.
        bursts.take(24.0, 1000.0);
        assert_eq!(bursts.precise_step(150.0, 1.5), 150.0);
    }

    #[test]
    fn pushes_once_a_burst() {
        let mut bursts = Bursts::default();
        let mut take = |want| bursts.take(want, 300.0);
        assert!(take(250.0));
        // Into the end: it pushes, but not as it coasts on.
        assert!(take(320.0));
        assert!(!take(330.0));
        assert!(!take(310.0));
        // Back away, and into it again.
        assert!(take(280.0));
        assert!(take(320.0));
        // After a pause, another push.
        let pause = std::time::Duration::from_millis(BURST_GAP_MS as u64 + 1);
        bursts.last = bursts.last.map(|at| at - pause);
        assert!(bursts.take(320.0, 300.0));
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
