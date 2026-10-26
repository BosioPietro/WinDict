//! The typed-word box: opening it, typing into it, and submitting what's in
//! it, to look it up or, opened by the translation shortcut, to translate it.

use super::*;

impl App {
    /// Nothing was selected: opens the card as a box to type in, for text
    /// that can't be selected (in an image, a protected PDF, a game), to
    /// do with it what the `purpose` shortcut does. It opens at `anchor`,
    /// the text caret, when there is one.
    pub(super) fn open_search(&mut self, purpose: Shortcut, anchor: Option<RECT>) {
        // Translations come from the internet, like the online answers.
        if purpose == Shortcut::Translate && !self.config.online_lookup {
            let (title, detail) = (tr().translate_offline, tr().translate_offline_detail);
            return self.show_message(anchor, title, detail, true);
        }
        let title = match purpose {
            Shortcut::Lookup => tr().search_title,
            Shortcut::Translate => tr().translate_title,
        };
        self.card.current = None;
        self.card.history.clear();
        self.card.clear_content();
        self.sync_back_keys();
        let previous = unsafe { GetForegroundWindow() };
        self.dismiss_menu();
        let (theme, zoom) = (self.theme(), self.config.zoom_factor());
        let Some(popup) = self.ensure_popup() else {
            return;
        };
        // With no caret to open at (an image, a game), the middle of the
        // screen the app is on.
        let anchor = anchor_or_centre(anchor, previous, zoom);
        // A list of languages the card had open is gone.
        popup.hold_region(false);
        if let Err(e) = popup.show_search(anchor, theme, title, purpose) {
            self.tray.notify(tr().draw_window_failed, &e.message());
            return;
        }
        let hwnd = popup.hwnd;
        self.on_shown(false);
        if !selection::take_foreground(hwnd) {
            let detail = fill(
                tr().nothing_selected_detail,
                &[("hotkey", &self.config.hotkey(purpose))],
            );
            return self.show_message(None, tr().nothing_selected, &detail, true);
        }
        // Opened again from the card itself, it keeps the original app.
        if previous != hwnd {
            self.card.return_focus = Some(previous);
        }
        // A passage can be translated, where only a word or two is looked
        // up; and it's translated the way the languages under it say.
        let (edit, languages) = match purpose {
            Shortcut::Lookup => (LineEdit::default(), None),
            Shortcut::Translate => {
                let way = typed_direction("", &self.config);
                let languages = Languages {
                    from: way.from,
                    to: way.to,
                    picking: None,
                    picked: false,
                };
                let edit = LineEdit::new("", translate::MAX_PASSAGE_CHARS);
                (edit, Some(languages))
            }
        };
        self.card.search = Some(Search {
            purpose,
            edit,
            title,
            languages,
            ..Search::default()
        });
        // Reading the words takes a few tens of ms per dictionary: not on
        // this thread, which would stop drawing and taking keys meanwhile.
        if purpose == Shortcut::Lookup {
            self.new_request();
            let (id, events) = (self.card.request, self.events.clone());
            std::thread::spawn(move || events.send(Event::Words(id, offline::Words::load())));
        }
        self.sync_link_keys();
        // The languages it'll be translated between show from the start.
        if purpose == Shortcut::Translate {
            self.show_typed_languages();
        }
    }

    /// A character typed into the card (WM_CHAR).
    pub(super) fn on_char(&mut self, unit: u16) {
        if self.picking().is_some() {
            return self.type_language(unit);
        }
        let Some(hwnd) = self.visible_popup().map(|p| p.hwnd) else {
            return;
        };
        let Some(search) = self.card.search.as_mut() else {
            return;
        };
        match search.edit.type_unit(unit) {
            Input::Edited => {}
            Input::Enter => {
                let picked = self.card.popup.as_ref().and_then(|p| p.suggestion());
                return self.submit_search(picked);
            }
            Input::Paste => {
                let pasted = selection::clipboard_text(hwnd).unwrap_or_default();
                search.edit.paste(&pasted);
            }
            Input::Nothing => return,
        }
        self.refresh_search();
    }

