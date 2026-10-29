//! The ring that Tab moves over the card's links and buttons, and what follows from it.

use super::*;

impl Popup {
    /// The link under a point in window coordinates, if any.
    pub fn link_at(&self, x: i32, y: i32) -> Option<&Target> {
        let (x, y) = self.body_point(x, y)?;
        self.content.link_at(x, y, self.scale)
    }

    /// A point in window coordinates in the body, in pixels from its top
    /// left, if it's on the part of it in view.
    pub(super) fn body_point(&self, x: i32, y: i32) -> Option<(f32, f32)> {
        let s = self.scale;
        let (x, y) = (x as f32 - MARGIN * s, y as f32 - self.frame_y());
        if y < self.header_h || y >= self.card_h || x < 0.0 || x >= CARD_W * s {
            return None;
        }
        Some((x - PAD_X * s, y - self.header_h + self.scroller.at))
    }

    /// Whether the card has links, or buttons, to Tab through.
    pub fn has_links(&self) -> bool {
        CardButton::ALL.iter().any(|&b| self.button(b).shown()) || !self.content.links().is_empty()
    }

    /// The link, or button, Tab moved to, if any.
    pub fn focused_link(&self) -> Option<&Target> {
        match self.current_stop()? {
            Stop::Button(button) => Some(button.target()),
            Stop::Link(i) => Some(&self.content.links()[i].target),
        }
    }

    /// Where Tab is, if anywhere.
    pub(super) fn current_stop(&self) -> Option<Stop> {
        let button = CardButton::ALL
            .into_iter()
            .find(|&b| self.button(b).focused());
        match button {
            Some(button) => Some(Stop::Button(button)),
            None => self.content.focused().map(Stop::Link),
        }
    }

    /// Moves the keyboard focus to the next stop, or the previous one,
    /// going round from the last to the first, and scrolls it into view.
    /// The buttons by the word come first, as they do reading the card, and
    /// the back button last, so Tab doesn't start on it.
    pub fn step_link(&mut self, forward: bool) -> Result<()> {
        let shown = |b: &CardButton| self.button(*b).shown();
        let (by_word, back): (Vec<_>, Vec<_>) = CardButton::ALL
            .into_iter()
            .filter(shown)
            .partition(|&b| b != CardButton::Back);
        let mut stops: Vec<_> = by_word.into_iter().map(Stop::Button).collect();
        stops.extend(self.content.stops().into_iter().map(Stop::Link));
        stops.extend(back.into_iter().map(Stop::Button));
        if stops.is_empty() {
            return Ok(());
        }
        let n = stops.len();
        let current = self.current_stop();
        let at = current.and_then(|f| stops.iter().position(|&s| s == f));
        let next = match (at, forward) {
            (None, true) => 0,
            // Backwards from nowhere is the last of the card, not its back button.
            (None, false) => stops
                .iter()
                .rposition(|&s| s != Stop::Button(CardButton::Back))
                .unwrap_or(n - 1),
            (Some(i), true) => (i + 1) % n,
            (Some(i), false) => (i + n - 1) % n,
        };
        self.go_to(current, stops[next])
    }

    /// Rings `to`, which the ring glides to from `from`. Between a button
    /// and a link it glides over the card, as they're not in the same space.
    pub(super) fn go_to(&mut self, from: Option<Stop>, to: Stop) -> Result<()> {
        let across = from.is_some_and(|f| f != to && (f.is_button() || to.is_button()));
        let from_scroll = self.visible_scroll();
        // What the ring lands on shows once the one gliding over has arrived.
        let reveal = if across { FOCUS_MS } else { 0 };
        match to {
            Stop::Button(button) => self.focus_button(button, reveal)?,
            Stop::Link(start) => {
                self.focus_link(Some(start), reveal)?;
                self.scroll_into_view(start)?;
            }
        }
        match from {
            Some(from) if across => self.glide_ring(from, from_scroll, to),
            _ => Ok(()),
        }
    }

