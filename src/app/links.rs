//! Keyboard focus on the card's links, and following them.

use super::*;

impl App {
    /// Tab and Shift+Tab move between the card's links, and Enter opens the
    /// one they reached, but only while there's one to go to: otherwise the
    /// keys belong to the app underneath. The search box reads its own keys.
    pub(super) fn sync_link_keys(&mut self) {
        // As does the list of languages.
        let picking = self
            .translating
            .as_ref()
            .is_some_and(|t| t.picking.is_some());
        let popup = self
            .popup
            .as_ref()
            .filter(|p| p.visible && self.search.is_none() && !picking);
        let tab = popup.is_some_and(|p| p.has_links());
        let enter = popup.is_some_and(|p| p.focused_link().is_some());
        // Ctrl+T, likewise, only while there's a translation to go to or from.
        let visible = self.popup.as_ref().is_some_and(|p| p.visible);
        let translate = visible && (self.can_translate() || self.translating.is_some());
        self.set_link_keys(tab, enter);
        self.translate_key.set(self.hwnd, translate);
    }

    pub(super) fn set_link_keys(&mut self, tab: bool, enter: bool) {
        self.link_keys.set(self.hwnd, tab);
        self.open_key.set(self.hwnd, enter);
    }

    /// Tab or Shift+Tab: rings the next or previous link.
    pub(super) fn step_link(&mut self, forward: bool) {
        if let Some(popup) = self.popup.as_mut().filter(|p| p.visible) {
            let _ = popup.step_link(forward);
        }
        self.sync_link_keys();
    }

    /// Enter: opens the link Tab moved to.
    pub(super) fn open_focused_link(&mut self) {
        let target = self
            .popup
            .as_ref()
            .filter(|p| p.visible)
            .and_then(|p| p.focused_link())
            .cloned();
        if let Some(target) = target {
            self.open_link(target);
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
            Target::Back => self.back(),
            Target::Language(_) | Target::Swap | Target::Pick(_) | Target::Confirm => {
                self.steer_translation(target)
            }
        }
    }
}