    /// A key pressed in the card (WM_KEYDOWN): Up, Down, Tab and Shift+Tab
    /// pick a suggestion; Left, Right, Home and End move the caret, by
    /// words with Ctrl; Delete removes what's after it.
    pub(super) fn on_key(&mut self, key: VIRTUAL_KEY) {
        if self.card.pinned && self.pinned_key(key) {
            return;
        }
        if self.picking().is_some() {
            return self.language_key(key);
        }
        let (Some(search), Some(popup)) = (self.card.search.as_mut(), self.card.popup.as_mut())
        else {
            return;
        };
        let ctrl = hotkey::is_held(VK_CONTROL);
        let down = match key {
            VK_DOWN => Some(true),
            VK_UP => Some(false),
            VK_TAB => Some(!hotkey::is_held(VK_SHIFT)),
            _ => None,
        };
        if let Some(down) = down {
            let _ = popup.step_suggestion(down);
            return;
        }
        let edit = &mut search.edit;
        let motion = match key {
            VK_LEFT if ctrl => Motion::WordBack,
            VK_LEFT => Motion::CharBack,
            VK_RIGHT if ctrl => Motion::WordForward,
            VK_RIGHT => Motion::CharForward,
            VK_HOME => Motion::Start,
            VK_END => Motion::End,
            VK_DELETE => {
                let to = if ctrl {
                    Motion::WordForward
                } else {
                    Motion::CharForward
                };
                if edit.delete(to) {
                    self.refresh_search();
                }
                return;
            }
            _ => return,
        };
        if edit.move_caret(motion) {
            let _ = popup.set_query(edit.text(), edit.caret_utf16());
        }
    }

    /// The words to suggest from have been read: what's typed already gets
    /// its suggestions.
    pub(super) fn on_words(&mut self, id: u64, words: offline::Words) {
        let Some(search) = self.card.search.as_mut() else {
            return;
        };
        if id != self.card.request || search.words.is_some() {
            return;
        }
        search.words = Some(words);
        self.refresh_search();
    }

    /// Shows what's typed in the search box, and the words it could be the
    /// start of.
    pub(super) fn refresh_search(&mut self) {
        let (Some(search), Some(popup)) = (self.card.search.as_mut(), self.card.popup.as_mut())
        else {
            return;
        };
        // Typing again answers the complaint: the heading goes back.
        if std::mem::take(&mut search.error) {
            let _ = popup.set_search_title(search.title);
        }
        let _ = popup.set_query(search.edit.text(), search.edit.caret_utf16());
        // What's to be translated shows, instead of words, the languages
        // it'll go between, following what it's typed in until one's picked.
        // They're drawn again only when that changes them.
        if let Some(l) = search.languages.as_mut() {
            if l.picked {
                return;
            }
            let way = typed_direction(search.edit.text(), &self.config);
            if (&way.from, &way.to) == (&l.from, &l.to) {
                return;
            }
            (l.from, l.to) = (way.from, way.to);
            return self.show_typed_languages();
        }
        let typed = search.edit.text().trim_start().to_lowercase();
        let typed_chars = typed.chars().count();
        // Until the words are read, there's nothing to say either way.
        let suggest = typed_chars >= SUGGEST_AFTER && search.words.is_some();
        let mut multilingual = false;
        search.suggestions = match search.words.as_ref().filter(|_| suggest) {
            Some(words) => {
                multilingual = words.multilingual();
                words.suggest(&typed, MAX_SUGGESTIONS)
            }
            None => Vec::new(),
        };
        // Suggestions come from the bundled dictionary only: say so when it
        // has nothing, and whether Enter will still look further.
        let note =
            (suggest && search.suggestions.is_empty()).then_some(if self.config.online_lookup {
                tr().no_suggestions_online
            } else {
                tr().no_suggestions
            });
        // With several languages, each says which it's in.
        let items: Vec<_> = search
            .suggestions
            .iter()
            .map(|s| {
                let badge = multilingual.then_some(s.language.as_str());
                (s.word.as_str(), s.base.as_deref(), s.typed, badge)
            })
            .collect();
        let _ = popup.set_suggestions(&items, note);
        self.on_resized();
    }

