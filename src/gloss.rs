//! Glossing a passage: what's too long to look up is shown whole, with its
//! uncommon words marked, each a link to its definition.
//!
//! What's common is what's on a list of each translation language's
//! commonest words, from wordfreq (`tools/build_frequency.py` describes the
//! file, `data/LICENSE-wordfreq.txt` where it comes from). The lists are
//! embedded, and unpacked only while a passage is glossed. They also tell
//! which language it's in: the one whose commonest words it has the most
//! of. Only the beginnings of the lists of the languages it could be in are
//! unpacked for that, and then the whole list of the one it is in.

use std::collections::HashMap;
use std::ops::Range;

use crate::dictionary::Detail;
use crate::lang::LangCode;
use crate::{offline, translate};

static LISTS: &[u8] = include_bytes!("../data/frequency.bin");

/// At least this share of a passage's words must be common in a language
/// for it to be taken as in it: less is likelier another one altogether.
const MIN_KNOWN: f32 = 0.5;
/// How much of each list tells the language: its commonest few thousand
/// words, which are what sets one language apart from another.
const DETECT_BYTES: usize = 48 * 1024;
/// The longest word, in characters, looked for in Japanese and Chinese,
/// which don't put spaces between them.
const MAX_CJK_WORD: usize = 8;
/// A Chinese character alone is a common word only if it's this common:
/// rarer words are often written with common characters.
const MAX_CJK_CHAR_RANK: u32 = 1500;
/// A Korean word ends with what it does in the sentence ("에서", "at"),
/// one of the commonest pieces of the language, at most this long.
const MAX_PARTICLE: usize = 3;
const MAX_PARTICLE_RANK: u32 = 300;

/// A passage, and its uncommon words.
#[derive(Clone, Debug, PartialEq)]
pub struct Gloss {
    pub text: String,
    pub language: LangCode,
    /// Where the uncommon words are in `text`, as byte ranges, in order.
    pub words: Vec<Range<usize>>,
}

impl Gloss {
    /// The uncommon words, as they're written.
    pub fn words(&self) -> impl Iterator<Item = &str> {
        self.words.iter().map(|r| &self.text[r.clone()])
    }

    /// The language its words are looked up in: the dictionaries call
    /// Chinese "zh", however it's written.
    pub fn lookup_language(&self) -> LangCode {
        match self.language.as_str() {
            "zh-CN" => LangCode::new("zh"),
            _ => self.language.clone(),
        }
    }
}

/// Marks the uncommon words of `text`, in `language`, or in the language it
/// seems to be in, the `preferred` ones first when it's a close call.
/// `base` gives a word's base form in a language ("ran" → "run"), if a
/// dictionary has it, which is common when the word is. Nothing when it's
/// in none of the languages WinDict has lists for.
pub fn gloss(
    text: &str,
    language: Option<&LangCode>,
    preferred: &[LangCode],
    base: &dyn Fn(&str, &str) -> Option<String>,
) -> Option<Gloss> {
    let candidates: Vec<&str> = match language {
        Some(language) => vec![sections().find(|(code, _)| language == code)?.0],
        None => candidates(text),
    };
    let language = match candidates[..] {
        [] => return None,
        [only] => only,
        // Told by the commonest words of each, which is quicker than by all.
        _ => {
            let heads: Vec<(&str, String)> = sections()
                .filter(|(code, _)| candidates.contains(code))
                .filter_map(|(code, packed)| Some((code, unpack(packed, Some(DETECT_BYTES))?)))
                .collect();
            let lists: Vec<List> = heads
                .iter()
                .map(|(code, words)| List::new(code, words))
                .collect();
            let found = detect(text, &lists, preferred)?.language;
            *candidates.iter().find(|code| **code == found)?
        }
    };
    let (code, packed) = sections().find(|(code, _)| *code == language)?;
    let words = unpack(packed, None)?;
    let list = List::new(code, &words);
    Some(Gloss {
        text: text.to_string(),
        language: LangCode::new(code),
        words: list.uncommon(text, base),
    })
}

