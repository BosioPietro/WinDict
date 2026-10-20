//! Dictionary entries, as the card shows them, and what's shared by where
//! they come from: the bundled dictionary (`offline.rs`) and the online
//! ones (`online.rs`).

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

impl Definition {
    /// Leaves out the example sentences, when they're turned off.
    pub fn without_examples(mut self) -> Definition {
        for sense in self.meanings.iter_mut().flat_map(|m| &mut m.senses) {
            sense.example = None;
        }
        self
    }
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
    let trimmed = raw.trim_matches(|c: char| !c.is_alphanumeric());
    let words: Vec<&str> = trimmed.split_whitespace().collect();
    if words.is_empty() {
        return Err(NormalizeError::Empty);
    }
    if words.len() > MAX_LOOKUP_WORDS || trimmed.chars().count() > MAX_LOOKUP_CHARS {
        return Err(NormalizeError::TooLong);
    }
    Ok(words.join(" ").to_lowercase())
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
        WIKTIONARY => Some(format!(
            "https://en.wiktionary.org/wiki/{}",
            percent_encode(&word.replace(' ', "_"))
        )),
        _ => crate::offline::link(source, word),
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
        }
    }

    #[test]
    fn merges_what_the_offline_entry_lacks() {
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
        assert_eq!(source_url("Elsewhere", "x"), None);
    }
}
