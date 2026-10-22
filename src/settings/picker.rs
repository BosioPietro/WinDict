//! The translation's language picker: choosing the two languages and typing to narrow the list.

use super::*;

impl Settings {
    // -----------------------------------------------------------------------
    // The default languages

    /// The language on `side`, as a code.
    pub(super) fn language(&self, side: Side) -> &str {
        let config = &self.values.config;
        match side {
            Side::From => &config.translate_from,
            Side::To => &config.translate_to,
        }
    }

    /// The change to make once the languages are as they are now.
    pub(super) fn languages_changed(&self) -> Change {
        let config = &self.values.config;
        Change::TranslateLanguages(config.translate_from.clone(), config.translate_to.clone())
    }

    /// The boxes of the default languages, with the list under the one
    /// being picked.
    /// Returns them with how much shorter they are than with the list
    /// full, in pixels: the view is built for the full list, and what comes
    /// after slides up by that much.
    pub(super) fn layout_languages(&self) -> Result<(Layout, f32)> {
        let picker = self.picking.as_ref().map(|p| Picker {
            side: p.side,
            query: p.query.text(),
            caret: p.query.caret_utf16(),
            matches: &p.matches,
            first: p.first,
            highlight: p.highlight,
        });
        let width = GROUP_W - 2.0 * ROW_PAD;
        let config = &self.values.config;
        let (from, to) = (&config.translate_from, &config.translate_to);
        let layout = self
            .renderer
            .layout_languages(from, to, picker.as_ref(), width)?;
        let spare = picker.as_ref().map_or(0.0, Picker::spare);
        let s = self.scale;
        let slack = ((layout.height + spare) * s).ceil() - (layout.height * s).ceil();
        Ok((layout, slack))
    }

    /// Builds the window's view again, for what the default languages show
    /// to have changed, which can change the height of the row too.
    pub(super) fn relayout(&mut self) -> Result<()> {
        if self.refresh_boxes()? {
            return self.refit();
        }
        self.slide_view()?;
        self.refit()
    }

    /// Lists the languages `side`'s can be picked from, or shuts the list
    /// again if it's open on that side.
    pub(super) fn open_picker(&mut self, side: Side) -> Result<()> {
        let open = self.picking.take().map(|p| p.side);
        if open != Some(side) {
            self.picking = Some(Picking::new(side, self.language(side)));
        }
        self.focus = Some(Stop::Select(side));
        self.relayout()?;
        let Some(view) = &self.view else {
            return Ok(());
        };
        let rect = view.translate_at;
        if self.picking.is_some() {
            self.scroll_into_view(rect)?;
        }
        Ok(())
    }

    /// Swaps the languages.
    pub(super) fn swap_languages(&mut self) -> Result<Option<Change>> {
        self.picking = None;
        let config = &mut self.values.config;
        std::mem::swap(&mut config.translate_from, &mut config.translate_to);
        self.relayout()?;
        Ok(Some(self.languages_changed()))
    }

    /// Makes `code` the language of the side being picked, closing the list.
    /// The other side's language swaps them round.
    pub(super) fn pick(&mut self, code: &str) -> Result<Option<Change>> {
        let Some(side) = self.picking.take().map(|p| p.side) else {
            return Ok(None);
        };
        self.focus = Some(Stop::Select(side));
        let config = &mut self.values.config;
        let (picked, other) = match side {
            Side::From => (&mut config.translate_from, &mut config.translate_to),
            Side::To => (&mut config.translate_to, &mut config.translate_from),
        };
        let changed = code != picked.as_str();
        if code == other.as_str() {
            std::mem::swap(picked, other);
        } else {
            *picked = code.into();
        }
        self.relayout()?;
        Ok(changed.then(|| self.languages_changed()))
    }

    /// Narrows the list of languages down with a character typed.
    pub(super) fn type_language(&mut self, unit: u16) -> Result<Option<Change>> {
        let hwnd = self.hwnd;
        let Some(side) = self.picking.as_ref().map(|p| p.side) else {
            return Ok(None);
        };
        let current = self.language(side).to_owned();
        let Some(picking) = self.picking.as_mut() else {
            return Ok(None);
        };
        match picking.query.type_unit(unit) {
            Input::Edited => {}
            Input::Paste => {
                let pasted = selection::clipboard_text(hwnd).unwrap_or_default();
                picking.query.paste(&pasted);
            }
            // The key going down picked it already.
            Input::Enter | Input::Nothing => return Ok(None),
        }
        picking.refilter(&current);
        self.relayout()?;
        Ok(None)
    }

    /// A key pressed while the list of languages is open: Up, Down, Tab,
    /// Shift+Tab, Page Up and Page Down move the highlight, Enter picks it,
    /// Esc shuts the list, and the others move the caret in the box.
    pub(super) fn picker_key(&mut self, key: VIRTUAL_KEY) -> Result<Option<Change>> {
        let ctrl = hotkey::is_held(VK_CONTROL);
        let Some(side) = self.picking.as_ref().map(|p| p.side) else {
            return Ok(None);
        };
        let current = self.language(side).to_owned();
        let Some(picking) = self.picking.as_mut() else {
            return Ok(None);
        };
        match key {
            VK_ESCAPE => {
                self.picking = None;
                self.focus = Some(Stop::Select(side));
                self.relayout()?;
                return Ok(None);
            }
            VK_RETURN => {
                return match picking.highlighted() {
                    Some(code) => self.pick(code),
                    None => Ok(None),
                };
            }
            _ => {}
        }
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
            self.relayout()?;
            return Ok(None);
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
                    picking.refilter(&current);
                }
                true
            }
            _ => false,
        };
        if changed {
            self.relayout()?;
        }
        Ok(None)
    }

    /// The email field's text changed from `before`, or may have: shows
    /// it, and saves it if it did.
    pub(super) fn typed(&mut self, before: &str) -> Result<Option<Change>> {
        self.show_field()?;
        if self.email.text() == before {
            return Ok(None);
        }
        let email = self.email.text().trim().to_owned();
        self.values.config.mymemory_email = email.clone();
        Ok(Some(Change::MyMemoryEmail(email)))
    }

    /// Puts the caret where the email field was clicked, `x` window pixels
    /// across.
    pub(super) fn place_caret(&mut self, x: i32) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let (x, _) = self.card_point(x, 0);
        let into = (x - view.field.at.x + self.email_shift) / self.scale;
        let at = self.renderer.field_position(self.email.text(), into)?;
        self.email.set_caret_utf16(at);
        self.show_field()
    }
}
