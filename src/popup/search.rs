//! The card's search box: opening it and its suggestions.

use super::*;

impl Popup {
    /// Shows the search box under `title`. An open card turns into it in
    /// place; otherwise a new one appears next to `anchor`.
    pub fn show_search(
        &mut self,
        anchor: RECT,
        theme: Theme,
        title: &str,
        placeholder: &str,
    ) -> Result<()> {
        if self.visible {
            return self.enter_search(title, placeholder, true);
        }
        // Its top stays put, so the field doesn't move as suggestions come and go.
        self.prepare(anchor, theme, true)?;
        self.enter_search(title, placeholder, false)?;
        self.appear()
    }

    /// Shows `text` in the search box, the caret `at` UTF-16 units in.
    pub fn set_query(&mut self, text: &str, at: u32) -> Result<()> {
        self.search
            .set_query(&self.renderer, &self.anim, &self.theme, text, at)
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

    /// Picks the next suggestion down, or up.
    pub fn step_suggestion(&mut self, down: bool) -> Result<()> {
        self.search.step(&self.anim, down)
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
    pub(super) fn enter_search(
        &mut self,
        title: &str,
        placeholder: &str,
        in_place: bool,
    ) -> Result<()> {
        self.set_back(false)?;
        self.set_translate(false)?;
        self.focus_link(None, 0)?;
        self.header_indent = 0.0;
        let anim = &self.anim;
        let [word, old_word] = self.header.visuals()?;
        let shown: [Visual; 4] = [
            word,
            old_word,
            self.content.visual()?,
            self.loader.visual().cast()?,
        ];
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
        let delay = if in_place { MAKE_WAY_MS } else { 0 };
        let (renderer, theme, s) = (&self.renderer, &self.theme, self.scale);
        let h = self
            .search
            .open(renderer, anim, theme, s, title, placeholder, delay)?;
        // Nothing below the box: no body to scroll or click.
        (self.header_h, self.body_h) = (h, 0.0);
        (self.scroll, self.scroll_anim) = (0.0, None);
        self.resize(h, in_place)?;
        self.update_scroll_chrome(false)?;
        self.sync_swap()
    }
}
