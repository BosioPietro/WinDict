//! Selecting the body's text with the mouse, to copy it.

use windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;

use super::*;

/// The left button, down on the body's text.
pub(super) struct Selecting {
    /// Where the selection begins.
    anchor: TextPoint,
    /// Where the button went down, in window pixels.
    from: (i32, i32),
    /// Whether the pointer has moved far enough to select, rather than
    /// click: a click still follows a link.
    dragged: bool,
}

/// When and where the left button last went down on the body, to tell a
/// double click.
#[derive(Clone, Copy)]
pub(super) struct Press {
    time: u32,
    at: (i32, i32),
}

impl Popup {
    /// The left button went down at a point in window pixels: on the body,
    /// it starts selecting from the text nearest it, and a second time in
    /// a row selects the word there. What was selected goes. Returns
    /// whether it was on the body's text.
    pub(super) fn press_text(&mut self, x: i32, y: i32) -> Result<bool> {
        let time = unsafe { GetMessageTime() } as u32;
        let last = self.last_press.replace(Press { time, at: (x, y) });
        let double = last.is_some_and(|p| {
            let near = |d: i32, metric| d.abs() <= unsafe { GetSystemMetrics(metric) } / 2;
            time.wrapping_sub(p.time) <= unsafe { GetDoubleClickTime() }
                && near(x - p.at.0, SM_CXDOUBLECLK)
                && near(y - p.at.1, SM_CYDOUBLECLK)
        });
        let point = self.body_point(x, y).and_then(|(bx, by)| {
            let s = self.scale;
            self.shown.as_ref()?.body.text_point(bx / s, by / s)
        });
        let Some(point) = point else {
            self.select(None)?;
            return Ok(false);
        };
        let (anchor, selection) = if double {
            // A third press starts afresh.
            self.last_press = None;
            let (from, to) = self
                .shown
                .as_ref()
                .map_or((point, point), |s| s.body.word_at(point));
            (from, Some((from, to)))
        } else {
            (point, None)
        };
        self.selecting = Some(Selecting {
            anchor,
            from: (x, y),
            dragged: double,
        });
        self.select(selection)?;
        Ok(true)
    }

    /// The pointer at a point in window pixels with the button down on the
    /// text: selects from where it went down to the text nearest the
    /// pointer, once it's moved far enough not to be a click.
    pub(super) fn drag_text(&mut self, x: i32, y: i32) -> Result<()> {
        let Some(selecting) = self.selecting.as_mut() else {
            return Ok(());
        };
        if !selecting.dragged {
            let far = |d: i32, metric| d.abs() > unsafe { GetSystemMetrics(metric) };
            let (dx, dy) = (x - selecting.from.0, y - selecting.from.1);
            if !far(dx, SM_CXDRAG) && !far(dy, SM_CYDRAG) {
                return Ok(());
            }
            selecting.dragged = true;
        }
        let anchor = selecting.anchor;
        // Off the body, it's the text at its top or bottom edge.
        let s = self.scale;
        let (x, y) = (x as f32 - MARGIN * s, y as f32 - self.frame_y());
        let y = y.clamp(self.header_h, (self.card_h - 1.0).max(self.header_h));
        let (bx, by) = (x - PAD_X * s, y - self.header_h + self.scroller.at);
        let point = (self.shown.as_ref()).and_then(|shown| shown.body.text_point(bx / s, by / s));
        match point {
            Some(point) => self.select(Some((anchor, point))),
            None => Ok(()),
        }
    }

    /// The left button came up. Returns whether it had gone down on the
    /// text, and if so whether that was a click rather than a selection.
    pub(super) fn release_text(&mut self) -> Option<bool> {
        self.selecting.take().map(|s| !s.dragged)
    }

    /// Selects the text between two places in the body, or nothing.
    pub(super) fn select(&mut self, selection: Option<(TextPoint, TextPoint)>) -> Result<()> {
        let body = self.shown.as_ref().map(|s| &s.body);
        (self.content).select(&self.anim.compositor, selection, body, self.scale)
    }

    /// What's selected of the body's text, if anything is.
    pub fn selected_text(&self) -> Option<String> {
        let (a, b) = self.content.selection()?;
        let text = self.shown.as_ref()?.body.selected_text(a, b);
        (!text.trim().is_empty()).then_some(text)
    }

    /// Whether a point in window pixels is on text that can be selected.
    pub fn text_at(&self, x: i32, y: i32) -> bool {
        let Some((bx, by)) = self.body_point(x, y) else {
            return false;
        };
        let s = self.scale;
        (self.shown.as_ref()).is_some_and(|shown| shown.body.over_text(bx / s, by / s))
    }
}
