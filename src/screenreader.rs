//! What screen readers are told about the card. It's drawn, not made of
//! controls, so Narrator and the like can't read it on their own: each
//! card's window has a UI Automation provider (what WM_GETOBJECT hands
//! out), named after the word it shows, and what the card comes to show is
//! read out through it as a notification, as is what Tab moves to.

use std::cell::RefCell;
use std::collections::HashMap;
use std::mem::ManuallyDrop;
use std::rc::Rc;

use windows::core::{implement, Error, IUnknown, Result, BSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, VARIANT_FALSE, VARIANT_TRUE, WPARAM};
use windows::Win32::System::Variant::{VARIANT, VT_BOOL, VT_BSTR, VT_I4};
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::WindowsAndMessaging::IsWindowVisible;

use crate::dictionary::capitalize;
use crate::dictionary::Definition;
use crate::i18n::{fill, tr};
use crate::render::{Body, Target, Translated};
use crate::{dicts, translate};

/// What the notifications are filed under, so a newer one takes the
/// place of an older one still being read.
const ACTIVITY: &str = "WinDict.Card";

thread_local! {
    /// Each card window's provider, for WM_GETOBJECT, which can come while
    /// the app is busy elsewhere up the stack.
    static PROVIDERS: RefCell<HashMap<isize, IRawElementProviderSimple>> =
        RefCell::new(HashMap::new());
}

/// A card window as screen readers see it, for as long as it's there.
pub struct Card {
    hwnd: HWND,
    provider: IRawElementProviderSimple,
    /// The word the card shows, which names it.
    name: Rc<RefCell<String>>,
    /// What was last read out, not to start it over for a card redrawn the
    /// same (its picture arriving, say).
    last: RefCell<String>,
    /// What's to be read out once the window is shown: a notification from
    /// a hidden window goes unheard.
    pending: RefCell<Option<String>>,
}

impl Card {
    pub fn new(hwnd: HWND) -> Card {
        let name = Rc::new(RefCell::new(String::new()));
        let provider: IRawElementProviderSimple = Provider {
            hwnd,
            name: name.clone(),
        }
        .into();
        PROVIDERS.with(|p| p.borrow_mut().insert(hwnd.0 as isize, provider.clone()));
        Card {
            hwnd,
            provider,
            name,
            last: RefCell::new(String::new()),
            pending: RefCell::new(None),
        }
    }

    /// The card now shows `name` over what `spoken` says, if there's
    /// anything to say about it yet.
    pub fn show(&self, name: &str, spoken: Option<String>) {
        *self.name.borrow_mut() = name.to_owned();
        if let Some(text) = spoken {
            self.say(&text);
        }
    }

    /// Has `text` read out, if a screen reader is listening, once the
    /// window is up (see [`Card::appeared`]).
    pub fn say(&self, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        if !unsafe { IsWindowVisible(self.hwnd) }.as_bool() {
            *self.pending.borrow_mut() = Some(text.to_owned());
            return;
        }
        if *self.last.borrow() == text || !unsafe { UiaClientsAreListening() }.as_bool() {
            return;
        }
        *self.last.borrow_mut() = text.to_owned();
        unsafe {
            let _ = UiaRaiseNotificationEvent(
                &self.provider,
                NotificationKind_ActionCompleted,
                NotificationProcessing_ImportantMostRecent,
                &BSTR::from(text),
                &BSTR::from(ACTIVITY),
            );
        }
    }

    /// The window has been shown, and UI Automation has had time to take it
    /// in: what was waiting for it is read out.
    pub fn appeared(&self) {
        // Taken first: said to a window that's hidden after all, it goes
        // back to waiting.
        let waiting = self.pending.borrow_mut().take();
        if let Some(text) = waiting {
            self.say(&text);
        }
    }

    /// The window has been hidden: the next card is read out whatever it is.
    pub fn hidden(&self) {
        self.last.borrow_mut().clear();
        *self.pending.borrow_mut() = None;
    }
}

impl Drop for Card {
    fn drop(&mut self) {
        PROVIDERS.with(|p| p.borrow_mut().remove(&(self.hwnd.0 as isize)));
        unsafe {
            // As a window does on WM_DESTROY: clients let go of it.
            UiaReturnRawElementProvider(self.hwnd, WPARAM(0), LPARAM(0), None);
            let _ = UiaDisconnectProvider(&self.provider);
        }
    }
}

/// Answers WM_GETOBJECT for a card window, when it's UI Automation asking
/// (`lparam` is `UiaRootObjectId`) and the window has a provider.
pub fn get_object(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    if lparam.0 as i32 != UiaRootObjectId {
        return None;
    }
    let provider = PROVIDERS.with(|p| p.borrow().get(&(hwnd.0 as isize)).cloned())?;
    Some(unsafe { UiaReturnRawElementProvider(hwnd, wparam, lparam, &provider) })
}

