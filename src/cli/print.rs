//! Printing answers: styled and wrapped to the window when stdout is a
//! console, as plain lines when it's piped or redirected, and what's being
//! waited for on stderr meanwhile.

use std::fmt::Write as _;
use std::io::{self, IsTerminal, StdoutLock, Write};
use std::ops::Range;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Console::{
    GetConsoleMode, GetConsoleScreenBufferInfo, GetStdHandle, SetConsoleMode, CONSOLE_MODE,
    CONSOLE_SCREEN_BUFFER_INFO, ENABLE_VIRTUAL_TERMINAL_PROCESSING, STD_OUTPUT_HANDLE,
};

use super::*;

/// Where a sense's text starts, past its number.
const SENSE_INDENT: usize = 4;
/// Where the translations' meanings back start, past the widest of them,
/// unless that's wider than this.
const MAX_TERM_WIDTH: usize = 24;

/// Where what follows words `widths` wide lines up: two spaces in, and two
/// past the widest, up to a limit.
fn indent_after(widths: impl Iterator<Item = usize>) -> usize {
    2 + widths.max().unwrap_or(0).min(MAX_TERM_WIDTH) + 2
}
/// The widest lines are, however wide the window: longer ones are hard to
/// read.
const MAX_COLUMNS: usize = 88;

/// What's being waited for, on stderr while it's a console, rubbed out
/// again once it's dropped.
pub(super) struct Waiting(usize);

impl Waiting {
    pub(super) fn show(text: &str) -> Waiting {
        if !io::stderr().is_terminal() {
            return Waiting(0);
        }
        eprint!("{text}");
        Waiting(text.chars().count())
    }
}

impl Drop for Waiting {
    fn drop(&mut self) {
        if self.0 > 0 {
            eprint!("\r{}\r", " ".repeat(self.0));
        }
    }
}

/// How a piece of the answer is printed, in a console that shows styles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Style {
    Plain,
    /// The word, and the translation.
    Strong,
    /// What's around them: pronunciations, notes, sources.
    Faint,
    /// The headings over the parts of speech, as the card has them.
    Accent,
    /// Examples.
    Quote,
    /// A passage's uncommon words.
    Marked,
}

impl Style {
    /// Its SGR parameters.
    fn sgr(self) -> &'static str {
        match self {
            Style::Plain => "",
            Style::Strong => "1",
            Style::Faint => "2",
            Style::Accent => "36",
            Style::Quote => "2;3",
            Style::Marked => "4;36",
        }
    }
}

/// Prints answers to stdout.
pub(super) struct Printer {
    pub(super) out: StdoutLock<'static>,
    styled: bool,
    /// How wide lines can be.
    columns: usize,
    /// The console's mode before styles were turned on, to put back.
    restore: Option<(HANDLE, CONSOLE_MODE)>,
}

impl Drop for Printer {
    fn drop(&mut self) {
        if let Some((handle, mode)) = self.restore {
            let _ = self.out.flush();
            let _ = unsafe { SetConsoleMode(handle, mode) };
        }
    }
}

