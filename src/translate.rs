//! Quick translation of the word on the card: Google Translate's dictionary
//! (the endpoint translate.google.com asks), which groups the translations
//! by part of speech, then the older keyless `gtx` endpoint should that
//! fail, and MyMemory (https://mymemory.translated.net) as a last fallback,
//! asked with the word's first definition for context.

use serde::Deserialize;
use serde_json::Value;

use crate::dictionary::percent_encode;
use crate::http;
use crate::i18n::{fill, tr};

const GOOGLE_HOST: &str = "translate.googleapis.com";
const GOOGLE_WEB_HOST: &str = "translate-pa.googleapis.com";
/// The key translate.google.com itself sends, in its page for anyone.
const GOOGLE_WEB_KEY: &str = "AIzaSyDLEeFI5OtFBwYBIoK_jj5m32rZK5CkCXA";
const MYMEMORY_HOST: &str = "api.mymemory.translated.net";
pub const GOOGLE: &str = "Google Translate";
pub const MYMEMORY: &str = "MyMemory";
/// Translations shown for each part of speech, and their own meanings.
const MAX_TERMS: usize = 5;
const MAX_BACK: usize = 4;
/// MyMemory's other translations shown, when it's the one that answered.
const MAX_ALTERNATIVES: usize = 5;
/// Longer contexts cost more of MyMemory's daily allowance than they help.
const MAX_CONTEXT_CHARS: usize = 160;
/// Joins the word and its definition, so the translation of one can be told
/// from that of the other.
const CONTEXT_SEPARATOR: &str = ": ";

/// The languages words can be translated between, as code and name, in the
/// language itself.
pub const LANGUAGES: &[(&str, &str)] = &[
    ("en", "English"),
    ("it", "Italiano"),
    ("fr", "Fran\u{E7}ais"),
    ("de", "Deutsch"),
    ("es", "Espa\u{F1}ol"),
    ("pt", "Portugu\u{EA}s"),
    ("nl", "Nederlands"),
    ("sv", "Svenska"),
    ("da", "Dansk"),
    ("no", "Norsk"),
    ("fi", "Suomi"),
    ("pl", "Polski"),
    ("cs", "\u{10C}e\u{161}tina"),
    ("ro", "Rom\u{E2}n\u{103}"),
    ("hu", "Magyar"),
    (
        "el",
        "\u{395}\u{3BB}\u{3BB}\u{3B7}\u{3BD}\u{3B9}\u{3BA}\u{3AC}",
    ),
    ("tr", "T\u{FC}rk\u{E7}e"),
    ("ru", "\u{420}\u{443}\u{441}\u{441}\u{43A}\u{438}\u{439}"),
    (
        "uk",
        "\u{423}\u{43A}\u{440}\u{430}\u{457}\u{43D}\u{441}\u{44C}\u{43A}\u{430}",
    ),
    ("ja", "\u{65E5}\u{672C}\u{8A9E}"),
    ("ko", "\u{D55C}\u{AD6D}\u{C5B4}"),
    ("zh-CN", "\u{4E2D}\u{6587}"),
];

/// `language`'s name, or its code when it isn't one of [`LANGUAGES`].
pub fn name(language: &str) -> String {
    LANGUAGES
        .iter()
        .find(|(code, _)| *code == language)
        .map_or_else(|| language.to_uppercase(), |(_, name)| name.to_string())
}

/// Whether words can be translated from or into `language`.
pub fn supported(language: &str) -> bool {
    LANGUAGES.iter().any(|(code, _)| *code == language)
}

/// `language`'s name in English, for languages named otherwise in
/// [`LANGUAGES`].
pub fn english_name(language: &str) -> Option<&'static str> {
    Some(match language {
        "it" => "Italian",
        "fr" => "French",
        "de" => "German",
        "es" => "Spanish",
        "pt" => "Portuguese",
        "nl" => "Dutch",
        "sv" => "Swedish",
        "da" => "Danish",
        "no" => "Norwegian",
        "fi" => "Finnish",
        "pl" => "Polish",
        "cs" => "Czech",
        "ro" => "Romanian",
        "hu" => "Hungarian",
        "el" => "Greek",
        "tr" => "Turkish",
        "ru" => "Russian",
        "uk" => "Ukrainian",
        "ja" => "Japanese",
        "ko" => "Korean",
        "zh-CN" => "Chinese",
        _ => return None,
    })
}

