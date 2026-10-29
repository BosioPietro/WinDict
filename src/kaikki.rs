//! Wiktionaries in the languages they define, online, as wiktextract
//! extracts them (<https://kaikki.org>, which the downloaded dictionaries are
//! built from). For Italian, Wikizionario's pages themselves are read too
//! (`wikizionario`), for the words the extract doesn't have yet.

use serde::Deserialize;

use crate::dictionary::{
    percent_encode, Definition, Lookup, Meaning, Sense, DEUTSCHES_WIKTIONARY, MAX_ANTONYMS,
    WIKCIONARIO, WIKCIONARIO_PT, WIKIZIONARIO, WIKTIONNAIRE,
};
use crate::i18n::{fill, tr};
use crate::offline::sentence;
use crate::online::{fetch, same_word};
use crate::wikizionario;

/// A Wiktionary in the language it defines.
pub struct Edition {
    /// The language it defines, as the dictionaries name it ("it").
    pub language: &'static str,
    /// Its name, as the card credits it.
    pub source: &'static str,
    /// Where its pages are.
    pub host: &'static str,
    /// Its pages themselves, read too when the extract is slow or lacks the
    /// word: (word, senses, synonyms).
    pub pages: Option<fn(&str, usize, usize) -> Lookup>,
    /// Where kaikki.org keeps it, and the folder of its own language's words
    /// there.
    dir: &'static str,
    heading: &'static str,
}

pub const EDITIONS: &[Edition] = &[
    Edition {
        language: "it",
        source: WIKIZIONARIO,
        host: "it.wiktionary.org",
        pages: Some(wikizionario::lookup_page),
        dir: "itwiktionary",
        heading: "Italiano",
    },
    Edition {
        language: "es",
        source: WIKCIONARIO,
        host: "es.wiktionary.org",
        pages: None,
        dir: "eswiktionary",
        heading: "Espa\u{F1}ol",
    },
    Edition {
        language: "fr",
        source: WIKTIONNAIRE,
        host: "fr.wiktionary.org",
        pages: None,
        dir: "frwiktionary",
        heading: "Fran\u{E7}ais",
    },
    Edition {
        language: "de",
        source: DEUTSCHES_WIKTIONARY,
        host: "de.wiktionary.org",
        pages: None,
        dir: "dewiktionary",
        heading: "Deutsch",
    },
    Edition {
        language: "pt",
        source: WIKCIONARIO_PT,
        host: "pt.wiktionary.org",
        pages: None,
        dir: "ptwiktionary",
        heading: "Portugu\u{EA}s",
    },
];

/// The Wiktionary in `language`, if WinDict reads one.
pub fn edition(language: &str) -> Option<&'static Edition> {
    EDITIONS.iter().find(|e| e.language == language)
}

/// The word's entries in the extract, following an inflected form ("case")
/// to its base ("casa") when that's all there is.
pub fn lookup(edition: &Edition, word: &str, max_senses: usize, max_synonyms: usize) -> Lookup {
    let mut base = None;
    let result = fetch("kaikki.org", &edition.path(word), |body| {
        let entries = edition.entries(body);
        base = base_form(&entries, word);
        parse(edition, &entries, max_senses, max_synonyms)
    });
    match (result, base) {
        (Lookup::NotFound, Some(base)) => {
            match fetch("kaikki.org", &edition.path(&base), |body| {
                parse(edition, &edition.entries(body), max_senses, max_synonyms)
            }) {
                Lookup::Found(def) => Lookup::Found(Definition {
                    note: Some(fill(tr().base_form_of, &[("word", &word)])),
                    ..def
                }),
                other => other,
            }
        }
        (result, _) => result,
    }
}

impl Edition {
    /// Where the extract keeps a word: under its first letter, then its
    /// first two ("c/ca/casa"), case and accents as they are.
    fn path(&self, word: &str) -> String {
        let first: String = word.chars().take(1).collect();
        let two: String = word.chars().take(2).collect();
        format!(
            "/{}/{}/meaning/{}/{}/{}.jsonl",
            self.dir,
            percent_encode(self.heading),
            percent_encode(&first),
            percent_encode(&two),
            percent_encode(word)
        )
    }

