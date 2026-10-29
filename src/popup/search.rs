//! The card's search box: opening it and its suggestions.

use super::*;
use crate::config::Shortcut;
use crate::ocr;
use crate::render::{Picker, Side, LANGUAGES_TOP, QUERY_SIZE};

impl Popup {
    /// Shows the search box under `title`, for what the `purpose` shortcut
    /// does. An open card turns into it in place; otherwise a new one
    /// appears next to `anchor`.
    pub fn show_search(
        &mut self,
        anchor: RECT,
        theme: Theme,
        title: &str,
        purpose: Shortcut,
    ) -> Result<()> {
        if self.visible {
            return self.enter_search(title, purpose, true, false);
        }
        // Its top stays put, so the field doesn't move as suggestions come and go.
        self.prepare(anchor, theme, true)?;
        self.enter_search(title, purpose, false, false)?;
        self.appear()
    }

    /// Turns the card back into the search box what it shows was typed in,
    /// under `title`: the heading sinks back into the field as what's
    /// typed, and the languages under it glide back under the field.
    pub fn return_to_search(&mut self, title: &str, purpose: Shortcut) -> Result<()> {
        self.enter_search(title, purpose, true, true)
    }

    /// Shows `text` in the search box, the caret `at` UTF-16 units in, and
    /// what's `selected`, from where to where, likewise.
    pub fn set_query(&mut self, text: &str, at: u32, selected: Option<(u32, u32)>) -> Result<()> {
        // Typing, or moving the caret, is back in the field.
        self.scan.set_focused(false)?;
        self.search
            .set_query(&self.renderer, &self.anim, &self.theme, text, at, selected)
    }

    /// Changes the search box's heading.
    pub fn set_search_title(&mut self, title: &str) -> Result<()> {
        self.search
            .set_title(&self.renderer, &self.anim, &self.theme, title)
    }

    /// Shakes the search field: what's typed can't be looked up.
    pub fn shake_search(&self) -> Result<()> {
        self.search.shake(&self.anim)
    }

    /// Whether a point in window pixels is on the search field.
    pub fn search_at(&self, x: i32, y: i32) -> bool {
        let (x, y) = self.card_point(x, y);
        self.search.field_at(x, y)
    }

    /// Shows `items` (each a word, what it's a form of, how many of its
    /// first characters to bold as typed, and the code of its language when
    /// there are several) under the search field, or else `note`, resizing
    /// the card to fit.
    pub fn set_suggestions(
        &mut self,
        items: &[(&str, Option<&str>, usize, Option<&str>)],
        note: Option<&str>,
    ) -> Result<()> {
        let (renderer, anim, theme) = (&self.renderer, &self.anim, &self.theme);
        let h = self
            .search
            .set_suggestions(renderer, anim, theme, items, note)?;
        self.header_h = h;
        if (h - self.card_h).abs() > 0.5 {
            self.resize(h, true)?;
        }
        Ok(())
    }

    /// Shows the languages what's typed is to be translated between, in
    /// boxes under the field like the translation card's, with `picker`
    /// open under the box of the side it's paired with. The card grows or
    /// shrinks to fit them.
    pub fn set_search_languages(
        &mut self,
        from: &str,
        to: &str,
        picker: Option<(Side, &Picker)>,
    ) -> Result<()> {
        let s = self.scale;
        let content_w = CARD_W - 2.0 * PAD_X;
        let layout = self
            .renderer
            .layout_languages(from, to, picker, content_w)?;
        let surface = self.renderer.draw(&layout, s, &self.theme)?;
        let first = self.search.room() == 0.0;
        let glide = std::mem::take(&mut self.body_glides);
        let size = v2(content_w * s, layout.height * s);
        let c = &self.anim.compositor;
        self.content.show(c, &surface, size, layout.caret, s)?;
        let list = layout.list.as_deref();
        self.content
            .show_list(&self.anim, &self.renderer, &self.theme, list, s)?;
        self.content.set_links(layout.links);
        self.focus_link(None, 0)?;
        // They're the body, under the box's field, and never scroll.
        let top = self.search.below_field();
        (self.header_h, self.body_h) = (top, size.Y);
        self.scroller.stop_at(0.0);
        self.content.keep_place(0.0, s)?;
        if glide {
            // From where they were on the translation, a little lower.
            self.content.shift_host(LANGUAGES_TOP * s)?;
        }
        self.content.move_host(&self.anim, top, glide)?;
        if first && !glide {
            // They come in after the field.
            let anim = &self.anim;
            let visual = self.content.visual()?;
            visual.SetOpacity(0.0)?;
            anim.scalar(&visual, "Opacity", 1.0, 220, MAKE_WAY_MS, &anim.decelerate)?;
        }
        let h = self.search.make_room(&self.anim, size.Y.ceil())?;
        self.resize(h, true)?;
        self.update_scroll_chrome(false)?;
        self.sync_swap()
    }

