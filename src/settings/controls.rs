//! The state of the switches, choices, counters and the shortcut being recorded.

use super::*;

impl Settings {
    /// Whether the language at `i` in [`dicts::languages`] is ticked.
    pub(super) fn ticked(&self, i: usize) -> bool {
        let Some((code, _)) = dicts::languages().nth(i) else {
            return false;
        };
        self.values.config.languages.iter().any(|l| l == code)
    }

    /// Ticks the language at `i`, or unticks it, unless it's the only one.
    pub(super) fn tick(&mut self, i: usize) -> Result<Option<Change>> {
        let Some((code, _)) = dicts::languages().nth(i) else {
            return Ok(None);
        };
        let on = !self.ticked(i);
        let languages = &mut self.values.config.languages;
        if on {
            // In the order of the list, which is the order of preference.
            languages.push(code.into());
            languages.sort_by_key(|l| dicts::rank(l));
        } else if languages.len() > 1 {
            languages.retain(|l| l != code);
        } else {
            // Words have to be looked up in something.
            if let Some(view) = &self.view {
                let item = &view.languages.items[i].on;
                self.anim.shake(item, item.Offset()?, self.scale * 0.5)?;
            }
            return Ok(None);
        }
        self.show_checks(true)?;
        let text = summary(&self.values.config.languages);
        self.set_note(Note::Language, &text, false)?;
        Ok(Some(Change::Language(code.into(), on)))
    }

    pub(super) fn switch(&self, sw: Switch) -> bool {
        match sw {
            Switch::Autostart => self.values.autostart,
            Switch::Online => self.values.config.online_lookup,
            Switch::Examples => self.values.config.show_examples,
            Switch::Animations => self.values.config.animated(),
            Switch::Updates => self.values.config.check_updates,
        }
    }

    pub(super) fn count(&self, counter: Counter) -> usize {
        let config = &self.values.config;
        match counter {
            Counter::Definitions => config.max_definitions,
            Counter::Synonyms => config.max_synonyms,
            Counter::Zoom => config::zoom_index(config.zoom),
        }
    }

    /// Where `counter` goes counting up or down one, if it can.
    pub(super) fn next(&self, counter: Counter, up: bool) -> Option<usize> {
        let (lo, hi) = counter.range();
        let n = self.count(counter);
        let n = if up { n + 1 } else { n.checked_sub(1)? };
        (lo..=hi).contains(&n).then_some(n)
    }

    /// Which of `choice`'s options is chosen.
    pub(super) fn chosen(&self, choice: Choice) -> usize {
        let config = &self.values.config;
        match choice {
            Choice::Theme => THEMES.iter().position(|(mode, _)| *mode == config.theme),
            Choice::Language => Language::ALL.iter().position(|&l| l == config.ui_language),
            Choice::Selection => SELECTIONS
                .iter()
                .position(|(mode, _, _)| *mode == config.selection),
        }
        .unwrap_or(0)
    }

    pub(super) fn choose(&mut self, choice: Choice, i: usize) -> Result<Option<Change>> {
        if i == self.chosen(choice) {
            return Ok(None);
        }
        self.pill_from = Some((choice, self.chosen(choice)));
        let change = match choice {
            Choice::Theme => {
                let mode = THEMES[i].0;
                self.values.config.theme = mode;
                Change::Theme(mode)
            }
            Choice::Language => {
                let language = Language::ALL[i];
                self.values.config.ui_language = language;
                Change::Interface(language)
            }
            Choice::Selection => {
                let mode = SELECTIONS[i].0;
                self.values.config.selection = mode;
                self.set_note(Note::Selection, selection_detail(mode), false)?;
                Change::Selection(mode)
            }
        };
        self.show_choice(choice, true)?;
        Ok(Some(change))
    }

    /// Counts `counter` up or down one. At either end, the number shakes.
    pub(super) fn step(&mut self, counter: Counter, up: bool) -> Result<Option<Change>> {
        let Some(next) = self.next(counter, up) else {
            if let Some(view) = &self.view {
                let host = &view.counters[counter as usize].host;
                self.anim.shake(host, host.Offset()?, self.scale * 0.5)?;
            }
            return Ok(None);
        };
        let change = match counter {
            Counter::Definitions => {
                self.values.config.max_definitions = next;
                Change::MaxDefinitions(next)
            }
            Counter::Synonyms => {
                self.values.config.max_synonyms = next;
                Change::MaxSynonyms(next)
            }
            Counter::Zoom => {
                self.values.config.zoom = ZOOMS[next];
                Change::Zoom(ZOOMS[next])
            }
        };
        self.show_count(counter, Some(up))?;
        Ok(Some(change))
    }

    pub(super) fn start_recording(&mut self) -> Result<()> {
        self.recording = true;
        self.preview = held_modifiers();
        self.show_caps(false)?;
        self.set_note(Note::Shortcut, tr().recording_detail, false)?;
        self.show_recording()
    }

    /// Stops recording, showing the shortcut there is.
    pub(super) fn stop_recording(&mut self) -> Result<()> {
        self.recording = false;
        self.show_caps(true)?;
        self.set_note(Note::Shortcut, tr().shortcut_detail, false)?;
        self.show_recording()
    }

    /// A key went down while recording, leaving `modifiers` held.
    pub(super) fn record(
        &mut self,
        key: VIRTUAL_KEY,
        modifiers: HOT_KEY_MODIFIERS,
    ) -> Result<Option<Change>> {
        if is_modifier(key) {
            self.set_preview(modifiers)?;
            return Ok(None);
        }
        if key == VK_ESCAPE && modifiers.0 == 0 {
            self.stop_recording()?;
            return Ok(Some(Change::RecordingStopped));
        }
        let vk = key.0 as u32;
        // Shift alone would take a capital letter away from every app.
        let strong = modifiers.0 & (MOD_CONTROL | MOD_ALT | MOD_WIN).0 != 0;
        let complaint = if hotkey::key_name(vk).is_none() {
            Some(tr().unusable_key)
        } else if !strong && !hotkey::is_function_key(vk) {
            Some(tr().needs_modifier)
        } else {
            None
        };
        if let Some(complaint) = complaint {
            self.shake_caps()?;
            self.set_note(Note::Shortcut, complaint, true)?;
            return Ok(None);
        }
        let text = Hotkey { modifiers, vk }.to_string();
        self.values.config.hotkey = text.clone();
        self.stop_recording()?;
        Ok(Some(Change::Hotkey(text)))
    }

    pub(super) fn set_preview(&mut self, modifiers: HOT_KEY_MODIFIERS) -> Result<()> {
        if modifiers == self.preview {
            return Ok(());
        }
        self.preview = modifiers;
        self.show_caps(false)
    }
}
