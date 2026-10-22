//! Scrolling the card's contents, and bringing a control into view.

use super::*;

impl Settings {
    // -----------------------------------------------------------------------
    // Scrolling

    pub(super) fn max_scroll(&self) -> f32 {
        let Some(view) = &self.view else {
            return 0.0;
        };
        ui::max_scroll(
            view.height(self.open),
            self.card.1 - view.header_h,
            self.scale,
        )
    }

    /// The scroll position on screen right now, following the running
    /// animation.
    pub(super) fn visible_scroll(&self) -> f32 {
        self.scroll_anim
            .filter(Glide::running)
            .map_or(self.scroll, |glide| glide.at())
    }

    /// Scrolls to `to`.
    pub(super) fn scroll_to(&mut self, to: f32) -> Result<()> {
        self.glide_to(to.clamp(0.0, self.max_scroll()), Push::None, SCROLL_MS)
    }

    /// Scrolls to `to`, pushed past it, over `ms`, starting from where the
    /// content is on screen now, not from the last target, which would lag
    /// behind the wheel.
    pub(super) fn glide_to(&mut self, to: f32, push: Push, ms: u64) -> Result<()> {
        let to = to.round();
        if to == self.scroll && push == Push::None {
            return Ok(());
        }
        let (max, most) = (self.max_scroll(), ui::stretch_limit(self.scale));
        let glide = Glide::new(self.scroll_anim, self.scroll, to, push, max, most, ms);
        self.scroll = to;
        self.scroll_anim = Some(glide);
        self.show_tip(None)?;
        if let Some(view) = &self.view {
            glide.play(&self.anim, &view.scroller, |scroll| v3(0.0, -scroll))?;
        }
        // Content that's there at once has its thumb there at once too.
        self.show_scroll(glide.running())
    }

    /// Scrolls just enough to show `rect`, in the scroller.
    pub(super) fn scroll_into_view(&mut self, rect: Rect) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let margin = ui::FOCUS_MARGIN * self.scale;
        let view_h = self.card.1 - view.header_h;
        let (top, bottom) = (rect.y - margin, rect.y + rect.h + margin);
        if top < self.scroll {
            self.scroll_to(top)
        } else if bottom > self.scroll + view_h {
            self.scroll_to(bottom - view_h)
        } else {
            Ok(())
        }
    }

    /// The fades at the ends of what scrolls, and the scrollbar thumb.
    pub(super) fn show_scroll(&self, animate: bool) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let (anim, s) = (&self.anim, self.scale);
        let max = self.max_scroll();
        view.fade.show(anim, self.scroll, max, s, animate)?;
        if max <= 0.0 {
            Animator::stop(&view.thumb, &["Opacity"]);
            return view.thumb.SetOpacity(0.0);
        }
        let viewport = self.card.1 - view.header_h;
        let content_h = view.height(self.open);
        let (top, thumb) = ui::thumb_span(viewport, content_h, self.scroll, max, s);
        let x = ui::size_thumb(&view.thumb, &view.thumb_clip, self.card.0, thumb, s)?;
        let to = v3(x, view.header_h + top);
        let started = self.scroll_anim.filter(Glide::running);
        if let (true, Some(glide)) = (animate, started) {
            glide.play(anim, &view.thumb, |scroll| {
                let (top, _) = ui::thumb_span(viewport, content_h, scroll, max, s);
                v3(x, view.header_h + top)
            })?;
        } else if animate {
            anim.vector3(&view.thumb, "Offset", to, SCROLL_MS, 0, &anim.decelerate)?;
        } else {
            Animator::stop(&view.thumb, &["Offset"]);
            view.thumb.SetOffset(to)?;
        }
        anim.scalar(&view.thumb, "Opacity", 1.0, 200, 0, &anim.decelerate)
    }
}
