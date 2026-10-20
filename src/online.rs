//! The online dictionaries: the Free Dictionary API
//! (https://dictionaryapi.dev, built from Wiktionary), with Wiktionary's own
//! API as a fallback.

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use serde::Deserialize;

use crate::dictionary::{
    percent_encode, Definition, Detail, Lookup, Meaning, Sense, MAX_ANTONYMS, WIKTIONARY,
};
use crate::http;

/// How long the Free Dictionary API gets to answer alone before Wiktionary
/// is asked too.
const FALLBACK_DELAY: Duration = Duration::from_millis(500);

/// Asks the Free Dictionary API first. If it hasn't produced a definition
/// within `FALLBACK_DELAY` (or fails or has no entry sooner), Wiktionary is
/// asked as well, and the first definition to arrive wins. The Free
/// Dictionary API is built from Wiktionary, so once Wiktionary has no entry
/// the word isn't waited on any longer.
pub fn lookup(word: &str, detail: Detail) -> Lookup {
    let (w1, w2) = (word.to_string(), word.to_string());
    let result = race(
        move || lookup_free_dictionary(&w1, detail.senses, detail.synonyms),
        move || lookup_wiktionary(&w2, detail.senses),
        FALLBACK_DELAY,
    );
    match result {
        Lookup::Found(def) => Lookup::Found(detail.apply(def)),
        result => result,
    }
}

fn race(
    primary: impl FnOnce() -> Lookup + Send + 'static,
    secondary: impl FnOnce() -> Lookup + Send + 'static,
    delay: Duration,
) -> Lookup {
    // Each answer says whether it's the secondary's.
    let (tx, rx) = mpsc::channel();
    let primary_tx = tx.clone();
    thread::spawn(move || {
        let _ = primary_tx.send((false, primary()));
    });
    let mut secondary = Some((secondary, tx));
    let mut start_secondary = move || {
        if let Some((f, tx)) = secondary.take() {
            thread::spawn(move || {
                let _ = tx.send((true, f()));
            });
            true
        } else {
            false
        }
    };

    let mut pending = 1;
    let mut failure: Option<String> = None;
    let mut not_found = false;
    while pending > 0 {
        let (from_secondary, result) = match rx.recv_timeout(delay) {
            Ok(result) => result,
            Err(_) => {
                // Still waiting on the primary: ask the secondary too.
                if start_secondary() {
                    pending += 1;
                }
                match rx.recv() {
                    Ok(result) => result,
                    Err(_) => break,
                }
            }
        };
        pending -= 1;
        match result {
            found @ Lookup::Found(_) => return found,
            Lookup::Failed(e) => failure = Some(e),
            // The primary can't know a word the secondary doesn't: no
            // sense waiting for it, should it hang.
            Lookup::NotFound if from_secondary => return Lookup::NotFound,
            Lookup::NotFound => not_found = true,
        }
        // The primary came up empty before the delay: don't wait any longer.
        if start_secondary() {
            pending += 1;
        }
    }
    // A definite "not found" from either source beats the other failing.
    match failure {
        Some(e) if !not_found => Lookup::Failed(e),
        _ => Lookup::NotFound,
    }
}

fn lookup_free_dictionary(word: &str, max_senses: usize, max_synonyms: usize) -> Lookup {
    let path = format!("/api/v2/entries/en/{}", percent_encode(word));
    fetch("api.dictionaryapi.dev", &path, |body| {
        parse(body, max_senses, max_synonyms)
    })
}

fn lookup_wiktionary(word: &str, max_senses: usize) -> Lookup {
    let path = format!(
        "/api/rest_v1/page/definition/{}",
        percent_encode(&word.replace(' ', "_"))
    );
    fetch("en.wiktionary.org", &path, |body| {
        parse_wiktionary(body, word, max_senses)
    })
}

