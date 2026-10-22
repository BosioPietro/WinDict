//! The card's glyph buttons, and the dots shown while a lookup runs.

use super::*;

/// A glyph button level with the word: the arrow left of it, shown once a
/// synonym has been followed, and the translate button right of it.
pub(super) struct IconButton {
    visual: ContainerVisual,
    /// Inside `visual`, which keeps the button's place: slides in and out.
    slide: ContainerVisual,
    hover: SpriteVisual,
    hover_clip: CompositionRoundedRectangleGeometry,
    hover_brush: CompositionColorBrush,
    glyph: SpriteVisual,
    glyph_brush: CompositionSurfaceBrush,
    /// Rings the button while Tab is on it.
    ring: ShapeVisual,
    ring_shape: CompositionSpriteShape,
    ring_clip: CompositionRoundedRectangleGeometry,
    ring_brush: CompositionColorBrush,
    /// Owned by the card's window, which destroys it.
    tooltip: Option<HWND>,
    /// The tooltip's tool, unique among the card's buttons.
    tool: usize,
    icon: &'static str,
    face: Face,
    ink: Ink,
    /// Left edge in the card, in DIPs.
    x: f32,
    /// Where it slides in from, in DIPs right of its place.
    slide_from: f32,
    shown: bool,
    hovered: bool,
    focused: bool,
    /// Top of the button in the card, physical pixels.
    top: f32,
}

/// What distinguishes one [`IconButton`] from another.
pub(super) struct IconSpec<'a> {
    pub(super) icon: &'static str,
    pub(super) face: Face,
    pub(super) ink: Ink,
    pub(super) tip: &'a str,
    /// The tooltip's tool, unique among the card's buttons.
    pub(super) tool: usize,
    /// Left edge in the card, in DIPs.
    pub(super) x: f32,
    /// Where it slides in from, in DIPs right of its place.
    pub(super) slide_from: f32,
}

impl IconButton {
    pub(super) fn new(
        c: &Compositor,
        card: &ContainerVisual,
        hwnd: HWND,
        spec: IconSpec,
    ) -> Result<IconButton> {
        let IconSpec {
            icon,
            face,
            ink,
            tip,
            tool,
            x,
            slide_from,
        } = spec;
        let visual = c.CreateContainerVisual()?;
        visual.SetOpacity(0.0)?;
        card.Children()?.InsertAtTop(&visual)?;
        let slide = c.CreateContainerVisual()?;
        visual.Children()?.InsertAtTop(&slide)?;
        let hover = c.CreateSpriteVisual()?;
        let hover_clip = c.CreateRoundedRectangleGeometry()?;
        hover.SetClip(&c.CreateGeometricClipWithGeometry(&hover_clip)?)?;
        let hover_brush = c.CreateColorBrush()?;
        hover.SetBrush(&hover_brush)?;
        hover.SetOpacity(0.0)?;
        slide.Children()?.InsertAtTop(&hover)?;
        let (glyph, glyph_brush) = text_sprite(c)?;
        slide.Children()?.InsertAtTop(&glyph)?;
        let ring = c.CreateShapeVisual()?;
        let ring_clip = c.CreateRoundedRectangleGeometry()?;
        let ring_shape = c.CreateSpriteShapeWithGeometry(&ring_clip)?;
        let ring_brush = c.CreateColorBrush()?;
        ring_shape.SetStrokeBrush(&ring_brush)?;
        ring.Shapes()?.Append(&ring_shape)?;
        ring.SetOpacity(0.0)?;
        slide.Children()?.InsertAtTop(&ring)?;
        Ok(IconButton {
            visual,
            slide,
            hover,
            hover_clip,
            hover_brush,
            glyph,
            glyph_brush,
            ring,
            ring_shape,
            ring_clip,
            ring_brush,
            tooltip: create_tooltip(hwnd, tip, tool),
            tool,
            icon,
            face,
            ink,
            x,
            slide_from,
            shown: false,
            hovered: false,
            focused: false,
            top: 0.0,
        })
    }