/// [`gloss`], with the base forms the dictionaries in use have.
pub fn with_dictionaries(
    text: &str,
    language: Option<&LangCode>,
    preferred: &[LangCode],
) -> Option<Gloss> {
    let dictionaries = offline::active();
    let brief = Detail {
        senses: 1,
        synonyms: 0,
        examples: false,
    };
    let base = |language: &str, word: &str| {
        let dictionary = dictionaries.iter().find(|d| *d.language() == language)?;
        Some(dictionary.lookup(word, brief)?.word)
    };
    gloss(text, language, preferred, &base)
}

/// A list's words, or as many of the commonest as fit in `limit` bytes.
fn unpack(packed: &[u8], limit: Option<usize>) -> Option<String> {
    use miniz_oxide::inflate::{decompress_to_vec_with_limit, TINFLStatus};
    let raw = match decompress_to_vec_with_limit(packed, limit.unwrap_or(usize::MAX)) {
        Ok(raw) => raw,
        // Cut at the last whole word.
        Err(e) if limit.is_some() && e.status == TINFLStatus::HasMoreOutput => {
            let mut raw = e.output;
            raw.truncate(raw.iter().rposition(|&b| b == b'\n')?);
            raw
        }
        Err(_) => return None,
    };
    String::from_utf8(raw).ok()
}

/// Each language's list, as code and packed words, in the order of the
/// translation languages.
fn sections() -> impl Iterator<Item = (&'static str, &'static [u8])> {
    let mut rest = LISTS;
    std::iter::from_fn(move || {
        let (&len, after) = rest.split_first()?;
        let (code, after) = after.split_at_checked(len as usize)?;
        let (size, after) = after.split_at_checked(4)?;
        let size = u32::from_le_bytes(size.try_into().ok()?) as usize;
        let (packed, after) = after.split_at_checked(size)?;
        rest = after;
        Some((std::str::from_utf8(code).ok()?, packed))
    })
}

/// The writing systems that tell languages apart.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Script {
    Latin,
    Greek,
    Cyrillic,
    Han,
    Kana,
    Hangul,
    Other,
}

fn script(c: char) -> Script {
    match c as u32 {
        0x3040..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9F => Script::Kana,
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x2FFFF => Script::Han,
        0x1100..=0x11FF | 0x3130..=0x318F | 0xAC00..=0xD7AF => Script::Hangul,
        0x0370..=0x03FF | 0x1F00..=0x1FFF => Script::Greek,
        0x0400..=0x04FF => Script::Cyrillic,
        0x0041..=0x024F | 0x1E00..=0x1EFF => Script::Latin,
        _ => Script::Other,
    }
}

fn is_katakana(c: char) -> bool {
    matches!(c as u32, 0x30A0..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9F)
}

/// The languages `text` could be in, by how it's written: Japanese once
/// there's any kana, Chinese for characters alone, and otherwise the
/// languages of the script most of its letters are in.
fn candidates(text: &str) -> Vec<&'static str> {
    let letters = || text.chars().filter(|c| c.is_alphabetic()).map(script);
    if letters().any(|s| s == Script::Kana) {
        return vec!["ja"];
    }
    let count = |script| letters().filter(|&s| s == script).count();
    let scripts = [
        Script::Han,
        Script::Hangul,
        Script::Greek,
        Script::Cyrillic,
        Script::Latin,
    ];
    let most = scripts.into_iter().max_by_key(|&s| count(s));
    match most.filter(|&s| count(s) > 0) {
        Some(Script::Han) => vec!["zh-CN"],
        Some(Script::Hangul) => vec!["ko"],
        Some(Script::Greek) => vec!["el"],
        Some(Script::Cyrillic) => vec!["ru", "uk"],
        Some(Script::Latin) => translate::LANGUAGES
            .iter()
            .map(|(code, _)| *code)
            .filter(|code| !["ja", "zh-CN", "ko", "el", "ru", "uk"].contains(code))
            .collect(),
        _ => Vec::new(),
    }
}

/// The list of the language `text` is likeliest in: the one with the most
/// of its words, the `preferred` languages first in a tie.
fn detect<'a>(text: &str, lists: &'a [List], preferred: &[LangCode]) -> Option<&'a List<'a>> {
    let words: Vec<String> = tokens(text)
        .filter(|t| !t.has_digits)
        .map(|t| fold(&text[t.range], ""))
        .collect();
    if words.is_empty() {
        return None;
    }
    let rank = |list: &List| {
        let preference = preferred.iter().position(|p| p == list.language);
        let known = words.iter().filter(|w| list.has(w)).count();
        (known, std::cmp::Reverse(preference.unwrap_or(usize::MAX)))
    };
    // The first of the most, as the lists are in the order of preference.
    let best = lists.iter().rev().max_by_key(|l| rank(l))?;
    let known = rank(best).0;
    (known as f32 >= words.len() as f32 * MIN_KNOWN).then_some(best)
}

