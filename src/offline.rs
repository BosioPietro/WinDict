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
}

/// How many blocks of entries a search for suggestions reads at most, so
/// that a short, common prefix stays quick.
const SUGGEST_BLOCKS: usize = 256;

/// Up to `limit` words starting with `prefix` (lowercase), the likeliest
/// first: the word itself, then single words before phrases, then the ones
/// with the most meanings, which tend to be the common ones.
pub fn suggest(prefix: &str, limit: usize) -> Vec<Suggestion> {
    match Dict::open(DATA) {
        Some(dict) if !prefix.is_empty() => dict.suggest(prefix, limit),
        _ => Vec::new(),
    }
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

    fn suggest(&self, prefix: &str, limit: usize) -> Vec<Suggestion> {
        // Entries are sorted, so the matches are consecutive.
        let first = self.block_of(prefix).unwrap_or(0);
        let last = self.entry_blocks.min(first + SUGGEST_BLOCKS);
        let mut found = Vec::new();
        'blocks: for block in first..last {
            let Some(text) = self.entry_block(block) else {
                break;
            };
            for record in text.split('\x1e') {
                let key = record.split('\n').next().unwrap_or_default();
                if !key.starts_with(prefix) {
                    if key > prefix {
                        break 'blocks;
                    }
                    continue;
                }
                let entry = Entry::parse(record);
                let senses: usize = entry.senses.iter().map(|(_, ids)| ids.len()).sum();
                let exact = key == prefix;
                // An inflected form ("geese") only when it's what was typed.
                let base = match (senses, entry.redirects.first()) {
                    (0, Some(base)) if exact => Some(base.clone()),
                    (0, _) => continue,
                    _ => None,
                };
                let word = entry.display.unwrap_or_else(|| key.to_string());
                let rank = (
                    !exact,
                    key.contains(' '),
                    std::cmp::Reverse(senses),
                    key.len(),
                );
                found.push((rank, Suggestion { word, base }));
            }
        }
        // Stable: equal ranks stay in alphabetical order.
        found.sort_by_key(|(rank, _)| *rank);
        found.into_iter().take(limit).map(|(_, s)| s).collect()
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

    fn words(prefix: &str) -> Vec<String> {
        suggest(prefix, 5).into_iter().map(|s| s.word).collect()
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
        assert!(suggest("gees", 5).iter().all(|s| s.base.is_none()));
    }

    #[test]
    fn suggests_quickly() {
        let start = std::time::Instant::now();
        for prefix in ["co", "re", "st", "pr", "ma"] {
            assert_eq!(suggest(prefix, 5).len(), 5);
        }
        let each = start.elapsed() / 5;
        eprintln!("suggest: {each:?} per prefix");
    }

    #[test]
    fn formats_senses() {
        assert_eq!(sentence("a canine"), "A canine.");
        assert_eq!(sentence("done!"), "done!".replacen('d', "D", 1));
    }
}
