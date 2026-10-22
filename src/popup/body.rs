//! The scrolling part of the card: the rendered definition, its links, the
//! rings Tab puts around them, and the swap button's highlight.

use super::*;

/// The scrolling part of the card: a clipped host below the header, and the
/// rendered definition inside it.
pub(super) struct Content {
    host: ContainerVisual,
    /// Shows `host`, faded at the ends it scrolls towards.
    fade: ScrollFade,
    visual: SpriteVisual,
    brush: CompositionSurfaceBrush,
    links: Vec<Link>,
    rings: Rings,
    /// The caret in the language box being typed in; inside `visual` too.
    caret: SpriteVisual,
    caret_brush: CompositionColorBrush,
    /// The first of the focused link's rectangles in `links`.
    focused: Option<usize>,
    swap: SwapHighlight,
}

impl Content {
    pub(super) fn new(c: &Compositor, card: &ContainerVisual, hwnd: HWND) -> Result<Content> {
        let host = c.CreateContainerVisual()?;
        host.SetClip(&c.CreateInsetClip()?)?;
        let (visual, brush) = text_sprite(c)?;
        host.Children()?.InsertAtTop(&visual)?;
        // Inside `visual`, so they scroll with it, in this order.
        let swap = SwapHighlight::new(c, &visual, hwnd)?;
        let rings = Rings::new(c, &visual)?;
        let caret = c.CreateSpriteVisual()?;
        let caret_brush = c.CreateColorBrush()?;
        caret.SetBrush(&caret_brush)?;
        caret.SetOpacity(0.0)?;
        visual.Children()?.InsertAtTop(&caret)?;
        let fade = ScrollFade::new(c, card, &host)?;
        Ok(Content {
            host,
            fade,
            visual,
            brush,
            links: Vec::new(),
            rings,
            caret,
            caret_brush,
            focused: None,
            swap,
        })
    }

    /// The clipped viewport, for what shows in the body's place.
    pub(super) fn host(&self) -> &ContainerVisual {
        &self.host
    }

    /// What shows the body, to fade or reset with the rest of the card.
    pub(super) fn visual(&self) -> Result<Visual> {
        self.visual.cast()
    }

    /// The brush the rings are painted with, which a ring gliding over the
    /// card shares.
    pub(super) fn focus_brush(&self) -> &CompositionColorBrush {
        &self.rings.brush
    }

    pub(super) fn apply_theme(&self, c: &Compositor, t: &Theme, s: f32) -> Result<()> {
        self.rings.brush.SetColor(t.focus())?;
        self.swap.brush.SetColor(t.highlight())?;
        self.caret_brush.SetColor(t.accent)?;
        self.fade.set_scale(c, s)
    }

    /// Lets go of the body's surface while the card is idle.
    pub(super) fn release(&self) {
        let _ = self.brush.SetSurface(None::<&ICompositionSurface>);
    }

    pub(super) fn links(&self) -> &[Link] {
        &self.links
    }

    pub(super) fn set_links(&mut self, links: Vec<Link>) {
        self.links = links;
    }

    /// The link at a point in the body, in DIPs from its top left.
    pub(super) fn link_at(&self, x: f32, y: f32) -> Option<&Target> {
        self.links
            .iter()
            .find(|l| x >= l.x && x < l.x + l.width && y >= l.y && y < l.y + l.height)
            .map(|l| &l.target)
    }

    /// Where the link Tab is on starts in [`Content::links`], if it's on one.
    pub(super) fn focused(&self) -> Option<usize> {
        self.focused
    }

    /// The swap button's link, if the card has one.
    pub(super) fn swap_link(&self) -> Option<&Link> {
        self.links.iter().find(|l| l.target == Target::Swap)
    }

    /// The rectangles of the link starting at `start`: more than one when
    /// a synonym wraps.
    pub(super) fn rects(&self, start: usize) -> impl Iterator<Item = &Link> {
        let target = &self.links[start].target;
        self.links[start..]
            .iter()
            .take_while(move |l| &l.target == target)
    }

    /// Where each link starts in `links`, in reading order.
    pub(super) fn stops(&self) -> Vec<usize> {
        (0..self.links.len())
            .filter(|&i| i == 0 || self.links[i - 1].target != self.links[i].target)
            .collect()
    }

