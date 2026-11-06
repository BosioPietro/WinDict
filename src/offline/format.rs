//! Reading a dictionary file: its header, the compressed blocks of entries
//! and of synsets they point to, the definitions built from them, and the
//! translations the entries and the reverse blocks hold (see
//! `tools/wdict.py` for the layout).

use crate::dictionary::{same_word, Definition, Detail, Meaning, Sense, MAX_ANTONYMS};
use crate::i18n::{fill, tr};

use super::sentence;

/// The header of a version 4 file, the shortest there is: it's as much as
/// a file must have to be looked at.
pub(super) const HEADER: usize = 28;
/// A version 5 header has the count of reverse blocks after it.
const HEADER_V5: usize = 32;

/// The most a block may inflate to. They're a few KB; this keeps a broken
/// file from taking all the memory.
const MAX_BLOCK: usize = 1 << 20;

/// A table of blocks: where each one's row is in the header, and how many
/// there are. Entry and reverse rows are 12 bytes (the block's first key,
/// its data and its length), synset rows 8 (no key).
#[derive(Clone, Copy)]
struct Table {
    start: usize,
    blocks: usize,
    row: usize,
}

pub(super) struct Dict<'a> {
    data: &'a [u8],
    entries: Table,
    synsets: Table,
    /// The translations into the dictionary's language: none in a version
    /// 4 file.
    reverse: Table,
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
    /// The word in other languages, by their code, best first.
    pub(super) translations: Vec<(String, Vec<String>)>,
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
        let u32_at = |offset: usize| {
            data.get(offset..offset + 4)
                .map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
        };
        let header = match u32_at(4) {
            4 => HEADER,
            5 if data.len() >= HEADER_V5 => HEADER_V5,
            _ => return None,
        };
        let (entry_blocks, synset_blocks) = (u32_at(8), u32_at(12));
        let reverse_blocks = if header == HEADER_V5 { u32_at(28) } else { 0 };
        let entries = Table {
            start: header,
            blocks: entry_blocks,
            row: 12,
        };
        let synsets = Table {
            start: entries.start.checked_add(entry_blocks.checked_mul(12)?)?,
            blocks: synset_blocks,
            row: 8,
        };
        let reverse = Table {
            start: synsets.start.checked_add(synset_blocks.checked_mul(8)?)?,
            blocks: reverse_blocks,
            row: 12,
        };
        // The tables of blocks must be in the file: a broken header
        // mustn't send a reader through billions of empty ones.
        let tables = reverse.start.checked_add(reverse_blocks.checked_mul(12)?)?;
        if tables > data.len() {
            return None;
        }
        Some(Dict {
            data,
            entries,
            synsets,
            reverse,
            synsets_per_block: u32_at(16).max(1),
        })
    }

    /// How many blocks of entries there are.
    pub(super) fn entry_blocks(&self) -> usize {
        self.entries.blocks
    }

    /// Whether the file carries translations (version 5 and later).
    pub(super) fn translates(&self) -> bool {
        self.u32(4) >= 5
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

    /// The first key of `block` in `table`, whose rows start with where it is.
    fn first_key(&self, table: Table, block: usize) -> &[u8] {
        let start = self.u32(table.start + table.row * block);
        let rest = self.data.get(start..).unwrap_or_default();
        &rest[..rest.iter().position(|&b| b == 0).unwrap_or(rest.len())]
    }

    /// The last block of `table` whose first key is <= `key`.
    fn block_of(&self, table: Table, key: &str) -> Option<usize> {
        let (mut lo, mut hi) = (0, table.blocks);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.first_key(table, mid) <= key.as_bytes() {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo.checked_sub(1)
    }

    /// The records of `block` in `table`, a keyed one, inflated.
    fn keyed_block(&self, table: Table, block: usize) -> Option<String> {
        let row = table.start + table.row * block;
        self.inflate(self.u32(row + 4), self.u32(row + 8))
    }

    pub(super) fn entry_block(&self, block: usize) -> Option<String> {
        self.keyed_block(self.entries, block)
    }

    /// The record with `key` in `table`, as stored: the key, then one line
    /// per field.
    fn record(&self, table: Table, key: &str) -> Option<String> {
        let text = self.keyed_block(table, self.block_of(table, key)?)?;
        text.split('\x1e')
            .find(|r| r.split('\n').next() == Some(key))
            .map(str::to_string)
    }

    fn entry(&self, key: &str) -> Option<Entry> {
        self.entry_spelled(key)
            // Some keys keep the curly apostrophe, which reading text folds.
            .or_else(|| {
                let curly = key.replace('\'', "\u{2019}");
                (curly != key).then(|| self.entry_spelled(&curly))?
            })
            // A leading apostrophe may only have opened a quote.
            .or_else(|| self.entry(key.strip_prefix('\'')?))
    }

    fn entry_spelled(&self, key: &str) -> Option<Entry> {
        self.record(self.entries, key)
            .map(|record| Entry::parse(&record))
    }

    /// What `word` is a form of, if it isn't a word in its own right:
    /// "geese" → "goose", "walked" → "walk".
    pub(super) fn base_of(&self, word: &str) -> Option<String> {
        let entry = self.entry(word).unwrap_or_default();
        if !entry.senses.is_empty() {
            return None;
        }
        self.base(&entry).map(|(base, _)| base.to_string())
    }

    /// The first word `entry` is a form of that's defined, with its entry:
    /// "geese" → "goose".
    fn base<'e>(&self, entry: &'e Entry) -> Option<(&'e str, Entry)> {
        entry
            .redirects
            .iter()
            .find_map(|base| Some((base.as_str(), self.defined(base)?)))
    }

    /// `word`, or the word it's a form of, or the accented word it was
    /// typed as, in the language with code `to`, as the dictionary has it:
    /// its translations, best first. A word may have those without a
    /// definition, when the extractor couldn't make out its page's glosses.
    pub(super) fn translations(&self, word: &str, to: &str) -> Option<Vec<String>> {
        let of = |entry: &Entry| {
            entry
                .translations
                .iter()
                .find(|(code, _)| code == to)
                .map(|(_, words)| words.clone())
                .filter(|words| !words.is_empty())
        };
        for candidate in spellings(word) {
            let Some(entry) = self.entry(&candidate) else {
                continue;
            };
            if let Some(words) = of(&entry) {
                return Some(words);
            }
            let followed = entry.redirects.iter().chain(&entry.spellings);
            if let Some(words) = followed.filter_map(|w| of(&self.entry(w)?)).next() {
                return Some(words);
            }
        }
        None
    }

    /// The words of the dictionary's language that `word`, in the language
    /// with code `from`, translates to, best first: the reverse blocks.
    pub(super) fn reverse(&self, from: &str, word: &str) -> Option<Vec<String>> {
        let key = format!("{from}\t{}", word.to_lowercase());
        let record = self.record(self.reverse, &key)?;
        let words: Vec<String> = record
            .split('\n')
            .skip(1)
            .filter(|w| !w.is_empty())
            .map(str::to_string)
            .collect();
        (!words.is_empty()).then_some(words)
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
                Some('T') => {
                    if let Some((code, words)) = chars.as_str().split_once(' ') {
                        let words = words
                            .split('|')
                            .filter(|w| !w.is_empty())
                            .map(str::to_string)
                            .collect();
                        entry.translations.push((code.to_string(), words));
                    }
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
        if block >= self.synsets.blocks {
            return None;
        }
        let row = self.synsets.start + self.synsets.row * block;
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

    /// The entry `word` is looked up as: its own, the base form's when it's
    /// an inflected form ("ran", "walked"), or the accented word's when it
    /// was typed without its accents ("arleng"); under the key it's stored
    /// by, and with a note on what was followed, if anything was.
    fn lookup_entry(&self, word: &str) -> Option<(String, Entry, Option<String>)> {
        for candidate in spellings(word) {
            let Some(entry) = self.entry(&candidate) else {
                continue;
            };
            if !entry.senses.is_empty() {
                let note = entry
                    .redirects
                    .first()
                    .map(|b| fill(tr().also_form_of, &[("word", b)]));
                return Some((candidate, entry, note));
            }
            // An inflected form ("ran", "walked"): show its base.
            if let Some((base, base_entry)) = self.base(&entry) {
                let note = fill(tr().base_form_of, &[("word", &word)]);
                return Some((base.to_string(), base_entry, Some(note)));
            }
            for spelling in &entry.spellings {
                if let Some(written) = self.defined(spelling) {
                    return Some((spelling.clone(), written, None));
                }
            }
        }
        None
    }

    pub(super) fn lookup(
        &self,
        word: &str,
        detail: Detail,
        source: &'static str,
    ) -> Option<Definition> {
        let (key, entry, note) = self.lookup_entry(word)?;
        self.build(&key, &entry, note, detail, source)
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

/// The word as selected, plus common spelling variants of compounds.
fn spellings(word: &str) -> Vec<String> {
    let mut out = vec![word.to_string()];
    if !word.contains([' ', '-']) {
        return out;
    }
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
pub(super) mod tests {
    use super::*;

    #[test]
    fn compares_words_but_for_case() {
        assert!(same_word("Señor", "señor"));
        assert!(!same_word("senor", "señor"));
    }

    #[test]
    fn opens_only_whole_tables() {
        let header = |version: u32, entry_blocks: u32, synset_blocks: u32| {
            let mut data = b"WDIC".to_vec();
            for n in [version, entry_blocks, synset_blocks, 1, 0, 0] {
                data.extend(n.to_le_bytes());
            }
            data
        };
        assert!(Dict::open(&header(4, 0, 0)).is_some());
        let mut one = header(4, 1, 1);
        one.extend([0; 20]);
        assert!(Dict::open(&one).is_some());
        assert!(Dict::open(&one[..one.len() - 1]).is_none());
        assert!(Dict::open(&header(4, u32::MAX, u32::MAX)).is_none());
        // Version 5 has the count of reverse blocks too, and their table.
        assert!(Dict::open(&header(5, 0, 0)).is_none());
        let mut five = header(5, 0, 0);
        five.extend(1u32.to_le_bytes());
        assert!(Dict::open(&five).is_none());
        five.extend([0; 12]);
        let dict = Dict::open(&five).unwrap();
        assert!(dict.translates() && dict.reverse.blocks == 1);
        assert!(!Dict::open(&header(4, 0, 0)).unwrap().translates());
        assert!(Dict::open(&header(6, 0, 0)).is_none());
    }

    #[test]
    fn lists_compound_spellings() {
        assert_eq!(spellings("e-mail"), ["e-mail", "e mail", "email"]);
        assert_eq!(spellings("ice cream"), ["ice cream", "ice-cream"]);
        assert_eq!(spellings("dog"), ["dog"]);
    }

    #[test]
    fn parses_translations() {
        let entry = Entry::parse("casa\nPn0 1\nTen house|home\nTde Haus\nTxx \nbroken");
        assert_eq!(entry.senses, [('n', vec![0, 1])]);
        assert_eq!(
            entry.translations,
            [
                (
                    "en".to_string(),
                    vec!["house".to_string(), "home".to_string()]
                ),
                ("de".to_string(), vec!["Haus".to_string()]),
                ("xx".to_string(), Vec::new()),
            ]
        );
    }

    /// A version 5 file, written as `tools/wdict.py` writes one, with
    /// `entries` (key, record lines after it), `synsets` (their records)
    /// and `reverse` (key, the words), each in order, one block each.
    pub(crate) fn file(
        info: &str,
        entries: &[(&str, &str)],
        synsets: &[&str],
        reverse: &[(&str, &str)],
    ) -> Vec<u8> {
        let deflate = |text: String| miniz_oxide::deflate::compress_to_vec(text.as_bytes(), 9);
        let records = |rows: &[(&str, &str)]| {
            rows.iter()
                .map(|(key, rest)| format!("{key}\n{rest}"))
                .collect::<Vec<_>>()
                .join("\x1e")
        };
        let entry_block = deflate(records(entries));
        let synset_block = deflate(synsets.join("\x1e"));
        let reverse_block = deflate(records(reverse));
        let has_reverse = !reverse.is_empty();
        let header = HEADER_V5 + 12 + 8 + if has_reverse { 12 } else { 0 };
        let mut keys = entries[0].0.as_bytes().to_vec();
        keys.push(0);
        let entry_key_at = header;
        let reverse_key_at = header + keys.len();
        if has_reverse {
            keys.extend(reverse[0].0.as_bytes());
            keys.push(0);
        }
        let info_at = header + keys.len();
        let data_at = info_at + info.len();
        let mut out = b"WDIC".to_vec();
        let counts = [5, 1, 1, 64, info_at, info.len(), usize::from(has_reverse)];
        for n in counts {
            out.extend((n as u32).to_le_bytes());
        }
        let mut row = |values: &[usize]| {
            for &v in values {
                out.extend((v as u32).to_le_bytes());
            }
        };
        row(&[entry_key_at, data_at, entry_block.len()]);
        row(&[data_at + entry_block.len(), synset_block.len()]);
        if has_reverse {
            let at = data_at + entry_block.len() + synset_block.len();
            row(&[reverse_key_at, at, reverse_block.len()]);
        }
        out.extend(keys);
        out.extend(info.as_bytes());
        out.extend(entry_block);
        out.extend(synset_block);
        if has_reverse {
            out.extend(reverse_block);
        }
        out
    }

    #[test]
    fn reads_version_5_files() {
        let data = file(
            "language=it\nname=Test",
            &[
                ("casa", "Pn0\nTen house|home\nTde Haus"),
                ("case", "Rcasa"),
                ("gatto", "Pn1\nTen cat"),
                // Translations, but nothing to define it by.
                ("pane", "Ten bread"),
                ("pani", "Rpane"),
            ],
            &["casa\n\nedificio\nLa casa è grande.", "gatto\n\nfelino"],
            &[("en\tcat", "gatto"), ("en\thouse", "casa\ndimora")],
        );
        let dict = Dict::open(&data).unwrap();
        assert!(dict.translates());
        assert_eq!(dict.translations("pane", "en").unwrap(), ["bread"]);
        assert_eq!(dict.translations("pani", "en").unwrap(), ["bread"]);
        let detail = Detail {
            senses: 1,
            synonyms: 0,
            examples: false,
        };
        assert!(dict.lookup("pane", detail, "Test").is_none());
        assert!(dict.has("pane") && dict.defined("pane").is_none());
        assert_eq!(dict.info(), Some("language=it\nname=Test"));
        let detail = Detail {
            senses: 4,
            synonyms: 6,
            examples: true,
        };
        let casa = dict.lookup("casa", detail, "Test").unwrap();
        assert_eq!(casa.meanings[0].senses[0].text, "Edificio.");
        assert_eq!(dict.translations("casa", "en").unwrap(), ["house", "home"]);
        assert_eq!(dict.translations("casa", "de").unwrap(), ["Haus"]);
        assert_eq!(dict.translations("casa", "fr"), None);
        // Through an inflected form, too.
        assert_eq!(dict.translations("case", "en").unwrap(), ["house", "home"]);
        assert_eq!(dict.translations("gatto", "en").unwrap(), ["cat"]);
        assert_eq!(dict.translations("cane", "en"), None);
        // And the other way round, whatever the case.
        assert_eq!(dict.reverse("en", "House").unwrap(), ["casa", "dimora"]);
        assert_eq!(dict.reverse("en", "cat").unwrap(), ["gatto"]);
        assert_eq!(dict.reverse("de", "cat"), None);
        assert_eq!(dict.reverse("en", "dog"), None);
    }
}
