//! Scrolling the card's body, and the chrome that shows it.

use super::*;

impl Popup {
    /// Whether the body is taller than the card, and so scrolls.
    pub fn scrolls(&self) -> bool {
        self.max_scroll() > 0.0
    }

    /// Scrolls the body by a mouse wheel delta.
    pub fn scroll_by(&mut self, wheel_delta: i32) -> Result<()> {
        if !self.scrolls() {
            return Ok(());
        }
        let mut step = -(wheel_delta as f32 / 120.0) * 56.0 * self.scale;
        // A touchpad's scroll is smoothed already, so it follows at once.
        let precise = self.wheel_bursts.precise(wheel_delta);
        if precise {
            step = self.wheel_bursts.precise_step(step, self.scale);
        }
        self.scroll_step(step, precise)
    }

    /// Scrolls the list of languages by a mouse wheel delta, if the pointer
    /// is on it, however far it can go. Returns whether it was.
    pub fn scroll_list(&mut self, wheel_delta: i32) -> Result<bool> {
        let at = self.pointer.and_then(|(x, y)| self.body_point(x, y));
        let list = self.content.list();
        if !at.is_some_and(|(x, y)| list.contains(x, y)) {
            return Ok(false);
        }
        list.wheel(&self.anim, wheel_delta)?;
        Ok(true)
    }

    /// The row of the list of languages under a point in window pixels.
    pub fn list_row_at(&mut self, x: i32, y: i32) -> Option<usize> {
        let (x, y) = self.body_point(x, y)?;
        self.content.list().row_at(x, y)
    }

    /// Puts the fill behind `row` in the list of languages, fading from
    /// where it was.
    pub fn highlight_list(&mut self, row: Option<usize>) -> Result<()> {
        self.content.list().set_highlight(&self.anim, row)
    }

    /// Up or Down: scrolls the body as far as a notch of the wheel does.
    pub fn scroll_line(&mut self, down: bool) -> Result<()> {
        if !self.scrolls() {
            return Ok(());
        }
        let step = 56.0 * self.scale;
        self.scroll_step(if down { step } else { -step }, false)
    }

    /// Scrolls the body by `step` pixels, at once if it's `precise`.
    fn scroll_step(&mut self, step: f32, precise: bool) -> Result<()> {
        let max = self.max_scroll();
        if !self.wheel_bursts.take(self.scroll + step, max) {
            return Ok(());
        }
        // Past the end, it gives a little, and springs back.
        let visible = self.visible_scroll();
        let (to, push) = ui::stretch(visible, self.scroll, step, max, self.scale);
        let ms = if precise {
            self.scroll_anim.map_or(0, |glide| glide.easing_left())
        } else {
            SCROLL_MS
        };
        self.scroll_to(to, push, ms)
    }

    /// Scrolls the body smoothly to `to`, in pixels from the top, pushed
    /// past it, over `ms`.
    pub(super) fn scroll_to(&mut self, to: f32, push: Push, ms: u64) -> Result<()> {
        // Start from where the content is on screen right now, not from the
        // previous target: restarting from a stale value is what made
        // scrolling feel like it lagged behind the wheel.
        let (at, max) = (self.scroll, self.max_scroll());
        self.scroll = to.round();
        let most = ui::stretch_limit(self.scale);
        let glide = Glide::new(self.scroll_anim, at, self.scroll, push, max, most, ms);
        self.scroll_anim = Some(glide);
        let x = PAD_X * self.scale;
        self.content.play_scroll(&self.anim, &glide, x)?;
        self.update_scroll_chrome(true)?;
        self.sync_swap()
    }

    /// The scroll position currently on screen, following the running animation.
    pub(super) fn visible_scroll(&self) -> f32 {
        self.scroll_anim
            .filter(Glide::running)
            .map_or(self.scroll, |glide| glide.at())
    }

    pub(super) fn max_scroll(&self) -> f32 {
        ui::max_scroll(self.body_h, self.card_h - self.header_h, self.scale)
    }

