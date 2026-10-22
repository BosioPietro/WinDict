//! The typed-word box: opening it, typing into it, and submitting what's in it.

use super::*;

impl App {
    /// Nothing was selected: opens the card as a search box, for text that
    /// can't be selected (in an image, a protected PDF, a game). It opens at
    /// `anchor`, the text caret, when there is one.
    pub(super) fn open_search(&mut self, title: &'static str, anchor: Option<RECT>) {
        self.current = None;
        self.history.clear();
        (self.translatable, self.translating) = (None, None);
        self.sync_back_keys();
        let previous = unsafe { GetForegroundWindow() };
        self.dismiss_menu();
        let theme = self.theme();
        let Some(popup) = self.ensure_popup() else {
            return;
        };
        // With no caret to open at (an image, a game), the middle of the
        // screen the app is on.
        let anchor = anchor_or_centre(anchor, previous);
        if let Err(e) = popup.show_search(anchor, theme, title, tr().search_placeholder) {
            self.tray.notify(tr().draw_window_failed, &e.message());
            return;
        }
        let hwnd = popup.hwnd;
        self.on_shown(false);
        if !selection::take_foreground(hwnd) {
            let detail = fill(
                tr().nothing_selected_detail,
                &[("hotkey", &self.config.hotkey)],
            );
            return self.show_message(None, tr().nothing_selected, &detail, true);
        }
        // Opened again from the card itself, it keeps the original app.
        if previous != hwnd {
            self.return_focus = Some(previous);
        }
        self.search = Some(Search {
            title,
            ..Search::default()
        });
        self.sync_link_keys();
    }

    /// A character typed into the card (WM_CHAR).
    pub(super) fn on_char(&mut self, unit: u16) {
        if self.picking().is_some() {
            return self.type_language(unit);
        }
        let Some(hwnd) = self.popup.as_ref().filter(|p| p.visible).map(|p| p.hwnd) else {
            return;
        };
        let Some(search) = self.search.as_mut() else {
            return;
        };
        match search.edit.type_unit(unit) {
            Input::Edited => {}
            Input::Enter => {
                let picked = self.popup.as_ref().and_then(|p| p.suggestion());
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
        if self.picking().is_some() {
            return self.language_key(key);
        }
        let (Some(search), Some(popup)) = (self.search.as_mut(), self.popup.as_mut()) else {
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

    /// Shows what's typed in the search box, and the words it could be the
    /// start of.
    pub(super) fn refresh_search(&mut self) {
        let (Some(search), Some(popup)) = (self.search.as_mut(), self.popup.as_mut()) else {
            return;
        };
        // Typing again answers the complaint: the heading goes back.
        if std::mem::take(&mut search.error) {
            let _ = popup.set_search_title(search.title);
        }
        let _ = popup.set_query(search.edit.text(), search.edit.caret_utf16());
        let typed = search.edit.text().trim_start().to_lowercase();
        let typed_chars = typed.chars().count();
        let suggest = typed_chars >= SUGGEST_AFTER;
        let mut multilingual = false;
        search.suggestions = if suggest {
            let words = search.words.get_or_insert_with(offline::Words::load);
            multilingual = words.multilingual();
            words.suggest(&typed, MAX_SUGGESTIONS)
        } else {
            Vec::new()
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
        let Some(search) = self.search.as_mut() else {
            return;
        };
        let picked_row = picked.filter(|&i| i < search.suggestions.len());
        let picked = picked_row.map(|i| &search.suggestions[i]);
        let text = picked.map_or(search.edit.text(), |s| s.word.as_str());
        let multilingual = search.words.as_ref().is_some_and(|w| w.multilingual());
        let language = picked.filter(|_| multilingual).map(|s| s.language.clone());
        let complaint = match dictionary::normalize(text) {
            Ok(word) => {
                self.search = None;
                // The heading grows out of the word that was picked.
                if let Some(popup) = self.popup.as_mut() {
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
        if let Some(popup) = self.popup.as_mut() {
            let _ = popup.shake_search();
            if let Some(complaint) = complaint {
                let _ = popup.set_search_title(complaint);
            }
        }
    }
}