/// The codes of the languages whose name, in their own language or in
/// English, or whose code, starts with `query` (any of their words does),
/// accents and case aside, in [`LANGUAGES`]' order. All of them for nothing.
pub fn search(query: &str) -> Vec<&'static str> {
    let query = fold(query.trim());
    LANGUAGES
        .iter()
        .filter(|(code, name)| {
            let names = [Some(*name), english_name(code), Some(*code)];
            names.into_iter().flatten().any(|n| {
                fold(n)
                    .split([' ', '-'])
                    .any(|word| word.starts_with(&query))
            })
        })
        .map(|(code, _)| *code)
        .collect()
}

/// Lower case, and the Latin letters without their accents: "Français" →
/// "francais".
fn fold(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ă' => 'a',
            'ç' | 'č' => 'c',
            'è' | 'é' | 'ê' | 'ë' | 'ě' => 'e',
            'ì' | 'í' | 'î' | 'ï' => 'i',
            'ñ' | 'ň' => 'n',
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' => 'o',
            'ù' | 'ú' | 'û' | 'ü' | 'ů' => 'u',
            'š' | 'ș' | 'ş' => 's',
            'ț' | 'ţ' => 't',
            'ž' => 'z',
            'ř' => 'r',
            'ý' => 'y',
            c => c,
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct Translation {
    /// The translation itself.
    pub text: String,
    /// What the word's definition says, translated: how MyMemory was told
    /// which sense was meant.
    pub context: Option<String>,
    /// The other translations, by part of speech ("" when it isn't known).
    pub groups: Vec<Group>,
    /// Where it comes from: [`GOOGLE`] or [`MYMEMORY`].
    pub source: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub part_of_speech: String,
    pub terms: Vec<Term>,
}

/// A translation, and what it means back in the word's language.
#[derive(Clone, Debug, PartialEq)]
pub struct Term {
    pub text: String,
    pub back: Vec<String>,
}

/// Which way to translate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Direction {
    pub from: String,
    pub to: String,
    /// Whether it's surely the way wanted: not so when the word is in both
    /// languages, and the user is asked.
    pub sure: bool,
}

/// Which way to translate a word that's in `found` (the card's languages,
/// in its order). With two languages looked up in, it's from the one the
/// word is in to the other; unless it's in both, when that's only a guess.
/// Otherwise it's from the word's language to `default.1`, or `default.0`
/// should that be the word's.
pub fn pick(found: &[&str], enabled: &[String], default: (&str, &str)) -> Direction {
    let direction = |from: &str, to: &str, sure| Direction {
        from: from.to_string(),
        to: to.to_string(),
        sure,
    };
    if let [a, b] = enabled {
        if supported(a) && supported(b) {
            let in_a = found.contains(&a.as_str());
            let in_b = found.contains(&b.as_str());
            match (in_a, in_b) {
                (true, false) => return direction(a, b, true),
                (false, true) => return direction(b, a, true),
                // In both: the card's first language comes first.
                (true, true) if found.first() == Some(&b.as_str()) => {
                    return direction(b, a, false)
                }
                (true, true) => return direction(a, b, false),
                (false, false) => {}
            }
        }
    }
    let from = found
        .first()
        .copied()
        .filter(|l| supported(l))
        .unwrap_or(default.0);
    let to = [default.1, default.0]
        .into_iter()
        .chain(LANGUAGES.iter().map(|(code, _)| *code))
        .find(|&l| l != from)
        .unwrap_or("en");
    direction(from, to, true)
}

/// Translates `word` between the languages of `direction`: Google first,
/// the website's endpoint and then gtx, then MyMemory, told what the word means by `context` (its first
/// definition), and `email`, if given, for its larger allowance.
pub fn translate(
    word: &str,
    from: &str,
    to: &str,
    context: Option<&str>,
    email: &str,
) -> Result<Translation, String> {
    google_web(word, from, to)
        .or_else(|_| google(word, from, to))
        .or_else(|_| mymemory(word, from, to, context, email))
}