    /// The entries in its language, one per line; the rest of the page's
    /// languages are in other files.
    fn entries(&self, body: &[u8]) -> Vec<Entry> {
        body.split(|&b| b == b'\n')
            .filter_map(|line| serde_json::from_slice::<Entry>(line).ok())
            .filter(|e| e.lang_code == self.language)
            .collect()
    }
}

#[derive(Deserialize)]
struct Entry {
    #[serde(default)]
    word: String,
    #[serde(default)]
    lang_code: String,
    #[serde(default)]
    pos: String,
    #[serde(default)]
    senses: Vec<ExtractSense>,
    #[serde(default)]
    sounds: Vec<Sound>,
    #[serde(default)]
    synonyms: Vec<Word>,
    #[serde(default)]
    antonyms: Vec<Word>,
}

#[derive(Deserialize)]
struct ExtractSense {
    #[serde(default)]
    glosses: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    form_of: Vec<Word>,
    #[serde(default)]
    examples: Vec<Example>,
}

impl ExtractSense {
    fn is_form(&self) -> bool {
        !self.form_of.is_empty() || self.tags.iter().any(|t| t == "form-of")
    }
}

#[derive(Deserialize)]
struct Sound {
    ipa: Option<String>,
}

#[derive(Deserialize)]
struct Word {
    #[serde(default)]
    word: String,
}

fn words(list: &[Word]) -> Vec<&str> {
    list.iter().map(|w| w.word.as_str()).collect()
}

#[derive(Deserialize)]
struct Example {
    #[serde(default)]
    text: String,
}

/// What `word` is a form of, when it is one.
fn base_form(entries: &[Entry], word: &str) -> Option<String> {
    entries
        .iter()
        .flat_map(|e| &e.senses)
        .flat_map(|s| &s.form_of)
        .map(|b| b.word.trim())
        .find(|b| !b.is_empty() && *b != word)
        .map(str::to_owned)
}

/// The extract's parts of speech, as the card names them. The rest
/// (prefixes, symbols, letters) aren't looked up as words.
fn part_of_speech(pos: &str) -> Option<&'static str> {
    Some(match pos {
        "noun" | "name" => "noun",
        "verb" => "verb",
        "adj" => "adjective",
        "adv" | "adv_phrase" => "adverb",
        "phrase" => "phrase",
        "prep" | "prep_phrase" => "preposition",
        "intj" => "interjection",
        "pron" => "pronoun",
        "conj" => "conjunction",
        "article" => "article",
        "abbrev" => "abbreviation",
        _ => return None,
    })
}

/// Glosses the extractor couldn't make out: links to further reading,
/// "casa ( approfondimento)", and the note that a sense lacks one.
fn broken_gloss(gloss: &str) -> bool {
    gloss.starts_with("Definizione mancante")
        || gloss.match_indices('(').any(|(i, _)| {
            let rest = gloss[i + 1..].trim_start();
            ["approfondimento", "citazioni"].iter().any(|w| {
                rest.strip_prefix(w)
                    .is_some_and(|r| r.trim_start().starts_with(')'))
            })
        })
}