    /// Shows the body drawn on `surface`, `size` pixels, with the caret at
    /// `caret` if it has one, everything stopped where it is.
    pub(super) fn show(
        &self,
        c: &Compositor,
        surface: &CompositionDrawingSurface,
        size: Vector2,
        caret: Option<Mark>,
        s: f32,
    ) -> Result<()> {
        self.brush.SetSurface(surface)?;
        self.visual.SetSize(size)?;
        self.show_caret(c, caret, s)?;
        Animator::stop(&self.visual, &["Offset", "Opacity"]);
        Animator::stop(&self.host, &["Offset"]);
        Ok(())
    }

    /// Puts the caret at `mark`, blinking, or takes it away.
    fn show_caret(&self, c: &Compositor, mark: Option<Mark>, s: f32) -> Result<()> {
        Animator::stop(&self.caret, &["Opacity"]);
        let Some(m) = mark else {
            return self.caret.SetOpacity(0.0);
        };
        self.caret
            .SetSize(v2((m.width * s).round().max(1.0), (m.height * s).round()))?;
        self.caret
            .SetOffset(v3((m.x * s).round(), (m.y * s).round()))?;
        blink(c, &self.caret)
    }

    /// Shows the body at the top, `header_h` pixels down the card, at once.
    pub(super) fn appear(&self, header_h: f32, s: f32) -> Result<()> {
        self.visual.SetOpacity(1.0)?;
        self.visual.SetOffset(v3(PAD_X * s, 0.0))?;
        self.host.SetOffset(v3(0.0, header_h))
    }

    /// Slides the body up a touch from the top as it fades in.
    pub(super) fn slide_in(&self, anim: &Animator, s: f32) -> Result<()> {
        self.visual.SetOpacity(0.0)?;
        self.visual.SetOffset(v3(PAD_X * s, 8.0 * s))?;
        anim.scalar(&self.visual, "Opacity", 1.0, 220, 80, &anim.decelerate)?;
        let rest = v3(PAD_X * s, 0.0);
        anim.vector3(
            &self.visual,
            "Offset",
            rest,
            RESIZE_MS,
            80,
            &anim.decelerate,
        )
    }

    /// Shows the body scrolled `scroll` pixels, keeping the reader's place.
    pub(super) fn keep_place(&self, scroll: f32, s: f32) -> Result<()> {
        self.visual.SetOpacity(1.0)?;
        self.visual.SetOffset(v3(PAD_X * s, -scroll.round()))
    }

    /// Moves the body to start `top` pixels down the card, gliding there
    /// with `animate`.
    pub(super) fn move_host(&self, anim: &Animator, top: f32, animate: bool) -> Result<()> {
        let to = v3(0.0, top);
        if animate {
            anim.vector3(&self.host, "Offset", to, RESIZE_MS, 0, &anim.decelerate)
        } else {
            self.host.SetOffset(to)
        }
    }

    /// Sizes the viewport, along with the card's resize with `animate`.
    pub(super) fn size_host(&self, anim: &Animator, size: Vector2, animate: bool) -> Result<()> {
        if animate {
            anim.vector2(&self.host, "Size", size, RESIZE_MS)
        } else {
            Animator::stop(&self.host, &["Size"]);
            self.host.SetSize(size)
        }
    }

    /// Plays `glide` on the body, which sits `x` pixels in.
    pub(super) fn play_scroll(&self, anim: &Animator, glide: &Glide, x: f32) -> Result<()> {
        glide.play(anim, &self.visual, |scroll| v3(x, -scroll))
    }

    /// Fades the body towards the ends it scrolls to (see [`ScrollFade::show`]).
    pub(super) fn show_fades(
        &self,
        anim: &Animator,
        scroll: f32,
        max: f32,
        s: f32,
        animate: bool,
    ) -> Result<()> {
        self.fade.show(anim, scroll, max, s, animate)
    }

