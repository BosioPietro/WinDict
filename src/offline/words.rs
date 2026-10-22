//! The words the dictionaries define, suggested as the search box is typed
//! in, typos and all, and offered for a word that wasn't found.

use std::sync::Arc;

use crate::lang::LangCode;

use super::format::{Dict, Entry};
use super::{active, Dictionary};

/// A word offered for what's been typed in the search box.
#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    /// As it's written ("Paris", "ice cream").
    pub word: String,
    /// What an inflected form is a form of ("geese" → "goose").
    pub base: Option<String>,
    /// How many of its first characters are what was typed: none when it
    /// puts a typo right.
    pub typed: usize,
    /// The code of the language it's a word of ("en").
    pub language: LangCode,
}

/// Every word the dictionaries define, to suggest from as the search box is
/// typed in. Building it reads the whole dictionaries (a few tens of ms
/// each), so it's made when the search box first needs it and dropped with
/// the box.
pub struct Words {
    /// Each word's key, followed by its display form when that differs.
    text: String,
    /// In key order.
    words: Vec<Word>,
    /// Where they're from, for the forms of words.
    dictionaries: Vec<Arc<Dictionary>>,
}

struct Word {
    start: u32,
    key_len: u16,
    display_len: u16,
    senses: u16,
    /// Which of the dictionaries it's from.
    dictionary: u8,
}

impl Words {
    /// The words of the dictionaries in use.
    pub fn load() -> Words {
        Words::of(active())
    }

    pub(super) fn of(dictionaries: Vec<Arc<Dictionary>>) -> Words {
        let mut words = Words {
            text: String::new(),
            words: Vec::new(),
            dictionaries,
        };
        for (i, dictionary) in words.dictionaries.clone().iter().enumerate() {
            if let Some(dict) = dictionary.dict() {
                words.add(&dict, i as u8);
            }
        }
        // Each dictionary's are in key order; together, they're put back in
        // it. Stable: a word in several languages keeps their order.
        if words.dictionaries.len() > 1 {
            let mut list = std::mem::take(&mut words.words);
            list.sort_by(|a, b| words.key(a).cmp(words.key(b)));
            words.words = list;
        }
        words
    }

    fn add(&mut self, dict: &Dict, dictionary: u8) {
        let words = self;
        for block in 0..dict.entry_blocks {
            let Some(text) = dict.entry_block(block) else {
                continue;
            };
            for record in text.split('\x1e') {
                let key = record.split('\n').next().unwrap_or_default();
                let mut entry = Entry::parse(record);
                // Spelt without its accents ("arleng"): offered as written.
                if entry.senses.is_empty() {
                    if let Some(written) = entry.spellings.iter().find_map(|s| {
                        let e = dict.defined(s)?;
                        Some(Entry {
                            display: Some(e.display.clone().unwrap_or_else(|| s.clone())),
                            ..e
                        })
                    }) {
                        entry = written;
                    }
                }
                let senses: usize = entry.senses.iter().map(|(_, ids)| ids.len()).sum();
                if senses == 0 {
                    continue;
                }
                let display = entry.display.filter(|d| d != key).unwrap_or_default();
                words.words.push(Word {
                    start: words.text.len() as u32,
                    key_len: key.len() as u16,
                    display_len: display.len() as u16,
                    senses: senses.min(u16::MAX as usize) as u16,
                    dictionary,
                });
                words.text.push_str(key);
                words.text.push_str(&display);
            }
        }
    }

    fn key(&self, word: &Word) -> &str {
        let start = word.start as usize;
        &self.text[start..start + word.key_len as usize]
    }

    fn language(&self, word: &Word) -> &LangCode {
        self.dictionaries[word.dictionary as usize].language()
    }

    /// Whether the words are from dictionaries in more than one language.
    pub fn multilingual(&self) -> bool {
        self.dictionaries.len() > 1
    }

    fn display(&self, word: &Word) -> &str {
        let start = word.start as usize + word.key_len as usize;
        match word.display_len {
            0 => self.key(word),
            len => &self.text[start..start + len as usize],
        }
    }

