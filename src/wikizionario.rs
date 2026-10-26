//! Wikizionario, the Italian Wiktionary, online: its entries as wiktextract
//! extracts them (<https://kaikki.org/itwiktionary/>, which the Italian
//! dictionary is built from), and its pages themselves, through the
//! MediaWiki API, for the words the extract doesn't have yet.

use serde::Deserialize;

use crate::dictionary::{
    percent_encode, Definition, Lookup, Meaning, Sense, MAX_ANTONYMS, WIKIZIONARIO,
};
use crate::i18n::{fill, tr};
use crate::offline::sentence;
use crate::online::{fetch, same_word, strip_html};

/// The word's entries in the extract, following an inflected form ("case")
/// to its base ("casa") when that's all there is.
pub fn lookup_extract(word: &str, max_senses: usize, max_synonyms: usize) -> Lookup {
    let mut base = None;
    let result = fetch("kaikki.org", &extract_path(word), |body| {
        let entries = extract_entries(body);
        base = base_form(&entries, word);
        parse_extract(&entries, max_senses, max_synonyms)
    });
    match (result, base) {
        (Lookup::NotFound, Some(base)) => {
            match fetch("kaikki.org", &extract_path(&base), |body| {
                parse_extract(&extract_entries(body), max_senses, max_synonyms)
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

/// The word's page, as it's written.
pub fn lookup_page(word: &str, max_senses: usize, max_synonyms: usize) -> Lookup {
    let path = format!(
        "/w/api.php?action=parse&format=json&formatversion=2&prop=wikitext&redirects=1&page={}",
        percent_encode(word)
    );
    fetch("it.wiktionary.org", &path, |body| {
        // A missing page is an error in an otherwise successful response.
        let parsed = serde_json::from_slice::<PageResponse>(body).ok()?.parse?;
        parse_page(&parsed.title, &parsed.wikitext, max_senses, max_synonyms)
    })
}

/// Where the extract keeps a word: under its first letter, then its first
/// two ("c/ca/casa"), case and accents as they are.
fn extract_path(word: &str) -> String {
    let first: String = word.chars().take(1).collect();
    let two: String = word.chars().take(2).collect();
    format!(
        "/itwiktionary/Italiano/meaning/{}/{}/{}.jsonl",
        percent_encode(&first),
        percent_encode(&two),
        percent_encode(word)
    )
}

#[derive(Deserialize)]
struct ExtractEntry {
    #[serde(default)]
    word: String,
    #[serde(default)]
    lang_code: String,
    #[serde(default)]
    pos: String,
    #[serde(default)]
    senses: Vec<ExtractSense>,
    #[serde(default)]
    sounds: Vec<ExtractSound>,
    #[serde(default)]
    synonyms: Vec<ExtractWord>,
    #[serde(default)]
    antonyms: Vec<ExtractWord>,
}

#[derive(Deserialize)]
struct ExtractSense {
    #[serde(default)]
    glosses: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    form_of: Vec<ExtractWord>,
    #[serde(default)]
    examples: Vec<ExtractExample>,
}

impl ExtractSense {
    fn is_form(&self) -> bool {
        !self.form_of.is_empty() || self.tags.iter().any(|t| t == "form-of")
    }
}

#[derive(Deserialize)]
struct ExtractSound {
    ipa: Option<String>,
}

#[derive(Deserialize)]
struct ExtractWord {
    #[serde(default)]
    word: String,
}

fn words(list: &[ExtractWord]) -> Vec<&str> {
    list.iter().map(|w| w.word.as_str()).collect()
}

#[derive(Deserialize)]
struct ExtractExample {
    #[serde(default)]
    text: String,
}

/// The Italian entries, one per line; the rest of the page's languages are
/// in other files.
fn extract_entries(body: &[u8]) -> Vec<ExtractEntry> {
    body.split(|&b| b == b'\n')
        .filter_map(|line| serde_json::from_slice::<ExtractEntry>(line).ok())
        .filter(|e| e.lang_code == "it")
        .collect()
}

/// What `word` is a form of, when it is one.
fn base_form(entries: &[ExtractEntry], word: &str) -> Option<String> {
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
fn extract_part_of_speech(pos: &str) -> Option<&'static str> {
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

fn parse_extract(
    entries: &[ExtractEntry],
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
        let Some(pos) = extract_part_of_speech(&entry.pos) else {
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
    definition(word?, phonetic, meanings, synonyms, antonyms)
}

#[derive(Deserialize)]
struct PageResponse {
    parse: Option<Page>,
}

#[derive(Deserialize)]
struct Page {
    title: String,
    #[serde(default)]
    wikitext: String,
}

/// What a line of the page's Italian section is part of.
enum Section {
    /// A part of speech's senses, `None` for those not looked up as words.
    Senses(Option<&'static str>),
    Synonyms,
    Antonyms,
    Pronunciation,
    Other,
}

/// A section's heading, "{{-sost-|it}}" or "{{-sin-}}": senses come under
/// headings for a language.
fn section(line: &str) -> Option<Section> {
    let inside = line.strip_prefix("{{-")?.strip_suffix("}}")?;
    let mut args = inside.split('|');
    let name = args.next()?.trim_end_matches('-').trim();
    let language = args.next().map(str::trim);
    Some(match (name, language) {
        (name, Some("it")) => Section::Senses(page_part_of_speech(name)),
        (_, Some(_)) => Section::Other,
        ("sin", None) => Section::Synonyms,
        ("ant", None) => Section::Antonyms,
        ("pron", None) => Section::Pronunciation,
        _ => Section::Other,
    })
}

/// A page's parts of speech, as the card names them. An inflected form's
/// ("sost form") are those of its base.
fn page_part_of_speech(name: &str) -> Option<&'static str> {
    let name = name.strip_suffix(" form").unwrap_or(name);
    Some(match name {
        "sost" | "nome" => "noun",
        "verb" => "verb",
        "agg" => "adjective",
        "avv" => "adverb",
        "pronome" => "pronoun",
        "prep" => "preposition",
        "cong" => "conjunction",
        "inter" => "interjection",
        "art" => "article",
        "acron" | "abbr" | "sigla" => "abbreviation",
        "espr" => "phrase",
        _ if name.starts_with("loc ") => "phrase",
        _ => return None,
    })
}

fn parse_page(
    title: &str,
    wikitext: &str,
    max_senses: usize,
    max_synonyms: usize,
) -> Option<Definition> {
    let mut meanings: Vec<Meaning> = Vec::new();
    let mut phonetic = None;
    let mut synonyms: Vec<String> = Vec::new();
    let mut antonyms: Vec<String> = Vec::new();
    let mut italian = false;
    let mut current = Section::Other;
    let mut senses: Vec<Sense> = Vec::new();
    // Under a line that stands for the word itself ("# {{Pn}} ''f sing''"),
    // the senses are a level down.
    let mut nested = false;
    // Whether the last sense was kept, for its examples.
    let mut kept = false;
    let flush = |meanings: &mut Vec<Meaning>, current: &Section, senses: &mut Vec<Sense>| {
        if let Section::Senses(Some(pos)) = current {
            add_senses(meanings, pos, std::mem::take(senses), max_senses);
        }
    };

    for line in wikitext.lines().map(str::trim) {
        if line.starts_with("==") && line.ends_with("==") {
            flush(&mut meanings, &current, &mut senses);
            italian = line.contains("{{-it-}}");
            current = Section::Other;
            continue;
        }
        if !italian {
            continue;
        }
        if let Some(next) = section(line) {
            flush(&mut meanings, &current, &mut senses);
            current = next;
            nested = false;
            kept = false;
            continue;
        }
        match current {
            Section::Senses(Some(_)) => {
                let marks = line
                    .find(|c| !matches!(c, '#' | '*' | ':'))
                    .unwrap_or(line.len());
                let (marks, rest) = line.split_at(marks);
                if !marks.starts_with('#') {
                    continue;
                }
                let depth = marks.len() - usize::from(nested);
                if marks == "#" && rest.trim_start().starts_with("{{Pn") {
                    nested = true;
                } else if depth == 1 && (marks == "#" || nested) {
                    let text = plain(rest, title);
                    kept = !text.is_empty() && senses.len() < max_senses;
                    if kept {
                        senses.push(Sense {
                            text: sentence(&text),
                            example: None,
                        });
                    }
                } else if depth == 2 && kept {
                    let text = plain(rest, title);
                    if let Some(sense) = senses.last_mut() {
                        if sense.example.is_none() && !text.is_empty() {
                            sense.example = Some(text);
                        }
                    }
                }
            }
            Section::Synonyms | Section::Antonyms if line.starts_with('*') => {
                let list = match current {
                    Section::Synonyms => &mut synonyms,
                    _ => &mut antonyms,
                };
                list.extend(listed(line.trim_start_matches('*'), title));
            }
            Section::Pronunciation if phonetic.is_none() => {
                phonetic = line
                    .split("{{IPA|")
                    .nth(1)
                    .and_then(|ipa| ipa.split(['|', '}']).next())
                    .map(str::trim)
                    .filter(|ipa| !ipa.is_empty())
                    .map(str::to_owned);
            }
            _ => {}
        }
    }
    flush(&mut meanings, &current, &mut senses);
    let synonyms = related(
        synonyms.iter().map(String::as_str).collect(),
        title,
        max_synonyms,
    );
    let antonyms = related(
        antonyms.iter().map(String::as_str).collect(),
        title,
        MAX_ANTONYMS,
    );
    definition(title.to_string(), phonetic, meanings, synonyms, antonyms)
}

/// The words a line of synonyms lists: each word it links to where it
/// runs several together, else what's between the commas.
fn listed(line: &str, word: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in line.split(',') {
        let links = link_targets(part);
        if links.len() > 1 {
            out.extend(links);
        } else {
            out.push(replace_spans(&plain(part, word), "(", ")", &|_| {
                String::new()
            }));
        }
    }
    out
}

fn link_targets(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("[[") {
        let inside = &rest[start + 2..];
        let Some(end) = inside.find("]]") else { break };
        out.push(link(&inside[..end]));
        rest = &inside[end + 2..];
    }
    out
}

/// A line of wikitext as text: templates left out (but the one for the
/// word itself), links as their labels, no bold or italics.
fn plain(wikitext: &str, word: &str) -> String {
    let text = without_refs(wikitext);
    let text = replace_spans(&text, "{{", "}}", &|inside| match inside
        .split('|')
        .next()
        .map(str::trim)
    {
        Some("Pn") => word.to_string(),
        _ => String::new(),
    });
    let text = replace_spans(&text, "[[", "]]", &link);
    strip_html(&text.replace("'''", "").replace("''", ""))
}

/// A link's label; none for files and categories.
fn link(inside: &str) -> String {
    let target = inside.split('|').next().unwrap_or("").trim();
    let namespace = target.split(':').next().unwrap_or("").to_lowercase();
    if target.contains(':')
        && ["file", "immagine", "image", "categoria", "category"].contains(&namespace.as_str())
    {
        return String::new();
    }
    match inside.split_once('|') {
        Some((_, label)) => replace_spans(label, "[[", "]]", &link),
        None => target.split('#').next().unwrap_or("").to_string(),
    }
}

/// Leaves out footnotes, `<ref>…</ref>` and `<ref … />`.
fn without_refs(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<ref") {
        out.push_str(&rest[..start]);
        let tag = &rest[start..];
        let Some(close) = tag.find('>') else {
            return out;
        };
        rest = if tag[..close].ends_with('/') {
            &tag[close + 1..]
        } else {
            match tag.find("</ref>") {
                Some(end) => &tag[end + "</ref>".len()..],
                None => "",
            }
        };
    }
    out.push_str(rest);
    out
}

/// `text` with each `open … close` span, the nested ones included, replaced
/// by what `replace` makes of its inside. An unclosed one is left out.
fn replace_spans(text: &str, open: &str, close: &str, replace: &dyn Fn(&str) -> String) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(open) {
        out.push_str(&rest[..start]);
        let inside = &rest[start + open.len()..];
        let (mut depth, mut i) = (1, 0);
        let end = loop {
            let next_open = inside[i..].find(open).map(|p| p + i);
            let next_close = inside[i..].find(close).map(|p| p + i);
            match (next_open, next_close) {
                (Some(o), Some(c)) if o < c => {
                    depth += 1;
                    i = o + open.len();
                }
                (_, Some(c)) => {
                    depth -= 1;
                    if depth == 0 {
                        break Some(c);
                    }
                    i = c + close.len();
                }
                (_, None) => break None,
            }
        };
        match end {
            Some(end) => {
                out.push_str(&replace(&inside[..end]));
                rest = &inside[end + close.len()..];
            }
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// Synonyms or opposites worth showing: words or short phrases, not
/// questions or fragments, each once, and not the word itself.
fn related(words: Vec<&str>, word: &str, max: usize) -> Vec<String> {
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
fn add_senses(meanings: &mut Vec<Meaning>, pos: &str, senses: Vec<Sense>, max_senses: usize) {
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
fn definition(
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
        language: "it".into(),
        phonetic,
        meanings,
        note: None,
        sources: vec![WIKIZIONARIO],
        others: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_words_in_the_extract() {
        assert_eq!(
            extract_path("casa"),
            "/itwiktionary/Italiano/meaning/c/ca/casa.jsonl"
        );
        assert_eq!(
            extract_path("a"),
            "/itwiktionary/Italiano/meaning/a/a/a.jsonl"
        );
        assert_eq!(
            extract_path("è"),
            "/itwiktionary/Italiano/meaning/%C3%A8/%C3%A8/%C3%A8.jsonl"
        );
        assert_eq!(
            extract_path("a priori"),
            "/itwiktionary/Italiano/meaning/a/a%20/a%20priori.jsonl"
        );
    }

    #[test]
    fn parses_the_extract() {
        let body = br#"{"word": "correre", "lang_code": "it", "pos": "verb", "senses": [{"glosses": ["procedere velocemente"], "examples": [{"text": "Corse via dalla paura."}]}, {"glosses": ["fare qualcosa in fretta"]}, {"glosses": ["casa ( approfondimento) f sing"]}], "sounds": [{"ipa": "/korrere/"}], "synonyms": [{"word": "accorrere"}, {"word": "correre"}, {"word": "?"}], "antonyms": [{"word": "camminare"}]}
{"word": "correre", "lang_code": "it", "pos": "noun", "senses": [{"glosses": ["il correre"]}]}
{"word": "correre", "lang_code": "it", "pos": "letter", "senses": [{"glosses": ["x"]}]}"#;
        let entries = extract_entries(body);
        assert_eq!(base_form(&entries, "correre"), None);
        let def = parse_extract(&entries, 5, 5).unwrap();
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
            parse_extract(&entries, 1, 5).unwrap().meanings[0]
                .senses
                .len(),
            1
        );
    }

    #[test]
    fn follows_forms_in_the_extract() {
        let body = br#"{"word": "case", "lang_code": "it", "pos": "noun", "senses": [{"glosses": ["plurale di casa"], "tags": ["form-of"], "form_of": [{"word": "casa"}]}]}
{"word": "case", "lang_code": "en", "pos": "noun", "senses": [{"glosses": ["box"]}]}"#;
        let entries = extract_entries(body);
        assert_eq!(entries.len(), 1);
        assert_eq!(base_form(&entries, "case").as_deref(), Some("casa"));
        assert!(parse_extract(&entries, 5, 5).is_none());
    }

    const PAGE: &str = "{{Vedi|casă}}
== {{-it-}} ==
[[File:Casa.jpg|thumb|una  [[vecchia]] casa]]
{{-sost-|it}}
# {{Pn|w}} ''f sing'' {{Linkp|case}}
#* {{Term|architettura|it}} [[edificio]] costruito per essere [[abitazione|abitato]]<ref>Treccani</ref> [[File:X.jpg|thumb|una [[casa]]]]
#** ''un mio amico ha acquistato una bella '''{{Pn}}''' ''
#* [[dimora]] di una [[persona]]
{{-agg-|it}}
#che corrisponde ai canoni
#: ''questo è bello''
# {{Fig}} che [[suscita]] entusiasmo
{{-lett-|it}}
# la lettera c
{{-pron-}}
{{IPA|/ˈkasa/}}
{{-sin-}}
*''(esteticamente)'' [[abitazione]], [[dimora]] [[magione]], appartamento
* {{Fig}} ''(di famiglia)'' [[famiglia]]
{{-ant-}}
* [[casa]], senzatetto?
== {{-en-}} ==
{{-sost-|en}}
# box
";

    #[test]
    fn parses_a_page() {
        let def = parse_page("casa", PAGE, 5, 10).unwrap();
        assert_eq!(def.word, "casa");
        assert_eq!(def.phonetic.as_deref(), Some("/ˈkasa/"));
        let parts: Vec<_> = def.meanings.iter().map(|m| &*m.part_of_speech).collect();
        assert_eq!(parts, ["noun", "adjective"]);
        let noun = &def.meanings[0];
        assert_eq!(
            noun.senses[0].text,
            "Edificio costruito per essere abitato."
        );
        assert_eq!(
            noun.senses[0].example.as_deref(),
            Some("un mio amico ha acquistato una bella casa")
        );
        assert_eq!(noun.senses[1].text, "Dimora di una persona.");
        assert_eq!(
            noun.synonyms,
            [
                "abitazione",
                "dimora",
                "magione",
                "appartamento",
                "famiglia"
            ]
        );
        assert!(noun.antonyms.is_empty());
        let adjective = &def.meanings[1];
        assert_eq!(adjective.senses[0].text, "Che corrisponde ai canoni.");
        assert_eq!(
            adjective.senses[0].example.as_deref(),
            Some("questo è bello")
        );
        assert_eq!(adjective.senses[1].text, "Che suscita entusiasmo.");
        assert_eq!(
            parse_page("casa", PAGE, 1, 10).unwrap().meanings[0]
                .senses
                .len(),
            1
        );
        assert!(parse_page("box", "== {{-en-}} ==\n{{-sost-|en}}\n# box", 5, 5).is_none());
    }

    #[test]
    fn makes_wikitext_plain() {
        assert_eq!(
            plain("[[a|b]] [[c#Italiano]] '''d''' ''e''", "w"),
            "b c d e"
        );
        assert_eq!(plain("x {{T|{{U}}|[[y]]}} {{Pn}}", "w"), "x w");
        assert_eq!(plain("a<ref name=\"n\"/> b<ref>c</ref> d", "w"), "a b d");
        assert_eq!(
            plain("a [[File:x.jpg|thumb|[[b]] c]] d {{unclosed", "w"),
            "a d"
        );
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
