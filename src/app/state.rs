//! What the app keeps track of: the card and what it shows, the search box
//! and translations, and what the other threads hand the UI thread.

use std::rc::Rc;
use std::sync::mpsc::Sender;

use windows::core::HSTRING;
use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use super::edit::LineEdit;
use super::picking::Picking;
use super::WM_APP_EVENT;
use crate::config::Shortcut;
use crate::dictionary::{Definition, Lookup};
use crate::dicts;
use crate::gloss::Gloss;
use crate::installed;
use crate::lang::LangCode;
use crate::ocr;
use crate::offline;
use crate::popup::{Popup, Transition};
use crate::render::{self, Picture, Side, Target};
use crate::selection::Selection;
use crate::translate::Translation;
use crate::update::Release;
use crate::wikipedia::Article;

pub(super) struct Captured {
    pub(super) id: u64,
    pub(super) selection: Option<Selection>,
}

pub(super) struct Looked {
    pub(super) id: u64,
    pub(super) query: Query,
    pub(super) result: Lookup,
}

/// A word to look up, in every language it's in, or only in the one it
/// was picked in from the search box's suggestions.
#[derive(Clone, PartialEq)]
pub(super) struct Query {
    pub(super) word: String,
    pub(super) language: Option<LangCode>,
}

impl Query {
    /// `def`, or only its part in the language asked for.
    pub(super) fn within(&self, def: Definition) -> Option<Definition> {
        match &self.language {
            Some(language) => def.only(language),
            None => Some(def),
        }
    }
}

/// The titles a word is looked for under on Wikipedia, each in the
/// language whose Wikipedia it's looked for in, by preference.
pub(super) type Titles = Vec<(LangCode, String)>;

/// What Wikipedia had for the titles, for the request they were asked
/// with, or why it couldn't be asked.
pub(super) struct Found {
    pub(super) id: u64,
    pub(super) titles: Titles,
    pub(super) result: Result<Option<Article>, String>,
}

/// What Wikipedia has for the word the card shows.
#[derive(Clone, Default)]
pub(super) enum Wiki {
    /// It isn't asked: it's turned off, or there's nothing to ask.
    #[default]
    Off,
    Asking,
    Answered(Option<Rc<Article>>),
}

impl Wiki {
    /// The article, unless the title is several things'.
    pub(super) fn article(&self) -> Option<&Rc<Article>> {
        match self {
            Wiki::Answered(Some(article)) if !article.ambiguous => Some(article),
            _ => None,
        }
    }
}

/// The card turned into a Wikipedia article.
pub(super) struct Reading {
    pub(super) article: Rc<Article>,
    pub(super) picture: Picture,
    /// Opened from the definition, which Back goes back to; otherwise
    /// it's the answer, the dictionaries having none.
    pub(super) from_definition: bool,
}

/// A result that wasn't found, or couldn't be looked up, and the word it
/// was likely a typo of.
pub(super) struct Corrected {
    pub(super) id: u64,
    pub(super) place: Place,
    pub(super) word: String,
    pub(super) result: Lookup,
    pub(super) correction: Option<String>,
}

/// A passage glossed, for the request it was asked with, to show at
/// `anchor`, unless the card is open `in_place` already: nothing when
/// which language it's in couldn't be told.
pub(super) struct Glossed {
    pub(super) id: u64,
    pub(super) anchor: Option<RECT>,
    pub(super) in_place: bool,
    pub(super) gloss: Option<Gloss>,
    /// For a phrase the dictionary hadn't: where to say so instead, if
    /// there's nothing to gloss in it either.
    pub(super) not_found: Option<(Place, String)>,
}

/// The passage the card glosses, which the words looked up from it go
/// back to.
pub(super) struct Passage {
    pub(super) gloss: Rc<Gloss>,
    /// What had the focus when a word was looked up from it, where it goes
    /// back to.
    pub(super) focus: Option<Target>,
}

/// A word, from one language into another.
pub(super) type TranslationKey = (String, LangCode, LangCode);

/// A translation, or why there's none, for the request it was asked with.
pub(super) struct Translated {
    pub(super) id: u64,
    pub(super) key: TranslationKey,
    pub(super) result: Result<Translation, String>,
}

/// What the definition on the card gives a translation of its word.
#[derive(Clone)]
pub(super) struct Translatable {
    pub(super) word: String,
    /// The languages it's in, each with the first thing the word means in
    /// it: context for a translation, should it need any.
    pub(super) senses: Vec<(LangCode, Option<String>)>,
}

impl Translatable {
    pub(super) fn of(def: &Definition) -> Translatable {
        let first = |section: &Definition| {
            let meaning = section.meanings.first()?;
            Some(meaning.senses.first()?.text.clone())
        };
        Translatable {
            word: def.word.clone(),
            senses: def
                .sections()
                .map(|s| (s.language.clone(), first(s)))
                .collect(),
        }
    }
}

