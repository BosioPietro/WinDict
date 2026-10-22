//! The translation card: fetching it, showing it, and picking its languages.

use super::*;

impl App {
    /// Whether the card shows a definition it can turn into a translation.
    pub(super) fn can_translate(&self) -> bool {
        self.config.online_lookup
            && self.card.translatable.is_some()
            && self.card.translating.is_none()
    }

    /// Ctrl+T: turns the definition into a translation, or back.
    pub(super) fn toggle_translation(&mut self) {
        if self.card.translating.is_some() {
            self.back();
        } else {
            self.translate();
        }
    }

    /// The translation shortcut: turns `text`, the selection, into a
    /// translation straight away, a word or a whole passage alike. The card
    /// opens at `anchor`, unless it's open `in_place` already. A word the
    /// dictionary has is translated as Ctrl+T would from its definition,
    /// which Back then shows.
    pub(super) fn translate_selection(&mut self, text: &str, anchor: Option<RECT>, in_place: bool) {
        let text = match translate::passage(text) {
            Ok(text) => text,
            Err(NormalizeError::Empty) => {
                let detail = fill(
                    tr().nothing_selected_detail,
                    &[("hotkey", &self.config.translate_hotkey)],
                );
                return self.show_message(anchor, tr().nothing_selected, &detail, true);
            }
            Err(NormalizeError::TooLong) => {
                return self.show_message(anchor, tr().too_long, tr().translate_too_long, true);
            }
        };
        // Translations come from the internet, like the online answers.
        if !self.config.online_lookup {
            return self.show_message(
                anchor,
                tr().translate_offline,
                tr().translate_offline_detail,
                true,
            );
        }
        let detail = Detail {
            senses: 1,
            synonyms: 0,
            examples: false,
        };
        let word = dictionary::normalize(&text).ok();
        let def = word.as_deref().and_then(|w| offline::lookup(w, detail));
        // What's selected is translated as it's written, in the languages
        // the dictionary has it in.
        let (current, senses) = match (word, def) {
            (Some(word), Some(def)) => {
                let query = Query {
                    word,
                    language: None,
                };
                (Some(query), Translatable::of(&def).senses)
            }
            _ => (None, Vec::new()),
        };
        if !in_place {
            self.show_body(
                anchor_or_cursor(anchor),
                "WinDict",
                None,
                Body::Loading,
                false,
            );
        }
        (self.card.current, self.card.speakable) = (current, None);
        self.card.translatable = Some(Translatable { word: text, senses });
        self.translate();
    }

    /// Turns the card into a translation of its word: from the language
    /// it's in to the other one in use, or as the settings say when that
    /// can't be told.
    pub(super) fn translate(&mut self) {
        let visible = self.card.popup.as_ref().is_some_and(|p| p.visible);
        if !visible || !self.can_translate() {
            return;
        }
        let Some(about) = self.card.translatable.clone() else {
            return;
        };
        let found: Vec<&str> = about.senses.iter().map(|(l, _)| l.as_str()).collect();
        let defaults = (
            self.config.translate_from.as_str(),
            self.config.translate_to.as_str(),
        );
        let direction = translate::pick(&found, &self.config.languages, defaults);
        let asking = !direction.sure;
        self.card.translating = Some(Translating {
            about,
            from: direction.from,
            to: direction.to,
            asking,
            picking: None,
            result: None,
            waiting: false,
            focus: None,
            shown: false,
        });
        // Answers still on their way are for the definition.
        self.new_request();
        if !asking {
            self.fetch_translation();
        }
        let focus = asking.then_some(Target::Confirm);
        self.show_translation(Transition::Replace, focus);
        self.sync_back_keys();
    }

