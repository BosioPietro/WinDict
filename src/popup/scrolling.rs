//! Scrolling the card's body, and the chrome that shows it.

use super::*;

impl Popup {
    /// Scrolls the body by a mouse wheel delta.
    pub fn scroll_by(&mut self, wheel_delta: i32) -> Result<()> {
        let max = self.max_scroll();
        if max <= 0.0 {
            return Ok(());
        }
        let mut step = -(wheel_delta as f32 / 120.0) * 56.0 * self.scale;
        // A touchpad's scroll is smoothed already, so it follows at once.
        let precise = self.wheel_bursts.precise(wheel_delta);
        if precise {
            step = self.wheel_bursts.precise_step(step, self.scale);
        }
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
        glide.play(&self.anim, &self.content.visual, |scroll| v3(x, -scroll))?;
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
        let chrome = &self.scroll_chrome;
        self.content.fade.show(anim, self.scroll, max, s, animate)?;

        if max <= 0.0 {
            Animator::stop(&chrome.thumb, &["Opacity"]);
            chrome.thumb.SetOpacity(0.0)?;
            return Ok(());
        }
        let viewport = self.card_h - self.header_h;
        let thumb_y =
            |scroll: f32| self.header_h + ui::thumb_span(viewport, self.body_h, scroll, max, s).0;
        let (_, thumb) = ui::thumb_span(viewport, self.body_h, self.scroll, max, s);
        let y = thumb_y(self.scroll);
        let x = ui::size_thumb(&chrome.thumb, &chrome.thumb_clip, CARD_W * s, thumb, s)?;
        let glide = self.scroll_anim.filter(|glide| animate && glide.running());
        if let Some(glide) = glide {
            // Track the content exactly, from where the thumb is now.
            glide.play(anim, &chrome.thumb, |scroll| v3(x, thumb_y(scroll)))?;
        } else {
            Animator::stop(&chrome.thumb, &["Offset"]);
            chrome.thumb.SetOffset(v3(x, y))?;
        }
        anim.scalar(&chrome.thumb, "Opacity", 1.0, 200, 150, &anim.decelerate)
    }
}

/// The scrollbar thumb, shown only when the content scrolls.
pub(super) struct ScrollChrome {
    thumb: SpriteVisual,
    thumb_clip: CompositionRoundedRectangleGeometry,
    thumb_brush: CompositionColorBrush,
}

impl ScrollChrome {
    pub(super) fn new(c: &Compositor, card: &ContainerVisual) -> Result<ScrollChrome> {
        let thumb = c.CreateSpriteVisual()?;
        let thumb_clip = c.CreateRoundedRectangleGeometry()?;
        thumb.SetClip(&c.CreateGeometricClipWithGeometry(&thumb_clip)?)?;
        let thumb_brush = c.CreateColorBrush()?;
        thumb.SetBrush(&thumb_brush)?;
        thumb.SetOpacity(0.0)?;
        card.Children()?.InsertAtTop(&thumb)?;

        Ok(ScrollChrome {
            thumb,
            thumb_clip,
            thumb_brush,
        })
    }

    pub(super) fn apply_theme(&self, t: &Theme) -> Result<()> {
        self.thumb_brush.SetColor(t.thumb())
    }
}