/// The card turned into a translation of its word.
pub(super) struct Translating {
    pub(super) about: Translatable,
    pub(super) pair: LanguagePair,
    /// The word is in both languages: which way waits on the user.
    pub(super) asking: bool,
    /// `None` while it's on its way.
    pub(super) result: Option<Result<Translation, String>>,
    /// The card waits a moment for it before showing it on its way, so a
    /// quick answer comes in one change rather than two.
    pub(super) waiting: bool,
    /// Where the focus goes once the card stops waiting.
    pub(super) focus: Option<Target>,
    /// Whether the card shows the translation yet, rather than the
    /// definition it was asked from.
    pub(super) shown: bool,
    /// The box what's translated was typed in, which Back goes back to.
    pub(super) typed: Option<Search>,
}

impl Translating {
    /// How the card changes as the translation comes: from the definition
    /// it's another card; on the translation, only what's under the
    /// languages changes.
    pub(super) fn transition(&self) -> Transition {
        if self.shown {
            Transition::Refine
        } else {
            Transition::Replace
        }
    }
}

/// A dictionary being downloaded, for the language it's for.
pub(super) struct Installing {
    pub(super) download: &'static dicts::Download,
    /// Fetched again at start, rather than ticked in the settings: only
    /// failing is told, and only when it leaves the language out.
    pub(super) quiet: bool,
    /// Whether it's still wanted: one unticked as it downloads finishes,
    /// unused.
    pub(super) wanted: bool,
}