    /// Translates the word the way the card says, from the cache when it can.
    pub(super) fn fetch_translation(&mut self) {
        let Some(t) = self.card.translating.as_ref() else {
            return;
        };
        let key = (t.about.word.clone(), t.from.clone(), t.to.clone());
        let context = t
            .about
            .senses
            .iter()
            .find(|(language, _)| *language == t.from)
            .and_then(|(_, sense)| sense.clone());
        let cached = self.translations.get(&key);
        let Some(t) = self.card.translating.as_mut() else {
            return;
        };
        t.result = cached.map(Ok);
        t.waiting = t.result.is_none();
        if !t.waiting {
            return;
        }
        if let Some(hwnd) = self.card.hwnd() {
            win::set_timer(hwnd, TIMER_TRANSLATING, TRANSLATING_WAIT_MS);
        }
        self.new_request();
        let (id, events) = (self.card.request, self.events.clone());
        let email = self.config.mymemory_email.clone();
        std::thread::spawn(move || {
            let (word, from, to) = &key;
            let result = translate::translate(word, from, to, context.as_deref(), &email);
            events.send(Event::Translated(Translated { id, key, result }));
        });
    }

    pub(super) fn on_translated(&mut self, translated: Translated) {
        // Remember translations; retry failures (e.g. offline) next time.
        if let Ok(translation) = &translated.result {
            self.translations
                .put(translated.key.clone(), translation.clone());
        }
        let visible = self.card.popup.as_ref().is_some_and(|p| p.visible);
        if translated.id != self.card.request || !visible {
            return;
        }
        let hwnd = self.card.hwnd();
        let Some(t) = self.card.translating.as_mut() else {
            return;
        };
        t.result = Some(translated.result);
        t.waiting = false;
        if let Some(hwnd) = hwnd {
            win::kill_timer(hwnd, TIMER_TRANSLATING);
        }
        let focus = t.focus.take();
        let transition = t.transition();
        self.show_translation(transition, focus);
    }

    /// No quick answer: shows the translation on its way.
    pub(super) fn stop_waiting(&mut self) {
        let Some(t) = self.card.translating.as_mut().filter(|t| t.waiting) else {
            return;
        };
        t.waiting = false;
        let focus = t.focus.take();
        let transition = t.transition();
        self.show_translation(transition, focus);
    }

    /// Shows the translation as it stands, the focus on `focus`, or where
    /// it was.
    pub(super) fn show_translation(&mut self, transition: Transition, focus: Option<Target>) {
        // Back goes to the definition, when there's one to go to.
        let back = self.card.current.is_some();
        let Some(t) = self.card.translating.as_mut() else {
            return;
        };
        if t.waiting && !t.shown {
            // The definition stays a moment longer.
            t.focus = focus.or(t.focus.take());
            return;
        }
        let Some(popup) = self.card.popup.as_mut().filter(|p| p.visible) else {
            return;
        };
        // The list has the keyboard while it's open.
        let focus = match t.picking {
            Some(_) => None,
            None => focus.or_else(|| popup.focused_link().cloned()),
        };
        let state = match (&t.result, t.asking) {
            (_, true) => render::Translated::Asking,
            (None, false) if t.waiting => render::Translated::Waiting,
            (None, false) => render::Translated::Loading,
            (Some(Ok(translation)), false) => render::Translated::Done(translation),
            (Some(Err(error)), false) => render::Translated::Failed(error),
        };
        let picker = t.picking.as_ref().map(|p| render::Picker {
            side: p.side,
            query: p.query.text(),
            caret: p.query.caret_utf16(),
            matches: &p.matches,
            first: p.first,
            highlight: p.highlight,
        });
        let card = TranslationCard {
            word: &t.about.word,
            from: &t.from,
            to: &t.to,
            state,
            picker,
        };
        popup.hold_region(t.picking.is_some());
        let _ = popup.set_back(back);
        let _ = popup.set_translate(false);
        let _ = popup.set_speak(false);
        let body = Body::Translation(card);
        let title = translate::heading(&t.about.word);
        if popup.set_content(&title, None, body, transition).is_err() {
            return;
        }
        t.shown = true;
        if t.picking.is_some() {
            let _ = popup.unfocus();
        } else if let Some(target) = focus {
            let _ = popup.focus_target(&target);
        }
        self.on_resized();
        self.sync_link_keys();
        self.sync_back_keys();
    }

    /// The list a language is being picked from, if it's open, and the
    /// language that side has now.
    pub(super) fn picking(&mut self) -> Option<(&mut Picking, &str)> {
        let t = self.card.translating.as_mut()?;
        let picking = t.picking.as_mut()?;
        let current = match picking.side {
            Side::From => &t.from,
            Side::To => &t.to,
        };
        Some((picking, current))
    }