    pub(super) fn update_scroll_chrome(&self, animate: bool) -> Result<()> {
        let s = self.scale;
        let max = self.max_scroll();
        let anim = &self.anim;
        let thumb = self.scroll_chrome.bar.visual();
        self.content
            .show_fades(anim, self.scroll, max, s, animate)?;

        if max <= 0.0 {
            Animator::stop(thumb, &["Opacity"]);
            thumb.SetOpacity(0.0)?;
            return Ok(());
        }
        let viewport = self.card_h - self.header_h;
        let thumb_y =
            |scroll: f32| self.header_h + ui::thumb_span(viewport, self.body_h, scroll, max, s).0;
        let (_, h) = ui::thumb_span(viewport, self.body_h, self.scroll, max, s);
        let y = thumb_y(self.scroll);
        let x = self.scroll_chrome.bar.size(CARD_W * s, h, s)?;
        let glide = self.scroll_anim.filter(|glide| animate && glide.running());
        if let Some(glide) = glide {
            // Track the content exactly, from where the thumb is now.
            glide.play(anim, thumb, |scroll| v3(x, thumb_y(scroll)))?;
        } else {
            Animator::stop(thumb, &["Offset"]);
            thumb.SetOffset(v3(x, y))?;
        }
        anim.scalar(thumb, "Opacity", 1.0, 200, 150, &anim.decelerate)
    }

    fn track(&self) -> Track {
        Track {
            viewport: self.card_h - self.header_h,
            content_h: self.body_h,
            max: self.max_scroll(),
            s: self.scale,
            ends: ui::CARD_ENDS,
        }
    }

    /// Whether a point in window pixels is on the scrollbar, and how far
    /// down the body it is.
    fn bar_point(&self, x: i32, y: i32) -> (bool, f32) {
        let (x, y) = self.card_point(x, y);
        let y = y - self.header_h;
        (Scrollbar::over(x, y, CARD_W * self.scale, self.track()), y)
    }

    /// The left button went down at a point in window pixels: on the
    /// scrollbar, it grabs the thumb, or pages towards the point. Returns
    /// whether it was on it.
    pub fn press(&mut self, x: i32, y: i32) -> Result<bool> {
        if let Some((bx, by)) = self.body_point(x, y) {
            if self.content.list().press(&self.anim, bx, by)? {
                return Ok(true);
            }
        }
        let (over, y) = self.bar_point(x, y);
        if !over {
            return Ok(false);
        }
        let (scroll, track) = (self.visible_scroll(), self.track());
        let page = self.scroll_chrome.bar.press(y, scroll, track);
        self.scroll_chrome.bar.hover(&self.anim, true)?;
        if let Some(to) = page {
            self.scroll_to(to, Push::None, SCROLL_MS)?;
        }
        Ok(true)
    }

    /// The left button came up. Returns whether it had gone down on the
    /// scrollbar.
    pub fn release(&mut self) -> Result<bool> {
        let held = self.content.list().release(&self.anim)? | self.scroll_chrome.bar.release();
        if held {
            self.hover(self.pointer)?;
        }
        Ok(held)
    }

    /// The pointer at a point in window pixels, or gone: widens the thumb
    /// over the scrollbar, and drags it while it's held. Returns whether
    /// the pointer's on the scrollbar, or dragging it.
    pub(super) fn hover_bar(&mut self, point: Option<(i32, i32)>) -> Result<bool> {
        let (over, y) = point.map_or((false, 0.0), |(x, y)| self.bar_point(x, y));
        self.scroll_chrome.bar.hover(&self.anim, over)?;
        let to = point.and_then(|_| self.scroll_chrome.bar.drag(y, self.track()));
        if let Some(to) = to.filter(|&to| to != self.scroll) {
            self.scroll_to(to, Push::None, 0)?;
        }
        Ok(over || self.scroll_chrome.bar.dragging())
    }
}

/// The scrollbar, shown only when the content scrolls.
pub(super) struct ScrollChrome {
    bar: Scrollbar,
    thumb_brush: CompositionColorBrush,
}

impl ScrollChrome {
    pub(super) fn new(c: &Compositor, card: &ContainerVisual) -> Result<ScrollChrome> {
        let thumb_brush = c.CreateColorBrush()?;
        let bar = Scrollbar::new(c, card, &thumb_brush.cast()?)?;
        Ok(ScrollChrome { bar, thumb_brush })
    }

    pub(super) fn apply_theme(&self, t: &Theme) -> Result<()> {
        self.thumb_brush.SetColor(t.thumb())
    }
}
