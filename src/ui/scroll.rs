//! Scrolling: how far content goes, the scrollbar thumb, the wheel's and a
//! touchpad's steps, the glide and the bounce past the ends, and the fades
//! towards them.

use std::time::Instant;

use windows::core::{Interface, Result, HSTRING};
use windows::UI::Composition::*;
use windows_numerics::Vector3;

use super::{cubic_bezier, motion, span, v2, Animator, RADIUS, SCROLL_EASE};

// Geometry, in DIPs.
/// The scrollbar thumb's width, and how far its left edge is from the
/// card's right edge.
const THUMB_W: f32 = 3.0;
const THUMB_RIGHT: f32 = 7.0;
/// Room above the thumb, and its shortest.
const THUMB_PAD: f32 = 4.0;
const THUMB_MIN_H: f32 = 28.0;
/// Room below the thumb: it ends where the card's corner starts to curve.
const THUMB_END: f32 = RADIUS;
/// The room above and below a card's thumb, in DIPs.
pub const CARD_ENDS: (f32, f32) = (THUMB_PAD, THUMB_END);

/// How far content fades out towards an edge more of it is past.
const FADE: f32 = 20.0;
/// The farthest content can be pulled past its end, and how soon pulling
/// gets hard.
const STRETCH_MAX: f32 = 16.0;
const STRETCH_SOFT: f32 = 48.0;

// Durations, in ms.
/// How soon content pushed past its end gets furthest, before it springs
/// back; it's come back after `BOUNCE_SPAN` times that.
const BOUNCE_MS: f32 = 60.0;
const BOUNCE_SPAN: f32 = 8.0;
/// How often a bounce is sampled for the compositor to play.
const BOUNCE_FRAME_MS: f32 = 16.0;
/// How long a scrolling viewport's fades take to change.
const FADE_MS: u64 = 180;

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
    thumb_between(viewport, content_h, scroll, max, s, CARD_ENDS)
}

/// Like [`thumb_span`], for a thumb with `ends` of room above and below it,
/// in DIPs.
fn thumb_between(
    viewport: f32,
    content_h: f32,
    scroll: f32,
    max: f32,
    s: f32,
    (pad, end): (f32, f32),
) -> (f32, f32) {
    let track = viewport - (pad + end) * s;
    let h = (track * viewport / content_h).max(THUMB_MIN_H * s);
    let top = pad * s + (track - h) * (scroll / max).clamp(0.0, 1.0);
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

/// Where content that scrolls is headed, the glide taking it there, and
/// the wheel's bursts that move it.
#[derive(Default)]
pub struct Scroller {
    /// Where it's headed, in pixels from the top.
    pub at: f32,
    glide: Option<Glide>,
    bursts: Bursts,
}

impl Scroller {
    /// Still, at `at`.
    pub fn stop_at(&mut self, at: f32) {
        (self.at, self.glide) = (at, None);
    }

    /// The glide under way, if one is.
    pub fn running(&self) -> Option<Glide> {
        self.glide.filter(Glide::running)
    }

    /// Where it is on screen now, following the glide under way.
    pub fn visible(&self) -> f32 {
        self.running().map_or(self.at, |glide| glide.at())
    }

    /// Heads for `to`, pushed past it, over `ms`, in content that scrolls
    /// `0..=max` at `s` pixels per DIP. It starts from where the content
    /// is on screen now, as fast as it's going, not from the last target,
    /// which would lag behind the wheel.
    pub fn glide_to(&mut self, to: f32, push: Push, max: f32, s: f32, ms: u64) -> Glide {
        let to = to.round();
        let glide = Glide::new(self.glide, self.at, to, push, max, stretch_limit(s), ms);
        (self.at, self.glide) = (to, Some(glide));
        glide
    }

    /// A wheel's `delta`, as far as `step` pixels a notch: where to, how
    /// far past it, and over how long, or nothing when it shouldn't play.
    pub fn wheel(
        &mut self,
        delta: i32,
        step: f32,
        max: f32,
        s: f32,
        ms: u64,
    ) -> Option<(f32, Push, u64)> {
        let mut step = -(delta as f32 / 120.0) * step;
        // A touchpad's scroll is smoothed already, so it follows at once.
        let precise = self.bursts.precise(delta);
        if precise {
            step = self.bursts.precise_step(step, s);
        }
        self.step(step, precise, max, s, ms)
    }

    /// A step by `step` pixels, over `ms`, or at once if it's `precise`:
    /// where to, how far past it, and over how long, or nothing when it
    /// shouldn't play. Past the end, it gives a little, and springs back.
    pub fn step(
        &mut self,
        step: f32,
        precise: bool,
        max: f32,
        s: f32,
        ms: u64,
    ) -> Option<(f32, Push, u64)> {
        if !self.bursts.take(self.at + step, max) {
            return None;
        }
        let (to, push) = stretch(self.visible(), self.at, step, max, s);
        let ms = if precise {
            self.glide.map_or(0, |glide| glide.easing_left())
        } else {
            ms
        };
        Some((to, push, ms))
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
        if !motion() {
            return Glide {
                to,
                base: to,
                past: 0.0,
                kick: 0.0,
                ms: 0,
                started,
            };
        }
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
        a.SetDuration(span(self.duration()))?;
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
            a.SetDuration(span(FADE_MS))?;
            stop.SetColor(to)?;
            stop.StartAnimation(&HSTRING::from("Color"), &a)?;
        }
        Ok(())
    }
}