fn get(host: &str, path: &str) -> Result<Vec<u8>, String> {
    match http::get(host, path) {
        Ok((200, body)) => Ok(body),
        Ok((429, _)) => Err(tr().too_many_translations.into()),
        Ok((code, _)) => Err(fill(tr().translation_service_error, &[("code", &code)])),
        Err(_) => Err(tr().translation_unreachable.into()),
    }
}

fn google(word: &str, from: &str, to: &str) -> Result<Translation, String> {
    let path = format!(
        "/translate_a/single?client=gtx&sl={}&tl={}&dt=t&dt=bd&q={}",
        percent_encode(from),
        percent_encode(to),
        percent_encode(word)
    );
    let body = get(GOOGLE_HOST, &path)?;
    parse_google(&body).ok_or_else(|| tr().no_translation.into())
}

/// The endpoint translate.google.com asks, with its dictionary. Its model
/// knows rarer words ("crateful") that gtx's leaves as they are.
fn google_web(word: &str, from: &str, to: &str) -> Result<Translation, String> {
    let path = format!(
        "/v1/translate?params.client=gtx&query.source_language={}&query.target_language={}\
         &query.display_language=en-US&query.text={}&key={GOOGLE_WEB_KEY}\
         &data_types=TRANSLATION&data_types=BILINGUAL_DICTIONARY_FULL",
        percent_encode(from),
        percent_encode(to),
        percent_encode(word)
    );
    let body = get(GOOGLE_WEB_HOST, &path)?;
    parse_google_web(&body).ok_or_else(|| tr().no_translation.into())
}

/// Reads the website endpoint's answer: the translation, then the
/// dictionary, as [`parse_google`] does gtx's.
fn parse_google_web(body: &[u8]) -> Option<Translation> {
    let json: Value = serde_json::from_slice(body).ok()?;
    let text = json.get("translation")?.as_str()?.trim().to_string();
    if text.is_empty() {
        return None;
    }
    let groups = json
        .get("bilingualDictionary")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let part_of_speech = entry.get("pos")?.as_str()?.to_lowercase();
            let terms: Vec<Term> = entry
                .get("entry")?
                .as_array()?
                .iter()
                .filter_map(|term| {
                    let text = term.get("word")?.as_str()?.to_string();
                    let back = term
                        .get("reverseTranslation")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .take(MAX_BACK)
                        .map(str::to_string)
                        .collect();
                    Some(Term { text, back })
                })
                .take(MAX_TERMS)
                .collect();
            (!terms.is_empty()).then_some(Group {
                part_of_speech,
                terms,
            })
        })
        .collect();
    Some(Translation {
        text,
        context: None,
        groups,
        source: GOOGLE,
    })
}

/// Reads gtx's nested arrays: the translated sentences first, then the
/// dictionary, one entry for each part of speech, with each translation
/// and what it means back in the word's language.
fn parse_google(body: &[u8]) -> Option<Translation> {
    let json: Value = serde_json::from_slice(body).ok()?;
    let text: String = json
        .get(0)?
        .as_array()?
        .iter()
        .filter_map(|sentence| sentence.get(0)?.as_str())
        .collect();
    let text = text.trim().to_string();
    if text.is_empty() {
        return None;
    }
    let groups = json
        .get(1)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let part_of_speech = entry.get(0)?.as_str()?.to_lowercase();
            let terms: Vec<Term> = entry
                .get(2)?
                .as_array()?
                .iter()
                .filter_map(|term| {
                    let text = term.get(0)?.as_str()?.to_string();
                    let back = term
                        .get(1)
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .take(MAX_BACK)
                        .map(str::to_string)
                        .collect();
                    Some(Term { text, back })
                })
                .take(MAX_TERMS)
                .collect();
            (!terms.is_empty()).then_some(Group {
                part_of_speech,
                terms,
            })
        })
        .collect();
    Some(Translation {
        text,
        context: None,
        groups,
        source: GOOGLE,
    })
}

