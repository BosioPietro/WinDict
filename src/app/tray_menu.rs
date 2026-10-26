//! The tray icon and the menu it opens.

use super::*;

impl App {
    // -----------------------------------------------------------------------
    // Tray

    /// A button went down on the tray icon. If that's what closes the menu
    /// (the click-away dismissal gets there first), the release that follows
    /// must not open it again.
    pub(super) fn on_tray_press(&mut self) {
        let closing = self.menu.as_ref().is_some_and(|m| m.visible)
            || win::now_ms().saturating_sub(self.menu_closed_at) < MENU_REOPEN_GUARD_MS;
        self.swallow_tray_release = closing;
    }

    /// A click on the tray icon, or a key on it, `at` the pointer or the icon.
    pub(super) fn on_tray_release(&mut self, at: POINT) {
        if std::mem::take(&mut self.swallow_tray_release) {
            return;
        }
        self.open_menu(at);
    }

    /// Opens the menu `at` a point, with the keyboard, so the arrows and
    /// Enter work in it.
    fn open_menu(&mut self, at: POINT) {
        if self.menu.as_ref().is_some_and(|m| m.visible) {
            return;
        }
        self.dismiss();
        let (theme, hotkey) = (self.theme(), self.config.hotkey.clone());
        let update = self.update.is_some().then_some(if self.updating {
            tr().menu_updating
        } else {
            tr().menu_update
        });
        let Some(menu) = self.ensure_menu() else {
            return;
        };
        let autostart = autostart::is_enabled();
        if let Err(e) = menu.show(at, theme, &hotkey, autostart, update) {
            self.tray.notify(tr().draw_menu_failed, &e.message());
            return;
        }
        selection::take_foreground(menu.hwnd);
        CARD_RECT.set(menu.card_rect());
        win::kill_timer(self.hwnd, TIMER_MENU_HIDE);
        win::kill_timer(self.hwnd, TIMER_MENU_TEARDOWN);
        SHOWN_AT.set(win::now_ms());
        self.install_dismiss_hooks();
    }

    /// Starts closing the tray menu, if it's open.
    pub(super) fn dismiss_menu(&mut self) {
        let Some(menu) = self.menu.as_mut().filter(|m| m.visible) else {
            return;
        };
        let _ = menu.begin_hide();
        self.menu_closed_at = win::now_ms();
        win::set_timer(self.hwnd, TIMER_MENU_HIDE, HIDE_DONE_MS);
    }

    pub(super) fn menu_hover(&mut self, point: Option<(i32, i32)>) {
        if let Some(menu) = self.menu.as_mut().filter(|m| m.visible) {
            let _ = menu.hover(point);
        }
    }

    pub(super) fn menu_click(&mut self, x: i32, y: i32) {
        let command = self
            .menu
            .as_ref()
            .filter(|m| m.visible)
            .and_then(|m| m.command_at(x, y));
        if let Some(command) = command {
            self.run_menu_command(command);
        }
    }

    pub(super) fn menu_key(&mut self, key: VIRTUAL_KEY) {
        let Some(menu) = self.menu.as_mut().filter(|m| m.visible) else {
            return;
        };
        let _ = match key {
            VK_DOWN => menu.step(true),
            VK_UP => menu.step(false),
            VK_HOME => menu.jump(false),
            VK_END => menu.jump(true),
            VK_RETURN | VK_SPACE => {
                if let Some(command) = menu.selected_command() {
                    self.run_menu_command(command);
                }
                return;
            }
            VK_ESCAPE => return self.dismiss(),
            _ => return,
        };
    }

    pub(super) fn run_menu_command(&mut self, command: Command) {
        self.dismiss();
        self.on_command(command);
    }
}