/// Where content scrolls to for the thumb's top to be at `top`, below the
/// viewport's: the other way round from [`thumb_span`].
pub fn scroll_for_thumb(track: Track, top: f32) -> f32 {
    let Track { max, s, ends, .. } = track;
    let (_, h) = track.thumb(0.0);
    let room = track.viewport - (ends.0 + ends.1) * s - h;
    if room <= 0.0 {
        return 0.0;
    }
    ((top - ends.0 * s) / room).clamp(0.0, 1.0) * max
}

/// What a scrollbar runs down, in pixels at `s` per DIP: a viewport that
/// tall, over content `content_h` tall that scrolls `0..=max`, with `ends`
/// of room above and below the thumb, in DIPs.
#[derive(Clone, Copy)]
pub struct Track {
    pub viewport: f32,
    pub content_h: f32,
    pub max: f32,
    pub s: f32,
    pub ends: (f32, f32),
}

impl Track {
    /// The thumb with the content scrolled `scroll`: its top, below the
    /// viewport's, and its height.
    pub fn thumb(&self, scroll: f32) -> (f32, f32) {
        let Track {
            viewport,
            content_h,
            max,
            s,
            ends,
        } = *self;
        thumb_between(viewport, content_h, scroll, max, s, ends)
    }
}

/// How far in from the card's right edge, in DIPs, the pointer is over the
/// scrollbar.
const BAR_HIT: f32 = 12.0;
/// How wide the thumb gets under the pointer.
const THUMB_WIDE: f32 = 6.0;
/// How long the thumb takes to widen or narrow, and how long it stays wide
/// once the pointer's gone, as Windows' does, in ms.
const WIDEN_MS: u64 = 120;
const NARROW_DELAY_MS: u64 = 500;
/// How much of the viewport a press on the track above or below the thumb
/// scrolls by.
const PAGE: f32 = 0.875;

/// A scrollbar's thumb: a thin line that widens, as Windows 11's does, while
/// the pointer's over it, and that can be dragged.
pub struct Scrollbar {
    thumb: SpriteVisual,
    /// How far between thin (0) and wide (1) the thumb is, which its clip
    /// follows, and the two widths, in pixels.
    props: CompositionPropertySet,
    wide: bool,
    /// Whether the left button went down on it, and, while the thumb's
    /// dragged, how far below its top it was grabbed.
    held: bool,
    grab: Option<f32>,
}

impl Scrollbar {
    /// A hidden thumb painted with `brush`, at the top of `parent`.
    pub fn new(c: &Compositor, parent: &ContainerVisual, brush: &CompositionBrush) -> Result<Self> {
        let thumb = c.CreateSpriteVisual()?;
        thumb.SetBrush(brush)?;
        thumb.SetOpacity(0.0)?;
        let props = thumb.Properties()?;
        for name in ["Thin", "Wide", "Hover"] {
            props.InsertScalar(&HSTRING::from(name), 0.0)?;
        }
        // The visual is as wide as the thumb gets; the clip shows as much of
        // it as it is now, along its right edge.
        let clip = c.CreateRoundedRectangleGeometry()?;
        let w = "Lerp(p.Thin, p.Wide, p.Hover)";
        for (property, expression) in [
            ("Size", format!("Vector2({w}, v.Size.Y)")),
            ("Offset", format!("Vector2(v.Size.X - {w}, 0)")),
            ("CornerRadius", format!("Vector2({w} / 2, {w} / 2)")),
        ] {
            let e = c.CreateExpressionAnimationWithExpression(&HSTRING::from(expression))?;
            e.SetReferenceParameter(&HSTRING::from("p"), &props)?;
            e.SetReferenceParameter(&HSTRING::from("v"), &thumb)?;
            clip.StartAnimation(&HSTRING::from(property), &e)?;
        }
        thumb.SetClip(&c.CreateGeometricClipWithGeometry(&clip)?)?;
        parent.Children()?.InsertAtTop(&thumb)?;
        Ok(Scrollbar {
            thumb,
            props,
            wide: false,
            held: false,
            grab: None,
        })
    }

