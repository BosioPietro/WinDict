//! Dictionary entries, as the card shows them, and what's shared by where
//! they come from: the bundled dictionary (`offline.rs`) and the online
//! ones (`online.rs`).

use crate::lang::LangCode;

/// A word in one language, followed by what it is in the others looked up
/// in, which the card shows under it, each under a heading of its own.
#[derive(Clone, Debug)]
pub struct Definition {
    pub word: String,
    /// The code of the language it's in ("en").
    pub language: LangCode,
    pub phonetic: Option<String>,
    pub meanings: Vec<Meaning>,
    /// Shown above the senses, e.g. when an inflection was resolved.
    pub note: Option<String>,
    /// Where the content came from, for the footer.
    pub sources: Vec<&'static str>,
    /// The word in the other languages, in order of preference. Their own
    /// `others` are empty.
    pub others: Vec<Definition>,
}

impl Definition {
    /// Leaves out the example sentences, when they're turned off.
    pub fn without_examples(mut self) -> Definition {
        for sense in self.meanings.iter_mut().flat_map(|m| &mut m.senses) {
            sense.example = None;
        }
        self
    }

    /// Each language's part: this one, then the others.
    pub fn sections(&self) -> impl Iterator<Item = &Definition> {
        std::iter::once(self).chain(&self.others)
    }

    /// Only the part in `language`, if the word is in it.
    pub fn only(self, language: &str) -> Option<Definition> {
        self.into_sections()
            .into_iter()
            .find(|s| *s.language == *language)
    }

    fn into_sections(mut self) -> Vec<Definition> {
        let others = std::mem::take(&mut self.others);
        std::iter::once(self).chain(others).collect()
    }

    /// Where every language's part came from, for the footer: each source
    /// once, with the word it has the page for.
    pub fn all_sources(&self) -> Vec<(&'static str, &str)> {
        let mut out: Vec<(&'static str, &str)> = Vec::new();
        for section in self.sections() {
            for &source in &section.sources {
                if !out.iter().any(|(s, _)| *s == source) {
                    out.push((source, &section.word));
                }
            }
        }
        out
    }
}

/// One definition of the word in each of the languages it was found in,
/// the first of `order` (the languages, by preference) leading.
pub fn combine(mut sections: Vec<Definition>, order: &[LangCode]) -> Option<Definition> {
    let rank = |d: &Definition| {
        order
            .iter()
            .position(|l| *l == d.language)
            .unwrap_or(order.len())
    };
    // Stable: those in no known order stay as they came.
    sections.sort_by_key(rank);
    let mut sections = sections.into_iter().flat_map(Definition::into_sections);
    let mut lead = sections.next()?;
    lead.others = sections.collect();
    Some(lead)
}

/// How much of an entry a lookup returns, as the settings ask.
#[derive(Clone, Copy, Debug)]
pub struct Detail {
    /// The most senses per part of speech.
    pub senses: usize,
    /// The most synonyms per part of speech.
    pub synonyms: usize,
    /// Whether the example sentences are kept.
    pub examples: bool,
}

impl Detail {
    /// `def` with only what's asked for. The limits on senses and synonyms
    /// are kept as it's put together; this drops what's left.
    pub fn apply(self, def: Definition) -> Definition {
        if self.examples {
            def
        } else {
            def.without_examples()
        }
    }
}

pub const WIKTIONARY: &str = "Wiktionary";
/// The Italian Wiktionary, as the Italian dictionary also credits it.
pub const WIKIZIONARIO: &str = "Wikizionario";
/// The Spanish one, likewise.
pub const WIKCIONARIO: &str = "Wikcionario";

#[derive(Clone, Debug)]
pub struct Meaning {
    pub part_of_speech: String,
    pub senses: Vec<Sense>,
    pub synonyms: Vec<String>,
    pub antonyms: Vec<String>,
}

/// Most opposites shown per part of speech. Words rarely have more.
pub const MAX_ANTONYMS: usize = 6;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NormalizeError {
    /// No letters or digits at all.
    Empty,
    /// More than a short phrase.
    TooLong,
}

/// The most words, and characters, a lookup takes: a short phrase.
const MAX_LOOKUP_WORDS: usize = 4;
pub const MAX_LOOKUP_CHARS: usize = 60;

/// Cleans up raw selected text into something worth looking up.
pub fn normalize(raw: &str) -> Result<String, NormalizeError> {
    // Web pages and word processors write the apostrophe curly.
    let folded: String = raw
        .chars()
        .map(|c| match c {
            '\u{2018}' | '\u{2019}' | '\u{02BC}' => '\'',
            c => c,
        })
        .collect();
    let trimmed = folded.trim_matches(|c: char| !c.is_alphanumeric());
    // A leading apostrophe is part of some words ("'ndrangheta", "'tis"),
    // unless it opens a quote that closes at the end.
    let outer = folded.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'');
    let leading = outer
        .strip_prefix('\'')
        .is_some_and(|rest| rest.starts_with(char::is_alphanumeric) && !rest.ends_with('\''));
    let words: Vec<&str> = trimmed.split_whitespace().collect();
    if words.is_empty() {
        return Err(NormalizeError::Empty);
    }
    if words.len() > MAX_LOOKUP_WORDS || trimmed.chars().count() > MAX_LOOKUP_CHARS {
        return Err(NormalizeError::TooLong);
    }
    let word = words.join(" ").to_lowercase();
    Ok(if leading { format!("'{word}") } else { word })
}

