//! The card's heading, its glyph buttons, and the dots shown while a lookup
//! runs.

use super::*;

/// The word and its pronunciation. `back` holds the previous rendering
/// of the word while it cross-fades to the new one; the pronunciation,
/// inside `visual`, fades in on its own once it's known.
pub(super) struct Header {
    visual: SpriteVisual,
    brush: CompositionSurfaceBrush,
    back: SpriteVisual,
    back_brush: CompositionSurfaceBrush,
    surface: Option<CompositionDrawingSurface>,
    phonetic: SpriteVisual,
    phonetic_brush: CompositionSurfaceBrush,
    /// What the pronunciation shows.
    phonetic_key: Option<String>,
}

/// How long the heading takes to fade as it sinks into the search box's
/// field, as what's typed shows in its place: most of the way there, so
/// that the one turns into the other on the way rather than once there.
pub(super) const SINK_FADE_MS: u64 = 240;

/// How the heading comes to its place.
pub(super) enum Arrival {
    /// Growing from the word typed in the search box, at `from` and
    /// `scale` times its size.
    Rise { from: Vector3, scale: f32 },
    /// Gliding over from `from_x` DIPs in, after `delay` ms.
    Glide { from_x: f32, delay: u64 },
    /// Already there.
    Still,
}

impl Header {
    pub(super) fn new(c: &Compositor, card: &ContainerVisual) -> Result<Header> {
        let (back, back_brush) = text_sprite(c)?;
        back.SetOpacity(0.0)?;
        card.Children()?.InsertAtTop(&back)?;
        let (visual, brush) = text_sprite(c)?;
        card.Children()?.InsertAtTop(&visual)?;
        let (phonetic, phonetic_brush) = text_sprite(c)?;
        phonetic.SetOpacity(0.0)?;
        visual.Children()?.InsertAtTop(&phonetic)?;
        Ok(Header {
            visual,
            brush,
            back,
            back_brush,
            surface: None,
            phonetic,
            phonetic_brush,
            phonetic_key: None,
        })
    }

    /// What's in the heading and moves with it, such as the button after
    /// the pronunciation.
    pub(super) fn host(&self) -> Result<ContainerVisual> {
        self.visual.cast()
    }