fn parse(
    edition: &Edition,
    entries: &[Entry],
    max_senses: usize,
    max_synonyms: usize,
) -> Option<Definition> {
    let mut meanings: Vec<Meaning> = Vec::new();
    let mut word = None;
    let mut phonetic = None;
    // A page's synonyms and opposites are the same for each of its parts of
    // speech.
    let mut synonyms = Vec::new();
    let mut antonyms = Vec::new();
    for entry in entries {
        let Some(pos) = part_of_speech(&entry.pos) else {
            continue;
        };
        let senses: Vec<Sense> = entry
            .senses
            .iter()
            .filter(|s| !s.is_form())
            .filter_map(|s| {
                if broken_gloss(s.glosses.first()?) {
                    return None;
                }
                let gloss = s
                    .glosses
                    .last()?
                    .trim()
                    .trim_end_matches([' ', ';', ':', ',']);
                if gloss.is_empty() {
                    return None;
                }
                let example = s
                    .examples
                    .iter()
                    .map(|x| x.text.trim())
                    .find(|x| !x.is_empty())
                    .map(str::to_owned);
                Some(Sense {
                    text: sentence(gloss),
                    example,
                })
            })
            .take(max_senses)
            .collect();
        if senses.is_empty() {
            continue;
        }
        let word = word.get_or_insert_with(|| entry.word.trim().to_string());
        if phonetic.is_none() {
            phonetic = entry
                .sounds
                .iter()
                .filter_map(|s| s.ipa.as_deref())
                .map(str::trim)
                .find(|p| !p.is_empty())
                .map(str::to_owned);
        }
        if synonyms.is_empty() && antonyms.is_empty() {
            synonyms = related(words(&entry.synonyms), word, max_synonyms);
            antonyms = related(words(&entry.antonyms), word, MAX_ANTONYMS);
        }
        add_senses(&mut meanings, pos, senses, max_senses);
    }
    definition(edition, word?, phonetic, meanings, synonyms, antonyms)
}

/// Synonyms or opposites worth showing: words or short phrases, not
/// questions or fragments, each once, and not the word itself.
pub fn related(words: Vec<&str>, word: &str, max: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for w in words {
        let w = w.split_whitespace().collect::<Vec<_>>().join(" ");
        let count = w.chars().count();
        let is_word = (2..=40).contains(&count)
            && w.chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '\'' | '\u{2019}' | ' ' | '-'));
        if is_word
            && !same_word(&w, word)
            && !out.iter().any(|o| same_word(o, &w))
            && out.len() < max
        {
            out.push(w);
        }
    }
    out
}

/// Adds `senses` to the part of speech's, which the word may already have
/// from another etymology.
pub fn add_senses(meanings: &mut Vec<Meaning>, pos: &str, senses: Vec<Sense>, max_senses: usize) {
    if senses.is_empty() {
        return;
    }
    match meanings.iter_mut().find(|m| m.part_of_speech == pos) {
        Some(m) => {
            let room = max_senses.saturating_sub(m.senses.len());
            m.senses.extend(senses.into_iter().take(room));
        }
        None => meanings.push(Meaning {
            part_of_speech: pos.to_string(),
            senses,
            synonyms: Vec::new(),
            antonyms: Vec::new(),
        }),
    }
}

