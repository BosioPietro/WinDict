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

use crate::dictionary::{self, percent_encode, Definition, Detail};
use crate::i18n::{fill, tr};
use crate::lang::LangCode;

mod format;
mod words;

use format::{Dict, HEADER};
pub use words::{Suggestion, Words};

static ENGLISH: &[u8] = include_bytes!("../data/english.wdict");
/// The name and link `ENGLISH` gives itself, for [`link`].
const ENGLISH_NAME: &str = "Open English WordNet";
const ENGLISH_LINK: &str = "https://en-word.net/lemma/{}";

/// How many of a text's words are enough to tell its language.
const DETECT_WORDS: usize = 12;

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

/// The language of the dictionaries in use `text` is likeliest written
/// in: the one with the most of its first few words, if just one has the
/// most. Nothing when none has any, or when it's a tie.
pub fn detect(text: &str) -> Option<LangCode> {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().any(char::is_alphabetic))
        .take(DETECT_WORDS)
        .map(str::to_lowercase)
        .collect();
    let counts: Vec<(usize, LangCode)> = active()
        .iter()
        .map(|d| {
            let dict = d.dict();
            let known = dict.map_or(0, |dict| words.iter().filter(|w| dict.has(w)).count());
            (known, d.language().clone())
        })
        .collect();
    let most = counts.iter().map(|&(n, _)| n).max().filter(|&n| n > 0)?;
    let mut likeliest = counts.into_iter().filter(|&(n, _)| n == most);
    match (likeliest.next(), likeliest.next()) {
        (Some((_, language)), None) => Some(language),
        _ => None,
    }
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
pub fn languages() -> Vec<LangCode> {
    active().iter().map(|d| d.language().clone()).collect()
}

/// Looks words up in `languages` ("en", "it"), in that order of preference:
/// English is built in, others are read from their file. Those that can't
/// be are left out, and the error says why; English is used if none can.
pub fn set_languages(languages: &[LangCode]) -> Result<(), String> {
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
        return Err(fill(tr().not_language_code, &[("language", &language)]));
    }
    open_file(&dicts_dir().join(format!("{language}.wdict")), language).map(Arc::new)
}

fn open_file(path: &std::path::Path, language: &str) -> Result<Dictionary, String> {
    let file = Mapping::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let dictionary = Dictionary::new(Data::File(file));
    if dictionary.info.language != language {
        return Err(fill(
            tr().not_dictionary,
            &[("path", &path.display()), ("language", &language)],
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
    let word = percent_encode(word);
    match ACTIVE.read() {
        Ok(active) if !active.is_empty() => {
            let info = &active.iter().find(|d| d.info.name == source)?.info;
            Some(info.link.as_ref()?.replace("{}", &word))
        }
        // English's, as `active` would, but without reading it: the setup
        // draws a card with its source, and so doesn't carry the dictionary.
        _ => (source == ENGLISH_NAME).then(|| ENGLISH_LINK.replace("{}", &word)),
    }
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
    language: LangCode,
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

    pub fn language(&self) -> &LangCode {
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
                Some(("language", v)) => info.language = v.into(),
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

/// "a domesticated canid" → "A domesticated canid."
/// A definition as a sentence: capitalised, with a full stop.
pub fn sentence(text: &str) -> String {
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
    fn tells_the_language_of_text() {
        // Forms of words count, and so do capitals and punctuation.
        assert_eq!(detect("The dogs ran home.").as_deref(), Some("en"));
        assert_eq!(detect("zxqv brrk"), None);
        assert_eq!(detect("  ... "), None);
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
    }

    #[test]
    fn says_what_it_is() {
        let english = english();
        assert_eq!(english.language(), "en");
        assert_eq!(english.info.name, "Open English WordNet");
        assert_eq!(
            (english.info.name, english.info.link.as_deref()),
            (ENGLISH_NAME, Some(ENGLISH_LINK))
        );
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
            set_languages(&languages.iter().map(|&l| l.into()).collect::<Vec<_>>())
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
    fn formats_senses() {
        assert_eq!(sentence("a canine"), "A canine.");
        assert_eq!(sentence("done!"), "done!".replacen('d', "D", 1));
    }
}
