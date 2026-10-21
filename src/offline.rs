//! The offline dictionaries, in the format `tools/build_dict.py` describes.
//!
//! English (Open English WordNet) is embedded in the executable; other
//! languages are read from `%LOCALAPPDATA%\WinDict\dicts\<language>.wdict`.
//! Either way the file is read in place: Windows only pages in the parts a
//! lookup touches (the block index plus a few small compressed blocks), so
//! it costs no memory while the app sits idle.

use std::fs::File;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock, RwLock};

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Memory::{
    CreateFileMappingW, MapViewOfFile, UnmapViewOfFile, FILE_MAP_READ, MEMORY_MAPPED_VIEW_ADDRESS,
    PAGE_READONLY,
};

use crate::dictionary::{self, percent_encode, Definition, Detail, Meaning, Sense, MAX_ANTONYMS};

static ENGLISH: &[u8] = include_bytes!("../data/english.wdict");

const HEADER: usize = 28;

/// The most a block may inflate to. They're a few KB; this keeps a broken
/// file from taking all the memory.
const MAX_BLOCK: usize = 1 << 20;

/// The dictionaries lookups use, in order of preference, which the
/// `languages` setting picks.
static ACTIVE: RwLock<Vec<Arc<Dictionary>>> = RwLock::new(Vec::new());

/// `word` in every language it's in, the preferred one first.
pub fn lookup(word: &str, detail: Detail) -> Option<Definition> {
    let found = active()
        .iter()
        .filter_map(|d| d.lookup(word, detail))
        .collect();
    dictionary::combine(found, &languages())
}

/// The dictionaries lookups use, in order of preference: English until
/// [`set_languages`] picks others.
pub fn active() -> Vec<Arc<Dictionary>> {
    match ACTIVE.read() {
        Ok(active) if !active.is_empty() => active.clone(),
        _ => vec![english()],
    }
}

/// The languages of the dictionaries in use, in order of preference.
pub fn languages() -> Vec<String> {
    active().iter().map(|d| d.language().to_string()).collect()
}

/// Looks words up in `languages` ("en", "it"), in that order of preference:
/// English is built in, others are read from their file. Those that can't
/// be are left out, and the error says why; English is used if none can.
pub fn set_languages(languages: &[String]) -> Result<(), String> {
    let mut dictionaries = Vec::new();
    let mut errors = Vec::new();
    for language in languages {
        match open(language) {
            Ok(dictionary) => dictionaries.push(dictionary),
            Err(e) => errors.push(e),
        }
    }
    if dictionaries.is_empty() {
        dictionaries.push(english());
    }
    if let Ok(mut active) = ACTIVE.write() {
        *active = dictionaries;
    }
    match errors.is_empty() {
        true => Ok(()),
        false => Err(errors.join("\n")),
    }
}

fn open(language: &str) -> Result<Arc<Dictionary>, String> {
    if language == "en" {
        return Ok(english());
    }
    // It names a file: nothing that could lead out of the folder.
    if language.is_empty() || !language.chars().all(|c| c.is_ascii_lowercase() || c == '-') {
        return Err(format!(
            "\u{201C}{language}\u{201D} isn't a language code, like \u{201C}en\u{201D}."
        ));
    }
    open_file(&dicts_dir().join(format!("{language}.wdict")), language).map(Arc::new)
}

fn open_file(path: &std::path::Path, language: &str) -> Result<Dictionary, String> {
    let file = Mapping::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let dictionary = Dictionary::new(Data::File(file));
    if dictionary.info.language != language {
        return Err(format!(
            "{} isn't a WinDict dictionary for \u{201C}{language}\u{201D}, or is for another version of WinDict.",
            path.display()
        ));
    }
    Ok(dictionary)
}

fn english() -> Arc<Dictionary> {
    static DICTIONARY: OnceLock<Arc<Dictionary>> = OnceLock::new();
    DICTIONARY
        .get_or_init(|| Arc::new(Dictionary::new(Data::Embedded(ENGLISH))))
        .clone()
}