impl Printer {
    pub(super) fn new() -> Printer {
        let console = unsafe { console() };
        let restore = console.and_then(|c| c.restore);
        let (styled, columns) = console.map_or((false, usize::MAX), |c| {
            // A column spare: the console moves to the next line once the
            // last is written.
            (c.styled, c.columns.saturating_sub(1).min(MAX_COLUMNS))
        });
        // https://no-color.org
        let styled = styled && std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty());
        Printer {
            out: io::stdout().lock(),
            styled,
            columns,
            restore,
        }
    }

    /// Writes `spans` as a paragraph, see [`wrap`]. Errors, like a pipe
    /// closed early, aren't anything to tell anyone about.
    fn line(&mut self, margin: usize, lead: (Style, &str), indent: usize, spans: &[(Style, &str)]) {
        let text = wrap(
            margin,
            lead,
            indent,
            spans,
            self.columns.max(indent + 20),
            self.styled,
        );
        let _ = writeln!(self.out, "{text}");
    }

    fn text(&mut self, spans: &[(Style, &str)]) {
        self.line(0, (Style::Plain, ""), 0, spans);
    }

    fn blank(&mut self) {
        let _ = writeln!(self.out);
    }

    /// The word and how it's said, then each language's part (under its
    /// name, when there are several) with its parts of speech, then where
    /// it all comes from.
    pub(super) fn definition(&mut self, def: &Definition) {
        let mut title = vec![(Style::Strong, def.word.as_str())];
        if let Some(phonetic) = &def.phonetic {
            title.push((Style::Faint, phonetic));
        }
        self.text(&title);
        let grouped = !def.others.is_empty();
        for section in def.sections() {
            if grouped {
                self.blank();
                let name = dicts::name(&section.language);
                let mut heading = vec![(Style::Strong, name.as_str())];
                // The base form of what was looked up.
                if !same_word(&section.word, &def.word) {
                    heading.push((Style::Faint, &section.word));
                }
                self.text(&heading);
            }
            if let Some(note) = &section.note {
                self.text(&[(Style::Faint, note)]);
            }
            for (i, meaning) in section.meanings.iter().enumerate() {
                if i > 0 || !grouped {
                    self.blank();
                }
                let pos = tr().part_of_speech(&meaning.part_of_speech).to_uppercase();
                self.text(&[(Style::Accent, &pos)]);
                for (n, sense) in meaning.senses.iter().enumerate() {
                    let number = format!("{}.", n + 1);
                    let lead = (Style::Faint, number.as_str());
                    self.line(0, lead, SENSE_INDENT, &[(Style::Plain, &sense.text)]);
                    if let Some(example) = &sense.example {
                        let quoted = format!("\u{201C}{example}\u{201D}");
                        let none = (Style::Plain, "");
                        self.line(SENSE_INDENT, none, SENSE_INDENT, &[(Style::Quote, &quoted)]);
                    }
                }
                for (label, words) in [
                    (tr().similar, &meaning.synonyms),
                    (tr().opposite, &meaning.antonyms),
                ] {
                    if !words.is_empty() {
                        let label = format!("{label}:");
                        let lead = (Style::Faint, label.as_str());
                        let words = words.join(", ");
                        self.line(SENSE_INDENT, lead, SENSE_INDENT, &[(Style::Plain, &words)]);
                    }
                }
            }
        }
        self.blank();
        for (source, word) in def.all_sources() {
            let url = dictionary::source_url(source, word);
            self.source(source, url.as_deref());
        }
    }

    /// Which way it went, the translation and what the word was taken to
    /// mean, the other translations by part of speech, each with what it
    /// means back, then where it comes from.
    pub(super) fn translation(
        &mut self,
        translation: &Translation,
        text: &str,
        direction: &Direction,
    ) {
        let (from, to) = (&direction.from, &direction.to);
        let way = format!("{} \u{2192} {}", translate::name(from), translate::name(to));
        self.text(&[(Style::Faint, &way)]);
        // A passage is read, not glanced at.
        let style = if translate::is_passage(&translation.text) {
            Style::Plain
        } else {
            Style::Strong
        };
        self.text(&[(style, &capitalize(&translation.text))]);
        if let Some(context) = &translation.context {
            let quoted = format!("\u{201C}{}\u{201D}", capitalize(context));
            self.text(&[(Style::Quote, &quoted)]);
        }
        for group in &translation.groups {
            self.blank();
            let label = match group.part_of_speech.as_str() {
                "" => tr().also,
                pos => tr().part_of_speech(pos),
            };
            self.text(&[(Style::Accent, &label.to_uppercase())]);
            let indent = indent_after(group.terms.iter().map(|t| t.text.chars().count()));
            for term in &group.terms {
                let word = capitalize(&term.text);
                let back = capitalize(&term.back.join(", "));
                self.line(2, (Style::Plain, &word), indent, &[(Style::Faint, &back)]);
            }
        }
        self.blank();
        let source = translation.source;
        let url = (source == translate::GOOGLE).then(|| {
            format!(
                "https://translate.google.com/?sl={from}&tl={to}&text={}&op=translate",
                dictionary::percent_encode(text)
            )
        });
        self.source(source, url.as_deref());
    }

    /// The language of the passage, the passage with its uncommon words
    /// marked, then each of them with its first sense, under the base form
    /// it's of, when it's another.
    pub(super) fn gloss(&mut self, gloss: &gloss::Gloss, glossary: &[(&str, Option<Definition>)]) {
        self.text(&[(Style::Faint, &translate::name(&gloss.language))]);
        let columns = self.columns.max(20);
        let passage = mark(&gloss.text, &gloss.words, columns, self.styled);
        let _ = writeln!(self.out, "{passage}");
        self.blank();
        if glossary.is_empty() {
            self.text(&[(Style::Faint, tr().gloss_none)]);
            return;
        }
        let indent = indent_after(glossary.iter().map(|(w, _)| w.chars().count()));
        for (word, def) in glossary {
            let mut spans = Vec::new();
            let sense = def.as_ref().and_then(|def| {
                if !same_word(&def.word, word) {
                    spans.push((Style::Faint, def.word.as_str()));
                }
                Some(def.meanings.first()?.senses.first()?.text.as_str())
            });
            match sense {
                Some(sense) => spans.push((Style::Plain, sense)),
                None => spans = vec![(Style::Faint, tr().not_found)],
            }
            self.line(2, (Style::Strong, word), indent, &spans);
        }
    }

    /// The article's title and what it's about, its first sentences, and
    /// its page.
    pub(super) fn article(&mut self, article: &Article) {
        let mut title = vec![(Style::Strong, article.title.as_str())];
        if let Some(description) = &article.description {
            title.push((Style::Faint, description));
        }
        self.text(&title);
        self.blank();
        match article.ambiguous {
            true => self.text(&[(Style::Faint, tr().wikipedia_ambiguous)]),
            false => self.text(&[(Style::Plain, &article.extract)]),
        }
        self.blank();
        self.source(tr().wikipedia, Some(&article.url));
    }

    /// Where the answer comes from, and its page there, if it has one.
    fn source(&mut self, name: &str, url: Option<&str>) {
        let mut spans = vec![(Style::Faint, name)];
        if let Some(url) = url {
            spans.extend([(Style::Faint, "\u{B7}"), (Style::Faint, url)]);
        }
        self.text(&spans);
    }
}

