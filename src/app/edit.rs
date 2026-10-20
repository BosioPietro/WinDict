//! The text typed into the search box and the caret in it, edited as a
//! Windows text field would be.

use crate::dictionary::MAX_LOOKUP_CHARS;

// Keys that arrive as characters (WM_CHAR).
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

#[derive(Default)]
pub struct LineEdit {
    text: String,
    /// Where the caret is in `text`, in bytes.
    caret: usize,
    /// The first half of a character outside the BMP, waiting for the second.
    high_surrogate: Option<u16>,
}

impl LineEdit {
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Where the caret is, in the UTF-16 units DirectWrite counts.
    pub fn caret_utf16(&self) -> u32 {
        self.text[..self.caret].encode_utf16().count() as u32
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

    /// Moves the caret. Returns whether it moved.
    pub fn move_caret(&mut self, motion: Motion) -> bool {
        let to = self.target(motion);
        let moved = to != self.caret;
        self.caret = to;
        moved
    }

    /// Deletes from the caret to where `motion` leads. Returns whether
    /// anything was.
    pub fn delete(&mut self, motion: Motion) -> bool {
        let to = self.target(motion);
        let (from, to) = (to.min(self.caret), to.max(self.caret));
        self.text.replace_range(from..to, "");
        self.caret = from;
        from != to
    }

    fn insert(&mut self, c: char) {
        if !c.is_control() && self.text.chars().count() < MAX_LOOKUP_CHARS {
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
                let trimmed = before.trim_end();
                trimmed.rfind(char::is_whitespace).map_or(0, |i| i + 1)
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
    fn pastes_the_first_line() {
        let mut edit = typed("x");
        edit.move_caret(Motion::Start);
        edit.paste("\n  \n  ice\tcream \nsecond line");
        assert_eq!(edit.text(), "ice creamx");
        assert_eq!(edit.type_unit(CTRL_V), Input::Paste);
        assert_eq!(edit.type_unit(ENTER), Input::Enter);
    }
}