/// A language's common words, with how common each is: 0 is the commonest.
struct List<'a> {
    language: &'a str,
    ranks: HashMap<&'a str, u32>,
    /// The longest, in characters.
    longest: usize,
}

impl<'a> List<'a> {
    fn new(language: &'a str, words: &'a str) -> List<'a> {
        let ranks: HashMap<&str, u32> = words.split('\n').zip(0..).collect();
        let longest = ranks.keys().map(|w| w.chars().count()).max().unwrap_or(0);
        List {
            language,
            ranks,
            longest,
        }
    }

    fn has(&self, word: &str) -> bool {
        self.ranks.contains_key(word)
    }

    fn rank(&self, word: &str) -> Option<u32> {
        self.ranks.get(word).copied()
    }

    /// Where `text`'s uncommon words are.
    fn uncommon(
        &self,
        text: &str,
        base: &dyn Fn(&str, &str) -> Option<String>,
    ) -> Vec<Range<usize>> {
        let mut found = Vec::new();
        for token in tokens(text) {
            if token.has_digits || self.is_name(text, &token) {
                continue;
            }
            let word = &text[token.range.clone()];
            match (self.language, script(word.chars().next().unwrap_or(' '))) {
                ("ja" | "zh-CN", Script::Han | Script::Kana) => found.extend(
                    self.uncommon_cjk(word)
                        .into_iter()
                        .map(|r| shift(r, token.range.start)),
                ),
                _ => found.extend(
                    self.uncommon_word(word, base)
                        .into_iter()
                        .map(|r| shift(r, token.range.start)),
                ),
            }
        }
        found
    }

    /// Whether `token` is likely someone's or something's name: capitalized
    /// within a sentence, or all in capitals. In German every noun is
    /// capitalized, so there's no telling.
    fn is_name(&self, text: &str, token: &Token) -> bool {
        let word = &text[token.range.clone()];
        let mut chars = word.chars();
        let Some(first) = chars.next() else {
            return false;
        };
        if !first.is_uppercase() {
            return false;
        }
        if word.chars().count() > 1
            && word
                .chars()
                .filter(|c| c.is_alphabetic())
                .all(char::is_uppercase)
        {
            return true;
        }
        if self.language == "de" {
            return false;
        }
        // Mid-sentence: what's before it, past spaces and quotes, isn't
        // the end of one.
        let before = text[..token.range.start]
            .chars()
            .rev()
            .find(|c| !c.is_whitespace() && !"\"'«»“”‘’„(¿¡—–-".contains(*c));
        before.is_some_and(|c| !".!?:;…。！？".contains(c))
    }

    /// Where the uncommon parts of `word`, one word of a passage with
    /// spaces, are in it: the word, or the part of it a Korean particle
    /// isn't, or of one with apostrophes, the parts between them that
    /// aren't common.
    #[allow(clippy::single_range_in_vec_init)]
    fn uncommon_word(
        &self,
        word: &str,
        base: &dyn Fn(&str, &str) -> Option<String>,
    ) -> Vec<Range<usize>> {
        if word.chars().count() < 2 || self.is_common(word, base) {
            return Vec::new();
        }
        if word.contains(is_apostrophe) {
            let mut parts = Vec::new();
            let mut start = 0;
            for (i, c) in word
                .char_indices()
                .chain(std::iter::once((word.len(), '\'')))
            {
                if !is_apostrophe(c) {
                    continue;
                }
                let part = &word[start..i];
                if part.chars().count() > 1 && !self.is_common(part, base) {
                    parts.push(start..i);
                }
                start = i + c.len_utf8();
            }
            return parts;
        }
        if self.language == "ko" {
            return vec![0..self.korean_stem(word)];
        }
        vec![0..word.len()]
    }

    fn is_common(&self, word: &str, base: &dyn Fn(&str, &str) -> Option<String>) -> bool {
        let folded = fold(word, self.language);
        if self.has(&folded) {
            return true;
        }
        // "l'homme", "dell'anno": as common as what they're made of.
        if folded.contains('\'')
            && folded
                .split('\'')
                .filter(|p| !p.is_empty())
                .all(|p| self.has(p))
        {
            return true;
        }
        // A word and what's added to it, each common: "학교" + "에서".
        if self.language == "ko" && self.splits(&folded) {
            return true;
        }
        base(self.language, &word.to_lowercase()).is_some_and(|b| {
            b.to_lowercase() != word.to_lowercase() && self.has(&fold(&b, self.language))
        })
    }

    /// Whether `word` is two or three common pieces.
    fn splits(&self, word: &str) -> bool {
        let ends: Vec<usize> = word.char_indices().map(|(i, _)| i).skip(1).collect();
        ends.iter().any(|&i| {
            let (head, tail) = word.split_at(i);
            self.has(head)
                && (self.has(tail)
                    || tail
                        .char_indices()
                        .skip(1)
                        .any(|(j, _)| self.has(&tail[..j]) && self.has(&tail[j..])))
        })
    }

    /// Where a Korean word ends, before its particle, if it has one.
    fn korean_stem(&self, word: &str) -> usize {
        let starts: Vec<usize> = word.char_indices().map(|(i, _)| i).collect();
        let n = starts.len();
        (1..=MAX_PARTICLE.min(n - 1))
            .rev()
            .map(|len| starts[n - len])
            .find(|&i| {
                let particle = &word[i..];
                self.rank(particle).is_some_and(|r| r < MAX_PARTICLE_RANK)
            })
            .unwrap_or(word.len())
    }

    /// Where the uncommon words are in `run`, Japanese or Chinese written
    /// without spaces: it's cut into the longest common words it starts
    /// with, one after the other. A word written in katakana is one, as
    /// far as that goes; what's between the common words, unless it's
    /// only kana endings, is uncommon.
    fn uncommon_cjk(&self, run: &str) -> Vec<Range<usize>> {
        let chars: Vec<(usize, char)> = run.char_indices().collect();
        let at = |i: usize| chars.get(i).map_or(run.len(), |&(b, _)| b);
        let mut found: Vec<Range<usize>> = Vec::new();
        let mut pending: Option<usize> = None;
        let flush = |pending: &mut Option<usize>, end: usize, found: &mut Vec<Range<usize>>| {
            if let Some(start) = pending.take() {
                let piece = &run[start..end];
                if piece
                    .chars()
                    .any(|c| script(c) == Script::Han || is_katakana(c))
                {
                    found.push(start..end);
                }
            }
        };
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i].1;
            let len = if is_katakana(c) {
                let n = chars[i..]
                    .iter()
                    .take_while(|&&(_, c)| is_katakana(c))
                    .count();
                let word = &run[at(i)..at(i + n)];
                (self.has(word)).then_some(n).ok_or(n)
            } else {
                let longest = MAX_CJK_WORD.min(self.longest).min(chars.len() - i);
                (1..=longest)
                    .rev()
                    .find(|&n| {
                        let word = &run[at(i)..at(i + n)];
                        match self.rank(word) {
                            Some(rank) if n == 1 && script(c) == Script::Han => {
                                rank < MAX_CJK_CHAR_RANK
                            }
                            Some(_) => true,
                            None => false,
                        }
                    })
                    .ok_or(1)
            };
            match len {
                Ok(n) => {
                    flush(&mut pending, at(i), &mut found);
                    i += n;
                }
                // Uncommon: it goes on until a common word starts.
                Err(n) => {
                    pending.get_or_insert(at(i));
                    i += n;
                }
            }
        }
        flush(&mut pending, run.len(), &mut found);
        found
    }
}

