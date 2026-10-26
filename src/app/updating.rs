//! Looking for a new version once a day, and installing it from the tray menu.

use super::*;

/// The first check, this long after start: at sign-in the network may not
/// be up yet.
pub(super) const FIRST_CHECK_MS: u32 = 60 * 1000;
/// Once the setting is turned on.
pub(super) const SOON_MS: u32 = 2000;
const CHECK_EVERY_MS: u32 = 24 * 60 * 60 * 1000;
/// After a check that failed, offline say.
const RETRY_MS: u32 = 60 * 60 * 1000;

impl App {
    /// Looks for an update in `ms`, if the setting says to; if it doesn't,
    /// forgets the one found.
    pub(super) fn schedule_update_check(&mut self, ms: u32) {
        if self.config.check_updates {
            win::set_timer(self.hwnd, TIMER_UPDATE_CHECK, ms);
        } else {
            win::kill_timer(self.hwnd, TIMER_UPDATE_CHECK);
            if !self.updating {
                self.set_update(None);
            }
        }
    }

    pub(super) fn check_for_update(&mut self) {
        if !self.config.check_updates {
            return;
        }
        // Installing one already: if that fails, the checks go on.
        if self.updating {
            return self.schedule_update_check(RETRY_MS);
        }
        let events = self.events.clone();
        std::thread::spawn(move || events.send(Event::UpdateChecked(update::check())));
    }

    pub(super) fn on_update_checked(&mut self, result: Result<Option<Release>, String>) {
        if !self.config.check_updates {
            return;
        }
        if self.updating {
            return self.schedule_update_check(RETRY_MS);
        }
        let next = if result.is_ok() {
            CHECK_EVERY_MS
        } else {
            RETRY_MS
        };
        win::set_timer(self.hwnd, TIMER_UPDATE_CHECK, next);
        if let Ok(release) = result {
            self.set_update(release);
        }
    }

    /// Shows on the tray icon whether there's an update.
    fn set_update(&mut self, release: Option<Release>) {
        if self.update == release {
            return;
        }
        self.update = release;
        self.sync_badge();
        self.sync_tip();
    }

    /// The tray icon's dot, in the accent, if there's an update.
    pub(super) fn sync_badge(&mut self) {
        let accent = self.theme().accent;
        self.tray.set_badge(
            self.update
                .is_some()
                .then_some((accent.R, accent.G, accent.B)),
        );
    }

    /// Over the tray icon: the shortcut, and the update if there's one.
    pub(super) fn sync_tip(&mut self) {
        let mut tip = format!("WinDict - {}", self.config.hotkey);
        if let Some(release) = &self.update {
            tip.push('\n');
            tip += &fill(tr().update_available, &[("version", &release.tag)]);
        }
        self.tray.set_tip(&tip);
    }

    /// "Update app": downloads the new version and starts it, or has it
    /// installed.
    pub(super) fn start_update(&mut self) {
        let Some(release) = self.update.clone().filter(|_| !self.updating) else {
            return;
        };
        self.updating = true;
        let events = self.events.clone();
        std::thread::spawn(move || events.send(Event::Updated(update::install(&release))));
    }

    pub(super) fn on_updated(&mut self, result: Result<(), String>) {
        self.updating = false;
        // Should the update not happen after all, it's looked for again.
        self.schedule_update_check(RETRY_MS);
        match result {
            // The new version has started, and waits for this one to go.
            Ok(()) if !update::by_task() => self.on_command(Command::Quit),
            // The task closes this one once it's ready to replace it,
            // which takes a while: say it's coming.
            Ok(()) => {
                if let Some(release) = &self.update {
                    self.tray.notify(
                        tr().updating,
                        &fill(
                            tr().updating_detail,
                            &[("version", &release.tag.trim_start_matches('v'))],
                        ),
                    );
                }
            }
            Err(e) => {
                self.tray.notify(tr().update_failed, &e);
                if update::by_task() {
                    win::shell_open(&HSTRING::from(update::RELEASES_PAGE));
                }
            }
        }
    }
}
