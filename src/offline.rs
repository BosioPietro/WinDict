//! The bundled dictionary (Open English WordNet), built by `tools/build_dict.py`.
//!
//! The file is embedded in the executable and read in place: Windows only
//! pages in the parts a lookup touches (the block index plus a few small
//! compressed blocks), so it costs no memory while the app sits idle.

use crate::dictionary::{Definition, Meaning, Sense, WORDNET};

static DATA: &[u8] = include_bytes!("../data/english.wdict");

const HEADER: usize = 20;

pub fn lookup(word: &str, max_senses: usize, max_synonyms: usize) -> Option<Definition> {
    Dict::open(DATA)?.lookup(word, max_senses, max_synonyms)
}

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
}

/// Every word the dictionary defines, to suggest from as the search box is
/// typed in. Building it reads the whole dictionary (a few tens of ms), so
/// it's made when the search box first needs it and dropped with the box.
pub struct Words {
    /// Each word's key, followed by its display form when that differs.
    text: String,
    /// In key order.
    words: Vec<Word>,
}

struct Word {
    start: u32,
    key_len: u16,
    display_len: u16,
    senses: u16,
}

impl Words {
    pub fn load() -> Words {
        let mut words = Words {
            text: String::new(),
            words: Vec::new(),
        };
        let Some(dict) = Dict::open(DATA) else {
            return words;
        };
        for block in 0..dict.entry_blocks {
            let Some(text) = dict.entry_block(block) else {
                continue;
            };
            for record in text.split('\x1e') {
                let key = record.split('\n').next().unwrap_or_default();
                let entry = Entry::parse(record);
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
                });
                words.text.push_str(key);
                words.text.push_str(&display);
            }
        }
        words
    }

    fn key(&self, word: &Word) -> &str {
        let start = word.start as usize;
        &self.text[start..start + word.key_len as usize]
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
        let mut out: Vec<Suggestion> = found
            .into_iter()
            .take(limit)
            .map(|((_, typos, ..), i)| Suggestion {
                word: self.display(&self.words[i]).to_string(),
                base: None,
                typed: if typos == 0 { typed_chars.len() } else { 0 },
            })
            .collect();
        // An inflected form ("geese", "walked") is offered, as what it's a
        // form of, only once it's typed in full.
        let defined = out.first().is_some_and(|s| s.word.to_lowercase() == typed);
        if !defined {
            if let Some(base) = Dict::open(DATA).and_then(|d| d.base_of(typed)) {
                out.insert(
                    0,
                    Suggestion {
                        word: typed.to_string(),
                        base: Some(base),
                        typed: typed_chars.len(),
                    },
                );
                out.truncate(limit);
            }
        }
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

struct Dict<'a> {
    data: &'a [u8],
    entry_blocks: usize,
    synset_blocks: usize,
    synsets_per_block: usize,
}

#[derive(Default)]
struct Entry {
    display: Option<String>,
    redirects: Vec<String>,
    /// Part of speech and synset ids, most-used part of speech first.
    senses: Vec<(char, Vec<usize>)>,
}

struct Synset {
    lemmas: Vec<String>,
    definition: String,
    examples: Vec<String>,
}

/// Suffix rules from WordNet's morphy: (suffix, replacement).
const DETACHMENTS: &[(&str, &str)] = &[
    ("ies", "y"),
    ("ses", "s"),
    ("xes", "x"),
    ("zes", "z"),
    ("ches", "ch"),
    ("shes", "sh"),
    ("men", "man"),
    ("es", "e"),
    ("es", ""),
    ("s", ""),
    ("ing", "e"),
    ("ing", ""),
    ("ed", "e"),
    ("ed", ""),
    ("est", "e"),
    ("est", ""),
    ("er", "e"),
    ("er", ""),
];