/// Where the dictionaries other than English are kept.
pub fn dicts_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("WinDict").join("dicts")
}

/// The page for `word` at `source`, if it's a dictionary in use.
pub fn link(source: &str, word: &str) -> Option<String> {
    let active = active();
    let info = &active.iter().find(|d| d.info.name == source)?.info;
    Some(info.link.as_ref()?.replace("{}", &percent_encode(word)))
}

/// An offline dictionary, embedded or read from a file.
pub struct Dictionary {
    data: Data,
    info: Info,
}

enum Data {
    Embedded(&'static [u8]),
    File(Mapping),
}

/// What a dictionary says about itself.
#[derive(Default)]
struct Info {
    language: String,
    /// Credited on the card, which keeps it for as long as the app runs.
    name: &'static str,
    /// Its page for a word, with "{}" for the word.
    link: Option<String>,
}

impl Dictionary {
    fn new(data: Data) -> Dictionary {
        let mut dictionary = Dictionary {
            data,
            info: Info::default(),
        };
        if let Some(info) = dictionary.dict().and_then(|d| d.info()) {
            dictionary.info = Info::parse(info);
        }
        dictionary
    }

    fn dict(&self) -> Option<Dict<'_>> {
        let data = match &self.data {
            Data::Embedded(data) => data,
            Data::File(file) => file.bytes(),
        };
        Dict::open(data)
    }

    pub fn language(&self) -> &str {
        &self.info.language
    }

    pub fn lookup(&self, word: &str, detail: Detail) -> Option<Definition> {
        let def = self.dict()?.lookup(word, detail, self.info.name)?;
        Some(detail.apply(Definition {
            language: self.info.language.clone(),
            ..def
        }))
    }
}

impl Info {
    fn parse(text: &str) -> Info {
        let mut info = Info::default();
        for line in text.lines() {
            match line.split_once('=') {
                Some(("language", v)) => info.language = v.to_string(),
                // Leaked, once per dictionary opened: the card's sources are
                // `&'static str`, and a dictionary's name is a few bytes.
                Some(("name", v)) => info.name = Box::leak(v.into()),
                // Opened with the shell, so only a web page will do.
                Some(("link", v)) if v.starts_with("https://") => info.link = Some(v.to_string()),
                _ => {}
            }
        }
        info
    }
}

/// A file mapped into memory, read only. Nothing can write to it while
/// it's open, so the bytes stay as they were.
struct Mapping {
    view: MEMORY_MAPPED_VIEW_ADDRESS,
    len: usize,
    _file: File,
}

// The view is only read, from any thread.
unsafe impl Send for Mapping {}
unsafe impl Sync for Mapping {}

impl Mapping {
    fn open(path: &std::path::Path) -> std::io::Result<Mapping> {
        // FILE_SHARE_READ | FILE_SHARE_DELETE: others may read it, or move
        // a newer copy in its place, but not change it.
        let file = File::options()
            .read(true)
            .share_mode(0x1 | 0x4)
            .open(path)?;
        let len = usize::try_from(file.metadata()?.len()).unwrap_or(usize::MAX);
        if len < HEADER {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        unsafe {
            let mapping = CreateFileMappingW(
                HANDLE(file.as_raw_handle()),
                None,
                PAGE_READONLY,
                0,
                0,
                None,
            )?;
            // The view keeps the mapping open.
            let view = MapViewOfFile(mapping, FILE_MAP_READ, 0, 0, 0);
            let error = std::io::Error::last_os_error();
            let _ = CloseHandle(mapping);
            if view.Value.is_null() {
                return Err(error);
            }
            Ok(Mapping {
                view,
                len,
                _file: file,
            })
        }
    }

    fn bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.view.Value as *const u8, self.len) }
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        unsafe {
            let _ = UnmapViewOfFile(self.view);
        }
    }
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
}

impl Words {
    /// The words of the dictionaries in use.
    pub fn load() -> Words {
        Words::of(active())
    }

