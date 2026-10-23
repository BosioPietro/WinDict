//! The settings window, the changes made in it, and downloading the dictionaries.

use super::*;

impl App {
    // -----------------------------------------------------------------------
    // Settings window

    /// What the settings window shows: the settings, with the languages
    /// being downloaded for them ticked already.
    pub(super) fn settings_values(&self) -> Values {
        let mut config = self.config.clone();
        for installing in self.installing.iter().filter(|i| i.wanted && !i.quiet) {
            let language = installing.download.language;
            if !config.languages.iter().any(|l| l == language) {
                config.languages.push(language.into());
            }
        }
        config.languages.sort_by_key(|l| dicts::rank(l));
        Values {
            config,
            autostart: autostart::is_enabled(),
        }
    }

    /// Opens the settings window, or brings it back to the front.
    pub(super) fn open_settings(&mut self) {
        if let Some(hwnd) = self.settings.as_ref().filter(|s| s.visible).map(|s| s.hwnd) {
            unsafe {
                if IsIconic(hwnd).as_bool() {
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                }
            }
            selection::take_foreground(hwnd);
            return;
        }
        self.dismiss();
        win::kill_timer(self.hwnd, TIMER_SETTINGS_HIDE);
        let (theme, values) = (self.theme(), self.settings_values());
        let Some(settings) = self.ensure_settings(theme, values.clone()) else {
            return;
        };
        if let Err(e) = settings.show(win::cursor_pos(), theme, values) {
            self.tray.notify(tr().draw_settings_failed, &e.message());
            return;
        }
        selection::take_foreground(settings.hwnd);
        self.sync_recording();
    }

    pub(super) fn close_settings(&mut self) {
        let Some(settings) = self.settings.as_mut().filter(|s| s.visible) else {
            return;
        };
        let recording = settings.recording();
        let _ = settings.begin_hide();
        self.sync_recording();
        // Recording had put the shortcuts aside.
        if recording {
            self.register_hotkeys();
        }
        win::set_timer(self.hwnd, TIMER_SETTINGS_HIDE, HIDE_DONE_MS);
    }

    /// The settings changed from elsewhere (the tray menu, or the file): the
    /// window shows them as they are now.
    pub(super) fn refresh_settings(&mut self) {
        let (theme, values) = (self.theme(), self.settings_values());
        if let Some(settings) = self.settings.as_mut().filter(|s| s.visible) {
            let _ = settings.refresh(theme, values);
        }
        self.sync_recording();
    }

    /// Runs `f` on the open settings window, and carries out what it asks for.
    pub(super) fn in_settings(
        &mut self,
        f: impl FnOnce(&mut Settings) -> windows::core::Result<Option<Change>>,
    ) {
        let change = self
            .settings
            .as_mut()
            .filter(|s| s.visible)
            .and_then(|s| f(s).ok().flatten());
        if let Some(change) = change {
            self.on_setting(change);
        }
        self.sync_recording();
    }

    /// Lets the keyboard hook know whether a shortcut is being recorded,
    /// setting it up the first time one is.
    pub(super) fn sync_recording(&mut self) {
        let recording = self
            .settings
            .as_ref()
            .filter(|s| s.visible && s.recording());
        let hwnd = recording.map_or(HWND(std::ptr::null_mut()), |s| s.hwnd);
        RECORDING_IN.set(hwnd);
        if recording.is_some() && self.keyboard_hook.is_none() {
            self.keyboard_hook = win::Hook::low_level(WH_KEYBOARD_LL, Some(keyboard_hook));
        }
    }