/// The console stdout writes to.
#[derive(Clone, Copy)]
struct Console {
    /// Whether it shows styles.
    styled: bool,
    /// How many columns its window has.
    columns: usize,
    /// Its mode before styles were turned on, if they were.
    restore: Option<(HANDLE, CONSOLE_MODE)>,
}

/// The console stdout writes to, its styles turned on if they're off;
/// nothing when stdout isn't a console.
unsafe fn console() -> Option<Console> {
    let handle = GetStdHandle(STD_OUTPUT_HANDLE).ok()?;
    let mut mode = CONSOLE_MODE::default();
    GetConsoleMode(handle, &mut mode).ok()?;
    let on = mode.contains(ENABLE_VIRTUAL_TERMINAL_PROCESSING);
    let turned_on =
        !on && SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING).is_ok();
    let mut info = CONSOLE_SCREEN_BUFFER_INFO::default();
    GetConsoleScreenBufferInfo(handle, &mut info).ok()?;
    let columns = info.srWindow.Right - info.srWindow.Left + 1;
    Some(Console {
        styled: on || turned_on,
        columns: columns.max(0) as usize,
        restore: turned_on.then_some((handle, mode)),
    })
}

/// `spans` as a paragraph of lines at most `columns` wide, broken between
/// words, which are one space apart. The first line starts `margin` in,
/// with `lead` (a sense's number, say), and its text `indent` in, or a
/// space after a longer lead; the others start `indent` in.
fn wrap(
    margin: usize,
    lead: (Style, &str),
    indent: usize,
    spans: &[(Style, &str)],
    columns: usize,
    styled: bool,
) -> String {
    let paint = |out: &mut String, (style, text): (Style, &str)| {
        if styled && style != Style::Plain {
            let _ = write!(out, "\x1b[{}m{text}\x1b[0m", style.sgr());
        } else {
            out.push_str(text);
        }
    };
    let mut out = " ".repeat(margin);
    paint(&mut out, lead);
    let mut column = margin + lead.1.chars().count();
    let mut start = true;
    for &(style, text) in spans {
        for word in text.split_whitespace() {
            let width = word.chars().count();
            if start {
                let to = indent.max(column + usize::from(!lead.1.is_empty()));
                out.push_str(&" ".repeat(to - column));
                column = to;
                start = false;
            } else if column + 1 + width > columns {
                out.push('\n');
                out.push_str(&" ".repeat(indent));
                column = indent;
            } else {
                out.push(' ');
                column += 1;
            }
            paint(&mut out, (style, word));
            column += width;
        }
    }
    out
}

