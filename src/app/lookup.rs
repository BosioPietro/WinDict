//! Looking a word up and showing the card for the result.

use super::*;

impl App {
    /// Looks `query` up and shows the answer at `place`. The bundled
    /// dictionary answers instantly; the online one fills gaps.
    pub(super) fn lookup(&mut self, query: Query, place: Place) {
        let detail = Detail {
            senses: self.config.max_definitions,
            synonyms: self.config.max_synonyms,
            examples: self.config.show_examples,
        };
        let word = query.word.clone();
        self.card.current = Some(query.clone());
        self.card.local = offline::lookup(&word, detail).and_then(|def| query.within(def));
        let online_key = Query {
            word: match &self.card.local {
                Some(def) => def.word.to_lowercase(),
                None => word.clone(),
            },
            language: query.language,
        };
        let online = self.cache.get(&online_key);

        match (&self.card.local, online) {
            (Some(local), online) => {
                let order = offline::languages();
                let shown = dictionary::merge(local.clone(), online.as_ref(), &order);
                let title = shown.word.clone();
                self.present_body(
                    place,
                    &title,
                    shown.phonetic.as_deref(),
                    Body::Definition(&shown),
                );
                if online.is_some() || !self.config.online_lookup {
                    return;
                }
            }
            (None, Some(online)) => return self.present(place, &word, &online),
            (None, None) if !self.config.online_lookup => {
                return self.present(place, &word, &Lookup::NotFound);
            }
            (None, None) => self.present_body(place, &word, None, Body::Loading { status: false }),
        }

        let (id, events) = (self.card.request, self.events.clone());
        // In the languages of the dictionaries in use, which is English when
        // the chosen ones couldn't be read, or the one picked.
        let languages = match &online_key.language {
            Some(language) => vec![language.clone()],
            None => offline::languages(),
        };
        std::thread::spawn(move || {
            let result = online::lookup(&online_key.word, detail, &languages);
            events.send(Event::Looked(Looked {
                id,
                query: online_key,
                result,
            }));
        });
    }

    pub(super) fn on_result(&mut self, looked: Looked) {
        // Remember definite answers; retry failures (e.g. offline) next time.
        if !matches!(looked.result, Lookup::Failed(_)) {
            self.cache.put(looked.query.clone(), looked.result.clone());
        }
        let visible = self.visible_popup().is_some();
        if looked.id != self.card.request || !visible {
            return;
        }
        match self.card.local.clone() {
            // Already showing the offline entry: refine it in place, if the
            // online one adds anything. Errors are irrelevant here.
            Some(local) => {
                // What's new: a pronunciation, parts of speech, languages.
                let measure = |def: &Definition| {
                    let meanings: usize = def.sections().map(|s| s.meanings.len()).sum();
                    (def.phonetic.clone(), meanings, def.others.len())
                };
                let before = measure(&local);
                let order = offline::languages();
                let merged = dictionary::merge(local, Some(&looked.result), &order);
                if measure(&merged) != before {
                    // Another language's part is new content, perhaps above
                    // what's being read: it fades in rather than snapping.
                    let transition = if merged.others.len() != before.2 {
                        Transition::Replace
                    } else {
                        Transition::Refine
                    };
                    let place = Place::Same(transition);
                    self.present(place, &looked.query.word, &Lookup::Found(merged));
                }
            }
            None => {
                let place = Place::Same(Transition::Replace);
                self.present(place, &looked.query.word, &looked.result);
            }
        }
    }

