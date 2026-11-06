//! Answers as JSON, for scripts and other programs to read (`--json`): one
//! object on a line of stdout, whose `type` says which answer it is. What
//! went wrong still goes to stderr, in words, with nothing on stdout.

use serde::Serialize;

use super::*;

/// What `wd` answers, as `--json` prints it.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub(super) enum Answer<'a> {
    Definition(&'a Definition),
    Article(&'a Article),
    Translation(Translated<'a>),
    Gloss(Glossed<'a>),
}

/// A translation, with what was translated and which way.
#[derive(Serialize)]
pub(super) struct Translated<'a> {
    pub(super) input: &'a str,
    pub(super) from: &'a LangCode,
    pub(super) to: &'a LangCode,
    /// Whether which way was a guess, as the message on stderr says.
    pub(super) guessed: bool,
    #[serde(flatten)]
    pub(super) translation: &'a Translation,
}

/// A passage with its uncommon words marked, each with its definition, if
/// it has one.
#[derive(Serialize)]
pub(super) struct Glossed<'a> {
    language: &'a LangCode,
    text: &'a str,
    words: Vec<Marked<'a>>,
}

#[derive(Serialize)]
struct Marked<'a> {
    word: &'a str,
    /// Where it is in `text`, as byte offsets.
    start: usize,
    end: usize,
    definition: Option<&'a Definition>,
}

impl<'a> Glossed<'a> {
    /// `glossary` has each word once, as `gloss_text` lists them, with its
    /// definition: every place it's marked in the passage gets it.
    pub(super) fn new(
        gloss: &'a gloss::Gloss,
        glossary: &'a [(&str, Option<Definition>)],
    ) -> Glossed<'a> {
        let words = gloss
            .words
            .iter()
            .map(|range| {
                let word = &gloss.text[range.clone()];
                let definition = glossary
                    .iter()
                    .find(|(w, _)| w.to_lowercase() == word.to_lowercase())
                    .and_then(|(_, def)| def.as_ref());
                Marked {
                    word,
                    start: range.start,
                    end: range.end,
                    definition,
                }
            })
            .collect();
        Glossed {
            language: &gloss.language,
            text: &gloss.text,
            words,
        }
    }
}

/// Prints `answer` on a line of its own. A pipe closed early isn't anything
/// to tell anyone about, as for the other answers.
pub(super) fn emit(out: &mut Printer, answer: &Answer) -> ExitCode {
    if let Ok(json) = serde_json::to_string(answer) {
        let _ = writeln!(out.out, "{json}");
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dictionary::{Meaning, Sense};

    fn definition(word: &str) -> Definition {
        Definition {
            word: word.into(),
            language: "en".into(),
            phonetic: Some("/dɒɡ/".into()),
            meanings: vec![Meaning {
                part_of_speech: "noun".into(),
                senses: vec![Sense {
                    text: "A domesticated canid.".into(),
                    example: None,
                }],
                synonyms: vec!["hound".into()],
                antonyms: Vec::new(),
            }],
            note: None,
            sources: vec!["Open English WordNet"],
            others: Vec::new(),
        }
    }

    #[test]
    fn says_which_answer_it_is() {
        let def = definition("dog");
        let json: serde_json::Value = serde_json::to_value(Answer::Definition(&def)).unwrap();
        assert_eq!(json["type"], "definition");
        assert_eq!(json["word"], "dog");
        assert_eq!(json["language"], "en");
        assert_eq!(
            json["meanings"][0]["senses"][0]["text"],
            "A domesticated canid."
        );
        assert_eq!(json["meanings"][0]["synonyms"][0], "hound");
        assert_eq!(json["sources"][0], "Open English WordNet");
        let translation = Translation {
            text: "cane".into(),
            context: None,
            groups: Vec::new(),
            source: translate::GOOGLE,
        };
        let (from, to) = (LangCode::new("en"), LangCode::new("it"));
        let json = serde_json::to_value(Answer::Translation(Translated {
            input: "dog",
            from: &from,
            to: &to,
            guessed: true,
            translation: &translation,
        }))
        .unwrap();
        assert_eq!(json["type"], "translation");
        assert_eq!(json["input"], "dog");
        assert_eq!((&json["from"], &json["to"]), (&"en".into(), &"it".into()));
        assert_eq!(json["guessed"], true);
        // The translation's own fields come beside them.
        assert_eq!(json["text"], "cane");
        assert_eq!(json["source"], "Google Translate");
    }

    #[test]
    fn marks_each_place_a_word_is_glossed() {
        let gloss = gloss::Gloss {
            text: "The dog saw the dog.".into(),
            language: "en".into(),
            words: vec![4..7, 16..19],
        };
        let glossary = [("dog", Some(definition("dog"))), ("saw", None)];
        let json = serde_json::to_value(Answer::Gloss(Glossed::new(&gloss, &glossary))).unwrap();
        assert_eq!(json["type"], "gloss");
        assert_eq!(json["language"], "en");
        let words = json["words"].as_array().unwrap();
        assert_eq!(words.len(), 2);
        assert_eq!(words[1]["word"], "dog");
        assert_eq!(
            (&words[1]["start"], &words[1]["end"]),
            (&16.into(), &19.into())
        );
        assert_eq!(words[1]["definition"]["word"], "dog");
        assert_eq!(words[0]["definition"]["word"], "dog");
    }
}