    /// Where the ring of `stop` is in the card: its place, size and corner
    /// radius, with the body scrolled to `scroll`.
    pub(super) fn ring_rect(&self, stop: Stop, scroll: f32) -> Option<(Vector3, Vector2, f32)> {
        let s = self.scale;
        let stroke = (ui::FOCUS_STROKE * s).round();
        let grow = stroke / 2.0;
        match stop {
            Stop::Button(button) => {
                let button = self.button(button);
                let side = BACK_SIZE * s;
                Some((
                    button.card_offset(s),
                    v2(side, side),
                    BACK_RADIUS * s + grow,
                ))
            }
            Stop::Link(start) => {
                let l = self.content.rects(start).next()?;
                let (x, y) = ((l.x * s).round(), (l.y * s).round());
                let (w, h) = ((l.width * s).round(), (l.height * s).round());
                Some((
                    v3(PAD_X * s + x - grow, self.header_h - scroll + y - grow),
                    v2(w + stroke, h + stroke),
                    FOCUS_RADIUS * s + grow,
                ))
            }
        }
    }

    /// Glides a ring over the card from `from` to `to`, then takes it away.
    pub(super) fn glide_ring(&mut self, from: Stop, from_scroll: f32, to: Stop) -> Result<()> {
        let (Some((from_at, from_size, _)), Some((to_at, to_size, radius))) = (
            self.ring_rect(from, from_scroll),
            self.ring_rect(to, self.scroller.at),
        ) else {
            return Ok(());
        };
        let stroke = (ui::FOCUS_STROKE * self.scale).round();
        let side = (2.0 * radius).ceil() + 1.0;
        let mask = self
            .renderer
            .draw_rounded(side, side, radius, Some(stroke))?;
        self.glide_source.SetSurface(&mask)?;
        self.glide_grid.SetInsets(radius)?;
        let (glide, anim) = (&self.glide, &self.anim);
        Animator::stop(glide, &["Offset", "Size", "Opacity"]);
        glide.SetOffset(from_at)?;
        glide.SetSize(from_size)?;
        glide.SetOpacity(1.0)?;
        anim.vector3(glide, "Offset", to_at, FOCUS_MS, 0, &anim.decelerate)?;
        anim.vector2(glide, "Size", to_size, FOCUS_MS)?;
        anim.scalar(glide, "Opacity", 0.0, 1, FOCUS_MS, &anim.decelerate)
    }

    /// Moves the keyboard focus to the link going to `target`, if the card
    /// has one.
    pub fn focus_target(&mut self, target: &Target) -> Result<()> {
        let links = self.content.links();
        if let Some(start) = links.iter().position(|l| &l.target == target) {
            self.focus_link(Some(start), 0)?;
            self.scroll_into_view(start)?;
        }
        Ok(())
    }

    /// Takes the ring off whatever Tab moved to.
    pub fn unfocus(&mut self) -> Result<()> {
        self.focus_link(None, 0)
    }

    /// Rings `button`, `reveal` ms from now.
    pub(super) fn focus_button(&mut self, button: CardButton, reveal: u64) -> Result<()> {
        self.focus_link(None, reveal)?;
        let (button, anim) = self.button_mut(button);
        button.reveal(anim, reveal)
    }

    /// Rings the link starting at `start` in the body's links, or none,
    /// `reveal` ms from now. With a delay, the rings that are showing go at
    /// once, as another is gliding over the card to take their place.
    pub(super) fn focus_link(&mut self, start: Option<usize>, reveal: u64) -> Result<()> {
        self.speak.set_focused(false)?;
        self.translate.set_focused(false)?;
        self.back.set_focused(false)?;
        self.pin.set_focused(false)?;
        self.scan.set_focused(false)?;
        self.content
            .focus(&self.anim, &self.renderer, start, reveal, self.scale)
    }

    /// Scrolls the body just enough to show the link starting at `start`.
    pub(super) fn scroll_into_view(&mut self, start: usize) -> Result<()> {
        let max = self.max_scroll();
        if max <= 0.0 {
            return Ok(());
        }
        let (mut top, mut bottom) = (f32::MAX, f32::MIN);
        for l in self.content.rects(start) {
            top = top.min(l.y);
            bottom = bottom.max(l.y + l.height);
        }
        let s = self.scale;
        let (top, bottom) = (
            (top - ui::FOCUS_MARGIN) * s,
            (bottom + ui::FOCUS_MARGIN) * s,
        );
        let view = self.card_h - self.header_h;
        let to = if top < self.scroller.at {
            top
        } else if bottom > self.scroller.at + view {
            bottom - view
        } else {
            return Ok(());
        };
        self.scroll_to(to.clamp(0.0, max), Push::None, SCROLL_MS)
    }
}
