//! The tray menu: a flyout in the style of the definition card. Like the
//! card, it's a composition visual tree, so it fades and slides in, and a
//! highlight glides between the rows, all on the system compositor.

use windows::core::Result;
use windows::Win32::Foundation::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::*;
use windows_numerics::Vector3;

use crate::render::{Face, Ink, Renderer};
use crate::theme::Theme;
use crate::ui::{self, scale, v2, v3, Animator, Frame, MARGIN};
use crate::win::Window;

// Geometry, in DIPs.
const WIDTH: f32 = 204.0;
const PAD_Y: f32 = 3.0;
const ROW_H: f32 = 28.0;
const TITLE_H: f32 = 26.0;
const SEPARATOR_H: f32 = 7.0;
/// Space between the card's edge and the highlight.
const INSET: f32 = 3.0;
const TEXT_X: f32 = 14.0;
/// Distance from the card's right edge to the left of the check mark.
const CHECK_FROM_RIGHT: f32 = 26.0;
const HIGHLIGHT_RADIUS: f32 = 5.0;
const ANCHOR_GAP: f32 = 8.0;
const SCREEN_GAP: f32 = 8.0;
/// How far rows slide in from, and the card drifts in from.
const ROW_DRIFT: f32 = 6.0;
const CARD_DRIFT: f32 = 8.0;

// Durations, in ms.
const SHOW_MS: u64 = 300;
const FADE_IN_MS: u64 = 140;
const ROW_MS: u64 = 220;
const ROW_STAGGER_MS: u64 = 24;
const HIGHLIGHT_MOVE_MS: u64 = 150;

/// What a row of the menu does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Command {
    Autostart,
    Settings,
    Reload,
    Quit,
}

#[derive(Clone, Copy)]
enum Kind {
    Title,
    Separator,
    Command { id: Command, checked: bool },
}

struct Entry {
    kind: Kind,
    label: String,
}

impl Entry {
    fn height(&self) -> f32 {
        match self.kind {
            Kind::Title => TITLE_H,
            Kind::Separator => SEPARATOR_H,
            Kind::Command { .. } => ROW_H,
        }
    }

    fn command(id: Command, label: &str, checked: bool) -> Entry {
        Entry {
            kind: Kind::Command { id, checked },
            label: label.to_owned(),
        }
    }

    fn separator() -> Entry {
        Entry {
            kind: Kind::Separator,
            label: String::new(),
        }
    }
}

fn entries(hotkey: &str, autostart: bool) -> Vec<Entry> {
    vec![
        Entry {
            kind: Kind::Title,
            label: format!("WinDict  -  {hotkey}"),
        },
        Entry::separator(),
        Entry::command(Command::Autostart, "Start with Windows", autostart),
        Entry::command(Command::Settings, "Settings", false),
        Entry::command(Command::Reload, "Reload settings", false),
        Entry::separator(),
        Entry::command(Command::Quit, "Quit", false),
    ]
}

/// Where a row sits inside the card, in physical pixels.
struct Row {
    top: f32,
    height: f32,
    command: Option<Command>,
}

/// The soft rectangle behind the row under the pointer or the keyboard.
struct Highlight {
    visual: SpriteVisual,
    clip: CompositionRoundedRectangleGeometry,
    brush: CompositionColorBrush,
}

impl Highlight {
    fn new(c: &Compositor, card: &ContainerVisual) -> Result<Highlight> {
        let visual = c.CreateSpriteVisual()?;
        let clip = c.CreateRoundedRectangleGeometry()?;
        visual.SetClip(&c.CreateGeometricClipWithGeometry(&clip)?)?;
        let brush = c.CreateColorBrush()?;
        visual.SetBrush(&brush)?;
        visual.SetOpacity(0.0)?;
        card.Children()?.InsertAtTop(&visual)?;
        Ok(Highlight {
            visual,
            clip,
            brush,
        })
    }

