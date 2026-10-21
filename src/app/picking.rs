//! The list a translation's language is picked from: the languages what's
//! typed in its box matches, the one Enter picks, and how far it's scrolled.

use crate::render::{Side, PICKER_ROWS};
use crate::translate;

use super::edit::LineEdit;

/// No language's name is longer, in any of the ways it can be searched.
const MAX_QUERY_CHARS: usize = 12;

pub(super) struct Picking {
    pub side: Side,
    /// What's typed to narrow the list down.
    pub query: LineEdit,
    /// The codes of the languages that match it.
    pub matches: Vec<&'static str>,
    /// Which of them Enter picks.
    pub highlight: Option<usize>,
    /// The first one the list shows.
    pub first: usize,
}

impl Picking {
    /// Every language, `current` highlighted.
    pub fn new(side: Side, current: &str) -> Picking {
        let mut picking = Picking {
            side,
            query: LineEdit::new("", MAX_QUERY_CHARS),
            matches: Vec::new(),
            highlight: None,
            first: 0,
        };
        picking.refilter(current);
        picking
    }

    /// Lists the languages what's typed matches, highlighting the first, or,
    /// with nothing typed, `current`.
    pub fn refilter(&mut self, current: &str) {
        self.matches = translate::search(self.query.text());
        self.highlight = if self.query.text().trim().is_empty() {
            self.matches.iter().position(|code| *code == current)
        } else {
            (!self.matches.is_empty()).then_some(0)
        };
        self.first = 0;
        self.reveal();
    }

    /// The highlighted language's code.
    pub fn highlighted(&self) -> Option<&'static str> {
        self.highlight.and_then(|i| self.matches.get(i).copied())
    }

    /// Highlights `code`, if it's listed. Returns whether that changed anything.
    pub fn highlight(&mut self, code: &str) -> bool {
        let at = self.matches.iter().position(|c| *c == code);
        let changed = at.is_some() && at != self.highlight;
        if changed {
            self.highlight = at;
        }
        changed
    }

    /// Moves the highlight `by` rows, stopping at either end.
    pub fn step(&mut self, by: isize) {
        let Some(last) = self.matches.len().checked_sub(1) else {
            return;
        };
        let to = match self.highlight {
            Some(i) => i.saturating_add_signed(by).min(last),
            None if by > 0 => 0,
            None => last,
        };
        self.highlight = Some(to);
        self.reveal();
    }

    /// Scrolls the list `by` rows. Returns whether it moved.
    pub fn scroll(&mut self, by: isize) -> bool {
        let max = self.matches.len().saturating_sub(PICKER_ROWS);
        let first = self.first.saturating_add_signed(by).min(max);
        std::mem::replace(&mut self.first, first) != first
    }

    /// Scrolls the list just enough to show the highlighted language.
    fn reveal(&mut self) {
        let Some(i) = self.highlight else {
            return;
        };
        if i < self.first {
            self.first = i;
        } else if i >= self.first + PICKER_ROWS {
            self.first = i + 1 - PICKER_ROWS;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(text: &str) -> Picking {
        let mut picking = Picking::new(Side::To, "it");
        for unit in text.encode_utf16() {
            picking.query.type_unit(unit);
        }
        picking.refilter("it");
        picking
    }

    #[test]
    fn starts_on_the_current_language() {
        let picking = Picking::new(Side::To, "ko");
        assert_eq!(picking.matches.len(), translate::LANGUAGES.len());
        assert_eq!(picking.highlighted(), Some("ko"));
        // Scrolled down to it.
        let at = picking.highlight.unwrap();
        assert!(picking.first <= at && at < picking.first + PICKER_ROWS);
    }

    #[test]
    fn narrows_down_to_what_is_typed() {
        let picking = typed("ger");
        assert_eq!(picking.matches, ["de"]);
        assert_eq!(picking.highlighted(), Some("de"));
        let picking = typed("xyz");
        assert!(picking.matches.is_empty());
        assert_eq!(picking.highlighted(), None);
    }

    #[test]
    fn steps_and_scrolls_within_the_list() {
        let mut picking = Picking::new(Side::From, "en");
        assert_eq!(picking.highlight, Some(0));
        picking.step(-1);
        assert_eq!(picking.highlight, Some(0));
        picking.step(PICKER_ROWS as isize);
        assert_eq!(picking.highlight, Some(PICKER_ROWS));
        assert_eq!(picking.first, 1);
        picking.step(100);
        assert_eq!(picking.highlight, Some(picking.matches.len() - 1));
        assert_eq!(picking.first, picking.matches.len() - PICKER_ROWS);
        assert!(!picking.scroll(5));
        assert!(picking.scroll(-100));
        assert_eq!(picking.first, 0);
        assert!(picking.highlight("fr"));
        assert!(!picking.highlight("fr"));
        assert!(!picking.highlight("xx"));
    }
}
