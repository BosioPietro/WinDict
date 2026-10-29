//! Reading text off the screen into the search box: the card makes way for
//! an area of the screen to be picked, and comes back by it with the text
//! read there typed in, and, to translate, the language it's in.

use super::*;
use crate::ocr::{self, ScanError, Scanned};
use crate::snip::Snip;

/// How long after the card starts fading out the screen is taken: it's
/// gone from it by then.
const SNIP_AFTER_MS: u32 = HIDE_DONE_MS + 60;

/// The search box put aside while an area of the screen is picked.
pub(super) struct Scanning {
    search: Search,
    return_focus: Option<HWND>,
    /// Where the card was, to come back there if no area is picked.
    card: RECT,
    /// The window the area's picked in, once the card's out of the way.
    snip: Option<Snip>,
}

impl App {
    /// The scan button: the card makes way for the screen behind it.
    pub(super) fn scan(&mut self) {
        if self.card.pinned {
            return;
        }
        let Some(search) = self.card.search.take() else {
            return;
        };
        let card = self
            .card
            .popup
            .as_ref()
            .map_or(RECT::default(), |p| p.card_rect());
        // The keyboard stays with us, for the screen to be picked from.
        let return_focus = self.card.return_focus.take();
        self.scanning = Some(Scanning {
            search,
            return_focus,
            card,
            snip: None,
        });
        self.dismiss();
        win::set_timer(self.hwnd, TIMER_SNIP, SNIP_AFTER_MS);
    }

    /// The card's gone: the screen, as it is now, to pick an area of.
    pub(super) fn start_snip(&mut self) {
        let accent = self.theme().accent;
        let Some(scanning) = self.scanning.as_mut() else {
            return;
        };
        let rgb = (accent.R, accent.G, accent.B);
        let card = scanning.card;
        let near = POINT {
            x: (card.left + card.right) / 2,
            y: (card.top + card.bottom) / 2,
        };
        match Snip::open(self.hwnd, WM_APP_SNIPPED, rgb, near) {
            Some(snip) => scanning.snip = Some(snip),
            None => self.end_snip(false),
        }
    }

    /// An area was `picked`, or not: the search box comes back as it was,
    /// beside the area, and reads the text in it; or else where it was.
    pub(super) fn end_snip(&mut self, picked: bool) {
        let Some(mut scanning) = self.scanning.take() else {
            return;
        };
        let snip = scanning.snip.take();
        let area = snip.as_ref().filter(|_| picked).and_then(Snip::picked);
        let zoom = self.config.zoom_factor();
        let anchor = match &area {
            Some((_, on_screen)) => popup::anchor_beside(*on_screen, zoom),
            None => popup::anchor_where(scanning.card, zoom),
        };
        let mut search = scanning.search;
        // What's read is answered with this request's id.
        self.new_request();
        // Opened while the window's still up, it takes the keyboard from it.
        self.open_search(search.purpose, Some(anchor));
        drop(snip);
        let Some(fresh) = self.card.search.take() else {
            return;
        };
        // Words still being read when it was put aside are for the new box.
        if search.words.is_none() {
            search.words = fresh.words;
        }
        search.error = false;
        self.card.return_focus = scanning.return_focus;
        self.card.search = Some(search);
        self.refresh_search();
        if self
            .card
            .search
            .as_ref()
            .is_some_and(|s| s.languages.is_some())
        {
            self.show_typed_languages();
        }
        let Some((picture, _)) = area else {
            return;
        };
        if let (Some(search), Some(popup)) = (self.card.search.as_mut(), self.card.popup.as_mut()) {
            // The heading says it's reading; what's read answers it.
            search.error = true;
            let _ = popup.set_search_title(tr().scan_reading);
        }
        let (id, events) = (self.card.request, self.events.clone());
        std::thread::spawn(move || events.send(Event::Scanned(id, ocr::read(&picture))));
    }

    /// Gives up on picking an area, for something else the card's for.
    pub(super) fn cancel_scan(&mut self) {
        self.scanning = None;
        win::kill_timer(self.hwnd, TIMER_SNIP);
    }

    /// The text read off the screen for request `id`: it's typed in the
    /// search box, if it's still open for it.
    pub(super) fn on_scanned(&mut self, id: u64, result: Result<Scanned, ScanError>) {
        if id != self.card.request {
            return;
        }
        let (Some(search), Some(popup)) = (self.card.search.as_mut(), self.card.popup.as_mut())
        else {
            return;
        };
        match result {
            Ok(Scanned { text, language }) => {
                search.edit.replace(&text);
                search.scanned = language;
                self.refresh_search();
            }
            Err(e) => {
                let complaint = match e {
                    ScanError::NoText => tr().scan_nothing,
                    ScanError::NoRecognizer => tr().scan_unavailable,
                };
                // Typing again brings the heading back.
                search.error = true;
                let _ = popup.set_search_title(complaint);
                let _ = popup.shake_search();
            }
        }
    }
}