/// The definition, the page's synonyms and opposites going with its first
/// part of speech.
pub fn definition(
    edition: &Edition,
    word: String,
    phonetic: Option<String>,
    mut meanings: Vec<Meaning>,
    synonyms: Vec<String>,
    antonyms: Vec<String>,
) -> Option<Definition> {
    let first = meanings.first_mut()?;
    first.synonyms = synonyms;
    first.antonyms = antonyms;
    Some(Definition {
        word,
        language: edition.language.into(),
        phonetic,
        meanings,
        note: None,
        sources: vec![edition.source],
        others: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn italian() -> &'static Edition {
        edition("it").unwrap()
    }

    #[test]
    fn finds_words_in_the_extract() {
        let it = italian();
        assert_eq!(
            it.path("casa"),
            "/itwiktionary/Italiano/meaning/c/ca/casa.jsonl"
        );
        assert_eq!(it.path("a"), "/itwiktionary/Italiano/meaning/a/a/a.jsonl");
        assert_eq!(
            it.path("è"),
            "/itwiktionary/Italiano/meaning/%C3%A8/%C3%A8/%C3%A8.jsonl"
        );
        assert_eq!(
            it.path("a priori"),
            "/itwiktionary/Italiano/meaning/a/a%20/a%20priori.jsonl"
        );
        assert_eq!(
            edition("es").unwrap().path("ñu"),
            "/eswiktionary/Espa%C3%B1ol/meaning/%C3%B1/%C3%B1u/%C3%B1u.jsonl"
        );
        assert_eq!(
            edition("fr").unwrap().path("œuf"),
            "/frwiktionary/Fran%C3%A7ais/meaning/%C5%93/%C5%93u/%C5%93uf.jsonl"
        );
        assert_eq!(
            edition("de").unwrap().path("Übung"),
            "/dewiktionary/Deutsch/meaning/%C3%9C/%C3%9Cb/%C3%9Cbung.jsonl"
        );
        assert_eq!(
            edition("pt").unwrap().path("lampejo"),
            "/ptwiktionary/Portugu%C3%AAs/meaning/l/la/lampejo.jsonl"
        );
        assert!(edition("en").is_none());
    }

    #[test]
    fn parses_the_extract() {
        let body = br#"{"word": "correre", "lang_code": "it", "pos": "verb", "senses": [{"glosses": ["procedere velocemente"], "examples": [{"text": "Corse via dalla paura."}]}, {"glosses": ["fare qualcosa in fretta"]}, {"glosses": ["casa ( approfondimento) f sing"]}], "sounds": [{"ipa": "/korrere/"}], "synonyms": [{"word": "accorrere"}, {"word": "correre"}, {"word": "?"}], "antonyms": [{"word": "camminare"}]}
{"word": "correre", "lang_code": "it", "pos": "noun", "senses": [{"glosses": ["il correre"]}]}
{"word": "correre", "lang_code": "it", "pos": "letter", "senses": [{"glosses": ["x"]}]}"#;
        let it = italian();
        let entries = it.entries(body);
        assert_eq!(base_form(&entries, "correre"), None);
        let def = parse(it, &entries, 5, 5).unwrap();
        assert_eq!(def.language, "it");
        assert_eq!(def.sources, [WIKIZIONARIO]);
        assert_eq!(def.phonetic.as_deref(), Some("/korrere/"));
        let parts: Vec<_> = def.meanings.iter().map(|m| &*m.part_of_speech).collect();
        assert_eq!(parts, ["verb", "noun"]);
        let verb = &def.meanings[0];
        assert_eq!(verb.senses.len(), 2);
        assert_eq!(verb.senses[0].text, "Procedere velocemente.");
        assert_eq!(
            verb.senses[0].example.as_deref(),
            Some("Corse via dalla paura.")
        );
        assert_eq!(verb.synonyms, ["accorrere"]);
        assert_eq!(verb.antonyms, ["camminare"]);
        assert!(def.meanings[1].synonyms.is_empty());
        assert_eq!(
            parse(it, &entries, 1, 5).unwrap().meanings[0].senses.len(),
            1
        );
    }

    #[test]
    fn parses_each_edition_in_its_language() {
        let body = br#"{"word": "casa", "lang_code": "es", "pos": "noun", "senses": [{"glosses": ["Edificio para habitar."]}]}
{"word": "casa", "lang_code": "it", "pos": "noun", "senses": [{"glosses": ["edificio"]}]}"#;
        let es = edition("es").unwrap();
        let def = parse(es, &es.entries(body), 5, 5).unwrap();
        assert_eq!(
            (&*def.language, def.sources.as_slice()),
            ("es", &[WIKCIONARIO][..])
        );
        assert_eq!(def.meanings[0].senses[0].text, "Edificio para habitar.");
    }

    #[test]
    fn follows_forms_in_the_extract() {
        let body = br#"{"word": "case", "lang_code": "it", "pos": "noun", "senses": [{"glosses": ["plurale di casa"], "tags": ["form-of"], "form_of": [{"word": "casa"}]}]}
{"word": "case", "lang_code": "en", "pos": "noun", "senses": [{"glosses": ["box"]}]}"#;
        let it = italian();
        let entries = it.entries(body);
        assert_eq!(entries.len(), 1);
        assert_eq!(base_form(&entries, "case").as_deref(), Some("casa"));
        assert!(parse(it, &entries, 5, 5).is_none());
    }

    #[test]
    fn spots_broken_glosses() {
        assert!(broken_gloss("casa ( approfondimento) f sing"));
        assert!(broken_gloss("casa (citazioni)"));
        assert!(broken_gloss(
            "Definizione mancante; se vuoi, aggiungila tu."
        ));
        assert!(!broken_gloss("edificio (di pietra)"));
    }
}
