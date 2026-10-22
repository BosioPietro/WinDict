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
            view.height(&self.open),
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
        let thumb = view.bar.visual();
        view.fade.show(anim, self.scroll, max, s, animate)?;
        if max <= 0.0 {
            Animator::stop(thumb, &["Opacity"]);
            return thumb.SetOpacity(0.0);
        }
        let viewport = self.card.1 - view.header_h;
        let content_h = view.height(&self.open);
        let (top, h) = ui::thumb_span(viewport, content_h, self.scroll, max, s);
        let x = view.bar.size(self.card.0, h, s)?;
        let to = v3(x, view.header_h + top);
        let started = self.scroll_anim.filter(Glide::running);
        if let (true, Some(glide)) = (animate, started) {
            glide.play(anim, thumb, |scroll| {
                let (top, _) = ui::thumb_span(viewport, content_h, scroll, max, s);
                v3(x, view.header_h + top)
            })?;
        } else if animate {
            anim.vector3(thumb, "Offset", to, SCROLL_MS, 0, &anim.decelerate)?;
        } else {
            Animator::stop(thumb, &["Offset"]);
            thumb.SetOffset(to)?;
        }
        anim.scalar(thumb, "Opacity", 1.0, 200, 0, &anim.decelerate)
    }

    // -----------------------------------------------------------------------
    // The scrollbar

    fn track(&self) -> Option<Track> {
        let view = self.view.as_ref()?;
        Some(Track {
            viewport: self.card.1 - view.header_h,
            content_h: view.height(&self.open),
            max: self.max_scroll(),
            s: self.scale,
        })
    }

    /// Whether a point in window pixels is on the scrollbar, and how far
    /// down what scrolls it is.
    fn bar_point(&self, x: i32, y: i32) -> Option<(bool, f32)> {
        let (view, track) = (self.view.as_ref()?, self.track()?);
        let (x, y) = self.card_point(x, y);
        let y = y - view.header_h;
        Some((Scrollbar::over(x, y, self.card.0, track), y))
    }

    /// The left button went down at a point in window pixels: on the
    /// scrollbar, it grabs the thumb, or pages towards the point. Returns
    /// whether it was on it.
    pub(super) fn press_bar(&mut self, x: i32, y: i32) -> Result<bool> {
        let (Some((true, y)), Some(track)) = (self.bar_point(x, y), self.track()) else {
            return Ok(false);
        };
        let scroll = self.visible_scroll().clamp(0.0, track.max);
        let Some(view) = self.view.as_mut() else {
            return Ok(false);
        };
        let page = view.bar.press(y, scroll, track);
        view.bar.hover(&self.anim, true)?;
        if let Some(to) = page {
            self.scroll_to(to)?;
        }
        Ok(true)
    }

    /// The pointer at a point in window pixels, or gone: widens the thumb
    /// over the scrollbar, and drags it while it's held. Returns whether
    /// the pointer's on the scrollbar, or dragging it.
    pub(super) fn hover_bar(&mut self, point: Option<(i32, i32)>) -> Result<bool> {
        let at = point.and_then(|(x, y)| self.bar_point(x, y));
        let (over, y) = at.unwrap_or((false, 0.0));
        let track = self.track();
        let Some(view) = self.view.as_mut() else {
            return Ok(false);
        };
        view.bar.hover(&self.anim, over)?;
        let dragging = view.bar.dragging();
        let to = at.zip(track).and_then(|(_, track)| view.bar.drag(y, track));
        if let Some(to) = to {
            self.glide_to(to, Push::None, 0)?;
        }
        Ok(over || dragging)
    }

    /// The window lost the mouse, or stopped being the active one: the
    /// thumb lets go.
    pub fn lose_capture(&mut self) -> Result<()> {
        let Some(view) = self.view.as_mut() else {
            return Ok(());
        };
        if view.bar.release() {
            view.bar.hover(&self.anim, false)?;
        }
        Ok(())
    }
}
