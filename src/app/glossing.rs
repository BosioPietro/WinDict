//! Glossing a passage too long to look up: its uncommon words underlined,
//! each a link to its definition, which Back comes back from.

use super::*;
use crate::gloss;

impl App {
    /// Glosses `text`, the selection, at `anchor`, unless the card is open
    /// `in_place` already. That's done on a thread of its own: the lists of
    /// common words are unpacked for it (a few tens of ms), and dropped
    /// again straight after. For a phrase the dictionary hadn't, `not_found`
    /// is where to say so, if there's nothing to gloss in it.
    pub(super) fn gloss_selection(
        &mut self,
        text: &str,
        anchor: Option<RECT>,
        in_place: bool,
        not_found: Option<Place>,
    ) {
        let Ok(text) = translate::passage(text) else {
            return self.show_message(anchor, tr().too_long, tr().too_long_detail, true);
        };
        // Ctrl+Enter types its translation over it, as over any passage
        // translated.
        self.card.selected = Some(Selected {
            text: text.clone(),
            window: unsafe { GetForegroundWindow() },
        });
        let (id, events) = (self.card.request, self.events.clone());
        let preferred = self.config.languages.clone();
        let not_found = not_found.map(|place| (place, text.clone()));
        std::thread::spawn(move || {
            let gloss = gloss::with_dictionaries(&text, None, &preferred);
            events.send(Event::Glossed(Glossed {
                id,
                anchor,
                in_place,
                gloss,
                not_found,
            }));
        });
    }

    pub(super) fn on_glossed(&mut self, glossed: Glossed) {
        let visible = self.visible_popup().is_some();
        if glossed.id != self.card.request || (glossed.in_place && !visible) {
            return;
        }
        let gloss = glossed.gloss.filter(|g| !g.words.is_empty() || glossed.not_found.is_none());
        if let (None, Some((place, phrase))) = (&gloss, glossed.not_found) {
            self.card.selected = None;
            return self.present(place, &phrase, &Lookup::NotFound);
        }
        let Some(gloss) = gloss else {
            let (title, detail) = (tr().gloss_unknown, tr().gloss_unknown_detail);
            return self.show_message(glossed.anchor, title, detail, true);
        };
        self.card.passage = Some(Passage {
            gloss: Rc::new(gloss),
            focus: None,
        });
        let place = match glossed.in_place {
            true => Place::Same(Transition::Replace),
            false => Place::New(anchor_or_cursor(glossed.anchor)),
        };
        self.show_passage(place);
    }

    /// Shows the passage glossed at `place`, under the name of its
    /// language, the focus back where it was.
    pub(super) fn show_passage(&mut self, place: Place) {
        let Some(passage) = self.card.passage.as_mut() else {
            return;
        };
        let (gloss, focus) = (passage.gloss.clone(), passage.focus.take());
        self.card.current = None;
        self.card.history.clear();
        let title = translate::name(&gloss.language);
        self.present_body(place, &title, None, Body::Gloss(&gloss));
        if let Some(target) = focus {
            if let Some(popup) = self.visible_popup_mut() {
                let _ = popup.focus_target(&target);
            }
            self.sync_link_keys();
        }
        self.sync_back_keys();
    }

    /// Back, from a word looked up from the passage, or its translation,
    /// to the passage, if there's one.
    pub(super) fn back_to_passage(&mut self) {
        if self.card.passage.is_none() {
            return;
        }
        // Answers still on their way are for what's being left.
        self.new_request();
        self.show_passage(Place::Same(Transition::Replace));
    }
}
