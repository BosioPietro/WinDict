//! Reading a dictionary file: its header, the compressed blocks of entries
//! and of synsets they point to, and the definitions built from them.

use crate::dictionary::{Definition, Detail, Meaning, Sense, MAX_ANTONYMS};
use crate::i18n::{fill, tr};

use super::sentence;

pub(super) const HEADER: usize = 28;

/// The most a block may inflate to. They're a few KB; this keeps a broken
/// file from taking all the memory.
const MAX_BLOCK: usize = 1 << 20;

pub(super) struct Dict<'a> {
    data: &'a [u8],
    pub(super) entry_blocks: usize,
    synset_blocks: usize,
    synsets_per_block: usize,
}

#[derive(Default)]
pub(super) struct Entry {
    pub(super) display: Option<String>,
    pub(super) redirects: Vec<String>,
    /// How it's written, when it's typed without its accents.
    pub(super) spellings: Vec<String>,
    /// Part of speech and synset ids, most-used part of speech first.
    pub(super) senses: Vec<(char, Vec<usize>)>,
}

struct Synset {
    lemmas: Vec<String>,
    /// Each lemma's opposites, in the order of `lemmas`.
    antonyms: Vec<Vec<String>>,
    definition: String,
    examples: Vec<String>,
}

impl<'a> Dict<'a> {
    pub(super) fn open(data: &'a [u8]) -> Option<Dict<'a>> {
        if data.len() < HEADER || &data[..4] != b"WDIC" {
            return None;
        }
        let dict = Dict {
            data,
            entry_blocks: 0,
            synset_blocks: 0,
            synsets_per_block: 0,
        };
        if dict.u32(4) != 4 {
            return None;
        }
        Some(Dict {
            entry_blocks: dict.u32(8),
            synset_blocks: dict.u32(12),
            synsets_per_block: dict.u32(16).max(1),
            ..dict
        })
    }

    /// What the dictionary says about itself: "name=value" lines.
    pub(super) fn info(&self) -> Option<&'a str> {
        let (offset, len) = (self.u32(20), self.u32(24));
        std::str::from_utf8(self.data.get(offset..offset.checked_add(len)?)?).ok()
    }

    fn u32(&self, offset: usize) -> usize {
        self.data
            .get(offset..offset + 4)
            .map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
    }

    fn inflate(&self, offset: usize, len: usize) -> Option<String> {
        let packed = self.data.get(offset..offset.checked_add(len)?)?;
        let raw = miniz_oxide::inflate::decompress_to_vec_with_limit(packed, MAX_BLOCK).ok()?;
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

    pub(super) fn entry_block(&self, block: usize) -> Option<String> {
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
    pub(super) fn base_of(&self, word: &str) -> Option<String> {
        let entry = self.entry(word).unwrap_or_default();
        if !entry.senses.is_empty() {
            return None;
        }
        entry
            .redirects
            .into_iter()
            .find(|base| self.defined(base).is_some())
    }
}

impl Entry {
    /// An entry's record, as stored: its key, then one line per field.
    pub(super) fn parse(record: &str) -> Entry {
        let mut entry = Entry::default();
        for line in record.split('\n').skip(1) {
            let mut chars = line.chars();
            match chars.next() {
                Some('W') => entry.display = Some(chars.as_str().to_string()),
                Some('R') => entry.redirects.push(chars.as_str().to_string()),
                Some('S') => entry.spellings.push(chars.as_str().to_string()),
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
        let antonyms = lines
            .next()?
            .split('|')
            .map(|a| {
                a.split('\t')
                    .filter(|a| !a.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .collect();
        let definition = lines.next()?.to_string();
        let examples = lines.map(str::to_string).collect();
        Some(Synset {
            lemmas,
            antonyms,
            definition,
            examples,
        })
    }

    /// Whether `key` is a word of the dictionary's language: one it defines,
    /// a form of one, or one spelt without its accents.
    pub(super) fn has(&self, key: &str) -> bool {
        self.entry(key).is_some()
    }

    pub(super) fn defined(&self, key: &str) -> Option<Entry> {
        self.entry(key).filter(|e| !e.senses.is_empty())
    }

    pub(super) fn lookup(
        &self,
        word: &str,
        detail: Detail,
        source: &'static str,
    ) -> Option<Definition> {
        let build = |key: &str, entry: &Entry, note: Option<String>| {
            self.build(key, entry, note, detail, source)
        };
        for candidate in spellings(word) {
            let Some(entry) = self.entry(&candidate) else {
                continue;
            };
            if !entry.senses.is_empty() {
                let note = entry
                    .redirects
                    .first()
                    .map(|b| fill(tr().also_form_of, &[("word", b)]));
                return build(&candidate, &entry, note);
            }
            // An inflected form ("ran", "walked"): show its base.
            for base in &entry.redirects {
                if let Some(base_entry) = self.defined(base) {
                    return build(
                        base,
                        &base_entry,
                        Some(fill(tr().base_form_of, &[("word", &word)])),
                    );
                }
            }
            // Typed without its accents ("arleng"): show it as it's written.
            for spelling in &entry.spellings {
                if let Some(written) = self.defined(spelling) {
                    return build(spelling, &written, None);
                }
            }
        }
        None
    }

    fn build(
        &self,
        key: &str,
        entry: &Entry,
        note: Option<String>,
        detail: Detail,
        source: &'static str,
    ) -> Option<Definition> {
        let mut meanings = Vec::new();
        for (pos, ids) in &entry.senses {
            let mut senses = Vec::new();
            let mut synonyms: Vec<String> = Vec::new();
            let mut antonyms: Vec<String> = Vec::new();
            for synset in ids
                .iter()
                .take(detail.senses.max(1))
                .filter_map(|&id| self.synset(id))
            {
                for (i, lemma) in synset.lemmas.iter().enumerate() {
                    if same_word(lemma, key) {
                        // Opposites belong to a word, not to its synonyms.
                        for a in synset.antonyms.get(i).into_iter().flatten() {
                            if antonyms.len() < MAX_ANTONYMS && !antonyms.contains(a) {
                                antonyms.push(a.clone());
                            }
                        }
                    } else if synonyms.len() < detail.synonyms
                        && !synonyms.iter().any(|s| same_word(s, lemma))
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
                'p' => "phrase",
                'i' => "interjection",
                'o' => "pronoun",
                'e' => "preposition",
                'c' => "conjunction",
                't' => "article",
                'b' => "abbreviation",
                _ => "noun",
            };
            meanings.push(Meaning {
                part_of_speech: part_of_speech.into(),
                senses,
                synonyms,
                antonyms,
            });
        }
        if meanings.is_empty() {
            return None;
        }
        Some(Definition {
            word: entry.display.clone().unwrap_or_else(|| key.to_string()),
            language: Default::default(),
            phonetic: None,
            meanings,
            note,
            sources: vec![source],
            others: Vec::new(),
        })
    }
}

/// Whether `a` and `b` are the same word but for case, in any alphabet.
fn same_word(a: &str, b: &str) -> bool {
    a.chars()
        .flat_map(char::to_lowercase)
        .eq(b.chars().flat_map(char::to_lowercase))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_words_but_for_case() {
        assert!(same_word("Señor", "señor"));
        assert!(!same_word("senor", "señor"));
    }

    #[test]
    fn lists_compound_spellings() {
        assert_eq!(spellings("e-mail"), ["e-mail", "e mail", "email"]);
        assert_eq!(spellings("ice cream"), ["ice cream", "ice-cream"]);
        assert_eq!(spellings("dog"), ["dog"]);
    }
}