    fn apply(&self, theme: &Theme, s: f32) -> Result<()> {
        self.brush.SetColor(theme.highlight())?;
        let size = v2((WIDTH - 2.0 * INSET) * s, (ROW_H - 4.0) * s);
        self.visual.SetSize(size)?;
        self.clip.SetSize(size)?;
        self.clip
            .SetCornerRadius(v2(HIGHLIGHT_RADIUS * s, HIGHLIGHT_RADIUS * s))
    }
}

pub struct Menu {
    pub hwnd: HWND,
    _target: DesktopWindowTarget,
    renderer: Renderer,
    anim: Animator,
    /// Everything on the window; fades and scales as one when showing and hiding.
    root: ContainerVisual,
    frame: Frame,
    highlight: Highlight,
    /// The row texts, rebuilt on every show.
    content: ContainerVisual,
    rows: Vec<Row>,
    theme: Theme,
    scale: f32,
    grow_up: bool,
    /// Window position and size, physical pixels.
    window: RECT,
    /// Card size, physical pixels.
    card: (f32, f32),
    selected: Option<usize>,
    pub visible: bool,
    /// Last, so it goes after everything shown on it.
    _window: Window,
}

impl Menu {
    pub fn new(window: Window, compositor: Compositor, theme: Theme) -> Result<Menu> {
        let hwnd = window.hwnd();
        let target = ui::attach(hwnd, &compositor)?;
        let renderer = Renderer::new(&compositor)?;
        let c = &compositor;

        let root = c.CreateContainerVisual()?;
        target.SetRoot(&root)?;
        let frame = Frame::new(c, &root, &theme)?;
        let highlight = Highlight::new(c, &frame.card)?;
        let content = c.CreateContainerVisual()?;
        frame.card.Children()?.InsertAtTop(&content)?;

        Ok(Menu {
            hwnd,
            _target: target,
            renderer,
            anim: Animator::new(compositor)?,
            root,
            frame,
            highlight,
            content,
            rows: Vec::new(),
            theme,
            scale: 1.0,
            grow_up: false,
            window: RECT::default(),
            card: (0.0, 0.0),
            selected: None,
            visible: false,
            _window: window,
        })
    }

    // -----------------------------------------------------------------------
    // Public API

    /// Opens the menu next to the pointer at `cursor` (screen pixels).
    pub fn show(
        &mut self,
        cursor: POINT,
        theme: Theme,
        hotkey: &str,
        autostart: bool,
    ) -> Result<()> {
        let (work, s) = ui::monitor_at(cursor);
        self.theme = theme;
        self.scale = s;
        let px = |dip: f32| (dip * s).round();

        let entries = entries(hotkey, autostart);
        self.rows.clear();
        let mut y = px(PAD_Y);
        for entry in &entries {
            let height = px(entry.height());
            self.rows.push(Row {
                top: y,
                height,
                command: match entry.kind {
                    Kind::Command { id, .. } => Some(id),
                    _ => None,
                },
            });
            y += height;
        }
        self.card = (px(WIDTH), y + px(PAD_Y));

        let size = (self.card.0 as i32, self.card.1 as i32);
        let (card, grow_up) = placement(
            cursor,
            work,
            size,
            px(ANCHOR_GAP) as i32,
            px(SCREEN_GAP) as i32,
        );
        self.grow_up = grow_up;
        let m = px(MARGIN) as i32;
        self.window = RECT {
            left: card.left - m,
            top: card.top - m,
            right: card.right + m,
            bottom: card.bottom + m,
        };

        self.frame.apply_theme(&self.renderer, &theme, s)?;
        self.frame.resize(
            &self.anim,
            self.card.0,
            self.card.1,
            v3(m as f32, m as f32),
            s,
            false,
        )?;
        self.highlight.apply(&theme, s)?;
        self.selected = None;
        Animator::stop(&self.highlight.visual, &["Offset", "Opacity"]);
        self.highlight.visual.SetOpacity(0.0)?;

        self.content.Children()?.RemoveAll()?;
        let mut reveal = Vec::new();
        for (entry, row) in entries.iter().zip(&self.rows) {
            reveal.push(self.add_row(entry, row)?);
        }

        // Start from the corner nearest the pointer, small and a little off.
        Animator::stop(&self.root, &["Opacity", "Offset", "Scale"]);
        let pivot_x = (cursor.x - card.left).clamp(0, size.0) as f32;
        self.root.SetCenterPoint(Vector3 {
            X: m as f32 + pivot_x,
            Y: if grow_up {
                m as f32 + self.card.1
            } else {
                m as f32
            },
            Z: 0.0,
        })?;
        self.root.SetOpacity(0.0)?;
        self.root.SetScale(scale(0.94))?;
        let drift = CARD_DRIFT * s;
        self.root
            .SetOffset(v3(0.0, if grow_up { drift } else { -drift }))?;

        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                self.window.left,
                self.window.top,
                self.window.right - self.window.left,
                self.window.bottom - self.window.top,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }
        self.visible = true;