    /// A character typed while the list of languages is open: narrows it
    /// down, or, with Enter, picks the highlighted one.
    pub(super) fn type_language(&mut self, unit: u16) {
        let hwnd = self.card.popup.as_ref().map(|p| p.hwnd);
        let Some((picking, current)) = self.picking() else {
            return;
        };
        match picking.query.type_unit(unit) {
            Input::Edited => {}
            Input::Enter => {
                if let Some(code) = picking.highlighted() {
                    self.steer_translation(Target::Pick(code.into()));
                }
                return;
            }
            Input::Paste => {
                let pasted = hwnd.and_then(selection::clipboard_text).unwrap_or_default();
                picking.query.paste(&pasted);
            }
            Input::Nothing => return,
        }
        let current = current.to_string();
        picking.refilter(&current);
        self.show_translation(Transition::Refine, None);
    }

    /// A key pressed while the list of languages is open: Up, Down, Tab,
    /// Shift+Tab, Page Up and Page Down move the highlight; the others move
    /// the caret, as in the search box.
    pub(super) fn language_key(&mut self, key: VIRTUAL_KEY) {
        let ctrl = hotkey::is_held(VK_CONTROL);
        let Some((picking, current)) = self.picking() else {
            return;
        };
        let page = PICKER_ROWS as isize;
        let step = match key {
            VK_DOWN => Some(1),
            VK_UP => Some(-1),
            VK_TAB if hotkey::is_held(VK_SHIFT) => Some(-1),
            VK_TAB => Some(1),
            VK_NEXT => Some(page),
            VK_PRIOR => Some(-page),
            _ => None,
        };
        if let Some(by) = step {
            picking.step(by);
            return self.show_translation(Transition::Refine, None);
        }
        let edit = &mut picking.query;
        let changed = match key {
            VK_LEFT if ctrl => edit.move_caret(Motion::WordBack),
            VK_LEFT => edit.move_caret(Motion::CharBack),
            VK_RIGHT if ctrl => edit.move_caret(Motion::WordForward),
            VK_RIGHT => edit.move_caret(Motion::CharForward),
            VK_HOME => edit.move_caret(Motion::Start),
            VK_END => edit.move_caret(Motion::End),
            VK_DELETE => {
                let to = if ctrl {
                    Motion::WordForward
                } else {
                    Motion::CharForward
                };
                if edit.delete(to) {
                    let current = current.to_string();
                    picking.refilter(&current);
                }
                true
            }
            _ => false,
        };
        if changed {
            self.show_translation(Transition::Refine, None);
        }
    }

    /// A click, or Enter, on the translation card.
    pub(super) fn steer_translation(&mut self, target: Target) {
        let Some(t) = self.card.translating.as_mut() else {
            return;
        };
        let mut focus = None;
        match target {
            // Open the list, or close it again.
            Target::Language(side) => {
                let open = t.picking.take().map(|p| p.side);
                if open == Some(side) {
                    focus = Some(Target::Language(side));
                } else {
                    let current = match side {
                        Side::From => &t.from,
                        Side::To => &t.to,
                    };
                    t.picking = Some(Picking::new(side, current));
                    self.take_keyboard();
                }
                return self.show_translation(Transition::Refine, focus);
            }
            Target::Swap => {
                t.picking = None;
                std::mem::swap(&mut t.from, &mut t.to);
            }
            // The other side's language swaps them round.
            Target::Pick(language) => {
                let Some(side) = t.picking.take().map(|p| p.side) else {
                    return;
                };
                focus = Some(Target::Language(side));
                let (picked, other) = match side {
                    Side::From => (&mut t.from, &mut t.to),
                    Side::To => (&mut t.to, &mut t.from),
                };
                if language == *picked {
                    return self.show_translation(Transition::Refine, focus);
                }
                if language == *other {
                    std::mem::swap(picked, other);
                } else {
                    *picked = language;
                }
            }
            Target::Confirm => t.asking = false,
            _ => return,
        }
        // Until the way is confirmed, only the offer changes.
        if !t.asking {
            self.new_request();
            self.fetch_translation();
        }
        self.show_translation(Transition::Refine, focus);
    }
}