/// GETs `path` and parses a successful response with `parse`.
fn fetch(host: &str, path: &str, parse: impl FnOnce(&[u8]) -> Option<Definition>) -> Lookup {
    match http::get(host, path) {
        Ok((200, body)) => parse(&body).map_or(Lookup::NotFound, Lookup::Found),
        Ok((404, _)) => Lookup::NotFound,
        Ok((429, _)) => Lookup::Failed("Too many lookups, try again in a moment.".into()),
        Ok((code, _)) => Lookup::Failed(format!(
            "The dictionary service returned an error ({code})."
        )),
        Err(e) => Lookup::Failed(e),
    }
}

/// A part of speech as shown on the card; sources sometimes leave it out.
fn part_of_speech(raw: &str) -> String {
    if raw.is_empty() {
        "other".to_string()
    } else {
        raw.to_lowercase()
    }
}

#[derive(Deserialize)]
struct WiktEntry {
    #[serde(rename = "partOfSpeech", default)]
    part_of_speech: String,
    #[serde(default)]
    definitions: Vec<WiktDefinition>,
}

#[derive(Deserialize)]
struct WiktDefinition {
    #[serde(default)]
    definition: String,
    #[serde(rename = "parsedExamples", default)]
    examples: Vec<WiktExample>,
}

#[derive(Deserialize)]
struct WiktExample {
    #[serde(default)]
    example: String,
}

fn parse_wiktionary(body: &[u8], word: &str, max_senses: usize) -> Option<Definition> {
    let mut languages: std::collections::HashMap<String, Vec<WiktEntry>> =
        serde_json::from_slice(body).ok()?;
    let entries = languages.remove("en")?;

    let mut meanings: Vec<Meaning> = Vec::new();
    for entry in entries {
        let pos = part_of_speech(&entry.part_of_speech);
        let senses: Vec<Sense> = entry
            .definitions
            .iter()
            .filter_map(|d| {
                let text = strip_html(&d.definition);
                if text.is_empty() {
                    return None;
                }
                let example = d
                    .examples
                    .first()
                    .map(|e| strip_html(&e.example))
                    .filter(|e| !e.is_empty());
                Some(Sense { text, example })
            })
            .take(max_senses)
            .collect();
        if senses.is_empty() {
            continue;
        }
        match meanings.iter_mut().find(|m| m.part_of_speech == pos) {
            Some(m) => {
                let room = max_senses.saturating_sub(m.senses.len());
                m.senses.extend(senses.into_iter().take(room));
            }
            None => meanings.push(Meaning {
                part_of_speech: pos,
                senses,
                synonyms: Vec::new(),
                antonyms: Vec::new(),
            }),
        }
    }
    if meanings.is_empty() {
        return None;
    }
    Some(Definition {
        word: word.to_string(),
        phonetic: None,
        meanings,
        note: None,
        sources: vec![WIKTIONARY],
    })
}

/// Removes tags from the HTML snippets Wiktionary returns and decodes the
/// few entities that show up in them.
fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    let out = out
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&");
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn parse(body: &[u8], max_senses: usize, max_synonyms: usize) -> Option<Definition> {
    let entries: Vec<ApiEntry> = serde_json::from_slice(body).ok()?;
    let first = entries.first()?;
    let word = first.word.clone();

    let phonetic = entries
        .iter()
        .flat_map(|e| {
            e.phonetic
                .iter()
                .chain(e.phonetics.iter().filter_map(|p| p.text.as_ref()))
        })
        .map(|p| p.trim())
        .find(|p| !p.is_empty())
        .map(str::to_owned);

    // The API often splits one word into several entries (etymologies);
    // merge meanings that share a part of speech.
    let mut meanings: Vec<Meaning> = Vec::new();
    for meaning in entries.iter().flat_map(|e| &e.meanings) {
        let pos = part_of_speech(&meaning.part_of_speech);
        let idx = match meanings.iter().position(|m| m.part_of_speech == pos) {
            Some(i) => i,
            None => {
                meanings.push(Meaning {
                    part_of_speech: pos,
                    senses: Vec::new(),
                    synonyms: Vec::new(),
                    antonyms: Vec::new(),
                });
                meanings.len() - 1
            }
        };
        let target = &mut meanings[idx];
        for d in &meaning.definitions {
            if target.senses.len() >= max_senses {
                break;
            }
            let text = d.definition.trim();
            if text.is_empty() {
                continue;
            }
            target.senses.push(Sense {
                text: text.to_string(),
                example: d
                    .example
                    .as_ref()
                    .map(|e| e.trim().to_string())
                    .filter(|e| !e.is_empty()),
            });
        }
        let synonyms = meaning
            .synonyms
            .iter()
            .chain(meaning.definitions.iter().flat_map(|d| &d.synonyms));
        for s in synonyms {
            if target.synonyms.len() >= max_synonyms {
                break;
            }
            if !target.synonyms.contains(s) && !s.eq_ignore_ascii_case(&word) {
                target.synonyms.push(s.clone());
            }
        }
        let antonyms = meaning
            .antonyms
            .iter()
            .chain(meaning.definitions.iter().flat_map(|d| &d.antonyms));
        for a in antonyms {
            if target.antonyms.len() >= MAX_ANTONYMS {
                break;
            }
            if !target.antonyms.contains(a) && !a.eq_ignore_ascii_case(&word) {
                target.antonyms.push(a.clone());
            }
        }
    }
    meanings.retain(|m| !m.senses.is_empty());
    if meanings.is_empty() {
        return None;
    }
    Some(Definition {
        word,
        phonetic,
        meanings,
        note: None,
        sources: vec![WIKTIONARY],
    })
}