    /// Shows the pronunciation `text`, drawn on `surface`, `size` pixels,
    /// at `at` in the heading. With `fade`, a new one fades in, sliding a
    /// little into place, and one that moved glides there.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn show_phonetic(
        &mut self,
        anim: &Animator,
        text: &str,
        surface: &CompositionDrawingSurface,
        size: Vector2,
        at: Vector3,
        fade: bool,
        s: f32,
    ) -> Result<()> {
        let new = self.phonetic_key.as_deref() != Some(text);
        self.phonetic_key = Some(text.to_owned());
        self.phonetic_brush.SetSurface(surface)?;
        self.phonetic.SetSize(size)?;
        if !new {
            return self.place_phonetic(anim, at, fade);
        }
        Animator::stop(&self.phonetic, &["Opacity", "Offset"]);
        if !fade {
            self.phonetic.SetOffset(at)?;
            return self.phonetic.SetOpacity(1.0);
        }
        self.phonetic
            .SetOffset(v3(at.X - PHONETIC_SLIDE * s, at.Y))?;
        self.phonetic.SetOpacity(0.0)?;
        let ease = &anim.decelerate;
        anim.scalar(&self.phonetic, "Opacity", 1.0, PHONETIC_MS, 0, ease)?;
        anim.vector3(&self.phonetic, "Offset", at, PHONETIC_MS, 0, ease)
    }

    /// Moves the pronunciation to `at` in the heading, gliding with `glide`.
    fn place_phonetic(&self, anim: &Animator, at: Vector3, glide: bool) -> Result<()> {
        if !glide {
            Animator::stop(&self.phonetic, &["Offset"]);
            return self.phonetic.SetOffset(at);
        }
        anim.vector3(&self.phonetic, "Offset", at, RESIZE_MS, 0, &anim.decelerate)
    }

    /// Takes the pronunciation away, for a heading without one.
    pub(super) fn hide_phonetic(&mut self) -> Result<()> {
        self.phonetic_key = None;
        Animator::stop(&self.phonetic, &["Opacity", "Offset"]);
        self.phonetic.SetOpacity(0.0)
    }

    /// What the heading shows now, if anything.
    pub(super) fn surface(&self) -> Option<CompositionDrawingSurface> {
        self.surface.clone()
    }

    /// Shows the heading drawn on `surface`, `size` pixels. With
    /// `crossfade`, the one it showed fades out over it.
    pub(super) fn show(
        &mut self,
        anim: &Animator,
        surface: &CompositionDrawingSurface,
        size: Vector2,
        crossfade: bool,
        s: f32,
    ) -> Result<()> {
        let old = self.surface.replace(surface.clone());
        Animator::stop(&self.visual, &["Opacity"]);
        Animator::stop(&self.back, &["Opacity"]);
        self.back.SetOpacity(0.0)?;
        if let (true, Some(old)) = (crossfade, old) {
            self.back_brush.SetSurface(&old)?;
            self.back.SetSize(self.visual.Size()?)?;
            self.back.SetOffset(v3(PAD_X * s, PAD_TOP * s))?;
            self.back.SetOpacity(1.0)?;
            self.visual.SetOpacity(0.0)?;
            anim.scalar(&self.back, "Opacity", 0.0, 240, 0, &anim.decelerate)?;
            anim.scalar(&self.visual, "Opacity", 1.0, 240, 0, &anim.decelerate)?;
        } else {
            self.visual.SetOpacity(1.0)?;
        }
        self.brush.SetSurface(surface)?;
        self.visual.SetSize(size)
    }

    /// Brings the heading to its place as `arrival` says.
    pub(super) fn arrive(&self, anim: &Animator, arrival: Arrival, s: f32) -> Result<()> {
        let rest = v3(PAD_X * s, PAD_TOP * s);
        let full = scale(1.0);
        Animator::stop(&self.visual, &["Offset", "Scale"]);
        self.visual.SetScale(full)?;
        match arrival {
            Arrival::Rise { from, scale: k } => {
                self.visual.SetScale(Vector3 { X: k, Y: k, Z: 1.0 })?;
                self.visual.SetOffset(from)?;
                self.visual.SetOpacity(0.0)?;
                let ease = &anim.decelerate;
                anim.vector3(&self.visual, "Offset", rest, RESIZE_MS, 0, ease)?;
                anim.vector3(&self.visual, "Scale", full, RESIZE_MS, 0, ease)?;
                anim.scalar(&self.visual, "Opacity", 1.0, 160, 0, ease)
            }
            Arrival::Glide { from_x, delay } => {
                self.visual.SetOffset(v3(from_x * s, PAD_TOP * s))?;
                let ease = &anim.decelerate;
                anim.vector3(&self.visual, "Offset", rest, RESIZE_MS, delay, ease)
            }
            Arrival::Still => self.visual.SetOffset(rest),
        }
    }

    /// Shrinks the heading back into the search box's field, to `to` and
    /// `scale` times its size, fading as it gets there: Rise, backwards.
    pub(super) fn sink(&self, anim: &Animator, to: Vector3, k: f32) -> Result<()> {
        Animator::stop(&self.visual, &["Opacity", "Offset", "Scale"]);
        let ease = &anim.decelerate;
        let fade_ms = SINK_FADE_MS;
        anim.vector3(&self.visual, "Offset", to, RESIZE_MS, 0, ease)?;
        let small = Vector3 { X: k, Y: k, Z: 1.0 };
        anim.vector3(&self.visual, "Scale", small, RESIZE_MS, 0, ease)?;
        let delay = RESIZE_MS - fade_ms;
        anim.scalar(&self.visual, "Opacity", 0.0, fade_ms, delay, &anim.fade)
    }

    /// What shows the heading, and the one it's fading from, to fade with
    /// the rest of the card.
    pub(super) fn visuals(&self) -> Result<[Visual; 2]> {
        Ok([self.visual.cast()?, self.back.cast()?])
    }

    /// Back at rest, for a new card.
    pub(super) fn reset(&mut self) -> Result<()> {
        Animator::stop(&self.visual, &["Opacity", "Offset", "Scale"]);
        self.hide_phonetic()?;
        self.visual.SetScale(scale(1.0))
    }

    /// Lets go of the heading's surfaces while the card is idle.
    pub(super) fn release(&mut self) {
        let _ = self.brush.SetSurface(None::<&ICompositionSurface>);
        let _ = self.back_brush.SetSurface(None::<&ICompositionSurface>);
        let _ = self.phonetic_brush.SetSurface(None::<&ICompositionSurface>);
        let _ = self.hide_phonetic();
        self.surface = None;
    }
}

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
    /// Fills the glyph in, over it, when the button's on.
    fill: SpriteVisual,
    fill_brush: CompositionSurfaceBrush,
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
    /// The glyph filling `icon` in, if it's filled.
    fill_icon: Option<&'static str>,
    face: Face,
    ink: Ink,
    /// Left edge in what it's in, in DIPs.
    x: f32,
    /// Where what it's in is in the card, in DIPs.
    origin: Vector2,
    /// Where it slides in from, in DIPs right of its place.
    slide_from: f32,
    shown: bool,
    /// Whether it's been put in its place since it was last shown.
    placed: bool,
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
    /// Left edge in what it's in, in DIPs.
    pub(super) x: f32,
    /// Where what it's in is in the card, in DIPs.
    pub(super) origin: Vector2,
    /// Where it slides in from, in DIPs right of its place.
    pub(super) slide_from: f32,
}