/// Fills what the offline entry lacks (pronunciation, parts of speech the
/// dictionary doesn't cover, languages it hasn't the word in) from an
/// online entry. `order` is the languages, by preference.
pub fn merge(local: Definition, online: Option<&Lookup>, order: &[LangCode]) -> Definition {
    let Some(Lookup::Found(online)) = online else {
        return local;
    };
    let mut sections = local.clone().into_sections();
    for theirs in online.sections() {
        match sections.iter_mut().find(|s| s.language == theirs.language) {
            // The same word: what the dictionary has, it says better. A
            // word whose base form was shown ("case" → "casa") is left as
            // the dictionary has it.
            Some(ours) if ours.word.to_lowercase() == theirs.word.to_lowercase() => {
                fill(ours, theirs)
            }
            Some(_) => {}
            None => sections.push(Definition {
                others: Vec::new(),
                ..theirs.clone()
            }),
        }
    }
    combine(sections, order).unwrap_or(local)
}

fn fill(local: &mut Definition, online: &Definition) {
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
    for &source in online.sources.iter().filter(|_| used) {
        if !local.sources.contains(&source) {
            local.sources.push(source);
        }
    }
}

/// A web page for `word` at `source`, for the card's footer links.
pub fn source_url(source: &str, word: &str) -> Option<String> {
    match source {
        WIKTIONARY => Some(format!(
            "https://en.wiktionary.org/wiki/{}",
            percent_encode(&word.replace(' ', "_"))
        )),
        _ => match crate::kaikki::EDITIONS.iter().find(|e| e.source == source) {
            Some(edition) => Some(format!(
                "https://{}/wiki/{}",
                edition.host,
                percent_encode(&word.replace(' ', "_"))
            )),
            None => crate::offline::link(source, word),
        },
    }
}

