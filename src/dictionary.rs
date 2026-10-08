//! Word lookup against the Free Dictionary API (https://dictionaryapi.dev).

use serde::Deserialize;

use crate::http;

#[derive(Clone, Debug)]
pub struct Definition {
    pub word: String,
    pub phonetic: Option<String>,
    pub meanings: Vec<Meaning>,
}

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

pub fn lookup(word: &str, max_senses: usize, max_synonyms: usize) -> Lookup {
    let path = format!("/api/v2/entries/en/{}", percent_encode(word));
    match http::get("api.dictionaryapi.dev", &path) {
        Ok((200, body)) => match parse(&body, max_senses, max_synonyms) {
            Some(def) => Lookup::Found(def),
            None => Lookup::NotFound,
        },
        Ok((404, _)) => Lookup::NotFound,
        Ok((429, _)) => Lookup::Failed("Too many lookups — try again in a moment.".into()),
        Ok((code, _)) => Lookup::Failed(format!(
            "The dictionary service returned an error ({code})."
        )),
        Err(e) => Lookup::Failed(e),
    }
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
    })
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
