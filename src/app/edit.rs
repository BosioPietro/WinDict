//! The text typed into a field, like the search box, and the caret in it,
//! edited as a Windows text field would be.

use crate::dictionary::MAX_LOOKUP_CHARS;

// Keys that arrive as characters (WM_CHAR).
const CTRL_A: u16 = 0x01;
const ENTER: u16 = 0x0D;
const BACKSPACE: u16 = 0x08;
const CTRL_BACKSPACE: u16 = 0x7F;
const CTRL_V: u16 = 0x16;

/// What a typed character did.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Input {
    /// The text may have changed.
    Edited,
    /// Enter: look the text up.
    Enter,
    /// Ctrl+V: paste the clipboard's text with [`LineEdit::paste`].
    Paste,
    /// Ctrl+A: select it all with [`LineEdit::select_all`], where the field
    /// shows a selection.
    SelectAll,
    /// Nothing, or not yet (half of a character outside the BMP).
    Nothing,
}

/// Where the caret goes, or what's deleted up to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Motion {
    CharBack,
    CharForward,
    /// The start of the word before the caret, past any spaces after it.
    WordBack,
    /// The start of the next word, past the rest of this one and the spaces
    /// after it, as Ctrl+Right goes elsewhere in Windows.
    WordForward,
    Start,
    End,
}

pub struct LineEdit {
    text: String,
    /// Where the caret is in `text`, in bytes.
    caret: usize,
    /// The first half of a character outside the BMP, waiting for the second.
    high_surrogate: Option<u16>,
    /// How many characters it takes.
    limit: usize,
    /// Where the selection, by Ctrl+A or Shift and the keys that move the
    /// caret, reaches back to from the caret, in bytes; what's typed next
    /// replaces it. None when nothing's selected.
    anchor: Option<usize>,
}

/// Empty, and taking as much as a lookup does.
impl Default for LineEdit {
    fn default() -> Self {
        Self::new("", MAX_LOOKUP_CHARS)
    }
}