impl IconButton {
    /// A button in `parent`: the card, or a part of it.
    pub(super) fn new(
        c: &Compositor,
        parent: &ContainerVisual,
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
            origin,
            slide_from,
        } = spec;
        let visual = c.CreateContainerVisual()?;
        visual.SetOpacity(0.0)?;
        parent.Children()?.InsertAtTop(&visual)?;
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
        let (fill, fill_brush) = text_sprite(c)?;
        slide.Children()?.InsertAtTop(&fill)?;
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
            fill,
            fill_brush,
            ring,
            ring_shape,
            ring_clip,
            ring_brush,
            tooltip: create_tooltip(hwnd, tip, tool),
            tool,
            icon,
            fill_icon: None,
            face,
            ink,
            x,
            origin,
            slide_from,
            shown: false,
            placed: false,
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
        let layers = [
            (&self.glyph, &self.glyph_brush, Some(self.icon)),
            (&self.fill, &self.fill_brush, self.fill_icon),
        ];
        for (sprite, brush, icon) in layers {
            let Some(icon) = icon else {
                brush.SetSurface(None::<&ICompositionSurface>)?;
                continue;
            };
            let layout = renderer.layout_line(icon, self.face, self.ink, BACK_SIZE)?;
            brush.SetSurface(&renderer.draw(&layout, s, t)?)?;
            sprite.SetSize(v2(layout.width * s, layout.height * s))?;
            sprite.SetOffset(v3(0.0, ((BACK_SIZE - layout.height) / 2.0 * s).round()))?;
        }
        Ok(())
    }

    /// Fades the button in, sliding from [`IconButton::slide_from`], or out.
    pub(super) fn set_shown(&mut self, anim: &Animator, shown: bool, s: f32) -> Result<()> {
        if shown == self.shown {
            return Ok(());
        }
        self.shown = shown;
        self.placed = false;
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

    /// Puts it `x` DIPs in and `top` pixels down what it's in. With
    /// `glide`, one already in its place glides over to the new one.
    pub(super) fn move_to(
        &mut self,
        anim: &Animator,
        x: f32,
        top: f32,
        glide: bool,
        s: f32,
    ) -> Result<()> {
        (self.x, self.top) = (x, top);
        let at = self.offset(s);
        if glide && std::mem::replace(&mut self.placed, true) {
            return anim.vector3(&self.visual, "Offset", at, RESIZE_MS, 0, &anim.decelerate);
        }
        Animator::stop(&self.visual, &["Offset"]);
        self.visual.SetOffset(at)
    }

    /// Where it is in what it's in, in pixels at `s` per DIP.
    fn offset(&self, s: f32) -> Vector3 {
        v3((self.x * s).round(), self.top)
    }

    /// Where it is in the card, in pixels at `s` per DIP.
    pub(super) fn card_offset(&self, s: f32) -> Vector3 {
        let at = self.offset(s);
        v3(at.X + self.origin.X * s, at.Y + self.origin.Y * s)
    }

    /// Changes the glyph, what fills it in, and its color, from the next
    /// [`IconButton::apply_theme`].
    pub(super) fn set_icon(&mut self, icon: &'static str, fill: Option<&'static str>, ink: Ink) {
        (self.icon, self.fill_icon, self.ink) = (icon, fill, ink);
    }

    /// Changes what its tooltip says.
    pub(super) fn set_tip(&self, owner: HWND, tip: &str) {
        if let Some(tooltip) = self.tooltip {
            win::set_tooltip_text(tooltip, owner, self.tool, tip);
        }
    }

    /// Lets go of the glyph's surface while the card is idle.
    pub(super) fn release(&self) {
        let _ = self.glyph_brush.SetSurface(None::<&ICompositionSurface>);
        let _ = self.fill_brush.SetSurface(None::<&ICompositionSurface>);
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

/// "Copied", shown for a moment at the bottom of the card, on acrylic of
/// its own so the text under it doesn't show through, a shade apart from
/// the card's and casting a shadow on it.
pub(super) struct Badge {
    visual: ContainerVisual,
    shadow: SpriteVisual,
    shadow_surface: CompositionSurfaceBrush,
    shadow_brush: CompositionNineGridBrush,
    /// Inside `visual`, clipped to its rounded corners.
    panel: ContainerVisual,
    clip: CompositionRoundedRectangleGeometry,
    tint: CompositionColorBrush,
    shade: CompositionColorBrush,
    border: ShapeVisual,
    border_geometry: CompositionRoundedRectangleGeometry,
    border_brush: CompositionColorBrush,
    text: SpriteVisual,
    text_brush: CompositionSurfaceBrush,
}

impl Badge {
    pub(super) fn new(c: &Compositor, card: &ContainerVisual) -> Result<Badge> {
        let visual = c.CreateContainerVisual()?;
        visual.SetOpacity(0.0)?;
        card.Children()?.InsertAtTop(&visual)?;
        let shadow = c.CreateSpriteVisual()?;
        let shadow_surface = c.CreateSurfaceBrush()?;
        shadow_surface.SetStretch(CompositionStretch::Fill)?;
        let shadow_brush = c.CreateNineGridBrush()?;
        shadow_brush.SetSource(&shadow_surface)?;
        shadow.SetBrush(&shadow_brush)?;
        visual.Children()?.InsertAtTop(&shadow)?;
        let panel = c.CreateContainerVisual()?;
        let clip = c.CreateRoundedRectangleGeometry()?;
        panel.SetClip(&c.CreateGeometricClipWithGeometry(&clip)?)?;
        visual.Children()?.InsertAtTop(&panel)?;
        // Acrylic, as on the card: the blurred desktop, then the tint, then
        // a shade that sets it apart from the card.
        let backdrop = c.CreateSpriteVisual()?;
        backdrop.SetBrush(&c.CreateHostBackdropBrush()?)?;
        backdrop.SetRelativeSizeAdjustment(v2(1.0, 1.0))?;
        panel.Children()?.InsertAtTop(&backdrop)?;
        let layer = |brush: &CompositionColorBrush| -> Result<()> {
            let sprite = c.CreateSpriteVisual()?;
            sprite.SetBrush(brush)?;
            sprite.SetRelativeSizeAdjustment(v2(1.0, 1.0))?;
            panel.Children()?.InsertAtTop(&sprite)
        };
        let (tint, shade) = (c.CreateColorBrush()?, c.CreateColorBrush()?);
        layer(&tint)?;
        layer(&shade)?;
        let border = c.CreateShapeVisual()?;
        let border_geometry = c.CreateRoundedRectangleGeometry()?;
        let border_shape = c.CreateSpriteShapeWithGeometry(&border_geometry)?;
        let border_brush = c.CreateColorBrush()?;
        border_shape.SetStrokeBrush(&border_brush)?;
        border_shape.SetStrokeThickness(1.0)?;
        border.Shapes()?.Append(&border_shape)?;
        visual.Children()?.InsertAtTop(&border)?;
        let (text, text_brush) = text_sprite(c)?;
        visual.Children()?.InsertAtTop(&text)?;
        Ok(Badge {
            visual,
            shadow,
            shadow_surface,
            shadow_brush,
            panel,
            clip,
            tint,
            shade,
            border,
            border_geometry,
            border_brush,
            text,
            text_brush,
        })
    }

    pub(super) fn apply_theme(&self, renderer: &Renderer, t: &Theme, s: f32) -> Result<()> {
        self.tint.SetColor(t.tint())?;
        self.shade.SetColor(t.badge_shade())?;
        self.border_brush.SetColor(t.border())?;
        let opacity = t.shadow_opacity() * BADGE_SHADOW_OPACITY;
        let (surface, inset) = renderer.draw_shadow(BADGE_SHADOW * s, BADGE_RADIUS * s, opacity)?;
        self.shadow_surface.SetSurface(&surface)?;
        self.shadow_brush.SetInsets(inset)
    }

    /// Shows it at the bottom of a card `card` pixels in size, then fades
    /// it out again.
    pub(super) fn flash(
        &self,
        anim: &Animator,
        renderer: &Renderer,
        theme: &Theme,
        card: Vector2,
        s: f32,
    ) -> Result<()> {
        let layout = renderer.layout_badge(COPIED_GLYPH, tr().copied)?;
        self.text_brush
            .SetSurface(&renderer.draw(&layout, s, theme)?)?;
        let (w, h) = (
            ((layout.width + 2.0 * BADGE_PAD_X) * s).round(),
            ((layout.height + 2.0 * BADGE_PAD_Y) * s).round(),
        );
        let r = BADGE_RADIUS * s;
        self.text.SetSize(v2(layout.width * s, layout.height * s))?;
        self.text
            .SetOffset(v3((BADGE_PAD_X * s).round(), (BADGE_PAD_Y * s).round()))?;
        self.visual.SetSize(v2(w, h))?;
        self.panel.SetSize(v2(w, h))?;
        self.clip.SetSize(v2(w, h))?;
        let spread = BADGE_SHADOW * s;
        self.shadow
            .SetOffset(v3(-spread, -spread + BADGE_SHADOW_DY * s))?;
        self.shadow
            .SetSize(v2(w + 2.0 * spread, h + 2.0 * spread))?;
        self.clip.SetCornerRadius(v2(r, r))?;
        // The hairline centred on the edge, inside it.
        self.border.SetSize(v2(w, h))?;
        self.border_geometry.SetOffset(v2(0.5, 0.5))?;
        self.border_geometry.SetSize(v2(w - 1.0, h - 1.0))?;
        self.border_geometry.SetCornerRadius(v2(r - 0.5, r - 0.5))?;
        let rest = v3(
            ((card.X - w) / 2.0).round(),
            (card.Y - h - BADGE_BOTTOM * s).round(),
        );

        // Fades in, stays a moment and fades out, all in one: a second
        // opacity animation would replace the first. It fades even with
        // no motion, as it would otherwise not be seen at all.
        let c = &anim.compositor;
        let fade = c.CreateScalarKeyFrameAnimation()?;
        fade.InsertKeyFrame(0.0, 0.0)?;
        fade.InsertKeyFrameWithEasingFunction(0.12, 1.0, &anim.decelerate)?;
        fade.InsertKeyFrame(0.78, 1.0)?;
        fade.InsertKeyFrameWithEasingFunction(1.0, 0.0, &anim.accelerate)?;
        fade.SetDuration(ts(BADGE_MS))?;
        self.visual.SetOpacity(0.0)?;
        self.visual
            .StartAnimation(&HSTRING::from("Opacity"), &fade)?;
        // Rises a little into place.
        Animator::stop(&self.visual, &["Offset"]);
        self.visual.SetOffset(v3(rest.X, rest.Y + BADGE_RISE * s))?;
        anim.vector3(
            &self.visual,
            "Offset",
            rest,
            BADGE_RISE_MS,
            0,
            &anim.decelerate,
        )
    }

    /// Hidden at once, for a new card.
    pub(super) fn reset(&self) -> Result<()> {
        Animator::stop(&self.visual, &["Opacity", "Offset"]);
        self.visual.SetOpacity(0.0)
    }

    /// Lets go of the text's surface while the card is idle.
    pub(super) fn release(&self) {
        let _ = self.reset();
        let _ = self.text_brush.SetSurface(None::<&ICompositionSurface>);
    }
}
