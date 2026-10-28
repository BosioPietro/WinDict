//! The language pickers, WinDict's own and the translation's two: choosing
//! a language and typing to narrow the list.

use super::*;

impl Settings {
    // -----------------------------------------------------------------------
    // The languages picked from a list

    /// The language `select`'s box shows, as a code.
    pub(super) fn language(&self, select: Select) -> &str {
        let config = &self.values.config;
        match select {
            Select::Interface => config.ui_language.code(),
            Select::Translate(Side::From) => &config.translate_from,
            Select::Translate(Side::To) => &config.translate_to,
        }
    }

    /// The change to make once the languages are as they are now.
    pub(super) fn languages_changed(&self) -> Change {
        let config = &self.values.config;
        Change::TranslateLanguages(config.translate_from.clone(), config.translate_to.clone())
    }

    /// The list, as it's drawn, while it's open, and the box it's under.
    fn picker(&self) -> Option<(Select, Picker<'_>)> {
        self.picking.as_ref().map(|p| {
            let picker = Picker {
                query: p.query.text(),
                caret: p.query.caret_utf16(),
                matches: &p.matches,
                highlight: p.highlight,
                rows: PICKER_ROWS,
            };
            (p.on, picker)
        })
    }

    /// The boxes of the default languages, with the list under the one
    /// being picked.
    /// Returns them with how much shorter they are than with the list
    /// full, in pixels: the view is built for the full list, and what comes
    /// after slides up by that much.
    pub(super) fn layout_languages(&self) -> Result<(Layout, f32)> {
        let picker = self.picker();
        let open = picker.as_ref().and_then(|(select, p)| match select {
            Select::Translate(side) => Some((*side, p)),
            Select::Interface => None,
        });
        let width = GROUP_W - 2.0 * ROW_PAD;
        let config = &self.values.config;
        let (from, to) = (&config.translate_from, &config.translate_to);
        let layout = self.renderer.layout_languages(from, to, open, width)?;
        let spare = open.map_or(0.0, |(_, p)| p.spare());
        let s = self.scale;
        let slack = ((layout.height + spare) * s).ceil() - (layout.height * s).ceil();
        Ok((layout, slack))
    }

    /// The box of the language WinDict is shown in, with the list that
    /// covers the rows under it while it's open, and what moves in the box
    /// as it opens or shuts.
    pub(super) fn layout_interface(&self) -> Result<(Layout, Moving)> {
        let picker = self.picker();
        let open = picker
            .as_ref()
            .filter(|(select, _)| *select == Select::Interface)
            .map(|(_, p)| p);
        let language = self.language(Select::Interface);
        self.renderer.layout_interface(language, open, INTERFACE_W)
    }

    /// Builds the window's view again, for what the languages' boxes show
    /// to have changed, which can change the height of the default
    /// languages' row too.
    pub(super) fn relayout(&mut self) -> Result<()> {
        self.refresh_interface()?;
        if self.refresh_boxes()? {
            self.show_list()?;
            return self.refit();
        }
        self.slide_view()?;
        self.refit()
    }

    /// Lists the languages `select`'s can be picked from, or shuts the
    /// list again if it's open under it.
    pub(super) fn open_picker(&mut self, select: Select) -> Result<()> {
        let open = self.picking.take().map(|p| p.on);
        if open != Some(select) {
            let current = self.language(select);
            self.picking = Some(match select {
                Select::Interface => {
                    Picking::among(select, current, |code| Language::from_code(code).is_some())
                }
                Select::Translate(_) => Picking::new(select, current),
            });
        }
        self.focus = Some(Stop::Select(select));
        self.relayout()?;
        let Some(view) = &self.view else {
            return Ok(());
        };
        let rect = match select {
            // The box, and the list under it.
            Select::Interface => Rect {
                x: view.interface.at.X,
                y: view.interface.at.Y,
                w: (INTERFACE_W * self.scale).round(),
                h: (self.layout_interface()?.0.height * self.scale).ceil(),
            },
            Select::Translate(_) => view.translate_at,
        };
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
        self.show_change(Self::relayout);
        Ok(Some(self.languages_changed()))
    }

    /// Makes `code` the language of the box being picked in, closing the
    /// list. For a translation, the other side's language swaps them round.
    pub(super) fn pick(&mut self, code: &str) -> Result<Option<Change>> {
        let Some(select) = self.picking.take().map(|p| p.on) else {
            return Ok(None);
        };
        self.focus = Some(Stop::Select(select));
        let config = &mut self.values.config;
        let side = match select {
            Select::Translate(side) => side,
            Select::Interface => {
                let language = Language::from_code(code).filter(|&l| l != config.ui_language);
                if let Some(language) = language {
                    config.ui_language = language;
                }
                self.show_change(Self::relayout);
                return Ok(language.map(Change::Interface));
            }
        };
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
        self.show_change(Self::relayout);
        Ok(changed.then(|| self.languages_changed()))
    }

    /// Narrows the list of languages down with a character typed.
    pub(super) fn type_language(&mut self, unit: u16) -> Result<Option<Change>> {
        let hwnd = self.hwnd;
        let Some(select) = self.picking.as_ref().map(|p| p.on) else {
            return Ok(None);
        };
        let current = self.language(select).to_owned();
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
        let Some(select) = self.picking.as_ref().map(|p| p.on) else {
            return Ok(None);
        };
        let current = self.language(select).to_owned();
        let Some(picking) = self.picking.as_mut() else {
            return Ok(None);
        };
        match key {
            VK_ESCAPE => {
                self.picking = None;
                self.focus = Some(Stop::Select(select));
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
        if self.email.text() == before {
            self.show_field()?;
            return Ok(None);
        }
        let email = self.email.text().trim().to_owned();
        self.values.config.mymemory_email = email.clone();
        self.show_change(Self::show_field);
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
