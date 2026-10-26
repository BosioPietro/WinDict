//! What the app keeps track of: the card and what it shows, the search box
//! and translations, and what the other threads hand the UI thread.

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
use crate::installed;
use crate::lang::LangCode;
use crate::offline;
use crate::popup::{Popup, Transition};
use crate::render::Target;
use crate::selection::Selection;
use crate::translate::Translation;
use crate::update::Release;

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

/// A result that wasn't found, or couldn't be looked up, and the word it
/// was likely a typo of.
pub(super) struct Corrected {
    pub(super) id: u64,
    pub(super) place: Place,
    pub(super) word: String,
    pub(super) result: Lookup,
    pub(super) correction: Option<String>,
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
    pub(super) from: LangCode,
    pub(super) to: LangCode,
    /// The word is in both languages: which way waits on the user.
    pub(super) asking: bool,
    /// The list a language is being picked from, when it's open.
    pub(super) picking: Option<Picking>,
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
    /// The words the search box suggests from, for the request it was
    /// opened with.
    Words(u64, offline::Words),
    /// How far the download of a language's dictionary has got, in percent.
    Installing(&'static str, u32),
    Installed(&'static str, Result<(), String>),
    Translated(Translated),
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
}

/// The languages what's typed in the box is translated between, shown in
/// boxes under the field.
pub(super) struct Languages {
    pub(super) from: LangCode,
    pub(super) to: LangCode,
    /// The list a language is being picked from, when it's open.
    pub(super) picking: Option<Picking>,
    /// Picked by hand: what's typed no longer changes them.
    pub(super) picked: bool,
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
}

impl Card {
    pub(super) fn hwnd(&self) -> Option<HWND> {
        self.popup.as_ref().map(|p| p.hwnd)
    }

    /// What the card shows is going: there's nothing to translate, or to
    /// say, until something else is shown.
    pub(super) fn clear_content(&mut self) {
        (self.translatable, self.translating, self.speakable) = (None, None, None);
    }
}
