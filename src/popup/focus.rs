//! The ring that Tab moves over the card's links and buttons, and what follows from it.

use super::*;

impl Popup {
    /// The link under a point in window coordinates, if any.
    pub fn link_at(&self, x: i32, y: i32) -> Option<&Target> {
        let s = self.scale;
        let (x, y) = (x as f32 - MARGIN * s, y as f32 - self.frame_y());
        // Only the visible part of the body is clickable.
        if y < self.header_h || y >= self.card_h || x < 0.0 || x >= CARD_W * s {
            return None;
        }
        let (bx, by) = ((x - PAD_X * s) / s, (y - self.header_h + self.scroll) / s);
        self.content.link_at(bx, by)
    }

    /// Whether the card has links, or buttons, to Tab through.
    pub fn has_links(&self) -> bool {
        self.speak.shown()
            || self.translate.shown()
            || self.back.shown()
            || !self.content.links().is_empty()
    }

    /// The link, or button, Tab moved to, if any.
    pub fn focused_link(&self) -> Option<&Target> {
        static SPEAK: Target = Target::Speak;
        static TRANSLATE: Target = Target::Translate;
        static BACK: Target = Target::Back;
        if self.speak.focused() {
            return Some(&SPEAK);
        }
        if self.translate.focused() {
            return Some(&TRANSLATE);
        }
        if self.back.focused() {
            return Some(&BACK);
        }
        let links = self.content.links();
        self.content.focused().map(|i| &links[i].target)
    }

    /// Where Tab is, if anywhere.
    pub(super) fn current_stop(&self) -> Option<Stop> {
        if self.speak.focused() {
            Some(Stop::Speak)
        } else if self.translate.focused() {
            Some(Stop::Translate)
        } else if self.back.focused() {
            Some(Stop::Back)
        } else {
            self.content.focused().map(Stop::Link)
        }
    }

    /// Moves the keyboard focus to the next stop, or the previous one,
    /// going round from the last to the first, and scrolls it into view.
    /// The buttons by the word come first, as they do reading the card, and
    /// the back button last, so Tab doesn't start on it.
    pub fn step_link(&mut self, forward: bool) -> Result<()> {
        let mut stops = Vec::new();
        if self.speak.shown() {
            stops.push(Stop::Speak);
        }
        if self.translate.shown() {
            stops.push(Stop::Translate);
        }
        stops.extend(self.content.stops().into_iter().map(Stop::Link));
        if self.back.shown() {
            stops.push(Stop::Back);
        }
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
                .rposition(|&s| s != Stop::Back)
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
            Stop::Speak => self.focus_speak(reveal)?,
            Stop::Translate => self.focus_translate(reveal)?,
            Stop::Back => self.focus_back(reveal)?,
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
            Stop::Speak | Stop::Translate | Stop::Back => {
                let button = match stop {
                    Stop::Speak => &self.speak,
                    Stop::Back => &self.back,
                    _ => &self.translate,
                };
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
            self.ring_rect(to, self.scroll),
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

    /// Rings the button saying the word, `reveal` ms from now.
    pub(super) fn focus_speak(&mut self, reveal: u64) -> Result<()> {
        self.focus_link(None, reveal)?;
        self.speak.reveal(&self.anim, reveal)
    }

    /// Rings the translate button, `reveal` ms from now.
    pub(super) fn focus_translate(&mut self, reveal: u64) -> Result<()> {
        self.focus_link(None, reveal)?;
        self.translate.reveal(&self.anim, reveal)
    }

    /// Rings the back button, `reveal` ms from now.
    pub(super) fn focus_back(&mut self, reveal: u64) -> Result<()> {
        self.focus_link(None, reveal)?;
        self.back.reveal(&self.anim, reveal)
    }

    /// Rings the link starting at `start` in the body's links, or none,
    /// `reveal` ms from now. With a delay, the rings that are showing go at
    /// once, as another is gliding over the card to take their place.
    pub(super) fn focus_link(&mut self, start: Option<usize>, reveal: u64) -> Result<()> {
        self.speak.set_focused(false)?;
        self.translate.set_focused(false)?;
        self.back.set_focused(false)?;
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
        let to = if top < self.scroll {
            top
        } else if bottom > self.scroll + view {
            bottom - view
        } else {
            return Ok(());
        };
        self.scroll_to(to.clamp(0.0, max), Push::None, SCROLL_MS)
    }
}
