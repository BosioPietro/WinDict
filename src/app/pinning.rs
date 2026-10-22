//! Cards pinned open: each a window of its own, above everything, that
//! stays until it's closed while lookups open cards of their own.
//!
//! The flows in the other modules work on `self.card`. A pinned card's
//! window is handled by swapping that card in for the while.

use super::*;

impl App {
    /// Gives the card a new request to wait on, with an id unique among
    /// every card's.
    pub(super) fn new_request(&mut self) {
        self.requests += 1;
        self.card.request = self.requests;
    }

    /// The card shown in `hwnd`, if it's one of ours.
    pub(super) fn popup_of(&self, hwnd: HWND) -> Option<&Popup> {
        std::iter::once(&self.card)
            .chain(&self.pinned)
            .find_map(|c| c.popup.as_ref().filter(|p| p.hwnd == hwnd))
    }

    /// Whether `hwnd` is a pinned card's window.
    pub(super) fn is_pinned(&self, hwnd: HWND) -> bool {
        self.pinned.iter().any(|c| c.hwnd() == Some(hwnd))
    }

    fn pinned_index(&self, hwnd: HWND) -> Option<usize> {
        self.pinned.iter().position(|c| c.hwnd() == Some(hwnd))
    }

    /// Runs `f` on the card shown in `hwnd`, as the app's card.
    pub(super) fn with_card<R>(&mut self, hwnd: HWND, f: impl FnOnce(&mut App) -> R) -> Option<R> {
        if self.card.hwnd() == Some(hwnd) {
            return Some(f(self));
        }
        let i = self.pinned_index(hwnd)?;
        Some(self.as_card(i, f))
    }

    /// Runs `f` on the card waiting on request `id`, or else on the app's
    /// own card, which tells it's not the answer it waits on.
    pub(super) fn with_request(&mut self, id: u64, f: impl FnOnce(&mut App)) {
        match self.pinned.iter().position(|c| c.request == id) {
            Some(i) => self.as_card(i, f),
            None => f(self),
        }
    }

    /// Runs `f` on every card, pinned or not.
    pub(super) fn each_card(&mut self, mut f: impl FnMut(&mut App)) {
        f(self);
        for i in 0..self.pinned.len() {
            self.as_card(i, &mut f);
        }
    }

    /// Runs `f` with pinned card `i` swapped in as the app's card.
    fn as_card<R>(&mut self, i: usize, f: impl FnOnce(&mut App) -> R) -> R {
        std::mem::swap(&mut self.card, &mut self.pinned[i]);
        let r = f(self);
        std::mem::swap(&mut self.card, &mut self.pinned[i]);
        r
    }

    /// The pin button: pins the card in `hwnd` open, or lets it go again.
    pub(super) fn toggle_pin(&mut self, hwnd: HWND) {
        if self.card.hwnd() == Some(hwnd) {
            self.pin();
        } else if let Some(i) = self.pinned_index(hwnd) {
            self.unpin(i);
        }
    }

    /// Enter on the pin button. Pinned, the card gets the keyboard straight
    /// away, so Tab and Enter keep working in it.
    pub(super) fn pin_by_key(&mut self, hwnd: HWND) {
        let pinning = self.card.hwnd() == Some(hwnd);
        self.toggle_pin(hwnd);
        if pinning && self.is_pinned(hwnd) {
            selection::take_foreground(hwnd);
        }
    }

    /// Pins the card open. Lookups open a card of their own from now on.
    fn pin(&mut self) {
        let visible = self.card.popup.as_ref().is_some_and(|p| p.visible);
        // Still on its way, its answer would go to the next card.
        if !visible || self.card.copying || self.card.search.is_some() {
            return;
        }
        // The keys and hooks are for a card that closes on its own.
        self.set_link_keys(false, false);
        self.scroll_keys.set(self.hwnd, false);
        self.translate_key.set(self.hwnd, false);
        self.copy_key.set(self.hwnd, false);
        self.zoom_keys.set(self.hwnd, false);
        self.back_keys.set(self.hwnd, false);
        self.remove_dismiss_hooks();
        for timer in [TIMER_AUTOHIDE, TIMER_COVERED] {
            win::kill_timer(self.hwnd, timer);
        }
        CARD_RECT.set(RECT::default());
        let mut card = std::mem::take(&mut self.card);
        card.pinned = true;
        card.return_focus = None;
        if let Some(popup) = card.popup.as_mut() {
            let _ = popup.set_pinned(true);
        }
        self.pinned.push(card);
    }