    /// The card's title, pronunciation and body for a lookup of `word`,
    /// offering `correction` when it wasn't found or couldn't be looked up.
    pub(super) fn card_for<'a>(
        word: &'a str,
        result: &'a Lookup,
        correction: Option<&'a str>,
    ) -> (&'a str, Option<&'a str>, Body<'a>) {
        match result {
            Lookup::Found(def) => (&def.word, def.phonetic.as_deref(), Body::Definition(def)),
            Lookup::NotFound => (
                word,
                None,
                Body::Message {
                    title: tr().not_found,
                    detail: match correction {
                        Some(_) => tr().not_found_detail,
                        None => tr().not_found_check_spelling,
                    },
                    suggestion: correction,
                },
            ),
            Lookup::Failed(error) => (
                word,
                None,
                Body::Message {
                    title: tr().lookup_failed,
                    detail: error,
                    suggestion: correction,
                },
            ),
        }
    }

    /// Shows `result` for `word` at `place`. A word that wasn't found, or
    /// couldn't be looked up, is shown once the word it was likely a typo of
    /// is found, on a thread of its own: that reads the whole word list (a
    /// few tens of ms), dropped again straight after.
    pub(super) fn present(&mut self, place: Place, word: &str, result: &Lookup) {
        if matches!(result, Lookup::Found(_)) {
            return self.present_corrected(place, word, result, None);
        }
        // A phrase selected that the dictionary hasn't is glossed instead,
        // if any of its words are uncommon.
        let missing = self.card.gloss_missing.take();
        if let (Lookup::NotFound, Some(phrase)) = (result, missing.filter(|p| p == word)) {
            let (anchor, in_place) = match place {
                Place::New(anchor) => (Some(anchor), false),
                Place::Same(_) => (None, true),
            };
            return self.gloss_selection(&phrase, anchor, in_place, Some(place));
        }
        let (id, events) = (self.card.request, self.events.clone());
        let (word, result) = (word.to_string(), result.clone());
        std::thread::spawn(move || {
            let correction = offline::Words::load().correct(&word).map(str::to_string);
            events.send(Event::Corrected(Corrected {
                id,
                place,
                word,
                result,
                correction,
            }));
        });
    }

    pub(super) fn on_corrected(&mut self, c: Corrected) {
        let visible = self.visible_popup().is_some();
        if c.id != self.card.request || (matches!(c.place, Place::Same(_)) && !visible) {
            return;
        }
        self.present_corrected(c.place, &c.word, &c.result, c.correction.as_deref());
    }

    pub(super) fn present_corrected(
        &mut self,
        place: Place,
        word: &str,
        result: &Lookup,
        correction: Option<&str>,
    ) {
        let (title, phonetic, body) = Self::card_for(word, result, correction);
        self.present_body(place, title, phonetic, body);
    }

    pub(super) fn present_body(
        &mut self,
        place: Place,
        title: &str,
        phonetic: Option<&str>,
        body: Body,
    ) {
        self.card.translatable = match &body {
            Body::Definition(def) => Some(Translatable::of(def)),
            Body::Gloss(gloss) => Some(Translatable {
                word: gloss.text.clone(),
                senses: vec![(gloss.language.clone(), None)],
            }),
            _ => None,
        };
        self.card.speakable = match &body {
            Body::Definition(def) => self.voice_for(&def.language).map(|v| (def.word.clone(), v)),
            // A passage's title is its language's name, which the button
            // would seem to say.
            _ => None,
        };
        self.card.translating = None;
        match place {
            // Pinned since it was asked for, the card stays where it is.
            Place::New(_) if self.card.pinned => {
                self.present_body(Place::Same(Transition::Replace), title, phonetic, body)
            }
            Place::New(anchor) => self.show_body(anchor, title, phonetic, body, false),
            Place::Same(transition) => {
                let back = self.has_word_behind();
                let translate = self.can_translate();
                let speak = self.card.speakable.is_some();
                let Some(popup) = self.card.popup.as_mut() else {
                    return;
                };
                let _ = popup.set_back(back);
                let _ = popup.set_translate(translate);
                let _ = popup.set_speak(speak);
                if popup
                    .set_content(title, phonetic, body, transition)
                    .is_err()
                {
                    return;
                }
                self.on_resized();
                self.sync_link_keys();
            }
        }
    }

    pub(super) fn show_message(
        &mut self,
        anchor: Option<RECT>,
        title: &str,
        detail: &str,
        auto_hide: bool,
    ) {
        self.card.clear_content();
        let anchor = anchor_or_cursor(anchor);
        self.show_body(
            anchor,
            "WinDict",
            None,
            Body::Message {
                title,
                detail,
                suggestion: None,
            },
            auto_hide,
        );
    }

    pub(super) fn show_body(
        &mut self,
        anchor: RECT,
        title: &str,
        phonetic: Option<&str>,
        body: Body,
        auto_hide: bool,
    ) {
        self.dismiss_menu();
        let theme = self.theme();
        let translate = self.can_translate();
        let speak = self.card.speakable.is_some();
        let Some(popup) = self.ensure_popup() else {
            return;
        };
        let _ = popup.set_translate(translate);
        let _ = popup.set_speak(speak);
        if let Err(e) = popup.show(anchor, theme, title, phonetic, body) {
            self.tray.notify(tr().draw_window_failed, &e.message());
            return;
        }
        self.on_shown(auto_hide);
    }

    /// The card just appeared, or turned into something else in place.
    pub(super) fn on_shown(&mut self, auto_hide: bool) {
        let hwnd = self.hwnd;
        self.on_resized();
        for timer in [TIMER_HIDE, TIMER_TEARDOWN, TIMER_AUTOHIDE] {
            win::kill_timer(hwnd, timer);
        }
        if auto_hide {
            win::set_timer(hwnd, TIMER_AUTOHIDE, AUTOHIDE_MS);
        }
        SHOWN_AT.set(win::now_ms());
        self.install_dismiss_hooks();
        self.sync_link_keys();
        win::set_timer(hwnd, TIMER_COVERED, COVERED_CHECK_MS);
    }

    /// The card changed size: clicks outside it now are those outside the
    /// new size, and its clickable area shrinks once the resize has played.
    pub(super) fn on_resized(&mut self) {
        let Some(popup) = self.card.popup.as_ref() else {
            return;
        };
        // A pinned card stays open whatever is clicked.
        if !self.card.pinned {
            CARD_RECT.set(popup.card_rect());
        }
        win::set_timer(popup.hwnd, TIMER_SETTLE, SETTLE_MS);
    }

    /// The id of a voice saying words in `language`, if one is installed.
    fn voice_for(&mut self, language: &str) -> Option<HSTRING> {
        let voices = self.voices.get_or_insert_with(Voices::load);
        voices.for_language(language).cloned()
    }

    /// Says the word the card shows out loud, in its own language.
    pub(super) fn speak(&mut self) {
        let visible = self.visible_popup().is_some();
        if let Some((word, voice)) = self.card.speakable.as_ref().filter(|_| visible) {
            self.speaker.say(word, voice);
        }
    }

    /// Safety net: if Start/Search still covers the card, close it.
    pub(super) fn uncover(&mut self) {
        if selection::has_ui_access() {
            return;
        }
        let Some(popup) = self.visible_popup() else {
            return;
        };
        let r = popup.card_rect();
        let center = POINT {
            x: (r.left + r.right) / 2,
            y: (r.top + r.bottom) / 2,
        };
        let top = unsafe { GetAncestor(WindowFromPoint(center), GA_ROOT) };
        if top != popup.hwnd && selection::is_shell_flyout(top) {
            SHOWN_AT.set(win::now_ms());
            selection::close_flyout();
        }
    }

    /// Shows a synonym's definition in the card, remembering the way back.
    /// A card in one language stays in it, and a word of a passage is
    /// looked up in the passage's.
    pub(super) fn follow(&mut self, word: &str) {
        let Ok(word) = dictionary::normalize(word) else {
            return;
        };
        let language = match &self.card.current {
            Some(current) => current.language.clone(),
            None => self.card.passage.as_ref().map(|p| p.gloss.lookup_language()),
        };
        let query = Query { word, language };
        if self.card.current.as_ref() == Some(&query) {
            return;
        }
        match self.card.current.take() {
            Some(current) => {
                self.card.history.push(current);
                if self.card.history.len() > HISTORY_SIZE {
                    self.card.history.remove(0);
                }
            }
            // From the passage, which keeps where the focus was.
            None => {
                let focus = self.visible_popup().and_then(|p| p.focused_link()).cloned();
                if let Some(passage) = self.card.passage.as_mut() {
                    passage.focus = focus;
                }
            }
        }
        // Answers still on their way are for the word we're leaving.
        self.new_request();
        self.lookup(query, Place::Same(Transition::Replace));
        self.sync_back_keys();
    }

    /// Goes back to the word shown before the last synonym was followed, or
    /// from a translation to the definition it was of.
    pub(super) fn back(&mut self) {
        if !self.visible_popup().is_some() {
            return;
        }
        // From the translation of what was typed, back to the box.
        if let Some(typed) = self.card.translating.as_mut().and_then(|t| t.typed.take()) {
            return self.reopen_search(typed);
        }
        // A passage translated straight away has no definition to go to,
        // but one glossed goes back to its gloss.
        if self.card.translating.is_some() && self.card.current.is_none() {
            return self.back_to_passage();
        }
        let query = match self.card.translating.take() {
            Some(_) => self.card.current.clone(),
            None => self.card.history.pop(),
        };
        // The first word looked up from a passage goes back to it.
        let Some(query) = query else {
            return self.back_to_passage();
        };
        self.new_request();
        self.lookup(query, Place::Same(Transition::Replace));
        self.sync_back_keys();
    }

    /// Whether the card shows a translation of a definition, of a passage
    /// glossed, or of what was typed in the box, which Back and Ctrl+T go
    /// back to.
    pub(super) fn has_definition_behind(&self) -> bool {
        let behind = self.card.current.is_some() || self.card.passage.is_some();
        self.card
            .translating
            .as_ref()
            .is_some_and(|t| behind || t.typed.is_some())
    }

    /// Whether Back goes from the definition to another: one shown before,
    /// or the passage it was looked up from.
    pub(super) fn has_word_behind(&self) -> bool {
        let from_passage = self.card.current.is_some() && self.card.passage.is_some();
        !self.card.history.is_empty() || from_passage
    }

    /// Backspace and Alt+Left go back, but only while there's somewhere to
    /// go: otherwise they belong to the app underneath.
    pub(super) fn sync_back_keys(&mut self) {
        // A pinned card reads its own keys, while it has the keyboard.
        if self.card.pinned {
            return;
        }
        // Backspace edits what's typed in the list of languages.
        let picking = self
            .card
            .translating
            .as_ref()
            .is_some_and(|t| t.pair.picking.is_some());
        let wanted = (self.has_word_behind() || self.has_definition_behind()) && !picking;
        self.back_keys.set(self.hwnd, wanted);
    }
}