#[derive(Deserialize)]
struct MyMemoryResponse {
    #[serde(rename = "responseData")]
    data: Option<MyMemoryData>,
    #[serde(rename = "quotaFinished", default)]
    quota_finished: Option<bool>,
    #[serde(rename = "responseStatus", default)]
    status: Value,
    #[serde(default)]
    matches: Vec<MyMemoryMatch>,
}

#[derive(Deserialize)]
struct MyMemoryData {
    #[serde(rename = "translatedText", default)]
    text: Option<String>,
}

#[derive(Deserialize)]
struct MyMemoryMatch {
    #[serde(default)]
    segment: String,
    #[serde(default)]
    translation: String,
}

const QUOTA_FINISHED: &str = "Today\u{2019}s free translations have run out. An email in \
     Settings \u{2192} Translation raises the limit.";

/// MyMemory, asked for `word` with its definition after it, when there's
/// one, so that it translates the sense meant; else for the word alone,
/// with the other translations it knows.
fn mymemory(
    word: &str,
    from: &str,
    to: &str,
    context: Option<&str>,
    email: &str,
) -> Result<Translation, String> {
    if let Some(context) = context.map(short_context).filter(|c| !c.is_empty()) {
        let query = format!("{word}{CONTEXT_SEPARATOR}{context}");
        let body = get(MYMEMORY_HOST, &mymemory_path(&query, from, to, email))?;
        let (text, _) = parse_mymemory(&body, &query)?;
        if let Some((text, context)) = split_context(&text) {
            return Ok(Translation {
                text,
                context: Some(context),
                groups: Vec::new(),
                source: MYMEMORY,
            });
        }
    }
    let body = get(MYMEMORY_HOST, &mymemory_path(word, from, to, email))?;
    let (text, alternatives) = parse_mymemory(&body, word)?;
    let groups = if alternatives.is_empty() {
        Vec::new()
    } else {
        vec![Group {
            part_of_speech: String::new(),
            terms: alternatives
                .into_iter()
                .map(|text| Term {
                    text,
                    back: Vec::new(),
                })
                .collect(),
        }]
    };
    Ok(Translation {
        text,
        context: None,
        groups,
        source: MYMEMORY,
    })
}

fn mymemory_path(query: &str, from: &str, to: &str, email: &str) -> String {
    let mut path = format!(
        "/get?q={}&langpair={}%7C{}",
        percent_encode(query),
        percent_encode(from),
        percent_encode(to)
    );
    let email = email.trim();
    if !email.is_empty() {
        path.push_str("&de=");
        path.push_str(&percent_encode(email));
    }
    path
}

/// The translation of `query`, and the other translations of it MyMemory
/// remembers, best first.
fn parse_mymemory(body: &[u8], query: &str) -> Result<(String, Vec<String>), String> {
    let response: MyMemoryResponse = serde_json::from_slice(body)
        .map_err(|_| "MyMemory\u{2019}s answer couldn\u{2019}t be read.".to_string())?;
    let text = response
        .data
        .and_then(|d| d.text)
        .map(|t| t.trim().to_string())
        .unwrap_or_default();
    // The allowance running out comes as a "translation" that says so.
    let warned = text.starts_with("MYMEMORY WARNING");
    let status = match &response.status {
        Value::Number(n) => n.as_u64().unwrap_or(200),
        Value::String(s) => s.parse().unwrap_or(200),
        _ => 200,
    };
    if response.quota_finished == Some(true) || warned || status == 429 {
        return Err(QUOTA_FINISHED.into());
    }
    if status != 200 || text.is_empty() {
        return Err("MyMemory had no translation.".into());
    }
    let mut alternatives: Vec<String> = Vec::new();
    for m in &response.matches {
        let alternative = m.translation.trim();
        let same = |a: &str, b: &str| a.to_lowercase() == b.to_lowercase();
        let new = !alternative.is_empty()
            && same(&m.segment, query)
            && !same(alternative, &text)
            && !same(alternative, query)
            && !alternatives.iter().any(|a| same(a, alternative));
        if new {
            alternatives.push(alternative.to_string());
        }
    }
    alternatives.truncate(MAX_ALTERNATIVES);
    Ok((text, alternatives))
}