/// What a card showing `body` reads as, for a screen reader to say: nothing
/// while it's still on its way.
pub fn spoken(body: &Body) -> Option<String> {
    let mut out = Vec::new();
    match body {
        Body::Loading { .. } => return None,
        Body::Definition(def) => {
            // Not the phonetic: screen readers spell IPA out letter by
            // letter, and it comes after the rest, which would start over.
            out.push(def.word.clone());
            let several = !def.others.is_empty();
            for section in def.sections() {
                if several {
                    out.push(dicts::name(&section.language));
                }
                out.extend(section.note.clone());
                out.extend(meanings(section));
            }
        }
        Body::Article(card) => {
            let article = card.article;
            out.push(article.title.clone());
            out.extend(article.description.clone());
            match article.ambiguous {
                true => out.push(tr().wikipedia_ambiguous.into()),
                false => out.push(article.extract.clone()),
            }
        }
        Body::Message {
            title,
            detail,
            suggestion,
        } => {
            out.push(title.to_string());
            out.push(detail.to_string());
            if let Some(word) = suggestion {
                out.push(fill(tr().did_you_mean, &[("word", word)]));
            }
        }
        Body::Translation(card) => match &card.state {
            Translated::Waiting | Translated::Loading => return None,
            Translated::Asking => {
                let names: [(&str, &dyn std::fmt::Display); 3] = [
                    ("word", &card.word),
                    ("from", &translate::name(card.from)),
                    ("to", &translate::name(card.to)),
                ];
                out.push(fill(tr().translate_ambiguous, &names));
                let into = translate::name(card.to);
                out.push(fill(tr().translate_into, &[("language", &into)]));
            }
            Translated::Done(translation) => {
                out.push(translation.text.clone());
                out.extend(translation.context.clone());
            }
            Translated::Failed(error) => out.push(error.to_string()),
        },
        Body::Gloss(gloss) => {
            out.push(translate::name(&gloss.language));
            out.push(gloss.text.clone());
        }
    }
    Some(sentences(out))
}

/// Each part of speech of `def`, with its senses, then the similar and
/// opposite words.
fn meanings(def: &Definition) -> Vec<String> {
    let mut out = Vec::new();
    for meaning in &def.meanings {
        out.push(capitalize(tr().part_of_speech(&meaning.part_of_speech)));
        out.extend(meaning.senses.iter().map(|s| s.text.clone()));
        for (label, words) in [
            (tr().similar, &meaning.synonyms),
            (tr().opposite, &meaning.antonyms),
        ] {
            if !words.is_empty() {
                out.push(format!("{label}: {}", words.join(", ")));
            }
        }
    }
    out
}

/// What the link or button Tab moved to reads as, if it has a name of its
/// own: a word, a web address, or what the button does.
pub fn spoken_target(target: &Target, pinned: bool) -> Option<String> {
    Some(match target {
        Target::Word(word) => word.clone(),
        Target::Url(url) => url.clone(),
        Target::Translate => tr().translate_tip.into(),
        Target::Wikipedia => tr().wikipedia_tip.into(),
        Target::Speak => tr().speak_tip.into(),
        Target::Back => tr().back_tip.into(),
        Target::Pin if pinned => tr().unpin_tip.into(),
        Target::Pin => tr().pin_tip.into(),
        Target::Scan => tr().scan_tip.into(),
        Target::Swap => tr().swap_tip.into(),
        Target::Language(_) | Target::Interface | Target::Pick(_) | Target::Confirm => return None,
    })
}

/// `parts` as one text, each ended like a sentence so that they're read
/// with a pause between them.
fn sentences(parts: Vec<String>) -> String {
    let mut out = String::new();
    for part in parts {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(part);
        if !part.ends_with(['.', '!', '?', ':', '\u{2026}']) {
            out.push('.');
        }
    }
    out
}

/// The card, to UI Automation: a pane named after the word, which clients
/// can't move into, read through the notifications.
#[implement(IRawElementProviderSimple)]
struct Provider {
    hwnd: HWND,
    name: Rc<RefCell<String>>,
}

impl IRawElementProviderSimple_Impl for Provider_Impl {
    /// On the thread the card is on, as COM would have it: the name is
    /// the card's to change there, and no other thread's to read.
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(ProviderOptions_ServerSideProvider | ProviderOptions_UseComThreading)
    }

    /// No patterns: nothing on the card is a control to a client.
    fn GetPatternProvider(&self, _pattern: UIA_PATTERN_ID) -> Result<IUnknown> {
        Err(Error::empty())
    }

    fn GetPropertyValue(&self, property: UIA_PROPERTY_ID) -> Result<VARIANT> {
        // Guards, as the ids are constants named like variables.
        Ok(match property {
            p if p == UIA_NamePropertyId => text(&self.name.borrow()),
            p if p == UIA_ControlTypePropertyId => number(UIA_PaneControlTypeId.0),
            p if p == UIA_IsControlElementPropertyId || p == UIA_IsContentElementPropertyId => {
                truth(true)
            }
            p if p == UIA_IsKeyboardFocusablePropertyId => truth(false),
            p if p == UIA_NativeWindowHandlePropertyId => number(self.hwnd.0 as usize as i32),
            p if p == UIA_ProviderDescriptionPropertyId => text("WinDict"),
            _ => VARIANT::default(),
        })
    }

    fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
        unsafe { UiaHostProviderFromHwnd(self.hwnd) }
    }
}