    /// Picks the next suggestion down, or up.
    pub fn step_suggestion(&mut self, down: bool) -> Result<()> {
        self.scan.set_focused(false)?;
        self.search.step(&self.anim, down)
    }

    /// Tab, or Shift+Tab with `forward` false: from the field through the
    /// suggestions to the scan button, if it's there, and back to the field.
    pub fn tab_search(&mut self, forward: bool) -> Result<()> {
        let count = self.search.count();
        let scan = self.scan.shown();
        // The field is the last stop, after the scan button.
        let stops = count + scan as usize + 1;
        let at = if self.scan.focused() {
            count
        } else {
            self.search.selected().unwrap_or(stops - 1)
        };
        let next = if forward {
            (at + 1) % stops
        } else {
            (at + stops - 1) % stops
        };
        self.search
            .pick(&self.anim, (next < count).then_some(next))?;
        if scan && next == count {
            self.focus_button(CardButton::Scan, 0)
        } else {
            self.scan.set_focused(false)
        }
    }

    /// Picks suggestion `row`, or none, as the one to look up.
    pub fn pick_suggestion(&mut self, row: Option<usize>) -> Result<()> {
        self.search.pick(&self.anim, row)
    }

    /// The picked suggestion, if any.
    pub fn suggestion(&self) -> Option<usize> {
        self.search.selected()
    }

    /// The suggestion under a point in window pixels.
    pub fn suggestion_at(&self, x: i32, y: i32) -> Option<usize> {
        let (x, y) = self.card_point(x, y);
        self.search.row_at(x, y)
    }

    /// Turns the card into the search box. `in_place` fades out what it
    /// showed first and resizes it smoothly; otherwise it's about to appear.
    /// `returning` to it from what was typed in it, the heading sinks back
    /// into the field, and the body stays for the languages to glide from.
    pub(super) fn enter_search(
        &mut self,
        title: &str,
        purpose: Shortcut,
        in_place: bool,
        returning: bool,
    ) -> Result<()> {
        self.set_back(false)?;
        self.set_translate(false)?;
        self.set_speak(false)?;
        self.pin.set_shown(&self.anim, false, self.scale)?;
        self.focus_link(None, 0)?;
        self.header_indent = 0.0;
        let anim = &self.anim;
        let [word, old_word] = self.header.visuals()?;
        let mut shown: Vec<Visual> = vec![old_word, self.loader.visual().cast()?];
        // Returning, the heading and the body move rather than fade.
        if !returning {
            shown.extend([word, self.content.visual()?]);
        }
        for v in &shown {
            if in_place {
                anim.scalar(v, "Opacity", 0.0, MAKE_WAY_MS, 0, &anim.accelerate)?;
            } else {
                Animator::stop(v, &["Opacity"]);
                v.SetOpacity(0.0)?;
            }
        }
        self.loader.forget();
        self.content.set_links(Vec::new());
        self.shown = None;
        let delay = if in_place { MAKE_WAY_MS } else { 0 };
        let (renderer, theme, s) = (&self.renderer, &self.theme, self.scale);
        let h = self
            .search
            .open(renderer, anim, theme, s, title, purpose, delay)?;
        if returning {
            let to = self.search.query_at()?;
            self.header.sink(anim, to, QUERY_SIZE / WORD_SIZE)?;
            // What's typed takes over from the heading once it's there.
            let fade = parts::SINK_FADE_MS;
            self.search.await_heading(anim, RESIZE_MS - fade, fade)?;
            self.body_glides = true;
        }
        // The scan button sits level with the heading, where the pin button
        // was; a pinned card stays put, so it can't be read from behind, and
        // without a language to read in, Windows can't read it at all.
        let s = self.scale;
        let top = ((PAD_TOP + (self.search.title_line() - BACK_SIZE) / 2.0) * s).round();
        self.scan.place(top, s)?;
        let scannable = !self.pinned && ocr::available();
        self.scan.set_shown(&self.anim, scannable, s)?;
        // Nothing below the box: no body to scroll or click.
        (self.header_h, self.body_h) = (h, 0.0);
        self.scroller.stop_at(0.0);
        self.resize(h, in_place)?;
        self.update_scroll_chrome(false)?;
        self.sync_tooltip();
        self.sync_swap()
    }
}