        self.anim.fade_in(&self.root, FADE_IN_MS, SHOW_MS)?;
        self.reveal_rows(&reveal)
    }

    /// Fades the menu out. The caller hides the window after [`ui::HIDE_MS`].
    pub fn begin_hide(&mut self) -> Result<()> {
        if !self.visible {
            return Ok(());
        }
        self.visible = false;
        let dy = if self.grow_up { 6.0 } else { -6.0 } * self.scale;
        self.anim.fade_out(&self.root, dy)
    }

    pub fn finish_hide(&mut self) {
        if self.visible {
            return;
        }
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        if let Ok(children) = self.content.Children() {
            let _ = children.RemoveAll();
        }
        self.renderer.trim();
    }

    /// The card's bounds in screen pixels.
    pub fn card_rect(&self) -> RECT {
        let m = (MARGIN * self.scale).round() as i32;
        RECT {
            left: self.window.left + m,
            top: self.window.top + m,
            right: self.window.right - m,
            bottom: self.window.bottom - m,
        }
    }

    /// Highlights the command under a point in window pixels. Over the
    /// heading or a separator the highlight stays put, so it glides across
    /// them to the next command; it only goes when the pointer leaves the card.
    pub fn hover(&mut self, point: Option<(i32, i32)>) -> Result<()> {
        match point {
            Some((x, y)) => match self.command_row_at(x, y) {
                Some(row) => self.select(Some(row)),
                None if self.in_card(x, y) => Ok(()),
                None => self.select(None),
            },
            None => self.select(None),
        }
    }

    /// The command under a point in window pixels, if any.
    pub fn command_at(&self, x: i32, y: i32) -> Option<Command> {
        self.command_row_at(x, y).and_then(|i| self.rows[i].command)
    }

    pub fn selected_command(&self) -> Option<Command> {
        self.selected.and_then(|i| self.rows[i].command)
    }

    /// Moves the keyboard selection to the next (or previous) command, wrapping.
    pub fn step(&mut self, forward: bool) -> Result<()> {
        let commands: Vec<usize> = (0..self.rows.len())
            .filter(|&i| self.rows[i].command.is_some())
            .collect();
        let at = self
            .selected
            .and_then(|s| commands.iter().position(|&c| c == s));
        let next = match (at, forward) {
            (None, true) => commands.first(),
            (None, false) => commands.last(),
            (Some(i), true) => commands.get((i + 1) % commands.len()),
            (Some(i), false) => commands.get((i + commands.len() - 1) % commands.len()),
        };
        self.select(next.copied())
    }

    /// Moves the keyboard selection to the first or last command.
    pub fn jump(&mut self, last: bool) -> Result<()> {
        let mut commands = (0..self.rows.len()).filter(|&i| self.rows[i].command.is_some());
        let row = if last {
            commands.next_back()
        } else {
            commands.next()
        };
        self.select(row)
    }

    // -----------------------------------------------------------------------
    // Internals

    /// A point in window pixels, relative to the card's top left.
    fn card_point(&self, x: i32, y: i32) -> (f32, f32) {
        let m = (MARGIN * self.scale).round();
        (x as f32 - m, y as f32 - m)
    }

    fn in_card(&self, x: i32, y: i32) -> bool {
        let (x, y) = self.card_point(x, y);
        x >= 0.0 && x < self.card.0 && y >= 0.0 && y < self.card.1
    }

    /// The command row under a point in window pixels.
    fn command_row_at(&self, x: i32, y: i32) -> Option<usize> {
        if !self.in_card(x, y) {
            return None;
        }
        let (_, y) = self.card_point(x, y);
        self.rows
            .iter()
            .position(|r| r.command.is_some() && y >= r.top && y < r.top + r.height)
    }

    fn select(&mut self, row: Option<usize>) -> Result<()> {
        if row == self.selected {
            return Ok(());
        }
        let was_selected = self.selected.is_some();
        self.selected = row;
        let anim = &self.anim;
        let visual = &self.highlight.visual;
        let Some(row) = row else {
            return anim.scalar(visual, "Opacity", 0.0, 120, 0, &anim.accelerate);
        };
        let s = self.scale;
        let to = v3(INSET * s, self.rows[row].top + 2.0 * s);
        if was_selected {
            anim.vector3(visual, "Offset", to, HIGHLIGHT_MOVE_MS, 0, &anim.decelerate)?;
        } else {
            // Appearing: fade in where it is wanted rather than sliding from wherever it was.
            Animator::stop(visual, &["Offset"]);
            visual.SetOffset(to)?;
        }
        anim.scalar(visual, "Opacity", 1.0, 100, 0, &anim.decelerate)
    }

    /// Builds the visuals for one row at its resting place; they're returned
    /// so [`Self::reveal_rows`] can animate them in.
    fn add_row(&self, entry: &Entry, row: &Row) -> Result<Vec<SpriteVisual>> {
        let s = self.scale;
        let mut visuals = Vec::new();
        match entry.kind {
            Kind::Separator => {
                let line = self.anim.compositor.CreateSpriteVisual()?;
                let color = self.theme.menu_separator();
                line.SetBrush(&self.anim.compositor.CreateColorBrushWithColor(color)?)?;
                let thickness = s.round().max(1.0);
                line.SetSize(v2(self.card.0 - 2.0 * INSET * s, thickness))?;
                line.SetOffset(v3(
                    INSET * s,
                    row.top + ((row.height - thickness) / 2.0).floor(),
                ))?;
                self.content.Children()?.InsertAtTop(&line)?;
                visuals.push(line);
            }
            Kind::Title => {
                let width = WIDTH - 2.0 * TEXT_X;
                visuals.push(self.text(
                    &entry.label,
                    Face::Small,
                    Ink::Secondary,
                    width,
                    TEXT_X,
                    row,
                )?);
            }
            Kind::Command { checked, .. } => {
                let width = WIDTH - TEXT_X - CHECK_FROM_RIGHT;
                visuals.push(self.text(
                    &entry.label,
                    Face::Menu,
                    Ink::Primary,
                    width,
                    TEXT_X,
                    row,
                )?);
                if checked {
                    visuals.push(self.text(
                        "\u{E73E}",
                        Face::Icon,
                        Ink::Primary,
                        16.0,
                        WIDTH - CHECK_FROM_RIGHT,
                        row,
                    )?);
                }
            }
        }
        Ok(visuals)
    }

    /// A line of text, vertically centred in `row`, `x` DIPs from the card's left.
    fn text(
        &self,
        text: &str,
        face: Face,
        ink: Ink,
        width: f32,
        x: f32,
        row: &Row,
    ) -> Result<SpriteVisual> {
        let s = self.scale;
        let layout = self.renderer.layout_line(text, face, ink, width)?;
        let surface = self.renderer.draw(&layout, s, &self.theme)?;
        let (sprite, brush) = ui::text_sprite(&self.anim.compositor)?;
        brush.SetSurface(&surface)?;
        let (w, h) = ((layout.width * s).ceil(), (layout.height * s).ceil());
        sprite.SetSize(v2(w, h))?;
        sprite.SetOffset(v3(x * s, row.top + ((row.height - h) / 2.0).round()))?;
        self.content.Children()?.InsertAtTop(&sprite)?;
        Ok(sprite)
    }

    /// Each row fades in and settles upwards, a beat after the one above it.
    fn reveal_rows(&self, rows: &[Vec<SpriteVisual>]) -> Result<()> {
        let anim = &self.anim;
        let drift = ROW_DRIFT * self.scale;
        for (i, visuals) in rows.iter().enumerate() {
            let delay = 40 + i as u64 * ROW_STAGGER_MS;
            for visual in visuals {
                let rest = visual.Offset()?;
                visual.SetOpacity(0.0)?;
                visual.SetOffset(Vector3 {
                    X: rest.X,
                    Y: rest.Y + if self.grow_up { drift } else { -drift },
                    Z: 0.0,
                })?;
                anim.scalar(visual, "Opacity", 1.0, ROW_MS, delay, &anim.decelerate)?;
                anim.vector3(visual, "Offset", rest, ROW_MS, delay, &anim.decelerate)?;
            }
        }
        Ok(())
    }
}