    pub(super) fn apply_theme(&self, renderer: &Renderer, t: &Theme, s: f32) -> Result<()> {
        self.hover_brush.SetColor(t.highlight())?;
        let size = v2(BACK_SIZE * s, BACK_SIZE * s);
        self.visual.SetSize(size)?;
        self.hover.SetSize(size)?;
        self.hover_clip.SetSize(size)?;
        self.hover_clip
            .SetCornerRadius(v2(BACK_RADIUS * s, BACK_RADIUS * s))?;
        // Inside the button, the stroke centred on the ring's edge.
        let stroke = (ui::FOCUS_STROKE * s).round();
        self.ring_brush.SetColor(t.focus())?;
        self.ring.SetSize(size)?;
        self.ring_shape.SetStrokeThickness(stroke)?;
        self.ring_clip.SetOffset(v2(stroke / 2.0, stroke / 2.0))?;
        self.ring_clip
            .SetSize(v2(BACK_SIZE * s - stroke, BACK_SIZE * s - stroke))?;
        self.ring_clip
            .SetCornerRadius(v2(BACK_RADIUS * s, BACK_RADIUS * s))?;
        let layout = renderer.layout_line(self.icon, self.face, self.ink, BACK_SIZE)?;
        self.glyph_brush
            .SetSurface(&renderer.draw(&layout, s, t)?)?;
        self.glyph
            .SetSize(v2(layout.width * s, layout.height * s))?;
        self.glyph
            .SetOffset(v3(0.0, ((BACK_SIZE - layout.height) / 2.0 * s).round()))
    }

    /// Fades the button in, sliding from [`IconButton::slide_from`], or out.
    pub(super) fn set_shown(&mut self, anim: &Animator, shown: bool, s: f32) -> Result<()> {
        if shown == self.shown {
            return Ok(());
        }
        self.shown = shown;
        let away = v3(self.slide_from * s, 0.0);
        if shown {
            // Slides in alongside the word making room for it.
            Animator::stop(&self.slide, &["Offset"]);
            self.slide.SetOffset(away)?;
            anim.scalar(&self.visual, "Opacity", 1.0, RESIZE_MS, 0, &anim.fade)?;
            let home = v3(0.0, 0.0);
            anim.vector3(&self.slide, "Offset", home, RESIZE_MS, 0, &anim.decelerate)?;
        } else {
            // Gone before the word slides over where it was.
            anim.scalar(&self.visual, "Opacity", 0.0, 100, 0, &anim.decelerate)?;
            anim.vector3(&self.slide, "Offset", away, 150, 0, &anim.accelerate)?;
            self.set_hovered(anim, false)?;
            self.set_focused(false)?;
        }
        Ok(())
    }

    pub(super) fn set_hovered(&mut self, anim: &Animator, hovered: bool) -> Result<()> {
        if hovered == self.hovered {
            return Ok(());
        }
        self.hovered = hovered;
        let opacity = if hovered { 1.0 } else { 0.0 };
        anim.scalar(&self.hover, "Opacity", opacity, 100, 0, &anim.decelerate)
    }

    pub(super) fn set_focused(&mut self, focused: bool) -> Result<()> {
        self.focused = focused;
        Animator::stop(&self.ring, &["Opacity"]);
        self.ring.SetOpacity(if focused { 1.0 } else { 0.0 })
    }

    /// Rings the button `delay` ms from now.
    pub(super) fn reveal(&mut self, anim: &Animator, delay: u64) -> Result<()> {
        self.focused = true;
        if delay == 0 {
            Animator::stop(&self.ring, &["Opacity"]);
            return self.ring.SetOpacity(1.0);
        }
        Animator::stop(&self.ring, &["Opacity"]);
        self.ring.SetOpacity(0.0)?;
        anim.scalar(&self.ring, "Opacity", 1.0, 1, delay, &anim.decelerate)
    }

    /// Back at rest, shown or not without animating, for a new card.
    pub(super) fn reset(&mut self) -> Result<()> {
        Animator::stop(&self.visual, &["Opacity"]);
        Animator::stop(&self.hover, &["Opacity"]);
        Animator::stop(&self.slide, &["Offset"]);
        self.visual.SetOpacity(if self.shown { 1.0 } else { 0.0 })?;
        self.hover.SetOpacity(0.0)?;
        self.slide.SetOffset(v3(0.0, 0.0))?;
        self.hovered = false;
        self.set_focused(false)
    }

    /// Hidden, without animating, for a card with no use for it.
    pub(super) fn hide_at_once(&mut self) -> Result<()> {
        self.shown = false;
        self.reset()
    }

    pub(super) fn shown(&self) -> bool {
        self.shown
    }

    /// Whether Tab is on it.
    pub(super) fn focused(&self) -> bool {
        self.focused
    }

    /// Puts it `top` pixels down the card, level with the word.
    pub(super) fn place(&mut self, top: f32, s: f32) -> Result<()> {
        self.top = top;
        self.visual.SetOffset(self.offset(s))
    }

    /// Where it is in the card, in pixels at `s` per DIP.
    pub(super) fn offset(&self, s: f32) -> Vector3 {
        v3((self.x * s).round(), self.top)
    }