    /// Carries out a change made in the settings window, and saves it.
    pub(super) fn on_setting(&mut self, change: Change) {
        match change {
            Change::Close => return self.close_settings(),
            Change::OpenConfig => return open_config_file(),
            Change::Autostart(on) => return autostart::set(on),
            // No shortcut may fire as it's pressed to be kept, the one being
            // replaced or the other.
            Change::RecordingStarted => return self.unregister_hotkeys(),
            Change::RecordingStopped => return self.register_hotkeys(),
            Change::Hotkey(shortcut, hotkey) => {
                let other = SHORTCUTS.into_iter().find(|&s| s != shortcut);
                let clash = other.filter(|&o| same_keys(self.config.hotkey(o), &hotkey));
                let previous = std::mem::replace(self.config.hotkey_mut(shortcut), hotkey);
                let bound = match clash {
                    Some(_) => Err(Unbound::Clash),
                    None => self.bind_hotkey(shortcut),
                };
                if let Err(unbound) = bound {
                    *self.config.hotkey_mut(shortcut) = previous;
                    self.register_hotkeys();
                    let why = match &unbound {
                        Unbound::Taken => tr().shortcut_taken,
                        Unbound::Clash => tr().shortcut_clash,
                        Unbound::Invalid(e) => e,
                    };
                    let hotkey = self.config.hotkey(shortcut).to_owned();
                    if let Some(settings) = self.settings.as_mut() {
                        let _ = settings.reject_hotkey(shortcut, &hotkey, why);
                    }
                    return;
                }
                // The other one was put aside while this was recorded.
                if let Some(other) = other {
                    self.register_hotkey(other);
                }
            }
            Change::Theme(mode) => {
                self.config.theme = mode;
                let theme = self.theme();
                if let Some(settings) = self.settings.as_mut() {
                    let _ = settings.retheme(theme);
                }
            }
            Change::Interface(language) => {
                self.config.ui_language = language;
                i18n::set(language);
                if let Some(settings) = self.settings.as_mut() {
                    let _ = settings.relanguage();
                }
            }
            Change::Language(language, on) => return self.tick_language(language, on),
            Change::MaxDefinitions(n) => self.config.max_definitions = n,
            Change::MaxSynonyms(n) => self.config.max_synonyms = n,
            Change::ShowExamples(on) => self.config.show_examples = on,
            Change::OnlineLookup(on) => {
                self.config.online_lookup = on;
                // The translate button needs the internet, like the answers.
                self.each_card(|app| {
                    let translate = app.can_translate();
                    if let Some(popup) = app.card.popup.as_mut() {
                        let _ = popup.set_translate(translate);
                    }
                    app.sync_link_keys();
                });
            }
            Change::Selection(mode) => self.config.selection = mode,
            Change::CheckUpdates(on) => {
                self.config.check_updates = on;
                self.schedule_update_check(updating::SOON_MS);
                return self.save_setting(|_| {});
            }
            Change::Zoom(zoom) => return self.zoom_to(zoom),
            Change::Animations(on) => {
                self.config.animations = Some(on);
                ui::set_motion(on);
                return self.save_setting(|_| {});
            }
            // These don't change any answer: there's nothing to forget.
            Change::TranslateLanguages(from, to) => {
                return self.save_setting(|c| (c.translate_from, c.translate_to) = (from, to));
            }
            Change::MyMemoryEmail(email) => {
                return self.save_setting(|c| c.mymemory_email = email);
            }
        }
        // Online answers were cut down to the old limits, examples and all.
        self.cache.clear();
        self.save_setting(|_| {});
    }

    /// Changes the config with `change`, and saves it.
    pub(super) fn save_setting(&mut self, change: impl FnOnce(&mut Config)) {
        change(&mut self.config);
        if let Err(e) = config::save(&self.config) {
            self.tray.notify(tr().settings_not_saved, &e);
        }
    }

    /// Looks words up in `language` too from now on, or no longer: English
    /// at once, another once its dictionary is downloaded, if it isn't
    /// already. The last language left stays.
    pub(super) fn tick_language(&mut self, language: LangCode, on: bool) {
        if on {
            if let Some(download) = dicts::find(&language) {
                return self.install(download, false);
            }
            if !self.config.languages.contains(&language) {
                self.config.languages.push(language);
            }
        } else {
            for installing in self.installing.iter_mut() {
                if language == installing.download.language {
                    installing.wanted = false;
                }
            }
            self.show_download(&language, None);
            if self.config.languages.len() > 1 {
                self.config.languages.retain(|l| *l != language);
            }
        }
        self.use_languages();
    }