#[derive(Deserialize)]
struct ApiEntry {
    word: String,
    phonetic: Option<String>,
    #[serde(default)]
    phonetics: Vec<ApiPhonetic>,
    #[serde(default)]
    meanings: Vec<ApiMeaning>,
}

#[derive(Deserialize)]
struct ApiPhonetic {
    text: Option<String>,
}

#[derive(Deserialize)]
struct ApiMeaning {
    #[serde(rename = "partOfSpeech", default)]
    part_of_speech: String,
    #[serde(default)]
    definitions: Vec<ApiDefinition>,
    #[serde(default)]
    synonyms: Vec<String>,
    #[serde(default)]
    antonyms: Vec<String>,
}

#[derive(Deserialize)]
struct ApiDefinition {
    definition: String,
    example: Option<String>,
    #[serde(default)]
    synonyms: Vec<String>,
    #[serde(default)]
    antonyms: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_html() {
        assert_eq!(strip_html("<b>bold</b>  and\n<i>x</i>"), "bold and x");
        // Entities are decoded once: "&amp;lt;" is the literal text "&lt;".
        assert_eq!(strip_html("a &amp;lt; b &lt; c"), "a &lt; b < c");
        assert_eq!(
            strip_html("x&nbsp;y &quot;z&quot; &#39;w&#39;"),
            "x y \"z\" 'w'"
        );
        assert_eq!(strip_html(""), "");
    }