    /// Lets pinned card `i` go: it closes on a click away again, in place
    /// of the card that comes and goes with lookups.
    fn unpin(&mut self, i: usize) {
        let card = self.pinned.remove(i);
        let previous = std::mem::replace(&mut self.card, card);
        self.retire(previous.popup);
        self.card.pinned = false;
        if let Some(popup) = self.card.popup.as_mut() {
            let _ = popup.set_pinned(false);
        }
        self.on_shown(false);
        self.sync_back_keys();
    }

    /// Ctrl+W, Alt+F4 or Close on the taskbar: closes the card in `hwnd`.
    pub(super) fn close_card(&mut self, hwnd: HWND) {
        if let Some(i) = self.pinned_index(hwnd) {
            let card = self.pinned.remove(i);
            self.retire(card.popup);
        } else if self.card.hwnd() == Some(hwnd) {
            self.dismiss();
        }
    }

    /// Fades a card out, and lets its window go once it has.
    fn retire(&mut self, popup: Option<Popup>) {
        let Some(mut popup) = popup else {
            return;
        };
        let _ = popup.begin_hide();
        self.closing.push(popup);
        win::set_timer(self.hwnd, TIMER_CLOSED, HIDE_DONE_MS);
    }

    /// A timer the card in `hwnd` set went off.
    pub(super) fn on_card_timer(&mut self, hwnd: HWND, id: usize) {
        win::kill_timer(hwnd, id);
        self.with_card(hwnd, |app| match id {
            TIMER_SETTLE => {
                if let Some(p) = app.card.popup.as_mut() {
                    p.settle();
                }
            }
            TIMER_TRANSLATING => app.stop_waiting(),
            _ => {}
        });
    }

    /// The pinned card in `hwnd` was dragged somewhere else.
    pub(super) fn card_moved(&mut self, hwnd: HWND) {
        self.with_card(hwnd, |app| {
            if let Some(p) = app.card.popup.as_mut() {
                p.moved();
            }
        });
    }

    /// A key pressed in a pinned card, which has the keyboard as any window
    /// does once clicked, rather than taking the keys while it's open.
    /// Returns whether it was one of the card's.
    pub(super) fn pinned_key(&mut self, key: VIRTUAL_KEY) -> bool {
        let ctrl = hotkey::is_held(VK_CONTROL);
        let picking = self.picking().is_some();
        match key {
            // Shuts the list of languages, if it's open.
            VK_ESCAPE if picking => self.escape(),
            k if k == VK_ESCAPE || (ctrl && k.0 == b'W' as u16) => {
                if let Some(hwnd) = self.card.hwnd() {
                    unsafe {
                        let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                    }
                }
            }
            // The list has the other keys while it's open.
            _ if picking => return false,
            k if ctrl && k.0 == b'C' as u16 => self.copy(),
            k if ctrl && k.0 == b'T' as u16 => self.toggle_translation(),
            VK_OEM_PLUS | VK_ADD if ctrl => self.zoom(Some(1)),
            VK_OEM_MINUS | VK_SUBTRACT if ctrl => self.zoom(Some(-1)),
            k if ctrl && (k.0 == b'0' as u16 || k == VK_NUMPAD0) => self.zoom(None),
            VK_TAB => self.step_link(!hotkey::is_held(VK_SHIFT)),
            VK_RETURN => self.open_focused_link(),
            VK_BACK => self.back(),
            VK_UP | VK_DOWN => self.scroll_line(key == VK_DOWN),
            _ => return false,
        }
        true
    }
}