/// `text`, its words one space apart, as a paragraph of lines at most
/// `columns` wide, with the stretches `marked` in it styled as such.
fn mark(text: &str, marked: &[Range<usize>], columns: usize, styled: bool) -> String {
    let mut out = String::new();
    let (mut start, mut column) = (0, 0);
    for word in text.split(' ') {
        let end = start + word.len();
        let width = word.chars().count();
        if column > 0 && column + 1 + width > columns {
            out.push('\n');
            column = 0;
        } else if column > 0 {
            out.push(' ');
            column += 1;
        }
        // A word may be marked only in part ("l'ornitorinco").
        let mut at = start;
        for range in marked.iter().filter(|r| r.start < end && r.end > start) {
            let (from, to) = (range.start.max(start), range.end.min(end));
            out.push_str(&text[at..from]);
            match styled {
                true => {
                    let _ = write!(
                        out,
                        "\x1b[{}m{}\x1b[0m",
                        Style::Marked.sgr(),
                        &text[from..to]
                    );
                }
                false => out.push_str(&text[from..to]),
            }
            at = to;
        }
        out.push_str(&text[at..end]);
        column += width;
        start = end + 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // `&[4..14]` is one word's range, not the numbers from 4 to 14.
    #[allow(clippy::single_range_in_vec_init)]
    fn marks_words_and_wraps() {
        let text = "the obsequious clerk l'ornitorinco";
        assert_eq!(mark(text, &[4..14], 80, false), text);
        assert_eq!(
            mark(text, &[4..14, 23..34], 80, true),
            "the \x1b[4;36mobsequious\x1b[0m clerk l'\x1b[4;36mornitorinco\x1b[0m"
        );
        assert_eq!(
            mark(text, &[], 15, false),
            "the obsequious\nclerk\nl'ornitorinco"
        );
    }

    #[test]
    fn wraps_paragraphs() {
        let plain = (Style::Plain, "");
        let spans = [(Style::Plain, "one two three four")];
        assert_eq!(wrap(0, plain, 0, &spans, 80, false), "one two three four");
        assert_eq!(
            wrap(0, plain, 2, &spans, 10, false),
            "  one two\n  three\n  four"
        );
        // A lead, with its text from the indent on, or a space after it.
        let number = (Style::Faint, "1.");
        assert_eq!(
            wrap(0, number, 4, &spans, 13, false),
            "1.  one two\n    three\n    four"
        );
        let label = (Style::Faint, "Similar:");
        assert_eq!(
            wrap(4, label, 4, &[(Style::Plain, "a, b")], 80, false),
            "    Similar: a, b"
        );
        // A word longer than the line has a line of its own.
        assert_eq!(
            wrap(0, plain, 0, &[(Style::Plain, "a bbbbbbbb c")], 4, false),
            "a\nbbbbbbbb\nc"
        );
        // Styles are word by word, and none for plain text.
        let styled = wrap(
            0,
            plain,
            0,
            &[(Style::Strong, "run"), (Style::Plain, "x")],
            80,
            true,
        );
        assert_eq!(styled, "\x1b[1mrun\x1b[0m x");
    }
}