    #[test]
    fn parses_wiktionary_response() {
        let body = br#"{"en":[
          {"partOfSpeech":"Adjective","language":"English","definitions":[
            {"definition":"Failing to <a href=\"/wiki/interest\">interest</a>; not &amp; exciting.",
             "parsedExamples":[{"example":"An <i>underwhelming</i> film."}]}]},
          {"partOfSpeech":"Verb","language":"English","definitions":[
            {"definition":"<span>present participle of <i>underwhelm</i></span>"}]}],
          "fr":[{"partOfSpeech":"Nom","definitions":[{"definition":"x"}]}]}"#;
        let def = parse_wiktionary(body, "underwhelming", 3).unwrap();
        assert_eq!(def.meanings.len(), 2);
        assert_eq!(def.meanings[0].part_of_speech, "adjective");
        assert_eq!(
            def.meanings[0].senses[0].text,
            "Failing to interest; not & exciting."
        );
        assert_eq!(
            def.meanings[0].senses[0].example.as_deref(),
            Some("An underwhelming film.")
        );
        assert_eq!(
            def.meanings[1].senses[0].text,
            "present participle of underwhelm"
        );
        assert!(parse_wiktionary(br#"{"fr":[]}"#, "x", 3).is_none());
    }

    fn found(word: &str) -> Lookup {
        Lookup::Found(Definition {
            word: word.into(),
            phonetic: None,
            meanings: Vec::new(),
            note: None,
            sources: Vec::new(),
        })
    }

    fn delayed(ms: u64, result: Lookup) -> impl FnOnce() -> Lookup + Send + 'static {
        move || {
            thread::sleep(Duration::from_millis(ms));
            result
        }
    }

    fn word_of(result: Lookup) -> String {
        match result {
            Lookup::Found(d) => d.word,
            other => format!("{other:?}"),
        }
    }

    #[test]
    fn race_prefers_a_prompt_primary() {
        let d = Duration::from_millis(100);
        let r = race(
            delayed(10, found("primary")),
            delayed(0, found("secondary")),
            d,
        );
        assert_eq!(word_of(r), "primary");
    }

    #[test]
    fn race_falls_back_after_the_delay() {
        let d = Duration::from_millis(50);
        let r = race(
            delayed(2000, found("primary")),
            delayed(10, found("secondary")),
            d,
        );
        assert_eq!(word_of(r), "secondary");
    }

    #[test]
    fn race_falls_back_at_once_when_primary_is_empty() {
        let start = std::time::Instant::now();
        let r = race(
            delayed(0, Lookup::NotFound),
            delayed(0, found("secondary")),
            Duration::from_secs(5),
        );
        assert_eq!(word_of(r), "secondary");
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn race_reports_not_found_over_failure() {
        let d = Duration::from_millis(10);
        let r = race(
            delayed(0, Lookup::NotFound),
            delayed(0, Lookup::Failed("x".into())),
            d,
        );
        assert!(matches!(r, Lookup::NotFound));
        let r = race(
            delayed(0, Lookup::Failed("x".into())),
            delayed(0, Lookup::NotFound),
            d,
        );
        assert!(matches!(r, Lookup::NotFound));
        let r = race(
            delayed(0, Lookup::NotFound),
            delayed(0, Lookup::NotFound),
            d,
        );
        assert!(matches!(r, Lookup::NotFound));
        // Only when both fail is it a failure.
        let r = race(
            delayed(0, Lookup::Failed("x".into())),
            delayed(0, Lookup::Failed("y".into())),
            d,
        );
        assert!(matches!(r, Lookup::Failed(_)));
    }

    #[test]
    fn race_stops_waiting_when_secondary_is_empty() {
        let start = std::time::Instant::now();
        let r = race(
            delayed(3000, found("primary")),
            delayed(0, Lookup::NotFound),
            Duration::from_millis(10),
        );
        assert!(matches!(r, Lookup::NotFound));
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn parses_api_response() {
        let body = br#"[
          {"word":"run","phonetics":[{"text":""},{"text":"/ran/"}],
           "meanings":[{"partOfSpeech":"verb","definitions":[
              {"definition":"To move swiftly.","example":"Run to the store.","synonyms":["sprint"],"antonyms":["walk"]},
              {"definition":"To manage.","synonyms":[]}],"synonyms":["dash"],"antonyms":["stop","walk"]}]},
          {"word":"run","meanings":[{"partOfSpeech":"verb","definitions":[{"definition":"Third."}]},
             {"partOfSpeech":"noun","definitions":[{"definition":"An act of running."}]}]}
        ]"#;
        let def = parse(body, 2, 5).unwrap();
        assert_eq!(def.word, "run");
        assert_eq!(def.phonetic.as_deref(), Some("/ran/"));
        assert_eq!(def.meanings.len(), 2);
        assert_eq!(def.meanings[0].senses.len(), 2);
        assert_eq!(
            def.meanings[0].senses[0].example.as_deref(),
            Some("Run to the store.")
        );
        assert_eq!(def.meanings[0].synonyms, vec!["dash", "sprint"]);
        assert_eq!(def.meanings[0].antonyms, vec!["stop", "walk"]);
        assert!(def.meanings[1].antonyms.is_empty());
        assert_eq!(def.meanings[1].part_of_speech, "noun");
    }
}