fn shift(r: Range<usize>, by: usize) -> Range<usize> {
    r.start + by..r.end + by
}

fn is_apostrophe(c: char) -> bool {
    matches!(c, '\'' | '\u{2019}' | '\u{02BC}')
}

/// A word as the lists have it: casefolded, with Turkish's dotted and
/// dotless i kept apart, and its apostrophes straight.
fn fold(word: &str, language: &str) -> String {
    let word = match language {
        "tr" => word.replace('I', "ı").replace('İ', "i"),
        _ => word.to_string(),
    };
    let mut folded = String::with_capacity(word.len());
    for c in word.to_lowercase().chars() {
        match c {
            'ß' => folded.push_str("ss"),
            'ς' => folded.push('σ'),
            c if is_apostrophe(c) => folded.push('\''),
            c => folded.push(c),
        }
    }
    folded
}

/// A word of a passage, as the lists cut them: letters and digits, with
/// apostrophes inside it; hyphens part words. Japanese and Chinese, which
/// don't put spaces between words, come in runs of a script, cut up later.
struct Token {
    range: Range<usize>,
    has_digits: bool,
}

fn tokens(text: &str) -> impl Iterator<Item = Token> + '_ {
    let mut chars = text.char_indices().peekable();
    std::iter::from_fn(move || {
        let is_cjk = |c: char| matches!(script(c), Script::Han | Script::Kana);
        let is_word = |c: char| c.is_alphanumeric() || ('\u{300}'..='\u{36F}').contains(&c);
        let (start, first) = loop {
            let (i, c) = chars.next()?;
            if is_word(c) {
                break (i, c);
            }
        };
        let cjk = is_cjk(first);
        let mut end = start + first.len_utf8();
        let mut has_digits = first.is_numeric();
        while let Some(&(i, c)) = chars.peek() {
            if is_word(c) && is_cjk(c) == cjk {
                has_digits |= c.is_numeric();
                end = i + c.len_utf8();
                chars.next();
                continue;
            }
            // An apostrophe inside a word: "don't", "l'homme".
            if !cjk && is_apostrophe(c) {
                let mut ahead = chars.clone();
                ahead.next();
                if ahead.peek().is_some_and(|&(_, n)| is_word(n) && !is_cjk(n)) {
                    chars.next();
                    continue;
                }
            }
            break;
        }
        Some(Token {
            range: start..end,
            has_digits,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: &str, _: &str) -> Option<String> {
        None
    }

    fn glossed(text: &str) -> Gloss {
        gloss(text, None, &[], &none).unwrap()
    }

    fn uncommon(text: &str) -> Vec<String> {
        glossed(text).words().map(str::to_string).collect()
    }

    #[test]
    fn reads_every_language() {
        let codes: Vec<&str> = sections().map(|(code, _)| code).collect();
        let languages: Vec<&str> = translate::LANGUAGES.iter().map(|(code, _)| *code).collect();
        assert_eq!(codes, languages);
    }

    #[test]
    fn marks_the_uncommon_words() {
        let text = "The magistrate was exasperated by the obsequious clerk, who kept bowing.";
        assert_eq!(glossed(text).language, "en");
        let words = uncommon(text);
        assert!(words.contains(&"obsequious".to_string()), "{words:?}");
        assert!(!words.contains(&"kept".to_string()));
        assert!(!words.contains(&"The".to_string()));
    }

    #[test]
    fn tells_close_languages_apart() {
        let cases = [
            ("es", "El gobierno anunció ayer que las nuevas medidas entrarán en vigor la próxima semana."),
            ("pt", "O governo anunciou ontem que as novas medidas entrarão em vigor na próxima semana."),
            ("da", "Regeringen meddelte i går, at de nye foranstaltninger træder i kraft i næste uge."),
            ("no", "Regjeringen kunngjorde i går at de nye tiltakene trer i kraft neste uke."),
            ("sv", "Regeringen meddelade igår att de nya åtgärderna träder i kraft nästa vecka."),
            ("ru", "Правительство вчера объявило, что новые меры вступят в силу на следующей неделе."),
            ("uk", "Уряд учора оголосив, що нові заходи набудуть чинності наступного тижня."),
            ("cs", "Vláda včera oznámila, že nová opatření vstoupí v platnost příští týden."),
            ("pl", "Rząd ogłosił wczoraj, że nowe środki wejdą w życie w przyszłym tygodniu."),
            ("el", "Η κυβέρνηση ανακοίνωσε χθες ότι τα νέα μέτρα θα τεθούν σε ισχύ την επόμενη εβδομάδα."),
            ("tr", "Hükümet dün, yeni önlemlerin gelecek hafta yürürlüğe gireceğini açıkladı."),
            ("ja", "政府は昨日、新しい措置が来週施行されると発表した。"),
            ("zh-CN", "政府昨天宣布，新措施将于下周生效。"),
            ("ko", "정부는 어제 새로운 조치가 다음 주에 시행된다고 발표했다."),
        ];
        for (language, text) in cases {
            assert_eq!(glossed(text).language, language, "{text}");
        }
    }

    #[test]
    fn prefers_the_languages_in_use_in_a_tie() {
        let text = "de la";
        let es = gloss(text, None, &["es".into()], &none).unwrap();
        assert_eq!(es.language, "es");
        let fr = gloss(text, None, &["fr".into()], &none).unwrap();
        assert_eq!(fr.language, "fr");
    }

    #[test]
    fn takes_the_language_given() {
        let gloss = gloss("de la", Some(&"pt".into()), &[], &none).unwrap();
        assert_eq!(gloss.language, "pt");
    }

    #[test]
    fn needs_a_language_it_has() {
        assert_eq!(gloss("Ỷ ỹ ỵ ỳ", None, &[], &none), None);
        assert_eq!(gloss("مرحبا بالعالم", None, &[], &none), None);
        assert_eq!(gloss("123 456", None, &[], &none), None);
    }

    #[test]
    fn leaves_out_names_and_numbers() {
        let words = uncommon(
            "Yesterday Ebenezer met NASA engineers in 1969 about the obsequious 3D printer.",
        );
        assert_eq!(words, ["obsequious"]);
    }

    #[test]
    fn checks_names_at_the_start_of_a_sentence() {
        let words =
            uncommon("It was a long day at the office. Obsequious people kept on talking to him.");
        assert_eq!(words, ["Obsequious"]);
    }

    #[test]
    fn capitalized_german_nouns_are_words() {
        let text = "Der Ausschuss hat die Unbedenklichkeitsbescheinigung gestern abgelehnt.";
        let words = uncommon(text);
        assert_eq!(glossed(text).language, "de");
        assert!(
            words.contains(&"Unbedenklichkeitsbescheinigung".to_string()),
            "{words:?}"
        );
        assert!(!words.contains(&"Ausschuss".to_string()), "{words:?}");
    }

    #[test]
    fn marks_the_uncommon_part_of_elisions() {
        let text = "Ieri l'ornitorinco e l'uomo sono andati al mare insieme.";
        assert_eq!(uncommon(text), ["ornitorinco"]);
    }

    #[test]
    fn goes_by_the_base_form() {
        let text = "The sanguine reports of yesterday were not believed by everyone in the room.";
        let base = |_: &str, word: &str| (word == "sanguine").then(|| "the".to_string());
        let gloss = gloss(text, None, &[], &base).unwrap();
        assert!(
            gloss.words().next().is_none(),
            "{:?}",
            gloss.words().collect::<Vec<_>>()
        );
    }

    #[test]
    fn folds_as_the_lists_do() {
        assert_eq!(fold("Straße", "de"), "strasse");
        assert_eq!(fold("ΤΗΣ", "el"), "τησ");
        assert_eq!(fold("Iİ", "tr"), "ıi");
        assert_eq!(fold("Don\u{2019}t", "en"), "don't");
    }

    #[test]
    fn cuts_korean_particles_off() {
        let text = "우리는 학교에서 공부했다. 그는 도롱뇽에게 말했다.";
        let gloss = glossed(text);
        assert_eq!(gloss.language, "ko");
        let words: Vec<&str> = gloss.words().collect();
        assert!(!words.contains(&"학교에서"), "{words:?}");
        assert!(words.contains(&"도롱뇽"), "{words:?}");
    }

    #[test]
    fn cuts_japanese_and_chinese_into_words() {
        let ja = glossed("私は東京で生き残った。");
        assert_eq!(
            ja.words().count(),
            0,
            "{:?}",
            ja.words().collect::<Vec<_>>()
        );
        let zh = glossed("我们在中国学习。");
        assert_eq!(
            zh.words().count(),
            0,
            "{:?}",
            zh.words().collect::<Vec<_>>()
        );
    }

    #[test]
    fn splits_words_at_hyphens() {
        let ranges: Vec<&str> = tokens("well-known e-mail")
            .map(|t| &"well-known e-mail"[t.range])
            .collect();
        assert_eq!(ranges, ["well", "known", "e", "mail"]);
    }
}