    fn of(dictionaries: Vec<Arc<Dictionary>>) -> Words {
        let mut words = Words {
            text: String::new(),
            words: Vec::new(),
            dictionaries,
        };
        for dictionary in words.dictionaries.clone() {
            if let Some(dict) = dictionary.dict() {
                words.add(&dict);
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

    fn add(&mut self, dict: &Dict) {
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
            // Once, though it's there with and without its accents.
            if out.iter().any(|s| s.word == word) {
                continue;
            }
            out.push(Suggestion {
                word: word.to_string(),
                base: None,
                typed: if typos == 0 { typed_chars.len() } else { 0 },
            });
        }
        // An inflected form ("geese", "walked") is offered, as what it's a
        // form of, only once it's typed in full.
        let defined = out.first().is_some_and(|s| s.word.to_lowercase() == typed);
        if !defined {
            let base = self
                .dictionaries
                .iter()
                .find_map(|d| d.dict().and_then(|d| d.base_of(typed)));
            if let Some(base) = base {
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
    /// How it's written, when it's typed without its accents.
    spellings: Vec<String>,
    /// Part of speech and synset ids, most-used part of speech first.
    senses: Vec<(char, Vec<usize>)>,
}

struct Synset {
    lemmas: Vec<String>,
    /// Each lemma's opposites, in the order of `lemmas`.
    antonyms: Vec<Vec<String>>,
    definition: String,
    examples: Vec<String>,
}

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
    fn info(&self) -> Option<&'a str> {
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

    fn defined(&self, key: &str) -> Option<Entry> {
        self.entry(key).filter(|e| !e.senses.is_empty())
    }

    fn lookup(&self, word: &str, detail: Detail, source: &'static str) -> Option<Definition> {
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
                    .map(|b| format!("Also a form of \u{201C}{b}\u{201D}"));
                return build(&candidate, &entry, note);
            }
            // An inflected form ("ran", "walked"): show its base.
            for base in &entry.redirects {
                if let Some(base_entry) = self.defined(base) {
                    return build(
                        base,
                        &base_entry,
                        Some(format!("Base form of \u{201C}{word}\u{201D}")),
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
            language: String::new(),
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

    const DETAIL: Detail = Detail {
        senses: 4,
        synonyms: 6,
        examples: true,
    };

    fn find(word: &str) -> Definition {
        lookup(word, DETAIL).unwrap_or_else(|| panic!("{word} not found"))
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
    fn finds_antonyms() {
        let adjective = |word: &str| {
            find(word)
                .meanings
                .into_iter()
                .find(|m| m.part_of_speech == "adjective")
                .unwrap()
        };
        let good = adjective("good");
        assert!(good.antonyms.contains(&"bad".to_string()), "{good:?}");
        assert!(!good.synonyms.contains(&"bad".to_string()));
        // A synonym's opposites aren't the word's: "large" is "small", "big" is "little".
        assert_eq!(adjective("big").antonyms, ["little"]);
        assert!(find("dog").meanings.iter().all(|m| m.antonyms.is_empty()));
    }

    #[test]
    fn keeps_what_is_asked_for() {
        let brief = Detail {
            senses: 1,
            synonyms: 0,
            examples: false,
        };
        let run = lookup("run", brief).unwrap();
        assert!(run.meanings.iter().all(|m| m.senses.len() == 1));
        assert!(run.meanings.iter().all(|m| m.synonyms.is_empty()));
        assert!(run
            .meanings
            .iter()
            .flat_map(|m| &m.senses)
            .all(|s| s.example.is_none()));
        assert!(find("run")
            .meanings
            .iter()
            .flat_map(|m| &m.senses)
            .any(|s| s.example.is_some()));
    }

    #[test]
    fn resolves_inflections() {
        assert_eq!(find("ran").word, "run");
        assert_eq!(find("geese").word, "goose");
        assert_eq!(find("cats").word, "cat");
        assert_eq!(find("walked").word, "walk");
        assert!(find("walked").note.unwrap().contains("walked"));
        assert_eq!(find("cities").word, "city");
        assert_eq!(find("hoping").word, "hope");
        assert_eq!(find("taller").word, "tall");
        // Phrases inflect their head: a noun's last word, a verb's first.
        assert_eq!(find("ice creams").word, "ice cream");
        assert_eq!(find("looking up").word, "look up");
        // "stopped" is also an adjective in its own right.
        let stopped = find("stopped");
        assert_eq!(stopped.word, "stopped");
        assert!(stopped.note.unwrap().contains("stop"));
    }

    #[test]
    fn finds_words_typed_without_accents() {
        assert_eq!(find("arleng").word, find("arlêng").word);
        assert!(find("arleng").word.contains('ê'));
        assert!(find("arleng").note.is_none());
        // Offered as written, once.
        let arleng = words("arleng");
        assert_eq!(
            arleng.iter().filter(|w| *w == "Arlêng").count(),
            1,
            "{arleng:?}"
        );
        assert!(same_word("Señor", "señor"));
        assert!(!same_word("senor", "señor"));
    }

    #[test]
    fn says_what_it_is() {
        let english = english();
        assert_eq!(english.language(), "en");
        assert_eq!(english.info.name, "Open English WordNet");
        assert_eq!(find("dog").sources, ["Open English WordNet"]);
        assert_eq!(
            link("Open English WordNet", "ice cream").as_deref(),
            Some("https://en-word.net/lemma/ice%20cream")
        );
        assert_eq!(link("Elsewhere", "ice cream"), None);
        // Only web pages are opened.
        let info = Info::parse("language=xx\nname=X\nlink=file:///C:/Windows/notepad.exe");
        assert_eq!(
            (info.language.as_str(), info.name, info.link),
            ("xx", "X", None)
        );
    }

    #[test]
    fn reads_dictionaries_from_files() {
        let dir = std::env::temp_dir().join(format!("windict-dicts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("en.wdict");
        std::fs::write(&path, ENGLISH).unwrap();
        {
            let dictionary = open_file(&path, "en").unwrap();
            let dog = dictionary.lookup("dog", DETAIL).unwrap();
            assert_eq!(dog.meanings[0].part_of_speech, "noun");
            assert!(Words::of(vec![Arc::new(open_file(&path, "en").unwrap())])
                .suggest("walked", 5)
                .iter()
                .any(|s| s.base.as_deref() == Some("walk")));
            // Nothing can change it while it's in use.
            assert!(std::fs::write(&path, b"x").is_err());
            // A dictionary for another language isn't taken for this one.
            let other = open_file(&path, "it").err().unwrap_or_default();
            assert!(other.contains("isn't a WinDict dictionary"), "{other}");
        }
        // Nor is what isn't a dictionary at all, or one cut short.
        for broken in [
            &b"not a dictionary, nor anything else"[..],
            &ENGLISH[..1000],
            b"",
        ] {
            std::fs::write(&path, broken).unwrap();
            assert!(open_file(&path, "en").is_err());
        }
        assert!(open_file(&dir.join("missing.wdict"), "en").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn falls_back_to_english() {
        // Codes can't lead out of the folder.
        for code in ["../evil", "C:\\x", "", "EN"] {
            assert!(open(code).is_err(), "{code}");
        }
        let set = |languages: &[&str]| {
            set_languages(&languages.iter().map(|l| l.to_string()).collect::<Vec<_>>())
        };
        assert!(set(&["../evil"]).is_err());
        assert_eq!(languages(), ["en"]);
        // Those that can be read are used.
        assert!(set(&["zz", "en"]).is_err());
        assert_eq!(languages(), ["en"]);
        assert!(set(&["en"]).is_ok());
        assert_eq!(find("dog").language, "en");
    }

    #[test]
    fn handles_missing_words() {
        assert!(lookup("qwzxv", DETAIL).is_none());
        assert!(lookup("", DETAIL).is_none());
        assert!(lookup("zzzzzzzzz", DETAIL).is_none());
        assert!(lookup("a", DETAIL).is_some());
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