    /// Up to `limit` words for what's been `typed` (lowercase), the likeliest
    /// first: the word itself, then words starting with what's typed, then
    /// words that do once a typo or two is put right ("recieve", "seperat").
    /// Among those, single words come before phrases, then the ones with the
    /// most meanings, which tend to be the common ones.
    pub fn suggest(&self, typed: &str, limit: usize) -> Vec<Suggestion> {
        if typed.is_empty() {
            return Vec::new();
        }
        let rank = |i: usize, typos: usize| {
            let word = &self.words[i];
            let key = self.key(word);
            let rank = (
                key != typed,
                typos,
                key.contains(' '),
                std::cmp::Reverse(word.senses),
                key.len(),
            );
            (rank, i)
        };
        // Words starting with what's typed are consecutive.
        let first = self.words.partition_point(|w| self.key(w) < typed);
        let mut found: Vec<_> = (first..self.words.len())
            .take_while(|&i| self.key(&self.words[i]).starts_with(typed))
            .map(|i| rank(i, 0))
            .collect();
        let typed_chars: Vec<char> = typed.chars().collect();
        let max = typos_allowed(typed_chars.len());
        // Words with typos rank below those, so they're looked for only
        // when there's room left.
        if found.len() < limit && max > 0 {
            let mut rows = Vec::new();
            for (i, word) in self.words.iter().enumerate() {
                let key = self.key(word);
                if key.starts_with(typed) {
                    continue;
                }
                if let Some(n) = typos(&typed_chars, key, max, false, &mut rows) {
                    found.push(rank(i, n));
                }
            }
        }
        // Stable: equal ranks stay in alphabetical order.
        found.sort_by_key(|(rank, _)| *rank);
        let mut out: Vec<Suggestion> = Vec::new();
        for ((_, typos, ..), i) in found {
            if out.len() == limit {
                break;
            }
            let word = self.display(&self.words[i]);
            let language = self.language(&self.words[i]);
            // Once in each language, though it's there with and without its
            // accents.
            if out
                .iter()
                .any(|s| s.word == word && s.language == *language)
            {
                continue;
            }
            out.push(Suggestion {
                word: word.to_string(),
                base: None,
                typed: if typos == 0 { typed_chars.len() } else { 0 },
                language: language.clone(),
            });
        }
        // An inflected form ("geese", "walked") is offered, as what it's a
        // form of, only once it's typed in full: in each language it isn't
        // a word of, after those it is.
        let mut at = out
            .iter()
            .take_while(|s| s.word.to_lowercase() == typed)
            .count();
        for dictionary in &self.dictionaries {
            let language = dictionary.language();
            if out[..at].iter().any(|s| s.language == *language) {
                continue;
            }
            if let Some(base) = dictionary.dict().and_then(|d| d.base_of(typed)) {
                out.insert(
                    at,
                    Suggestion {
                        word: typed.to_string(),
                        base: Some(base),
                        typed: typed_chars.len(),
                        language: language.clone(),
                    },
                );
                at += 1;
            }
        }
        out.truncate(limit);
        out
    }

    /// The word that was likely meant by `typed` (lowercase), which wasn't
    /// found: the closest one once a typo or two is put right ("emobdy" →
    /// "embody"), single words before phrases, then the one with the most
    /// meanings.
    pub fn correct(&self, typed: &str) -> Option<&str> {
        let typed_chars: Vec<char> = typed.chars().collect();
        let max = typos_allowed(typed_chars.len());
        if max == 0 {
            return None;
        }
        let mut rows = Vec::new();
        let word = self
            .words
            .iter()
            .filter(|w| self.key(w) != typed)
            .filter_map(|w| {
                let key = self.key(w);
                let n = typos(&typed_chars, key, max, true, &mut rows)?;
                Some(((n, key.contains(' '), std::cmp::Reverse(w.senses)), w))
            })
            .min_by_key(|(rank, _)| *rank)?
            .1;
        Some(self.display(word))
    }
}

/// A typo, in half typos: two letters swapped, the likeliest slip, counts
/// as half of one, so "recie" suggests "receive" before "review".
const TYPO: usize = 2;
const SWAP: usize = 1;

/// How many typos a suggestion may put right, in half typos: none in a
/// short word, where one would match too much.
fn typos_allowed(chars: usize) -> usize {
    match chars {
        0..=3 => 0,
        4..=7 => TYPO,
        _ => 2 * TYPO,
    }
}