impl<'a> Dict<'a> {
    fn open(data: &'a [u8]) -> Option<Dict<'a>> {
        if data.len() < HEADER || &data[..4] != b"WDIC" {
            return None;
        }
        let dict = Dict {
            data,
            entry_blocks: 0,
            synset_blocks: 0,
            synsets_per_block: 0,
        };
        if dict.u32(4) != 2 {
            return None;
        }
        Some(Dict {
            entry_blocks: dict.u32(8),
            synset_blocks: dict.u32(12),
            synsets_per_block: dict.u32(16).max(1),
            ..dict
        })
    }

    fn u32(&self, offset: usize) -> usize {
        self.data
            .get(offset..offset + 4)
            .map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
    }

    fn inflate(&self, offset: usize, len: usize) -> Option<String> {
        let packed = self.data.get(offset..offset + len)?;
        let raw = miniz_oxide::inflate::decompress_to_vec(packed).ok()?;
        String::from_utf8(raw).ok()
    }

    fn first_key(&self, block: usize) -> &[u8] {
        let start = self.u32(HEADER + 12 * block);
        let rest = self.data.get(start..).unwrap_or_default();
        &rest[..rest.iter().position(|&b| b == 0).unwrap_or(rest.len())]
    }

    /// The last block whose first key is <= `key`.
    fn block_of(&self, key: &str) -> Option<usize> {
        let (mut lo, mut hi) = (0, self.entry_blocks);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.first_key(mid) <= key.as_bytes() {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo.checked_sub(1)
    }

    fn entry_block(&self, block: usize) -> Option<String> {
        let row = HEADER + 12 * block;
        self.inflate(self.u32(row + 4), self.u32(row + 8))
    }

    fn entry(&self, key: &str) -> Option<Entry> {
        let text = self.entry_block(self.block_of(key)?)?;
        let record = text
            .split('\x1e')
            .find(|r| r.split('\n').next() == Some(key))?;
        Some(Entry::parse(record))
    }

    /// What `word` is a form of, if it isn't a word in its own right:
    /// "geese" → "goose", "walked" → "walk".
    fn base_of(&self, word: &str) -> Option<String> {
        let entry = self.entry(word).unwrap_or_default();
        if !entry.senses.is_empty() {
            return None;
        }
        entry
            .redirects
            .into_iter()
            .chain(inflection_bases(word))
            .find(|base| self.defined(base).is_some())
    }
}

impl Entry {
    /// An entry's record, as stored: its key, then one line per field.
    fn parse(record: &str) -> Entry {
        let mut entry = Entry::default();
        for line in record.split('\n').skip(1) {
            let mut chars = line.chars();
            match chars.next() {
                Some('W') => entry.display = Some(chars.as_str().to_string()),
                Some('R') => entry.redirects.push(chars.as_str().to_string()),
                Some('P') => {
                    let pos = chars.next().unwrap_or('n');
                    let ids = chars
                        .as_str()
                        .split(' ')
                        .filter_map(|id| id.parse().ok())
                        .collect();
                    entry.senses.push((pos, ids));
                }
                _ => {}
            }
        }
        entry
    }
}

impl Dict<'_> {
    fn synset(&self, id: usize) -> Option<Synset> {
        let block = id / self.synsets_per_block;
        if block >= self.synset_blocks {
            return None;
        }
        let row = HEADER + 12 * self.entry_blocks + 8 * block;
        let text = self.inflate(self.u32(row), self.u32(row + 4))?;
        let record = text.split('\x1e').nth(id % self.synsets_per_block)?;
        let mut lines = record.split('\n');
        let lemmas = lines.next()?.split('|').map(str::to_string).collect();
        let definition = lines.next()?.to_string();
        let examples = lines.map(str::to_string).collect();
        Some(Synset {
            lemmas,
            definition,
            examples,
        })
    }

    fn defined(&self, key: &str) -> Option<Entry> {
        self.entry(key).filter(|e| !e.senses.is_empty())
    }

    fn lookup(&self, word: &str, max_senses: usize, max_synonyms: usize) -> Option<Definition> {
        let build = |key: &str, entry: &Entry, note: Option<String>| {
            self.build(key, entry, note, max_senses, max_synonyms)
        };
        for candidate in spellings(word) {
            let Some(entry) = self.entry(&candidate) else {
                continue;
            };
            if !entry.senses.is_empty() {
                let note = entry
                    .redirects
                    .first()
                    .map(|b| format!("Also a form of \u{201C}{b}\u{201D}"));
                return build(&candidate, &entry, note);
            }
            // An irregular form ("ran", "geese"): show its base.
            for base in &entry.redirects {
                if let Some(base_entry) = self.defined(base) {
                    return build(
                        base,
                        &base_entry,
                        Some(format!("Base form of \u{201C}{word}\u{201D}")),
                    );
                }
            }
        }
        // Regular inflections ("cats", "walked", "faster").
        for candidate in inflection_bases(word) {
            if let Some(entry) = self.defined(&candidate) {
                return build(
                    &candidate,
                    &entry,
                    Some(format!("Base form of \u{201C}{word}\u{201D}")),
                );
            }
        }
        None
    }

    fn build(
        &self,
        key: &str,
        entry: &Entry,
        note: Option<String>,
        max_senses: usize,
        max_synonyms: usize,
    ) -> Option<Definition> {
        let mut meanings = Vec::new();
        for (pos, ids) in &entry.senses {
            let mut senses = Vec::new();
            let mut synonyms: Vec<String> = Vec::new();
            for synset in ids
                .iter()
                .take(max_senses.max(1))
                .filter_map(|&id| self.synset(id))
            {
                for lemma in &synset.lemmas {
                    if synonyms.len() < max_synonyms
                        && !lemma.eq_ignore_ascii_case(key)
                        && !synonyms.iter().any(|s| s.eq_ignore_ascii_case(lemma))
                    {
                        synonyms.push(lemma.clone());
                    }
                }
                senses.push(Sense {
                    text: sentence(&synset.definition),
                    example: synset.examples.into_iter().next(),
                });
            }
            if senses.is_empty() {
                continue;
            }
            let part_of_speech = match pos {
                'v' => "verb",
                'a' | 's' => "adjective",
                'r' => "adverb",
                _ => "noun",
            };
            meanings.push(Meaning {
                part_of_speech: part_of_speech.into(),
                senses,
                synonyms,
            });
        }
        if meanings.is_empty() {
            return None;
        }
        Some(Definition {
            word: entry.display.clone().unwrap_or_else(|| key.to_string()),
            phonetic: None,
            meanings,
            note,
            sources: vec![WORDNET],
        })
    }
}

/// The word as selected, plus common spelling variants of compounds.
fn spellings(word: &str) -> Vec<String> {
    let mut out = vec![word.to_string()];
    for v in [
        word.replace('-', " "),
        word.replace(' ', "-"),
        word.replace('-', ""),
    ] {
        if !out.contains(&v) {
            out.push(v);
        }
    }
    out
}

fn inflection_bases(word: &str) -> Vec<String> {
    // For phrases, inflect the first word ("looking up") and the last ("ice creams").
    let words: Vec<&str> = word.split(' ').collect();
    let mut out = Vec::new();
    let mut push = |candidate: String| {
        if candidate != word && !candidate.is_empty() && !out.contains(&candidate) {
            out.push(candidate);
        }
    };
    let positions = if words.len() > 1 {
        vec![0, words.len() - 1]
    } else {
        vec![0]
    };
    for (suffix, replacement) in DETACHMENTS {
        for &i in &positions {
            let Some(stem) = words[i].strip_suffix(suffix) else {
                continue;
            };
            if stem.chars().count() >= 2 {
                let base = format!("{stem}{replacement}");
                let mut w = words.clone();
                w[i] = &base;
                push(w.join(" "));
            }
        }
    }
    out
}

/// "a domesticated canid" → "A domesticated canid."
fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    let mut out: String = match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => return String::new(),
    };
    if !out.ends_with(['.', '!', '?', ')']) {
        out.push('.');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(word: &str) -> Definition {
        lookup(word, 4, 6).unwrap_or_else(|| panic!("{word} not found"))
    }

    #[test]
    fn finds_plain_words() {
        let dog = find("dog");
        assert_eq!(dog.meanings[0].part_of_speech, "noun");
        assert!(
            dog.meanings[0].senses[0].text.contains("domesticated"),
            "{dog:?}"
        );
        assert!(dog.note.is_none());
        assert_eq!(find("run").meanings[0].part_of_speech, "verb");
        assert_eq!(find("ice cream").word, "ice cream");
        assert_eq!(find("paris").word, "Paris");
    }

    #[test]
    fn resolves_inflections() {
        assert_eq!(find("ran").word, "run");
        assert_eq!(find("geese").word, "goose");
        assert_eq!(find("cats").word, "cat");
        assert_eq!(find("walked").word, "walk");
        assert!(find("walked").note.unwrap().contains("walked"));
        // "stopped" is also an adjective in its own right.
        let stopped = find("stopped");
        assert_eq!(stopped.word, "stopped");
        assert!(stopped.note.unwrap().contains("stop"));
    }

    #[test]
    fn handles_missing_words() {
        assert!(lookup("qwzxv", 4, 6).is_none());
        assert!(lookup("", 4, 6).is_none());
        assert!(lookup("zzzzzzzzz", 4, 6).is_none());
        assert!(lookup("a", 4, 6).is_some());
    }

    #[test]
    fn guesses_inflection_bases() {
        let bases = inflection_bases("cities");
        assert_eq!(bases[0], "city");
        assert!(bases.contains(&"citie".to_string()));
        // Phrases inflect their first and last words.
        assert!(inflection_bases("looking up").contains(&"look up".to_string()));
        assert!(inflection_bases("ice creams").contains(&"ice cream".to_string()));
        // Stems need at least two letters.
        assert!(inflection_bases("is").is_empty());
        assert!(!inflection_bases("bed").contains(&"b".to_string()));
    }

    #[test]
    fn lists_compound_spellings() {
        assert_eq!(spellings("e-mail"), ["e-mail", "e mail", "email"]);
        assert_eq!(spellings("ice cream"), ["ice cream", "ice-cream"]);
        assert_eq!(spellings("dog"), ["dog"]);
    }

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

    #[test]
    fn formats_senses() {
        assert_eq!(sentence("a canine"), "A canine.");
        assert_eq!(sentence("done!"), "done!".replacen('d', "D", 1));
    }
}