/// The start of a definition, up to a word's end within
/// [`MAX_CONTEXT_CHARS`], without anything that would read as the separator.
fn short_context(definition: &str) -> String {
    let flat = definition.replace(':', ",");
    let flat = flat.trim();
    if flat.chars().count() <= MAX_CONTEXT_CHARS {
        return flat.to_string();
    }
    let cut: String = flat.chars().take(MAX_CONTEXT_CHARS).collect();
    match cut.rfind(' ') {
        Some(at) => cut[..at].trim_end_matches([',', ';']).to_string(),
        None => cut,
    }
}

/// The word's translation and its definition's, from the translation of the
/// two joined by [`CONTEXT_SEPARATOR`].
fn split_context(translated: &str) -> Option<(String, String)> {
    let (word, context) = translated.split_once(':')?;
    let (word, context) = (word.trim(), context.trim());
    // The word is short: a long "word" means the separator moved.
    let words = word.split_whitespace().count();
    (!word.is_empty() && !context.is_empty() && words <= 4)
        .then(|| (word.to_string(), context.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_google_dictionary() {
        let body = br#"[[["incarnare","embody",null,null,10]],[["verb",["incarnare","personificare"],[["incarnare",["embody","incarnate","personify"],null,0.23],["personificare",["personify","embody"],null,0.04]],"embody",2]],"en",null,null,null,1]"#;
        let t = parse_google(body).unwrap();
        assert_eq!(t.text, "incarnare");
        assert_eq!(t.source, GOOGLE);
        assert_eq!(t.groups.len(), 1);
        assert_eq!(t.groups[0].part_of_speech, "verb");
        assert_eq!(t.groups[0].terms[1].text, "personificare");
        assert_eq!(
            t.groups[0].terms[0].back,
            ["embody", "incarnate", "personify"]
        );
    }

    #[test]
    fn reads_google_sentences_without_dictionary() {
        // A phrase: several sentence pieces, and no dictionary.
        let body = br#"[[["Ciao ","Hello ",null,null,1],["mondo","world",null,null,1]],null,"en"]"#;
        let t = parse_google(body).unwrap();
        assert_eq!(t.text, "Ciao mondo");
        assert!(t.groups.is_empty());
        assert!(parse_google(br#"[[],null,"en"]"#).is_none());
        assert!(parse_google(b"<html>").is_none());
    }

    #[test]
    fn reads_google_web() {
        let body = br#"{"translation":"incarnare","bilingualDictionary":[{"pos":"verb","entry":[{"word":"incarnare","reverseTranslation":["embody","incarnate"],"score":0.23}],"baseForm":"embody"}],"sourceLanguage":"en"}"#;
        let t = parse_google_web(body).unwrap();
        assert_eq!(t.text, "incarnare");
        assert_eq!(t.groups[0].part_of_speech, "verb");
        assert_eq!(t.groups[0].terms[0].back, ["embody", "incarnate"]);
        let plain = parse_google_web(br#"{"translation":"cassa piena","sentences":[]}"#).unwrap();
        assert_eq!(plain.text, "cassa piena");
        assert!(plain.groups.is_empty());
        assert!(parse_google_web(br#"{"error":{"code":403}}"#).is_none());
    }

    #[test]
    fn reads_mymemory() {
        let body = br#"{"responseData":{"translatedText":"cane","match":1},"quotaFinished":false,"responseStatus":200,"matches":[{"segment":"dog","translation":"cane"},{"segment":"Dog","translation":"Cane"},{"segment":"dog","translation":"segugio"},{"segment":"hot dog","translation":"panino"}]}"#;
        let (text, alternatives) = parse_mymemory(body, "dog").unwrap();
        assert_eq!(text, "cane");
        assert_eq!(alternatives, ["segugio"]);
    }

    #[test]
    fn tells_mymemory_ran_out() {
        let warned = br#"{"responseData":{"translatedText":"MYMEMORY WARNING: YOU USED ALL AVAILABLE FREE TRANSLATIONS FOR TODAY"},"quotaFinished":true,"responseStatus":"429","matches":[]}"#;
        assert_eq!(parse_mymemory(warned, "dog").unwrap_err(), QUOTA_FINISHED);
        let failed = br#"{"responseData":{"translatedText":""},"responseStatus":"403"}"#;
        assert!(parse_mymemory(failed, "dog").is_err());
    }

    #[test]
    fn asks_mymemory_with_the_email() {
        assert_eq!(
            mymemory_path("a dog", "en", "it", " me@example.com "),
            "/get?q=a%20dog&langpair=en%7Cit&de=me%40example.com"
        );
        assert_eq!(
            mymemory_path("dog", "en", "it", ""),
            "/get?q=dog&langpair=en%7Cit"
        );
    }

    #[test]
    fn splits_the_context_off() {
        assert_eq!(
            split_context("incarnare: rappresentare in forma fisica"),
            Some(("incarnare".into(), "rappresentare in forma fisica".into()))
        );
        assert_eq!(split_context("incarnare rappresentare"), None);
        assert_eq!(
            split_context("rappresentare in una forma molto fisica: x"),
            None
        );
        assert_eq!(short_context("a: b"), "a, b");
        let long = "word ".repeat(60);
        let short = short_context(&long);
        assert!(short.chars().count() <= MAX_CONTEXT_CHARS);
        assert!(short.ends_with("word"));
    }

    #[test]
    fn picks_the_direction() {
        let both = ["en".to_string(), "it".to_string()];
        let default = ("en", "fr");
        let pick = |found: &[&str], enabled: &[String]| pick(found, enabled, default);
        // From the language the word is in, to the other one.
        assert_eq!(
            pick(&["it"], &both),
            Direction {
                from: "it".into(),
                to: "en".into(),
                sure: true
            }
        );
        assert_eq!(
            pick(&["en"], &both),
            Direction {
                from: "en".into(),
                to: "it".into(),
                sure: true
            }
        );
        // In both: a guess, from the card's first.
        assert_eq!(
            pick(&["it", "en"], &both),
            Direction {
                from: "it".into(),
                to: "en".into(),
                sure: false
            }
        );
        assert_eq!(
            pick(&["en", "it"], &both),
            Direction {
                from: "en".into(),
                to: "it".into(),
                sure: false
            }
        );
        // One language: into the default.
        let english = ["en".to_string()];
        assert_eq!(
            pick(&["en"], &english),
            Direction {
                from: "en".into(),
                to: "fr".into(),
                sure: true
            }
        );
        // Nothing found: the defaults.
        assert_eq!(
            pick(&[], &english),
            Direction {
                from: "en".into(),
                to: "fr".into(),
                sure: true
            }
        );
        // Already in the default target language: into the other default.
        assert_eq!(
            pick(&["fr"], &english),
            Direction {
                from: "fr".into(),
                to: "en".into(),
                sure: true
            }
        );
    }

    #[test]
    fn names_languages() {
        assert_eq!(name("it"), "Italiano");
        assert_eq!(name("xx"), "XX");
        assert!(LANGUAGES.iter().all(|(code, _)| supported(code)));
        assert!(LANGUAGES
            .iter()
            .all(|(code, _)| *code == "en" || english_name(code).is_some()));
    }

    #[test]
    fn searches_languages() {
        assert_eq!(search("").len(), LANGUAGES.len());
        // In their own language, in English, or by code, accents aside.
        assert_eq!(search("franc"), ["fr"]);
        assert_eq!(search("Germ"), ["de"]);
        assert_eq!(search("deu"), ["de"]);
        assert_eq!(search("ES"), ["es"]);
        assert_eq!(search("cest"), ["cs"]);
        assert_eq!(search("chinese"), ["zh-CN"]);
        assert_eq!(search("cn"), ["zh-CN"]);
        assert_eq!(search("  ital "), ["it"]);
        assert!(search("klingon").is_empty());
    }
}