    /// Makes sure `download` is in place, fetching it if it isn't, then
    /// looks words up in its language too. Asked for again while it
    /// downloads, the download under way carries on.
    pub(super) fn install(&mut self, download: &'static dicts::Download, quiet: bool) {
        let language = download.language;
        if let Some(installing) = self
            .installing
            .iter_mut()
            .find(|i| i.download.language == language)
        {
            installing.wanted = true;
            installing.quiet &= quiet;
            return;
        }
        self.installing.push(Installing {
            download,
            quiet,
            wanted: true,
        });
        let events = self.events.clone();
        std::thread::spawn(move || {
            let result = dicts::install(download, |percent| {
                events.send(Event::Installing(language, percent));
            });
            events.send(Event::Installed(language, result));
        });
    }

    pub(super) fn on_installing(&mut self, language: &str, percent: u32) {
        let shown = self
            .installing
            .iter()
            .any(|i| i.download.language == language && i.wanted && !i.quiet);
        if !shown {
            return;
        }
        let note = fill(tr().downloading, &[("percent", &percent)]);
        self.show_download(language, Some((&note, false)));
    }

    pub(super) fn on_installed(&mut self, language: &str, result: Result<(), String>) {
        let Some(at) = self
            .installing
            .iter()
            .position(|i| i.download.language == language)
        else {
            return;
        };
        let installing = self.installing.remove(at);
        if !installing.wanted {
            return;
        }
        let download = installing.download;
        match result {
            Ok(()) => {
                self.show_download(language, None);
                let used = offline::languages().iter().any(|l| l == language);
                if !self.config.languages.iter().any(|l| l == language) {
                    self.config.languages.push(language.into());
                } else if used {
                    // Already in use, as it was at start.
                    return;
                }
                self.use_languages();
            }
            // At start, the one there is (if any) is used as it is.
            Err(e) if installing.quiet => {
                if !offline::languages().iter().any(|l| l == language) {
                    let title = fill(tr().not_downloaded, &[("language", &download.name)]);
                    self.tray.notify(&title, &e);
                }
            }
            Err(e) => {
                self.show_languages();
                self.show_download(language, Some((&e, true)));
            }
        }
    }

    /// Looks words up in the languages of the setting, in order of
    /// preference, and saves it.
    pub(super) fn use_languages(&mut self) {
        self.config.languages.sort_by_key(|l| dicts::rank(l));
        if let Err(e) = offline::set_languages(&self.config.languages) {
            self.tray.notify(tr().dictionary_not_loaded, &e);
        }
        // Online answers are for the languages there were.
        self.cache.clear();
        if let Err(e) = config::save(&self.config) {
            self.tray.notify(tr().settings_not_saved, &e);
        }
        self.show_languages();
    }

    /// Ticks the languages used, and those being downloaded, in the
    /// settings window.
    pub(super) fn show_languages(&mut self) {
        let languages = self.settings_values().config.languages;
        if let Some(settings) = self.settings.as_mut().filter(|s| s.visible) {
            let _ = settings.show_languages(&languages);
        }
    }

    /// Says how `language`'s download is going in the settings window.
    pub(super) fn show_download(&mut self, language: &str, status: Option<(&str, bool)>) {
        if let Some(settings) = self.settings.as_mut().filter(|s| s.visible) {
            let _ = settings.show_download(language, status);
        }
    }

    /// Windows turned its animations on or off: so do these, unless set here.
    pub(super) fn on_motion_changed(&mut self) {
        ui::set_motion(self.config.animated());
        self.refresh_settings();
    }

    /// The system's colors changed: the settings window and the
    /// introduction follow them.
    pub(super) fn on_colors_changed(&mut self) {
        let theme = self.theme();
        if let Some(settings) = self.settings.as_mut().filter(|s| s.visible) {
            let _ = settings.retheme(theme);
        }
        if let Some(intro) = self.intro.as_mut().filter(|i| i.visible) {
            let _ = intro.retheme(theme);
        }
    }
}