/// What the capture, lookup and download threads hand the UI thread.
pub(super) enum Event {
    /// The capture is waiting for a copy of the selection, for the request.
    Copying(u64),
    Captured(Captured),
    Looked(Looked),
    Corrected(Corrected),
    Glossed(Glossed),
    /// The words the search box suggests from, for the request it was
    /// opened with.
    Words(u64, offline::Words),
    /// The text read off the screen for the request, into the search box.
    Scanned(u64, Result<ocr::Scanned, ocr::ScanError>),
    /// How far the download of a language's dictionary has got, in percent.
    Installing(&'static str, u32),
    Installed(&'static str, Result<(), String>),
    Translated(Translated),
    /// What Wikipedia had for the word.
    Found(Found),
    /// An article's picture, for the request it was fetched for, by its
    /// address: nothing when it couldn't be fetched.
    Picture(u64, String, Option<Vec<u8>>),
    /// The latest release, if it's newer, or why it couldn't be looked up.
    UpdateChecked(Result<Option<Release>, String>),
    /// The update was started, or why it couldn't be.
    Updated(Result<(), String>),
    /// Why UI access couldn't be turned on or off: had it been, WinDict
    /// would be restarting.
    UiAccess(installed::UiAccessError),
}

/// Sends [`Event`]s to the UI thread. They go through a channel, and an
/// empty WM_APP_EVENT wakes the thread to read it: a message can't carry
/// them itself, as any app on the desktop can post one with any payload.
#[derive(Clone)]
pub(super) struct Events {
    pub(super) tx: Sender<Event>,
    /// The main window, as a number: HWND can't cross threads.
    pub(super) hwnd: isize,
}

impl Events {
    pub(super) fn send(&self, event: Event) {
        if self.tx.send(event).is_ok() {
            unsafe {
                let hwnd = HWND(self.hwnd as _);
                let _ = PostMessageW(Some(hwnd), WM_APP_EVENT, WPARAM(0), LPARAM(0));
            }
        }
    }
}

/// What's typed in the search box.
#[derive(Default)]
pub(super) struct Search {
    /// What the box is for: looking up what's typed, or translating it.
    pub(super) purpose: Shortcut,
    pub(super) edit: LineEdit,
    pub(super) title: &'static str,
    /// Whether the heading says why the last Enter did nothing.
    pub(super) error: bool,
    /// The words offered under the field.
    pub(super) suggestions: Vec<offline::Suggestion>,
    /// What they're picked from, read on a thread of its own as the box
    /// opens.
    pub(super) words: Option<offline::Words>,
    /// Opened to translate: the languages it's translated between.
    pub(super) languages: Option<Languages>,
    /// The language of the text read off the screen into it, while there's
    /// text in it: it says better than the words what's to be translated.
    pub(super) scanned: Option<LangCode>,
}

/// The languages what's typed in the box is translated between, shown in
/// boxes under the field.
pub(super) struct Languages {
    pub(super) pair: LanguagePair,
    /// Picked by hand: what's typed no longer changes them.
    pub(super) picked: bool,
}

/// The languages a translation goes between, and the list one of them is
/// being picked from, when it's open: on the translation card, and under
/// the box what's to be translated is typed in.
pub(super) struct LanguagePair {
    pub(super) from: LangCode,
    pub(super) to: LangCode,
    pub(super) picking: Option<Picking>,
}

impl LanguagePair {
    pub(super) fn new(from: LangCode, to: LangCode) -> LanguagePair {
        LanguagePair {
            from,
            to,
            picking: None,
        }
    }

    /// The list, as it's drawn, while it's open, and the side it's open on.
    pub(super) fn picker(&self) -> Option<(Side, render::Picker<'_>)> {
        self.picking.as_ref().map(|p| {
            let picker = render::Picker {
                query: p.query.text(),
                caret: p.query.caret_utf16(),
                matches: &p.matches,
                highlight: p.highlight,
                rows: render::PICKER_ROWS,
            };
            (p.on, picker)
        })
    }

    /// The language on `side`.
    pub(super) fn on(&self, side: Side) -> &LangCode {
        match side {
            Side::From => &self.from,
            Side::To => &self.to,
        }
    }

    /// Opens the list under `side`, or shuts it if it's open there. Returns
    /// whether it opened.
    pub(super) fn toggle(&mut self, side: Side) -> bool {
        let open = self.picking.take().map(|p| p.on);
        if open == Some(side) {
            return false;
        }
        self.picking = Some(Picking::new(side, self.on(side)));
        true
    }

    /// Swaps the languages round, shutting the list.
    pub(super) fn swap(&mut self) {
        self.picking = None;
        std::mem::swap(&mut self.from, &mut self.to);
    }

    /// Picks `language` on the side the list is open under, and shuts it:
    /// the other side's language swaps them round. Returns the side, and
    /// whether the languages changed, or nothing when no list was open.
    pub(super) fn pick(&mut self, language: LangCode) -> Option<(Side, bool)> {
        let side = self.picking.take()?.on;
        let (picked, other) = match side {
            Side::From => (&mut self.from, &mut self.to),
            Side::To => (&mut self.to, &mut self.from),
        };
        if language == *picked {
            return Some((side, false));
        }
        if language == *other {
            std::mem::swap(picked, other);
        } else {
            *picked = language;
        }
        Some((side, true))
    }
}

/// Where a lookup's answer goes.
#[derive(Clone, Copy)]
pub(super) enum Place {
    /// A new card next to the selection.
    New(RECT),
    /// The open card, in place of what it shows, changing as the
    /// transition says.
    Same(Transition),
}

/// A card and what it shows: the one that comes and goes with lookups, or
/// one pinned open.
#[derive(Default)]
pub(super) struct Card {
    /// Created when first needed, and dropped a while after it's hidden;
    /// a pinned card's is always there.
    pub(super) popup: Option<Popup>,
    /// Whether it's pinned open: it stays on top, has the keyboard when
    /// clicked, and closes only when told to.
    pub(super) pinned: bool,
    /// The id of the request the card waits on.
    pub(super) request: u64,
    /// The selection is being read for the request.
    pub(super) capturing: bool,
    /// The card is open on its way, while the capture waits for a copy.
    pub(super) copying: bool,
    /// The selection being read is to be translated, not looked up.
    pub(super) to_translate: bool,
    /// The offline entry for the current request, if any.
    pub(super) local: Option<Definition>,
    /// The word the card shows, as it was looked up.
    pub(super) current: Option<Query>,
    /// Words the card showed before synonyms were followed, most recent last.
    pub(super) history: Vec<Query>,
    /// The passage the first of them was looked up from, if it was.
    pub(super) passage: Option<Passage>,
    /// The phrase selected, of several words, glossed instead if the
    /// dictionary hasn't it.
    pub(super) gloss_missing: Option<String>,
    /// What was selected or typed, cleaned up but in the case it was
    /// written in, which names keep on Wikipedia.
    pub(super) written: Option<String>,
    /// What Wikipedia has for the word.
    pub(super) wiki: Wiki,
    /// No definition was found: where the answer goes, the word, and the
    /// answer, held while Wikipedia is asked, as its answer goes first.
    pub(super) held: Option<(Place, String, Lookup)>,
    /// The definition the card shows, or the one the article it shows was
    /// opened from.
    pub(super) definition: Option<Definition>,
    /// The Wikipedia article the card shows, if it does.
    pub(super) reading: Option<Reading>,
    /// The definition the card shows, if it does.
    pub(super) translatable: Option<Translatable>,
    /// The translation the card turned into, if it did.
    pub(super) translating: Option<Translating>,
    /// The word the card shows, and the voice that says it, if there's one
    /// for its language.
    pub(super) speakable: Option<(String, HSTRING)>,
    /// The search box, while the card shows one.
    pub(super) search: Option<Search>,
    /// The window the search box took the keyboard from.
    pub(super) return_focus: Option<HWND>,
    /// The selection the card translates, which Ctrl+Enter replaces with
    /// the translation.
    pub(super) selected: Option<Selected>,
}

/// Text selected in another app, and the window it's in.
pub(super) struct Selected {
    pub(super) text: String,
    pub(super) window: HWND,
}

impl Card {
    pub(super) fn hwnd(&self) -> Option<HWND> {
        self.popup.as_ref().map(|p| p.hwnd)
    }

    /// What the card shows is going: there's nothing to translate, or to
    /// say, until something else is shown.
    pub(super) fn clear_content(&mut self) {
        (self.translatable, self.translating, self.speakable) = (None, None, None);
        self.passage = None;
        (self.definition, self.reading, self.held) = (None, None, None);
    }
}