/// Where a `size` card goes when opened at `cursor` inside the monitor's
/// `work` area, all in screen pixels, and whether it opens upwards. Like a
/// native menu it's right-aligned to the pointer, above it when there's room
/// (the tray is usually at the bottom) and below it otherwise.
fn placement(cursor: POINT, work: RECT, size: (i32, i32), gap: i32, edge: i32) -> (RECT, bool) {
    let (w, h) = size;
    let left = (cursor.x - w).clamp(
        work.left + edge,
        (work.right - edge - w).max(work.left + edge),
    );
    let bottom = cursor.y.min(work.bottom) - gap;
    let (top, grow_up) = if bottom - h >= work.top + edge {
        (bottom - h, true)
    } else {
        let top = cursor.y.max(work.top) + gap;
        let top = top.min(work.bottom - edge - h).max(work.top + edge);
        (top, false)
    };
    (
        RECT {
            left,
            top,
            right: left + w,
            bottom: top + h,
        },
        grow_up,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: RECT = RECT {
        left: 0,
        top: 0,
        right: 1920,
        bottom: 1032,
    };
    const SIZE: (i32, i32) = (236, 200);

    fn at(x: i32, y: i32) -> POINT {
        POINT { x, y }
    }

    #[test]
    fn opens_above_a_taskbar_icon() {
        // The pointer is on the taskbar, below the work area.
        let (card, grow_up) = placement(at(1850, 1060), SCREEN, SIZE, 8, 8);
        assert!(grow_up);
        assert_eq!(card.bottom, SCREEN.bottom - 8);
        assert_eq!(card.right, 1850);
    }

    #[test]
    fn opens_below_a_top_taskbar_icon() {
        let work = RECT { top: 48, ..SCREEN };
        let (card, grow_up) = placement(at(1850, 20), work, SIZE, 8, 8);
        assert!(!grow_up);
        assert_eq!(card.top, work.top + 8);
    }

    #[test]
    fn stays_on_screen() {
        for (x, y) in [(0, 0), (1919, 0), (0, 1060), (1919, 1060), (960, 500)] {
            let (card, _) = placement(at(x, y), SCREEN, SIZE, 8, 8);
            assert!(card.left >= SCREEN.left + 8, "{x},{y}");
            assert!(card.right <= SCREEN.right - 8, "{x},{y}");
            assert!(card.top >= SCREEN.top + 8, "{x},{y}");
            assert!(card.bottom <= SCREEN.bottom - 8, "{x},{y}");
        }
    }
}
