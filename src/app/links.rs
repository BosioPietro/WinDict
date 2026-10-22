//! Keyboard focus on the card's links, following them, and copying them.

use super::*;

impl App {
    /// Tab and Shift+Tab move between the card's links, and Enter opens the
    /// one they reached, but only while there's one to go to: otherwise the
    /// keys belong to the app underneath. The search box reads its own keys.
    pub(super) fn sync_link_keys(&mut self) {
        // A pinned card reads its own keys, while it has the keyboard.
        if self.card.pinned {
            return;
        }
        // As does the list of languages.
        let picking = self
            .card
            .translating
            .as_ref()
            .is_some_and(|t| t.picking.is_some());
        let popup = self
            .card
            .popup
            .as_ref()
            .filter(|p| p.visible && self.card.search.is_none() && !picking);
        let tab = popup.is_some_and(|p| p.has_links());
        let enter = popup.is_some_and(|p| p.focused_link().is_some());
        // Up and Down, likewise, only while there's further to scroll.
        let scroll = popup.is_some_and(|p| p.scrolls());
        // Ctrl+T, likewise, only while there's a translation to go to or from.
        let visible = self.card.popup.as_ref().is_some_and(|p| p.visible);
        let translate = visible && (self.can_translate() || self.has_definition_behind());
        // And Ctrl+C only while there's something to copy.
        let copy = popup.is_some() && self.copyable().is_some();
        // Ctrl+plus and Ctrl+minus zoom any card but the search box.
        let zoom = visible && self.card.search.is_none();
        self.set_link_keys(tab, enter);
        self.scroll_keys.set(self.hwnd, scroll);
        self.translate_key.set(self.hwnd, translate);
        self.copy_key.set(self.hwnd, copy);
        self.zoom_keys.set(self.hwnd, zoom);
    }

    pub(super) fn set_link_keys(&mut self, tab: bool, enter: bool) {
        self.link_keys.set(self.hwnd, tab);
        self.open_key.set(self.hwnd, enter);
    }

    /// Tab or Shift+Tab: rings the next or previous link.
    pub(super) fn step_link(&mut self, forward: bool) {
        if let Some(popup) = self.card.popup.as_mut().filter(|p| p.visible) {
            let _ = popup.step_link(forward);
        }
        self.sync_link_keys();
    }

    /// Enter: opens the link Tab moved to.
    pub(super) fn open_focused_link(&mut self) {
        let target = self
            .card
            .popup
            .as_ref()
            .filter(|p| p.visible)
            .and_then(|p| p.focused_link())
            .cloned();
        if let Some(target) = target {
            self.open_link(target);
        }
    }

    /// What Ctrl+C copies: the word Tab is on, or else the translation the
    /// card shows, or the word it defines.
    fn copyable(&self) -> Option<String> {
        let popup = self.card.popup.as_ref().filter(|p| p.visible)?;
        if let Some(Target::Word(word)) = popup.focused_link() {
            return Some(word.clone());
        }
        match &self.card.translating {
            Some(t) if t.asking => None,
            Some(t) => match &t.result {
                Some(Ok(translation)) => Some(translation.text.clone()),
                _ => None,
            },
            None => self.card.translatable.as_ref().map(|t| t.word.clone()),
        }
    }

    /// Ctrl+C: copies what [`App::copyable`] says, and shows it did.
    pub(super) fn copy(&mut self) {
        let Some(text) = self.copyable() else {
            return;
        };
        if !selection::set_clipboard_text(self.hwnd, &text) {
            return;
        }
        if let Some(popup) = self.card.popup.as_ref() {
            let _ = popup.show_copied();
        }
    }

    pub(super) fn open_link(&mut self, target: Target) {
        match target {
            Target::Word(word) => self.follow(&word),
            Target::Url(url) => {
                win::shell_open(&HSTRING::from(url));
                self.dismiss();
            }
            Target::Translate => self.translate(),
            Target::Speak => self.speak(),
            Target::Back => self.back(),
            // Pinning moves the card out from where it's handled, so it's
            // done once this is.
            Target::Pin => {
                if let Some(hwnd) = self.card.hwnd() {
                    unsafe {
                        let _ = PostMessageW(
                            Some(self.hwnd),
                            WM_APP_PIN,
                            WPARAM(hwnd.0 as usize),
                            LPARAM(0),
                        );
                    }
                }
            }
            Target::Language(_) | Target::Swap | Target::Pick(_) | Target::Confirm => {
                self.steer_translation(target)
            }
        }
    }
}