fn text(value: &str) -> VARIANT {
    let mut variant = VARIANT::default();
    unsafe {
        let inner = &mut *variant.Anonymous.Anonymous;
        inner.vt = VT_BSTR;
        inner.Anonymous.bstrVal = ManuallyDrop::new(BSTR::from(value));
    }
    variant
}

fn number(value: i32) -> VARIANT {
    let mut variant = VARIANT::default();
    unsafe {
        let inner = &mut *variant.Anonymous.Anonymous;
        inner.vt = VT_I4;
        inner.Anonymous.lVal = value;
    }
    variant
}

fn truth(value: bool) -> VARIANT {
    let mut variant = VARIANT::default();
    unsafe {
        let inner = &mut *variant.Anonymous.Anonymous;
        inner.vt = VT_BOOL;
        inner.Anonymous.boolVal = if value { VARIANT_TRUE } else { VARIANT_FALSE };
    }
    variant
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dictionary::{Meaning, Sense};
    use crate::gloss::Gloss;
    use crate::render::TranslationCard;
    use crate::translate::Translation;

    fn definition() -> Definition {
        Definition {
            word: "dog".into(),
            language: "en".into(),
            phonetic: Some("/dɒɡ/".into()),
            meanings: vec![Meaning {
                part_of_speech: "noun".into(),
                senses: vec![Sense {
                    text: "A domesticated canid.".into(),
                    example: Some("the dog barked".into()),
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
    fn reads_the_card_out() {
        let def = definition();
        assert_eq!(
            spoken(&Body::Definition(&def)).unwrap(),
            "dog. Noun. A domesticated canid. Similar: hound."
        );
        // With more languages, each is named.
        let mut both = definition();
        both.others.push(Definition {
            language: "it".into(),
            ..definition()
        });
        assert!(spoken(&Body::Definition(&both))
            .unwrap()
            .contains("English. Noun."));
        assert_eq!(spoken(&Body::Loading { status: true }), None);
        let message = Body::Message {
            title: "Not found",
            detail: "Check the spelling.",
            suggestion: Some("embody"),
        };
        assert_eq!(
            spoken(&message).unwrap(),
            "Not found. Check the spelling. Did you mean embody?"
        );
        let gloss = Gloss {
            text: "The dog ran.".into(),
            language: "en".into(),
            words: Vec::new(),
        };
        assert_eq!(
            spoken(&Body::Gloss(&gloss)).unwrap(),
            "English. The dog ran."
        );
    }

    #[test]
    fn reads_translations_once_they_come() {
        let translation = Translation {
            text: "cane".into(),
            context: None,
            groups: Vec::new(),
            source: translate::GOOGLE,
        };
        let card = |state| TranslationCard {
            word: "dog",
            from: "en",
            to: "it",
            state,
            picker: None,
        };
        assert_eq!(spoken(&Body::Translation(card(Translated::Loading))), None);
        assert_eq!(spoken(&Body::Translation(card(Translated::Waiting))), None);
        assert_eq!(
            spoken(&Body::Translation(card(Translated::Done(&translation)))).unwrap(),
            "cane."
        );
        assert_eq!(
            spoken(&Body::Translation(card(Translated::Failed("No internet")))).unwrap(),
            "No internet."
        );
        let asking = spoken(&Body::Translation(card(Translated::Asking))).unwrap();
        assert!(
            asking.contains("English") && asking.contains("Italiano"),
            "{asking}"
        );
    }

    #[test]
    fn names_what_tab_moves_to() {
        assert_eq!(
            spoken_target(&Target::Word("run".into()), false).unwrap(),
            "run"
        );
        assert_eq!(spoken_target(&Target::Pin, false).unwrap(), tr().pin_tip);
        assert_eq!(spoken_target(&Target::Pin, true).unwrap(), tr().unpin_tip);
        assert_eq!(spoken_target(&Target::Confirm, false), None);
    }

    #[test]
    fn ends_each_part_like_a_sentence() {
        assert_eq!(
            sentences(vec![
                "One".into(),
                " ".into(),
                "Two!".into(),
                "Three:".into()
            ]),
            "One. Two! Three:"
        );
        assert_eq!(sentences(Vec::new()), "");
    }

    #[test]
    fn keeps_what_a_hidden_card_has_to_say() {
        // No window is as good as a hidden one: what there is to say waits
        // for it, and waits on when it's found still hidden.
        let card = Card::new(HWND::default());
        card.show("dog", Some("dog. A domesticated canid.".into()));
        assert!(card.pending.borrow().is_some());
        card.appeared();
        assert!(card.pending.borrow().is_some());
        card.hidden();
        assert!(card.pending.borrow().is_none());
        card.appeared();
    }
}