    /// Rings the link starting at `start` in the body's links, or none,
    /// `reveal` ms from now. With a delay, the rings that are showing go at
    /// once, as another is gliding over the card to take their place.
    pub(super) fn focus(
        &mut self,
        anim: &Animator,
        renderer: &Renderer,
        start: Option<usize>,
        reveal: u64,
        s: f32,
    ) -> Result<()> {
        self.focused = start;
        let Some(start) = start else {
            return self.rings.hide(anim, reveal > 0);
        };
        // The stroke is inside the mask, so each ring is its link grown by
        // half of it, centred on the link's edge.
        let stroke = (ui::FOCUS_STROKE * s).round();
        let grow = stroke / 2.0;
        let rects: Vec<_> = self
            .rects(start)
            .map(|l| {
                let (x, y) = ((l.x * s).round(), (l.y * s).round());
                let (w, h) = ((l.width * s).round(), (l.height * s).round());
                (v3(x - grow, y - grow), v2(w + stroke, h + stroke))
            })
            .collect();
        self.rings.show(anim, renderer, &rects, reveal, s)
    }

    /// Lightens the swap button while the pointer's `over` it, and keeps
    /// its tooltip over `rect`, its part in view in window pixels.
    pub(super) fn sync_swap(
        &mut self,
        anim: &Animator,
        owner: HWND,
        rect: RECT,
        over: bool,
        s: f32,
    ) -> Result<()> {
        let link = self.links.iter().find(|l| l.target == Target::Swap);
        self.swap.sync(anim, owner, link, rect, over, s)
    }

    /// Takes the swap button's highlight away at once, for a new card.
    pub(super) fn reset_swap(&mut self) -> Result<()> {
        self.swap.reset()
    }
}

/// Rings the link Tab moved to: one ring for each of the link's rectangles,
/// kept to glide from link to link; the ones not in use are faded out.
struct Rings {
    container: ContainerVisual,
    brush: CompositionColorBrush,
    rings: Vec<SpriteVisual>,
    /// The mask they all show through, stretched by its corners' radius.
    source: CompositionSurfaceBrush,
    grid: CompositionNineGridBrush,
    mask: CompositionMaskBrush,
    /// Whether a link was ringed last, for the ring to glide from.
    shown: bool,
}

impl Rings {
    fn new(c: &Compositor, parent: &SpriteVisual) -> Result<Rings> {
        let container = c.CreateContainerVisual()?;
        parent.Children()?.InsertAtTop(&container)?;
        let brush = c.CreateColorBrush()?;
        let source = c.CreateSurfaceBrush()?;
        source.SetStretch(CompositionStretch::Fill)?;
        let grid = c.CreateNineGridBrush()?;
        grid.SetSource(&source)?;
        let mask = c.CreateMaskBrush()?;
        mask.SetSource(&brush)?;
        mask.SetMask(&grid)?;
        Ok(Rings {
            container,
            brush,
            rings: Vec::new(),
            source,
            grid,
            mask,
            shown: false,
        })
    }

    /// Takes the rings away, `at_once` or fading.
    fn hide(&mut self, anim: &Animator, at_once: bool) -> Result<()> {
        self.shown = false;
        for ring in &self.rings {
            fade_out(anim, ring, at_once)?;
        }
        Ok(())
    }