    pub fn visual(&self) -> &SpriteVisual {
        &self.thumb
    }

    /// Sizes the thumb `h` pixels tall, at `s` pixels per DIP. Returns its
    /// left edge in a card `card_w` pixels wide.
    pub fn size(&self, card_w: f32, h: f32, s: f32) -> Result<f32> {
        self.props
            .InsertScalar(&HSTRING::from("Thin"), THUMB_W * s)?;
        self.props
            .InsertScalar(&HSTRING::from("Wide"), THUMB_WIDE * s)?;
        self.thumb.SetSize(v2(THUMB_WIDE * s, h))?;
        // Widening, it grows to the left: its right edge stays put.
        Ok(card_w - (THUMB_RIGHT - THUMB_W + THUMB_WIDE) * s)
    }

    /// Whether a point `x` across a card `card_w` wide and `y` down the
    /// viewport is on the strip the thumb runs down, when there's anything
    /// to scroll.
    pub fn over(x: f32, y: f32, card_w: f32, track: Track) -> bool {
        track.max > 0.0
            && x >= card_w - BAR_HIT * track.s
            && x < card_w
            && y >= 0.0
            && y < track.viewport
    }

    /// Widens the thumb while the pointer is `over` it, or it's dragged,
    /// and narrows it a while after.
    pub fn hover(&mut self, anim: &Animator, over: bool) -> Result<()> {
        let wide = over || self.grab.is_some();
        if wide == self.wide {
            return Ok(());
        }
        self.wide = wide;
        let (to, delay) = if wide {
            (1.0, 0)
        } else {
            (0.0, NARROW_DELAY_MS)
        };
        anim.scalar(&self.props, "Hover", to, WIDEN_MS, delay, &anim.decelerate)
    }

    /// The left button went down `y` down the viewport, on the strip, with
    /// the content scrolled `scroll`. On the thumb, it grabs it; above or
    /// below, it returns where to scroll to, a page that way.
    pub fn press(&mut self, y: f32, scroll: f32, track: Track) -> Option<f32> {
        self.held = true;
        let (top, h) = track.thumb(scroll);
        if (top..top + h).contains(&y) {
            self.grab = Some(y - top);
            return None;
        }
        let page = track.viewport * PAGE;
        let to = if y < top {
            scroll - page
        } else {
            scroll + page
        };
        Some(to.clamp(0.0, track.max))
    }

    /// Where to scroll to, while the thumb's dragged, for the pointer `y`
    /// down the viewport.
    pub fn drag(&self, y: f32, track: Track) -> Option<f32> {
        self.grab
            .map(|grab| scroll_for_thumb(track, y - grab).round())
    }

    pub fn dragging(&self) -> bool {
        self.grab.is_some()
    }

    /// The left button came up, or the window lost it. Returns whether it
    /// had gone down on the scrollbar.
    pub fn release(&mut self) -> bool {
        self.grab = None;
        std::mem::take(&mut self.held)
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
    fn dragging_the_thumb_scrolls_to_where_it_is() {
        let (viewport, content_h, s) = (200.0, 800.0, 1.5);
        let max = max_scroll(content_h, viewport, s);
        let track = Track {
            viewport,
            content_h,
            max,
            s,
            ends: CARD_ENDS,
        };
        for scroll in [0.0, 120.0, max / 2.0, max] {
            let (top, _) = thumb_span(viewport, content_h, scroll, max, s);
            assert!((scroll_for_thumb(track, top) - scroll).abs() < 1e-2);
        }
        // Dragged past either end, it stops there.
        assert_eq!(scroll_for_thumb(track, -50.0), 0.0);
        assert_eq!(scroll_for_thumb(track, 10_000.0), max);
        // A thumb with room of its own at the ends runs between them.
        let list = Track {
            ends: (4.0, 4.0),
            ..track
        };
        let (top, h) = list.thumb(max);
        assert!((top + h - (viewport - 4.0 * s)).abs() < 1e-3);
        for scroll in [0.0, max / 3.0, max] {
            let (top, _) = list.thumb(scroll);
            assert!((scroll_for_thumb(list, top) - scroll).abs() < 1e-2);
        }
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
}