/// The fewest typos (a letter added, dropped, changed, or two swapped) that
/// turn the start of `key`, or all of it when `whole`, into `typed`, in
/// half typos, if there are at most `max`. `rows` is scratch space, kept
/// between calls.
fn typos(
    typed: &[char],
    key: &str,
    max: usize,
    whole: bool,
    rows: &mut Vec<usize>,
) -> Option<usize> {
    // Edit distance, three rows at a time: one per letter of the key. With
    // at most `band` letters added or dropped, only cells that close to the
    // diagonal can be in reach; the ones just past it read as out of reach.
    const FAR: usize = usize::MAX / 4;
    let (m, band) = (typed.len(), max / TYPO);
    // Only the cells at the key's end count when all of it must match.
    let end = if whole {
        let len = key.chars().count();
        if len > m + band {
            return None;
        }
        Some(len)
    } else {
        None
    };
    let w = m + 1;
    rows.clear();
    rows.extend((0..w).map(|i| if i <= band { i * TYPO } else { FAR }));
    rows.resize(3 * w, FAR);
    let mut best = if m <= band && end.is_none_or(|e| e == 0) {
        m * TYPO
    } else {
        FAR
    };
    let mut before = '\0';
    for (j, c) in key.chars().take(m + band).enumerate() {
        let j = j + 1;
        let (cur, prev, prev2) = ((j % 3) * w, ((j - 1) % 3) * w, ((j + 1) % 3) * w);
        let (lo, hi) = (j.saturating_sub(band).max(1), (j + band).min(m));
        rows[cur + lo - 1] = if j <= band { j * TYPO } else { FAR };
        let mut row_min = rows[cur + lo - 1];
        for i in lo..=hi {
            let cost = if typed[i - 1] == c { 0 } else { TYPO };
            let mut d = (rows[prev + i] + TYPO)
                .min(rows[cur + i - 1] + TYPO)
                .min(rows[prev + i - 1] + cost);
            if i > 1 && j > 1 && typed[i - 1] == before && typed[i - 2] == c {
                d = d.min(rows[prev2 + i - 2] + SWAP);
            }
            rows[cur + i] = d;
            row_min = row_min.min(d);
        }
        if hi < m {
            rows[cur + hi + 1] = FAR;
        }
        if hi == m && end.is_none_or(|e| e == j) {
            best = best.min(rows[cur + m]);
        }
        if row_min > max {
            break;
        }
        before = c;
    }
    (best <= max).then_some(best)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::offline::{english, Data, ENGLISH};

    fn all_words() -> &'static Words {
        static WORDS: std::sync::OnceLock<Words> = std::sync::OnceLock::new();
        WORDS.get_or_init(Words::load)
    }

    fn suggest(typed: &str, limit: usize) -> Vec<Suggestion> {
        all_words().suggest(typed, limit)
    }

    fn words(typed: &str) -> Vec<String> {
        suggest(typed, 5).into_iter().map(|s| s.word).collect()
    }

    #[test]
    fn suggests_words() {
        // The word itself first, then the likeliest completions.
        let dog = words("dog");
        assert_eq!(dog[0], "dog");
        assert!(
            dog.iter().all(|w| w.to_lowercase().starts_with("dog")),
            "{dog:?}"
        );
        assert!(words("serendip").contains(&"serendipity".to_string()));
        // Single words come before phrases.
        assert!(!words("ice")[0].contains(' '));
        assert_eq!(words("ice cr")[0], "ice cream");
        // Shown as written.
        assert!(words("pari").contains(&"Paris".to_string()));
        assert!(words("qwzx").is_empty());
        assert!(words("").is_empty());
        // Inflected forms only when typed in full.
        let geese = suggest("geese", 5);
        assert_eq!(geese[0].base.as_deref(), Some("goose"));
        assert_eq!(suggest("walked", 5)[0].base.as_deref(), Some("walk"));
        assert!(suggest("gee", 5).iter().all(|s| s.base.is_none()));
    }

    #[test]
    fn offers_words_typed_without_accents_as_written() {
        let arleng = words("arleng");
        assert_eq!(
            arleng.iter().filter(|w| *w == "Arlêng").count(),
            1,
            "{arleng:?}"
        );
    }

    #[test]
    fn suggests_words_in_each_language() {
        // English again, as if it were another language.
        let mut other = Dictionary::new(Data::Embedded(ENGLISH));
        other.info.language = "xx".into();
        let words = Words::of(vec![english(), Arc::new(other)]);
        assert!(words.multilingual() && !all_words().multilingual());
        let rows = |typed| {
            words
                .suggest(typed, 5)
                .into_iter()
                .map(|s| (s.word, s.base, s.language))
                .collect::<Vec<_>>()
        };
        let dog = rows("dog");
        assert_eq!(
            dog[..2],
            [
                ("dog".into(), None, "en".into()),
                ("dog".into(), None, "xx".into())
            ]
        );
        // Inflected forms in each language, the preferred one first.
        let geese = rows("geese");
        assert_eq!(
            geese[..2],
            [
                ("geese".into(), Some("goose".into()), "en".into()),
                ("geese".into(), Some("goose".into()), "xx".into())
            ]
        );
    }

    #[test]
    fn suggests_despite_typos() {
        // Swapped, changed, missing and extra letters.
        assert_eq!(words("recieve")[0], "receive");
        assert_eq!(words("seperate")[0], "separate");
        assert_eq!(words("definately")[0], "definitely");
        assert!(words("acomodate").contains(&"accommodate".to_string()));
        assert!(words("embarass").contains(&"embarrass".to_string()));
        // While typing, too.
        assert!(words("necces").contains(&"necessary".to_string()));
        // What's typed correctly still comes first.
        let hous = words("hous");
        assert!(hous.iter().all(|w| w.to_lowercase().starts_with("hous")));
        // A swap is the likeliest typo.
        assert_eq!(words("recie")[0], "receive");
        // Short words are taken as typed.
        assert!(words("qwz").is_empty());
        assert!(words("xqzvw").is_empty());
    }

    #[test]
    fn counts_typos() {
        let count = |typed: &str, key: &str| {
            let typed: Vec<char> = typed.chars().collect();
            typos(&typed, key, 2 * TYPO, false, &mut Vec::new())
        };
        let whole = |typed: &str, key: &str| {
            let typed: Vec<char> = typed.chars().collect();
            typos(&typed, key, 2 * TYPO, true, &mut Vec::new())
        };
        assert_eq!(count("house", "house"), Some(0));
        assert_eq!(count("hous", "household"), Some(0));
        assert_eq!(count("hosue", "house"), Some(SWAP));
        assert_eq!(count("huse", "house"), Some(TYPO));
        assert_eq!(count("hoouse", "house"), Some(TYPO));
        assert_eq!(count("hause", "house"), Some(TYPO));
        assert_eq!(count("haase", "house"), Some(2 * TYPO));
        assert_eq!(count("hhaase", "house"), None);
        assert_eq!(count("accomodate", "accommodate"), Some(TYPO));
        assert_eq!(count("acomodate", "accommodate"), Some(2 * TYPO));
        assert_eq!(count("zzzzz", "house"), None);
        // All of the key, when whole.
        assert_eq!(whole("hous", "household"), None);
        assert_eq!(whole("hous", "house"), Some(TYPO));
        assert_eq!(whole("emobdy", "embody"), Some(SWAP));
        assert_eq!(whole("emobdy", "embodying"), None);
    }

    #[test]
    fn corrects_typos() {
        let correct = |typed: &str| all_words().correct(typed);
        assert_eq!(correct("emobdy"), Some("embody"));
        assert_eq!(correct("embnody"), Some("embody"));
        assert_eq!(correct("recieve"), Some("receive"));
        assert_eq!(correct("seperate"), Some("separate"));
        assert_eq!(correct("definately"), Some("definitely"));
        assert_eq!(correct("hosue"), Some("house"));
        // Short words are taken as typed.
        assert_eq!(correct("qwz"), None);
        assert_eq!(correct("xqzvwkj"), None);
    }

    #[test]
    fn suggests_quickly() {
        let start = std::time::Instant::now();
        let words = all_words();
        eprintln!("load: {:?}", start.elapsed());
        let start = std::time::Instant::now();
        let typed = [
            "co",
            "re",
            "st",
            "pr",
            "ma",
            "recie",
            "seperat",
            "definately",
        ];
        for t in typed {
            assert!(!words.suggest(t, 5).is_empty(), "{t}");
        }
        let each = start.elapsed() / typed.len() as u32;
        eprintln!("suggest: {each:?} per query");
    }
}