    /// Enter in the search box, or a click on a suggestion: the card turns
    /// into the definition of the `picked` suggestion, in its language when
    /// there are several, or of what's typed, in all of them.
    pub(super) fn submit_search(&mut self, picked: Option<usize>) {
        let Some(search) = self.card.search.as_mut() else {
            return;
        };
        if search.languages.is_some() {
            return self.submit_translation();
        }
        let picked_row = picked.filter(|&i| i < search.suggestions.len());
        let picked = picked_row.map(|i| &search.suggestions[i]);
        let text = picked.map_or(search.edit.text(), |s| s.word.as_str());
        let multilingual = search.words.as_ref().is_some_and(|w| w.multilingual());
        let language = picked.filter(|_| multilingual).map(|s| s.language.clone());
        let complaint = match dictionary::normalize(text) {
            Ok(word) => {
                self.card.search = None;
                // The heading grows out of the word that was picked.
                if let Some(popup) = self.card.popup.as_mut() {
                    let _ = popup.pick_suggestion(picked_row);
                }
                let query = Query { word, language };
                return self.lookup(query, Place::Same(Transition::Replace));
            }
            Err(NormalizeError::Empty) => None,
            Err(NormalizeError::TooLong) if !search.error => {
                search.error = true;
                Some(tr().search_too_long)
            }
            Err(NormalizeError::TooLong) => None,
        };
        if let Some(popup) = self.card.popup.as_mut() {
            let _ = popup.shake_search();
            if let Some(complaint) = complaint {
                let _ = popup.set_search_title(complaint);
            }
        }
    }

    /// Enter in the box opened to translate: the card turns into the
    /// translation of what's typed, between the languages under it.
    fn submit_translation(&mut self) {
        let Some(search) = self.card.search.as_ref() else {
            return;
        };
        let Some(l) = search.languages.as_ref() else {
            return;
        };
        let way = (l.from.clone(), l.to.clone());
        // The box takes no more than can be translated.
        let Ok(text) = translate::passage(search.edit.text()) else {
            if let Some(popup) = self.card.popup.as_ref() {
                let _ = popup.shake_search();
            }
            return;
        };
        let typed = self.card.search.take();
        (self.card.current, self.card.speakable) = (None, None);
        let about = Translatable {
            word: text,
            senses: Vec::new(),
        };
        self.card.translatable = Some(about.clone());
        if let Some(popup) = self.card.popup.as_mut() {
            popup.hold_region(false);
        }
        self.begin_translation(about, way, false, typed);
    }

    /// Back from the translation of what was typed: the box it was typed
    /// in comes back as it was, its text and languages and all.
    pub(super) fn reopen_search(&mut self, search: Search) {
        let Some(popup) = self.visible_popup_mut() else {
            return;
        };
        popup.hold_region(false);
        if let Err(e) = popup.return_to_search(search.title, search.purpose) {
            return self.tray.notify(tr().draw_window_failed, &e.message());
        }
        self.card.clear_content();
        // Answers still on their way are for the translation.
        self.new_request();
        self.card.search = Some(search);
        self.refresh_search();
        self.show_typed_languages();
        self.sync_link_keys();
        self.sync_back_keys();
    }
}

/// Which way `text`, typed to be translated, goes: from the language its
/// words are in, among the dictionaries', as the `config` says.
fn typed_direction(text: &str, config: &Config) -> translate::Direction {
    let defaults = (config.translate_from.as_str(), config.translate_to.as_str());
    let from = offline::detect(text);
    translate::typed(from.as_deref(), &config.languages, defaults)
}