    /// Rings each of `rects` (place and size in the body, in pixels),
    /// `reveal` ms from now: one in view glides there, the others fade in.
    fn show(
        &mut self,
        anim: &Animator,
        renderer: &Renderer,
        rects: &[(Vector3, Vector2)],
        reveal: u64,
        s: f32,
    ) -> Result<()> {
        // Drawn pixel for pixel and shown through a mask, as shapes are
        // jagged in what scrolls.
        let stroke = (ui::FOCUS_STROKE * s).round();
        let radius = FOCUS_RADIUS * s + stroke / 2.0;
        let side = (2.0 * radius).ceil() + 1.0;
        let mask = renderer.draw_rounded(side, side, radius, Some(stroke))?;
        self.source.SetSurface(&mask)?;
        self.grid.SetInsets(radius)?;
        let glide = self.shown && reveal == 0;
        for (i, &(at, size)) in rects.iter().enumerate() {
            if i == self.rings.len() {
                let ring = anim.compositor.CreateSpriteVisual()?;
                ring.SetBrush(&self.mask)?;
                ring.SetOpacity(0.0)?;
                self.container.Children()?.InsertAtTop(&ring)?;
                self.rings.push(ring);
            }
            let ring = &self.rings[i];
            // One that was in use glides to its place, one that wasn't
            // fades in there; either way the others of this link follow.
            let from_view = glide && ring.Opacity()? > 0.0;
            if from_view {
                anim.vector3(ring, "Offset", at, FOCUS_MS, 0, &anim.decelerate)?;
                anim.vector2(ring, "Size", size, FOCUS_MS)?;
            } else {
                Animator::stop(ring, &["Offset", "Size"]);
                ring.SetOffset(at)?;
                ring.SetSize(size)?;
            }
            if reveal > 0 {
                Animator::stop(ring, &["Opacity"]);
                ring.SetOpacity(0.0)?;
            }
            anim.scalar(
                ring,
                "Opacity",
                1.0,
                FOCUS_FADE_MS,
                reveal,
                &anim.decelerate,
            )?;
        }
        for ring in self.rings.iter().skip(rects.len()) {
            fade_out(anim, ring, reveal > 0)?;
        }
        self.shown = true;
        Ok(())
    }
}

fn fade_out(anim: &Animator, ring: &SpriteVisual, at_once: bool) -> Result<()> {
    if at_once {
        Animator::stop(ring, &["Opacity"]);
        ring.SetOpacity(0.0)
    } else {
        anim.scalar(ring, "Opacity", 0.0, FOCUS_FADE_MS, 0, &anim.decelerate)
    }
}

/// Lightens the button swapping a translation's languages under the
/// pointer, and says what it does.
struct SwapHighlight {
    visual: ShapeVisual,
    geometry: CompositionRoundedRectangleGeometry,
    brush: CompositionColorBrush,
    hovered: bool,
    /// Owned by the card's window.
    tip: Option<HWND>,
    /// The area of the window it shows over, last set.
    tip_rect: RECT,
}

impl SwapHighlight {
    fn new(c: &Compositor, parent: &SpriteVisual, hwnd: HWND) -> Result<SwapHighlight> {
        let visual = c.CreateShapeVisual()?;
        visual.SetRelativeSizeAdjustment(v2(1.0, 1.0))?;
        visual.SetOpacity(0.0)?;
        let geometry = c.CreateRoundedRectangleGeometry()?;
        let shape = c.CreateSpriteShapeWithGeometry(&geometry)?;
        let brush = c.CreateColorBrush()?;
        shape.SetFillBrush(&brush)?;
        visual.Shapes()?.Append(&shape)?;
        parent.Children()?.InsertAtTop(&visual)?;
        Ok(SwapHighlight {
            visual,
            geometry,
            brush,
            hovered: false,
            tip: create_tooltip(hwnd, tr().swap_tip, SWAP_TOOL),
            tip_rect: RECT::default(),
        })
    }

    /// Fits the highlight to `link`, lit while the pointer's `over` it,
    /// and moves the tooltip to `rect`.
    fn sync(
        &mut self,
        anim: &Animator,
        owner: HWND,
        link: Option<&Link>,
        rect: RECT,
        over: bool,
        s: f32,
    ) -> Result<()> {
        if let Some(l) = link {
            let geometry = &self.geometry;
            geometry.SetOffset(v2((l.x * s).round(), (l.y * s).round()))?;
            geometry.SetSize(v2((l.width * s).round(), (l.height * s).round()))?;
            geometry.SetCornerRadius(v2(BACK_RADIUS * s, BACK_RADIUS * s))?;
        }
        if over != self.hovered {
            self.hovered = over;
            let opacity = if over { 1.0 } else { 0.0 };
            anim.scalar(&self.visual, "Opacity", opacity, 100, 0, &anim.decelerate)?;
        }
        if rect != self.tip_rect {
            self.tip_rect = rect;
            if let Some(tip) = self.tip {
                set_tooltip_rect(tip, owner, SWAP_TOOL, rect);
            }
        }
        Ok(())
    }

    fn reset(&mut self) -> Result<()> {
        self.hovered = false;
        Animator::stop(&self.visual, &["Opacity"]);
        self.visual.SetOpacity(0.0)
    }
}