    /// Lets go of the glyph's surface while the card is idle.
    pub(super) fn release(&self) {
        let _ = self.glyph_brush.SetSurface(None::<&ICompositionSurface>);
    }

    /// Moves the tooltip's area to `rect`, in window pixels; an empty one turns it off.
    pub(super) fn set_tooltip_rect(&self, owner: HWND, rect: RECT) {
        if let Some(tooltip) = self.tooltip {
            set_tooltip_rect(tooltip, owner, self.tool, rect);
        }
    }
}

/// Three pulsing dots, shown while an online lookup runs.
pub(super) struct Loader {
    visual: ContainerVisual,
    dots: Vec<SpriteVisual>,
    brush: CompositionColorBrush,
    running: bool,
}

impl Loader {
    pub(super) fn new(c: &Compositor, host: &ContainerVisual, theme: &Theme) -> Result<Loader> {
        let visual = c.CreateContainerVisual()?;
        host.Children()?.InsertAtTop(&visual)?;
        let brush = c.CreateColorBrushWithColor(theme.accent)?;
        let mut dots = Vec::new();
        for _ in 0..3 {
            let dot = c.CreateSpriteVisual()?;
            dot.SetBrush(&brush)?;
            let ellipse = c.CreateEllipseGeometry()?;
            dot.SetClip(&c.CreateGeometricClipWithGeometry(&ellipse)?)?;
            visual.Children()?.InsertAtTop(&dot)?;
            dots.push(dot);
        }
        Ok(Loader {
            visual,
            dots,
            brush,
            running: false,
        })
    }

    pub(super) fn apply_theme(&self, t: &Theme) -> Result<()> {
        self.brush.SetColor(t.accent)
    }

    /// What shows it, to fade with the rest of the card's content.
    pub(super) fn visual(&self) -> &ContainerVisual {
        &self.visual
    }

    /// Starts the dots, or fades them out if they were running.
    pub(super) fn set_running(&mut self, anim: &Animator, running: bool, s: f32) -> Result<()> {
        let was_running = std::mem::replace(&mut self.running, running);
        if running {
            self.start(anim, s)?;
        } else if was_running {
            anim.scalar(&self.visual, "Opacity", 0.0, 120, 0, &anim.accelerate)?;
        }
        Ok(())
    }

    /// Takes the dots as no longer running, once they've been hidden with
    /// the rest of the content.
    pub(super) fn forget(&mut self) {
        self.running = false;
    }

    fn start(&self, anim: &Animator, s: f32) -> Result<()> {
        let size = 7.0 * s;
        Animator::stop(&self.visual, &["Opacity"]);
        self.visual.SetOpacity(1.0)?;
        self.visual.SetOffset(v3(PAD_X * s, 6.0 * s))?;
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
            if !ui::motion() {
                dot.SetOpacity(0.6)?;
                dot.SetScale(scale(1.0))?;
                continue;
            }
            let pulse = anim.compositor.CreateScalarKeyFrameAnimation()?;
            pulse.InsertKeyFrame(0.0, 0.25)?;
            pulse.InsertKeyFrameWithEasingFunction(0.3, 1.0, &anim.decelerate)?;
            pulse.InsertKeyFrameWithEasingFunction(0.7, 0.25, &anim.decelerate)?;
            pulse.InsertKeyFrame(1.0, 0.25)?;
            pulse.SetDuration(ts(1100))?;
            pulse.SetDelayTime(ts(i as u64 * 160))?;
            pulse.SetIterationBehavior(AnimationIterationBehavior::Forever)?;
            dot.SetOpacity(0.25)?;
            dot.StartAnimation(&HSTRING::from("Opacity"), &pulse)?;

            let bounce = anim.compositor.CreateVector3KeyFrameAnimation()?;
            let small = scale(0.8);
            bounce.InsertKeyFrame(0.0, small)?;
            bounce.InsertKeyFrameWithEasingFunction(0.3, scale(1.0), &anim.decelerate)?;
            bounce.InsertKeyFrameWithEasingFunction(0.7, small, &anim.decelerate)?;
            bounce.InsertKeyFrame(1.0, small)?;
            bounce.SetDuration(ts(1100))?;
            bounce.SetDelayTime(ts(i as u64 * 160))?;
            bounce.SetIterationBehavior(AnimationIterationBehavior::Forever)?;
            dot.SetScale(small)?;
            dot.StartAnimation(&HSTRING::from("Scale"), &bounce)?;
        }
        Ok(())
    }

    pub(super) fn stop(&self) {
        for dot in &self.dots {
            Animator::stop(dot, &["Opacity", "Scale"]);
        }
        let _ = self.visual.SetOpacity(0.0);
    }
}
