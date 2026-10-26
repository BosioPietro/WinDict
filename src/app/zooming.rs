//! Zooming the card and the settings window: Ctrl+plus, Ctrl+minus and
//! Ctrl+0, Ctrl with the mouse wheel, and the setting they change.

use super::*;

impl App {
    /// Ctrl+plus or Ctrl+minus on the card: zooms `by` steps in or out;
    /// Ctrl+0, with `None`, back to the usual size.
    pub(super) fn zoom(&mut self, by: Option<isize>) {
        // The search box keeps its size until it turns into a card.
        let visible = self.visible_popup().is_some();
        if !visible || self.card.search.is_some() {
            return;
        }
        let zoom = match by {
            Some(by) => config::step_zoom(self.config.zoom, by),
            None => 100,
        };
        self.zoom_to(zoom);
    }

    /// Ctrl and the wheel on the card: a step of zoom a notch, however
    /// finely the wheel turns.
    pub(super) fn zoom_by_wheel(&mut self, delta: i32) {
        let steps = self.wheel_steps(delta);
        if steps != 0 {
            self.zoom(Some(steps));
        }
    }

    /// Ctrl and the wheel on the settings window, likewise.
    pub(super) fn zoom_settings_by_wheel(&mut self, delta: i32) {
        let steps = self.wheel_steps(delta);
        if steps != 0 {
            self.zoom_to(config::step_zoom(self.config.zoom, steps));
        }
    }

    /// The whole notches the wheel has turned by with `delta` more.
    fn wheel_steps(&mut self, delta: i32) -> isize {
        // Turning back starts afresh.
        if self.zoom_wheel.signum() == -delta.signum() {
            self.zoom_wheel = 0;
        }
        self.zoom_wheel += delta;
        let steps = self.zoom_wheel / 120;
        self.zoom_wheel -= steps * 120;
        steps as isize
    }

    /// Zooms to `zoom` percent, and saves it.
    pub(super) fn zoom_to(&mut self, zoom: u32) {
        if zoom == self.config.zoom {
            return;
        }
        self.save_setting(|c| c.zoom = zoom);
        self.apply_zoom();
    }

    /// Draws every card open, and the settings window, at the zoom of the
    /// setting; the cards that aren't take it when they next open.
    pub(super) fn apply_zoom(&mut self) {
        if let Some(settings) = self.settings.as_mut().filter(|s| s.visible) {
            let _ = settings.set_zoom(self.config.zoom);
        }
        let zoom = self.config.zoom_factor();
        self.each_card(|app| {
            let Some(popup) = app.card.popup.as_mut() else {
                return;
            };
            let _ = popup.set_zoom(zoom);
            if popup.visible {
                app.on_resized();
                app.sync_link_keys();
            }
        });
    }
}
