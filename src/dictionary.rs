//! Dictionary entries, and the online source: the Free Dictionary API
//! (https://dictionaryapi.dev, Wiktionary data). The bundled offline
//! dictionary lives in `offline.rs`.

use serde::Deserialize;

use crate::http;

#[derive(Clone, Debug)]
pub struct Definition {
    pub word: String,
    pub phonetic: Option<String>,
    pub meanings: Vec<Meaning>,
    /// Shown above the senses, e.g. when an inflection was resolved.
    pub note: Option<String>,
    /// Where the content came from, for the footer.
    pub sources: Vec<&'static str>,
}

pub const WORDNET: &str = "Open English WordNet";
pub const WIKTIONARY: &str = "Wiktionary";

#[derive(Clone, Debug)]
pub struct Meaning {
    pub part_of_speech: String,
    pub senses: Vec<Sense>,
    pub synonyms: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Sense {
    pub text: String,
    pub example: Option<String>,
}

#[derive(Clone, Debug)]
pub enum Lookup {
    Found(Definition),
    NotFound,
    Failed(String),
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
}

#[derive(Deserialize)]
struct ApiDefinition {
    definition: String,
    example: Option<String>,
    #[serde(default)]
    synonyms: Vec<String>,
}

/// Cleans up raw selected text into something worth looking up.
pub fn normalize(raw: &str) -> Result<String, &'static str> {
    let trimmed = raw.trim_matches(|c: char| !c.is_alphanumeric());
    let words: Vec<&str> = trimmed.split_whitespace().collect();
    if words.is_empty() {
        return Err("empty");
    }
    if words.len() > 4 || trimmed.chars().count() > 60 {
        return Err("too long");
    }
    Ok(words.join(" ").to_lowercase())
}

/// Tries the Free Dictionary API, then falls back to Wiktionary directly
/// when that has no entry or is unavailable.
pub fn lookup_online(word: &str, max_senses: usize, max_synonyms: usize) -> Lookup {
    let first = lookup_free_dictionary(word, max_senses, max_synonyms);
    if matches!(first, Lookup::Found(_)) {
        return first;
    }
    match lookup_wiktionary(word, max_senses) {
        found @ Lookup::Found(_) => found,
        // Keep the more informative failure: a definite "not found" only
        // counts if neither service was unreachable.
        Lookup::NotFound => first,
        Lookup::Failed(e) => match first {
            Lookup::NotFound => Lookup::NotFound,
            _ => Lookup::Failed(e),
        },
    }
}

fn lookup_free_dictionary(word: &str, max_senses: usize, max_synonyms: usize) -> Lookup {
    let path = format!("/api/v2/entries/en/{}", percent_encode(word));
    match http::get("api.dictionaryapi.dev", &path) {
        Ok((200, body)) => match parse(&body, max_senses, max_synonyms) {
            Some(def) => Lookup::Found(def),
            None => Lookup::NotFound,
        },
        Ok((404, _)) => Lookup::NotFound,
        Ok((429, _)) => Lookup::Failed("Too many lookups, try again in a moment.".into()),
        Ok((code, _)) => Lookup::Failed(format!(
            "The dictionary service returned an error ({code})."
        )),
        Err(e) => Lookup::Failed(e),
    }
}

fn lookup_wiktionary(word: &str, max_senses: usize) -> Lookup {
    let path = format!(
        "/api/rest_v1/page/definition/{}",
        percent_encode(&word.replace(' ', "_"))
    );
    match http::get("en.wiktionary.org", &path) {
        Ok((200, body)) => match parse_wiktionary(&body, word, max_senses) {
            Some(def) => Lookup::Found(def),
            None => Lookup::NotFound,
        },
        Ok((404, _)) => Lookup::NotFound,
        Ok((429, _)) => Lookup::Failed("Too many lookups, try again in a moment.".into()),
        Ok((code, _)) => Lookup::Failed(format!(
            "The dictionary service returned an error ({code})."
        )),
        Err(e) => Lookup::Failed(e),
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
        let pos = entry.part_of_speech.to_lowercase();
        let pos = if pos.is_empty() { "other".to_string() } else { pos };
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
        let pos = if meaning.part_of_speech.is_empty() {
            "other".to_string()
        } else {
            meaning.part_of_speech.clone()
        };
        let idx = match meanings.iter().position(|m| m.part_of_speech == pos) {
            Some(i) => i,
            None => {
                meanings.push(Meaning {
                    part_of_speech: pos,
                    senses: Vec::new(),
                    synonyms: Vec::new(),
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

/// Fills what the offline entry lacks (pronunciation, parts of speech
/// WordNet doesn't cover) from an online entry.
pub fn merge(mut local: Definition, online: Option<&Lookup>) -> Definition {
    let Some(Lookup::Found(online)) = online else {
        return local;
    };
    let mut used = false;
    if local.phonetic.is_none() && online.phonetic.is_some() {
        local.phonetic = online.phonetic.clone();
        used = true;
    }
    for meaning in &online.meanings {
        if !local
            .meanings
            .iter()
            .any(|m| m.part_of_speech == meaning.part_of_speech)
        {
            local.meanings.push(meaning.clone());
            used = true;
        }
    }
    if used && !local.sources.contains(&WIKTIONARY) {
        local.sources.push(WIKTIONARY);
    }
    local
}

/// A web page for `word` at `source`, for the card's footer links.
pub fn source_url(source: &str, word: &str) -> Option<String> {
    match source {
        WORDNET => Some(format!(
            "https://en-word.net/lemma/{}",
            percent_encode(word)
        )),
        WIKTIONARY => Some(format!(
            "https://en.wiktionary.org/wiki/{}",
            percent_encode(&word.replace(' ', "_"))
        )),
        _ => None,
    }
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_selection() {
        assert_eq!(normalize("  Hello, ").unwrap(), "hello");
        assert_eq!(normalize("“serendipity”.").unwrap(), "serendipity");
        assert_eq!(normalize("ice  cream\r\n").unwrap(), "ice cream");
        assert_eq!(normalize("don't").unwrap(), "don't");
        assert!(normalize(" ... ").is_err());
        assert!(normalize("this is a much longer sentence than a word").is_err());
    }

    #[test]
    fn encodes_paths() {
        assert_eq!(percent_encode("ice cream"), "ice%20cream");
        assert_eq!(percent_encode("café"), "caf%C3%A9");
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
        assert_eq!(def.meanings[0].senses[0].text, "Failing to interest; not & exciting.");
        assert_eq!(
            def.meanings[0].senses[0].example.as_deref(),
            Some("An underwhelming film.")
        );
        assert_eq!(def.meanings[1].senses[0].text, "present participle of underwhelm");
        assert!(parse_wiktionary(br#"{"fr":[]}"#, "x", 3).is_none());
    }

    #[test]
    fn parses_api_response() {
        let body = br#"[
          {"word":"run","phonetics":[{"text":""},{"text":"/ran/"}],
           "meanings":[{"partOfSpeech":"verb","definitions":[
              {"definition":"To move swiftly.","example":"Run to the store.","synonyms":["sprint"]},
              {"definition":"To manage.","synonyms":[]}],"synonyms":["dash"]}]},
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
        assert_eq!(def.meanings[1].part_of_speech, "noun");
    }
}