impl LineEdit {
    /// Holds `text`, with the caret after it, and takes up to `limit` characters.
    pub fn new(text: &str, limit: usize) -> Self {
        let text: String = text.chars().take(limit).collect();
        Self {
            caret: text.len(),
            text,
            high_surrogate: None,
            limit,
            anchor: None,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// What's selected, from where to where, in UTF-16 units.
    pub fn selection_utf16(&self) -> Option<(u32, u32)> {
        let (from, to) = self.selection()?;
        let units = |at: usize| self.text[..at].encode_utf16().count() as u32;
        Some((units(from), units(to)))
    }

    /// Selects all of it, if there's anything, the caret after it.
    pub fn select_all(&mut self) {
        self.caret = self.text.len();
        self.anchor = (!self.text.is_empty()).then_some(0);
    }

    /// Moves the caret where `motion` leads, selecting what it passes, as
    /// Shift does. Returns whether it moved.
    pub fn extend(&mut self, motion: Motion) -> bool {
        let anchor = *self.anchor.get_or_insert(self.caret);
        let to = self.target(motion);
        let moved = to != self.caret;
        self.caret = to;
        if anchor == to {
            self.anchor = None;
        }
        moved
    }

    /// Holds `text` instead, as much of it as it takes, the caret after it.
    pub fn replace(&mut self, text: &str) {
        *self = Self::new(text, self.limit);
    }

    /// Where the caret is, in the UTF-16 units DirectWrite counts.
    pub fn caret_utf16(&self) -> u32 {
        self.text[..self.caret].encode_utf16().count() as u32
    }

    /// Puts the caret `at` UTF-16 units in, or at the end if that's past it.
    pub fn set_caret_utf16(&mut self, at: u32) {
        self.anchor = None;
        let mut units = 0;
        self.caret = self
            .text
            .char_indices()
            .find(|(_, c)| {
                units += c.len_utf16() as u32;
                units > at
            })
            .map_or(self.text.len(), |(i, _)| i);
    }

    /// A character typed (one UTF-16 unit of WM_CHAR).
    pub fn type_unit(&mut self, unit: u16) -> Input {
        let high = self.high_surrogate.take();
        match unit {
            ENTER => Input::Enter,
            // Typing counts even where there's nothing to delete.
            BACKSPACE => {
                self.delete(Motion::CharBack);
                Input::Edited
            }
            CTRL_BACKSPACE => {
                self.delete(Motion::WordBack);
                Input::Edited
            }
            CTRL_V => Input::Paste,
            CTRL_A => Input::SelectAll,
            0xD800..=0xDBFF => {
                self.high_surrogate = Some(unit);
                Input::Nothing
            }
            0xDC00..=0xDFFF => {
                let Some(high) = high else {
                    return Input::Nothing;
                };
                if let Some(Ok(c)) = char::decode_utf16([high, unit]).next() {
                    self.insert(c);
                }
                Input::Edited
            }
            _ => match char::from_u32(unit as u32) {
                Some(c) if !c.is_control() => {
                    self.insert(c);
                    Input::Edited
                }
                _ => Input::Nothing,
            },
        }
    }

    /// Pastes the first line of `text` with something on it.
    pub fn paste(&mut self, text: &str) {
        let line = text.lines().map(str::trim).find(|l| !l.is_empty());
        for c in line.unwrap_or_default().chars() {
            self.insert(if c.is_whitespace() { ' ' } else { c });
        }
    }

    /// Moves the caret. Returns whether it moved, or a selection went.
    pub fn move_caret(&mut self, motion: Motion) -> bool {
        // Left and Right go to the selection's ends, as in a Windows field.
        if let Some((from, to)) = self.selection() {
            self.caret = match motion {
                Motion::CharBack => from,
                Motion::CharForward => to,
                _ => self.target(motion),
            };
            self.anchor = None;
            return true;
        }
        let to = self.target(motion);
        let moved = to != self.caret;
        self.caret = to;
        moved
    }

    /// Deletes from the caret to where `motion` leads, or what's selected
    /// instead. Returns whether anything was.
    pub fn delete(&mut self, motion: Motion) -> bool {
        if self.take_selected() {
            return true;
        }
        let to = self.target(motion);
        let (from, to) = (to.min(self.caret), to.max(self.caret));
        self.text.replace_range(from..to, "");
        self.caret = from;
        from != to
    }

    /// What's selected, from where to where, in bytes.
    fn selection(&self) -> Option<(usize, usize)> {
        let anchor = self.anchor.filter(|&a| a != self.caret)?;
        Some((anchor.min(self.caret), anchor.max(self.caret)))
    }

    /// Deletes what's selected, if anything is. Returns whether it was.
    fn take_selected(&mut self) -> bool {
        let Some((from, to)) = self.selection() else {
            return false;
        };
        self.text.replace_range(from..to, "");
        (self.caret, self.anchor) = (from, None);
        true
    }

    fn insert(&mut self, c: char) {
        if c.is_control() {
            return;
        }
        self.take_selected();
        if self.text.chars().count() < self.limit {
            self.text.insert(self.caret, c);
            self.caret += c.len_utf8();
        }
    }

    /// Where `motion` leads from the caret, in bytes.
    fn target(&self, motion: Motion) -> usize {
        let (before, after) = self.text.split_at(self.caret);
        match motion {
            Motion::CharBack => before.char_indices().next_back().map_or(0, |(i, _)| i),
            Motion::CharForward => after
                .chars()
                .next()
                .map_or(self.caret, |c| self.caret + c.len_utf8()),
            Motion::WordBack => {
                // Past the space before the word: a space may take more
                // than a byte, as the ideographic one does.
                let trimmed = before.trim_end();
                trimmed
                    .char_indices()
                    .rfind(|(_, c)| c.is_whitespace())
                    .map_or(0, |(i, c)| i + c.len_utf8())
            }
            Motion::WordForward => {
                let word = after.find(char::is_whitespace).unwrap_or(after.len());
                let spaces = after[word..].len() - after[word..].trim_start().len();
                self.caret + word + spaces
            }
            Motion::Start => 0,
            Motion::End => self.text.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(text: &str) -> LineEdit {
        let mut edit = LineEdit::default();
        for unit in text.encode_utf16() {
            edit.type_unit(unit);
        }
        edit
    }

    #[test]
    fn types_text() {
        let edit = typed("ice cream");
        assert_eq!(edit.text(), "ice cream");
        assert_eq!(edit.caret_utf16(), 9);
        // Control characters aren't text.
        assert_eq!(LineEdit::default().type_unit(0x1B), Input::Nothing);
        assert_eq!(typed("a\tb").text(), "ab");
    }

    #[test]
    fn joins_surrogate_pairs() {
        let mut edit = LineEdit::default();
        let [high, low] = [0xD83D, 0xDE00];
        assert_eq!(edit.type_unit(high), Input::Nothing);
        assert_eq!(edit.type_unit(low), Input::Edited);
        assert_eq!(edit.text(), "\u{1F600}");
        assert_eq!(edit.caret_utf16(), 2);
        // A second half on its own is dropped.
        assert_eq!(edit.type_unit(low), Input::Nothing);
        assert_eq!(edit.text(), "\u{1F600}");
    }

    #[test]
    fn stops_at_what_a_lookup_takes() {
        let edit = typed(&"a".repeat(MAX_LOOKUP_CHARS + 5));
        assert_eq!(edit.text().chars().count(), MAX_LOOKUP_CHARS);
    }

    #[test]
    fn deletes_backwards() {
        let mut edit = typed("look  up");
        assert_eq!(edit.type_unit(BACKSPACE), Input::Edited);
        assert_eq!(edit.text(), "look  u");
        edit.type_unit(CTRL_BACKSPACE);
        assert_eq!(edit.text(), "look  ");
        edit.type_unit(CTRL_BACKSPACE);
        assert_eq!(edit.text(), "");
        // Nothing left to delete still counts as typing.
        assert_eq!(edit.type_unit(BACKSPACE), Input::Edited);
    }

    #[test]
    fn moves_by_characters_and_words() {
        let mut edit = typed("café au lait");
        assert!(edit.move_caret(Motion::WordBack));
        assert_eq!(edit.caret_utf16(), 8);
        edit.move_caret(Motion::WordBack);
        assert_eq!(edit.caret_utf16(), 5);
        edit.move_caret(Motion::CharBack);
        edit.move_caret(Motion::CharBack);
        assert_eq!(edit.caret_utf16(), 3);
        edit.move_caret(Motion::WordForward);
        assert_eq!(edit.caret_utf16(), 5);
        assert!(edit.move_caret(Motion::Start));
        assert!(!edit.move_caret(Motion::CharBack));
        edit.move_caret(Motion::End);
        assert!(!edit.move_caret(Motion::CharForward));
    }

    #[test]
    fn moves_by_words_past_wide_spaces() {
        // The ideographic space Japanese input types, and a no-break one.
        let mut edit = typed("日本\u{3000}語\u{a0}x");
        edit.move_caret(Motion::WordBack);
        assert_eq!(edit.caret_utf16(), 5);
        edit.move_caret(Motion::WordBack);
        assert_eq!(edit.caret_utf16(), 3);
        edit.move_caret(Motion::End);
        edit.type_unit(CTRL_BACKSPACE);
        edit.type_unit(CTRL_BACKSPACE);
        assert_eq!(edit.text(), "日本\u{3000}");
    }

    #[test]
    fn deletes_forwards() {
        let mut edit = typed("ice cream cone");
        edit.move_caret(Motion::Start);
        assert!(edit.delete(Motion::CharForward));
        assert_eq!(edit.text(), "ce cream cone");
        assert!(edit.delete(Motion::WordForward));
        assert_eq!(edit.text(), "cream cone");
        edit.move_caret(Motion::End);
        assert!(!edit.delete(Motion::CharForward));
    }

    #[test]
    fn holds_text_up_to_its_limit() {
        let mut edit = LineEdit::new("cafés", 6);
        assert_eq!(edit.caret_utf16(), 5);
        edit.set_caret_utf16(4);
        edit.type_unit(b'x' as u16);
        assert_eq!(edit.text(), "caféxs");
        edit.type_unit(b'y' as u16);
        assert_eq!(edit.text(), "caféxs");
        edit.set_caret_utf16(99);
        assert_eq!(edit.caret_utf16(), 6);
        assert_eq!(LineEdit::new("abc", 2).text(), "ab");
    }

    #[test]
    fn replaces_what_is_selected() {
        let mut edit = typed("ice cream");
        assert_eq!(edit.type_unit(CTRL_A), Input::SelectAll);
        edit.select_all();
        assert_eq!(edit.selection_utf16(), Some((0, 9)));
        edit.type_unit(b'x' as u16);
        assert_eq!(edit.text(), "x");
        assert_eq!(edit.selection_utf16(), None);
        // Backspace, or a paste, likewise.
        edit.select_all();
        edit.type_unit(BACKSPACE);
        assert_eq!(edit.text(), "");
        let mut edit = typed("ice");
        edit.select_all();
        edit.paste("cream");
        assert_eq!(edit.text(), "cream");
        // Nothing to select in an empty field.
        let mut edit = LineEdit::default();
        edit.select_all();
        assert_eq!(edit.selection_utf16(), None);
    }

    #[test]
    fn selects_as_the_caret_moves() {
        let mut edit = typed("ice cream cone");
        assert!(edit.extend(Motion::WordBack));
        assert!(edit.extend(Motion::CharBack));
        assert_eq!(edit.selection_utf16(), Some((9, 14)));
        // Back over where it started, it selects the other way.
        edit.move_caret(Motion::CharForward);
        edit.extend(Motion::CharBack);
        edit.extend(Motion::CharForward);
        assert_eq!(edit.selection_utf16(), None);
        assert!(!edit.extend(Motion::CharForward));
        assert_eq!(edit.selection_utf16(), None);
        edit.move_caret(Motion::Start);
        edit.extend(Motion::WordForward);
        edit.type_unit(b'x' as u16);
        assert_eq!(edit.text(), "xcream cone");
        // Delete takes what's selected, and Left goes to its start.
        edit.extend(Motion::End);
        assert!(edit.delete(Motion::CharForward));
        assert_eq!(edit.text(), "x");
        edit.extend(Motion::Start);
        assert!(edit.move_caret(Motion::CharBack));
        assert_eq!(edit.caret_utf16(), 0);
    }

    #[test]
    fn moves_off_what_is_selected() {
        let mut edit = typed("ice cream");
        edit.select_all();
        assert!(edit.move_caret(Motion::CharBack));
        assert_eq!(edit.caret_utf16(), 0);
        assert_eq!(edit.selection_utf16(), None);
        edit.select_all();
        // The caret's already at the end, but the selection goes.
        assert!(edit.move_caret(Motion::CharForward));
        assert_eq!(edit.caret_utf16(), 9);
        edit.select_all();
        assert!(edit.delete(Motion::CharForward));
        assert_eq!(edit.text(), "");
    }

    #[test]
    fn pastes_the_first_line() {
        let mut edit = typed("x");
        edit.move_caret(Motion::Start);
        edit.paste("\n  \n  ice\tcream \nsecond line");
        assert_eq!(edit.text(), "ice creamx");
        assert_eq!(edit.type_unit(CTRL_V), Input::Paste);
        assert_eq!(edit.type_unit(ENTER), Input::Enter);
    }
}