pub fn percent_encode(s: &str) -> String {
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

    /// The built-in dictionary's name.
    const WORDNET: &str = "Open English WordNet";

    #[test]
    fn normalizes_selection() {
        assert_eq!(normalize("  Hello, ").unwrap(), "hello");
        assert_eq!(normalize("“serendipity”.").unwrap(), "serendipity");
        assert_eq!(normalize("ice  cream\r\n").unwrap(), "ice cream");
        assert_eq!(normalize("don't").unwrap(), "don't");
        assert_eq!(normalize("Don\u{2019}t").unwrap(), "don't");
        assert_eq!(normalize("'ndrangheta,").unwrap(), "'ndrangheta");
        assert_eq!(normalize("\u{2018}hello\u{2019}").unwrap(), "hello");
        assert_eq!(normalize(" ... "), Err(NormalizeError::Empty));
        assert_eq!(
            normalize("this is a much longer sentence than a word"),
            Err(NormalizeError::TooLong)
        );
    }

    #[test]
    fn encodes_paths() {
        assert_eq!(percent_encode("ice cream"), "ice%20cream");
        assert_eq!(percent_encode("café"), "caf%C3%A9");
    }

    fn entry(phonetic: Option<&str>, parts: &[&str], sources: Vec<&'static str>) -> Definition {
        Definition {
            word: "run".into(),
            language: "en".into(),
            phonetic: phonetic.map(String::from),
            meanings: parts
                .iter()
                .map(|pos| Meaning {
                    part_of_speech: pos.to_string(),
                    senses: vec![Sense {
                        text: format!("A {pos} sense."),
                        example: Some("An example.".into()),
                    }],
                    synonyms: Vec::new(),
                    antonyms: Vec::new(),
                })
                .collect(),
            note: None,
            sources,
            others: Vec::new(),
        }
    }

    fn order() -> Vec<LangCode> {
        vec!["en".into(), "it".into()]
    }

    fn italian(word: &str, parts: &[&str], sources: Vec<&'static str>) -> Definition {
        Definition {
            word: word.into(),
            language: "it".into(),
            ..entry(None, parts, sources)
        }
    }

    #[test]
    fn merges_what_the_offline_entry_lacks() {
        let merge = |local, online| super::merge(local, online, &order());
        let local = entry(None, &["verb"], vec![WORDNET]);
        let online = Lookup::Found(entry(Some("/ɹʌn/"), &["verb", "noun"], vec![WIKTIONARY]));
        let merged = merge(local.clone(), Some(&online));
        assert_eq!(merged.phonetic.as_deref(), Some("/ɹʌn/"));
        let parts: Vec<_> = merged.meanings.iter().map(|m| &m.part_of_speech).collect();
        assert_eq!(parts, ["verb", "noun"]);
        assert_eq!(merged.sources, [WORDNET, WIKTIONARY]);
        // Nothing new: the offline entry stays as it was, sources and all.
        let same = Lookup::Found(entry(None, &["verb"], vec![WIKTIONARY]));
        assert_eq!(merge(local.clone(), Some(&same)).sources, [WORDNET]);
        for nothing in [
            None,
            Some(&Lookup::NotFound),
            Some(&Lookup::Failed("x".into())),
        ] {
            let kept = merge(local.clone(), nothing);
            assert!(kept.phonetic.is_none() && kept.meanings.len() == 1);
        }
    }

    #[test]
    fn combines_languages() {
        let it = italian("run", &["noun"], vec!["Wikizionario"]);
        let en = entry(None, &["verb"], vec![WORDNET]);
        // In order of preference, whatever order they're found in.
        let both = combine(vec![it.clone(), en.clone()], &order()).unwrap();
        let languages: Vec<_> = both.sections().map(|s| s.language.as_str()).collect();
        assert_eq!(languages, ["en", "it"]);
        assert_eq!(
            both.all_sources(),
            [(WORDNET, "run"), ("Wikizionario", "run")]
        );
        assert!(combine(Vec::new(), &order()).is_none());
        // Picked in one language, only that part is left.
        let only = both.clone().only("it").unwrap();
        assert_eq!((&*only.language, only.others.len()), ("it", 0));
        assert_eq!(only.all_sources(), [("Wikizionario", "run")]);
        assert!(both.clone().only("fr").is_none());
        // Online, a language the offline dictionaries lack is added in its
        // place; one they have the base form of is left as it is.
        let local = combine(
            vec![italian("casa", &["noun"], vec!["Wikizionario"])],
            &order(),
        );
        let online = combine(
            vec![
                italian("case", &["noun"], vec![WIKTIONARY]),
                entry(None, &["noun"], vec![WIKTIONARY]),
            ],
            &order(),
        )
        .unwrap();
        let merged = merge(local.unwrap(), Some(&Lookup::Found(online)), &order());
        let words: Vec<_> = merged
            .sections()
            .map(|s| (&*s.language, &*s.word))
            .collect();
        assert_eq!(words, [("en", "run"), ("it", "casa")]);
        assert_eq!(merged.others[0].sources, ["Wikizionario"]);
    }

    #[test]
    fn leaves_out_examples() {
        let def = entry(None, &["verb", "noun"], Vec::new()).without_examples();
        assert!(def
            .meanings
            .iter()
            .flat_map(|m| &m.senses)
            .all(|s| s.example.is_none()));
    }

    #[test]
    fn links_to_sources() {
        assert_eq!(
            source_url(WIKTIONARY, "ice cream").as_deref(),
            Some("https://en.wiktionary.org/wiki/ice_cream")
        );
        assert_eq!(
            source_url(WORDNET, "ice cream").as_deref(),
            Some("https://en-word.net/lemma/ice%20cream")
        );
        assert_eq!(
            source_url(WIKIZIONARIO, "a priori").as_deref(),
            Some("https://it.wiktionary.org/wiki/a_priori")
        );
        assert_eq!(
            source_url(WIKCIONARIO, "casa").as_deref(),
            Some("https://es.wiktionary.org/wiki/casa")
        );
        assert_eq!(source_url("Elsewhere", "x"), None);
    }
}
