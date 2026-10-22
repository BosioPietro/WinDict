//! The search box the card turns into when nothing was selected: a heading
//! that says so, a text field, and the keys that act on it. Like the rest of
//! the card it's composition visuals, so its parts rise in one after another,
//! the caret glides along the text, suggestions slide in under the field, and
//! the field shakes off what can't be looked up.

use windows::core::{Interface, Result, HSTRING};
use windows::UI::Composition::*;
use windows_numerics::{Vector2, Vector3};

use crate::config::Shortcut;
use crate::i18n::tr;
use crate::popup::{CARD_W, PAD_TOP, PAD_X};
use crate::render::{Face, Ink, Renderer, QUERY_SIZE, SUGGESTION_SIZE};
use crate::theme::Theme;
use crate::ui::{scale, ts, v2, v3, Animator, Text, RESIZE_MS};

// Geometry, in DIPs.
const TITLE_GAP: f32 = 10.0;
const FIELD_H: f32 = 40.0;
const FIELD_RADIUS: f32 = 6.0;
const GLYPH_X: f32 = 8.0;
const GLYPH_W: f32 = 24.0;
/// The text's left edge in the field, past the magnifier.
const TEXT_X: f32 = 36.0;
const TEXT_PAD_RIGHT: f32 = 12.0;
const UNDERLINE_H: f32 = 2.0;
const FOOTER_GAP: f32 = 12.0;
const PAD_BOTTOM: f32 = 16.0;
const CARET_W: f32 = 1.5;
/// How far each part rises from as it appears.
const DRIFT: f32 = 6.0;
/// "Search" in Segoe Fluent Icons and Segoe MDL2 Assets.
const SEARCH_GLYPH: &str = "\u{E721}";
/// "Translate", likewise.
const TRANSLATE_GLYPH: &str = "\u{E8C1}";
/// How many suggestions fit under the field.
pub(crate) const MAX_SUGGESTIONS: usize = 5;
const LIST_GAP: f32 = 6.0;
const ROW_H: f32 = 32.0;
const ROW_RADIUS: f32 = 5.0;

// Durations, in ms.
const REVEAL_MS: u64 = 280;
const REVEAL_STAGGER_MS: u64 = 45;
const UNDERLINE_MS: u64 = 380;
const CARET_BLINK_MS: u64 = 1060;
const CARET_GLIDE_MS: u64 = 90;
const PLACEHOLDER_MS: u64 = 120;
const TITLE_MS: u64 = 200;
const LEAVE_MS: u64 = 120;
const ROW_MS: u64 = 220;
const ROW_STAGGER_MS: u64 = 25;
const HIGHLIGHT_MOVE_MS: u64 = 150;

pub(crate) struct SearchBox {
    /// Holds everything; hidden while the card shows a definition.
    visual: ContainerVisual,
    /// The heading, and the one it replaces while they cross-fade.
    title: Text,
    old_title: Text,
    field: ContainerVisual,
    field_clip: CompositionRoundedRectangleGeometry,
    fill: CompositionColorBrush,
    stroke: ShapeVisual,
    stroke_geometry: CompositionRoundedRectangleGeometry,
    stroke_brush: CompositionColorBrush,
    underline: SpriteVisual,
    underline_brush: CompositionColorBrush,
    glyph: Text,
    /// Clips the text, which scrolls once it's wider than the field.
    text_host: ContainerVisual,
    placeholder: Text,
    query: Text,
    caret: SpriteVisual,
    caret_brush: CompositionColorBrush,
    /// The suggestions under the field, and the soft highlight behind the
    /// one picked with the arrow keys or the pointer.
    list: ContainerVisual,
    highlight: SpriteVisual,
    highlight_clip: CompositionRoundedRectangleGeometry,
    highlight_brush: CompositionColorBrush,
    rows: Vec<Text>,
    /// Says why there are no suggestions, in the first row's place.
    note: Text,
    noted: bool,
    /// How many rows show, and which one is picked.
    count: usize,
    selected: Option<usize>,
    /// Where each row's text rests in the list, physical pixels.
    row_rest: Vec<Vector3>,
    /// The room under the field the card shows something else in, in
    /// place of suggestions, physical pixels.
    room: f32,
    footer: Text,
    footer_h: f32,
    /// Whether the card shows the box.
    pub(crate) active: bool,
    scale: f32,
    /// The field's resting place in the card, physical pixels.
    field_at: Vector3,
    field_size: Vector2,
    /// Top of the text and of the caret in the field, physical pixels.
    query_y: f32,
    caret_y: f32,
    /// How far the text has scrolled left, physical pixels.
    shift: f32,
    empty: bool,
    /// How long the caret waits to show, next time what's typed is shown.
    caret_hold: u64,
}

impl SearchBox {
    /// The box, in the `card`, under `above`: the heading, which rises
    /// out of the field and sinks back into it over the box.
    pub(crate) fn new(c: &Compositor, card: &ContainerVisual, above: &Visual) -> Result<SearchBox> {
        let visual = c.CreateContainerVisual()?;
        visual.SetOpacity(0.0)?;
        card.Children()?.InsertBelow(&visual, above)?;
        let old_title = Text::new(c, &visual)?;
        let title = Text::new(c, &visual)?;

        let field = c.CreateContainerVisual()?;
        let field_clip = c.CreateRoundedRectangleGeometry()?;
        field.SetClip(&c.CreateGeometricClipWithGeometry(&field_clip)?)?;
        visual.Children()?.InsertAtTop(&field)?;
        let fill = c.CreateColorBrush()?;
        let fill_visual = c.CreateSpriteVisual()?;
        fill_visual.SetBrush(&fill)?;
        fill_visual.SetRelativeSizeAdjustment(v2(1.0, 1.0))?;
        field.Children()?.InsertAtTop(&fill_visual)?;
        let stroke = c.CreateShapeVisual()?;
        let stroke_geometry = c.CreateRoundedRectangleGeometry()?;
        let shape = c.CreateSpriteShapeWithGeometry(&stroke_geometry)?;
        let stroke_brush = c.CreateColorBrush()?;
        shape.SetStrokeBrush(&stroke_brush)?;
        shape.SetStrokeThickness(1.0)?;
        stroke.Shapes()?.Append(&shape)?;
        field.Children()?.InsertAtTop(&stroke)?;
        // The accent line along the bottom, clipped to the field's corners.
        let underline = c.CreateSpriteVisual()?;
        let underline_brush = c.CreateColorBrush()?;
        underline.SetBrush(&underline_brush)?;
        field.Children()?.InsertAtTop(&underline)?;
        let glyph = Text::new(c, &field)?;

        let text_host = c.CreateContainerVisual()?;
        text_host.SetClip(&c.CreateInsetClip()?)?;
        field.Children()?.InsertAtTop(&text_host)?;
        let placeholder = Text::new(c, &text_host)?;
        let query = Text::new(c, &text_host)?;
        let caret = c.CreateSpriteVisual()?;
        let caret_brush = c.CreateColorBrush()?;
        caret.SetBrush(&caret_brush)?;
        caret.SetOpacity(0.0)?;
        text_host.Children()?.InsertAtTop(&caret)?;

        let list = c.CreateContainerVisual()?;
        visual.Children()?.InsertAtTop(&list)?;
        let highlight = c.CreateSpriteVisual()?;
        let highlight_clip = c.CreateRoundedRectangleGeometry()?;
        highlight.SetClip(&c.CreateGeometricClipWithGeometry(&highlight_clip)?)?;
        let highlight_brush = c.CreateColorBrush()?;
        highlight.SetBrush(&highlight_brush)?;
        highlight.SetOpacity(0.0)?;
        list.Children()?.InsertAtTop(&highlight)?;
        let rows = (0..MAX_SUGGESTIONS)
            .map(|_| Text::new(c, &list))
            .collect::<Result<Vec<_>>>()?;
        let note = Text::new(c, &list)?;

        let footer = Text::new(c, &visual)?;

        Ok(SearchBox {
            visual,
            title,
            old_title,
            field,
            field_clip,
            fill,
            stroke,
            stroke_geometry,
            stroke_brush,
            underline,
            underline_brush,
            glyph,
            text_host,
            placeholder,
            query,
            caret,
            caret_brush,
            list,
            highlight,
            highlight_clip,
            highlight_brush,
            rows,
            note,
            noted: false,
            count: 0,
            selected: None,
            row_rest: vec![v3(0.0, 0.0); MAX_SUGGESTIONS],
            room: 0.0,
            footer,
            footer_h: 0.0,
            active: false,
            scale: 1.0,
            field_at: v3(0.0, 0.0),
            field_size: v2(0.0, 0.0),
            query_y: 0.0,
            caret_y: 0.0,
            shift: 0.0,
            empty: true,
            caret_hold: 0,
        })
    }

    /// Lays the box out, empty, under `title`, for what the `purpose`
    /// shortcut does, and reveals it starting `delay` ms from now. Returns
    /// its height in pixels.
    // The drawing context (renderer, animator, theme, scale) comes from the
    // card that owns the box, like for its other methods, plus what to show.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn open(
        &mut self,
        renderer: &Renderer,
        anim: &Animator,
        theme: &Theme,
        s: f32,
        title: &str,
        purpose: Shortcut,
        delay: u64,
    ) -> Result<f32> {
        let (glyph, placeholder, enter) = match purpose {
            Shortcut::Lookup => (SEARCH_GLYPH, tr().search_placeholder, tr().key_look_up),
            Shortcut::Translate => (
                TRANSLATE_GLYPH,
                tr().translate_placeholder,
                tr().key_translate,
            ),
        };
        (self.active, self.scale, self.shift, self.empty) = (true, s, 0.0, true);
        (self.room, self.caret_hold) = (0.0, 0);
        let width = CARD_W - 2.0 * PAD_X;
        self.fill.SetColor(theme.field())?;
        self.stroke_brush.SetColor(theme.border())?;
        self.underline_brush.SetColor(theme.accent)?;
        self.caret_brush.SetColor(theme.accent)?;

        let heading = renderer.layout_line(title, Face::Title, Ink::Primary, width)?;
        let title_size = self.title.show(renderer, &heading, s, theme)?;
        Animator::stop(&self.old_title.visual, &["Opacity"]);
        self.old_title.visual.SetOpacity(0.0)?;
        self.old_title.clear();
        let title_at = v3((PAD_X * s).round(), (PAD_TOP * s).round());

        // The field.
        let (field_w, field_h) = ((width * s).round(), (FIELD_H * s).round());
        let field_y = title_at.Y + title_size.Y + (TITLE_GAP * s).round();
        self.field_at = v3(title_at.X, field_y);
        self.field_size = v2(field_w, field_h);
        self.field.SetSize(self.field_size)?;
        self.field_clip.SetSize(self.field_size)?;
        let r = FIELD_RADIUS * s;
        self.field_clip.SetCornerRadius(v2(r, r))?;
        self.stroke.SetSize(self.field_size)?;
        self.stroke_geometry
            .SetSize(v2(field_w - 1.0, field_h - 1.0))?;
        self.stroke_geometry.SetOffset(v2(0.5, 0.5))?;
        self.stroke_geometry.SetCornerRadius(v2(r - 0.5, r - 0.5))?;
        let line = (UNDERLINE_H * s).round().max(1.0);
        self.underline.SetSize(v2(field_w, line))?;
        self.underline.SetOffset(v3(0.0, field_h - line))?;
        self.underline.SetCenterPoint(Vector3 {
            X: field_w / 2.0,
            Y: 0.0,
            Z: 0.0,
        })?;

        let magnifier = renderer.layout_line(glyph, Face::Button, Ink::Secondary, GLYPH_W)?;
        let size = self.glyph.show(renderer, &magnifier, s, theme)?;
        self.glyph.visual.SetOffset(v3(
            (GLYPH_X * s).round(),
            ((field_h - size.Y) / 2.0).round(),
        ))?;

        // The text, its placeholder and the caret.
        let host_x = (TEXT_X * s).round();
        self.text_host.SetOffset(v3(host_x, 0.0))?;
        self.text_host
            .SetSize(v2(field_w - host_x - (TEXT_PAD_RIGHT * s).round(), field_h))?;
        let hint = renderer.layout_line(placeholder, Face::Query, Ink::Tertiary, width)?;
        let size = self.placeholder.show(renderer, &hint, s, theme)?;
        self.query_y = ((field_h - size.Y) / 2.0).round();
        Animator::stop(&self.placeholder.visual, &["Opacity"]);
        self.placeholder.visual.SetOpacity(1.0)?;
        self.placeholder.visual.SetOffset(v3(0.0, self.query_y))?;
        self.query.clear();
        Animator::stop(&self.query.visual, &["Offset", "Opacity"]);
        self.query.visual.SetOffset(v3(0.0, self.query_y))?;
        self.query.visual.SetOpacity(1.0)?;
        let (_, _, caret) = renderer.layout_query("", 0, width)?;
        self.caret_y = self.query_y + (caret.top * s).round();
        self.caret.SetSize(v2(
            (CARET_W * s).round().max(1.0),
            (caret.height * s).round(),
        ))?;
        Animator::stop(&self.caret, &["Offset"]);
        self.caret.SetOffset(v3(0.0, self.caret_y))?;

        // No suggestions yet.
        self.list
            .SetOffset(v3(0.0, field_y + field_h + (LIST_GAP * s).round()))?;
        Animator::stop(&self.list, &["Opacity"]);
        self.list.SetOpacity(1.0)?;
        for row in &self.rows {
            Animator::stop(&row.visual, &["Opacity", "Offset"]);
            row.visual.SetOpacity(0.0)?;
            row.clear();
        }
        Animator::stop(&self.note.visual, &["Opacity", "Offset"]);
        self.note.visual.SetOpacity(0.0)?;
        self.note.clear();
        (self.count, self.selected, self.noted) = (0, None, false);
        self.highlight_brush.SetColor(theme.highlight())?;
        let row_size = v2(field_w, (ROW_H * s).round());
        self.highlight.SetSize(row_size)?;
        self.highlight_clip.SetSize(row_size)?;
        let r = ROW_RADIUS * s;
        self.highlight_clip.SetCornerRadius(v2(r, r))?;
        Animator::stop(&self.highlight, &["Opacity", "Offset"]);
        self.highlight.SetOpacity(0.0)?;

        let keys = renderer.layout_keys(&[("Enter", enter), ("Esc", tr().key_close)])?;
        self.footer_h = self.footer.show(renderer, &keys, s, theme)?.Y;
        let footer_at = v3(title_at.X, self.footer_y(0));
        let height = self.height(0);

        // Each part rises into place a beat after the one above it, then the
        // underline grows out from the middle and the caret starts blinking.
        self.visual.SetOpacity(1.0)?;
        let drift = DRIFT * s;
        let parts: [(Visual, Vector3); 3] = [
            (self.title.visual.cast()?, title_at),
            (self.field.cast()?, self.field_at),
            (self.footer.visual.cast()?, footer_at),
        ];
        for (i, (part, rest)) in parts.iter().enumerate() {
            let at = delay + i as u64 * REVEAL_STAGGER_MS;
            Animator::stop(part, &["Opacity", "Offset"]);
            part.SetOpacity(0.0)?;
            part.SetOffset(v3(rest.X, rest.Y + drift))?;
            anim.scalar(part, "Opacity", 1.0, REVEAL_MS, at, &anim.decelerate)?;
            anim.vector3(part, "Offset", *rest, REVEAL_MS, at, &anim.decelerate)?;
        }
        Animator::stop(&self.underline, &["Scale"]);
        self.underline.SetScale(Vector3 {
            X: 0.0,
            Y: 1.0,
            Z: 1.0,
        })?;
        let full = scale(1.0);
        let at = delay + REVEAL_STAGGER_MS + 80;
        anim.vector3(
            &self.underline,
            "Scale",
            full,
            UNDERLINE_MS,
            at,
            &anim.decelerate,
        )?;
        self.blink(anim, at)?;
        Ok(height)
    }

    /// Shows `text` in the field. The caret glides to `at` (UTF-16 units
    /// in), the text scrolls to keep it in view, and the placeholder fades
    /// out or back in.
    pub(crate) fn set_query(
        &mut self,
        renderer: &Renderer,
        anim: &Animator,
        theme: &Theme,
        text: &str,
        at: u32,
    ) -> Result<()> {
        let s = self.scale;
        let room = self.text_host.Size()?.X;
        let (layout, width, caret) = renderer.layout_query(text, at, room / s)?;
        if text.is_empty() {
            self.query.clear();
        } else {
            self.query.show(renderer, &layout, s, theme)?;
        }
        let (end, x) = (width * s, caret.x * s);
        // Scroll as little as keeps the caret in view, with a little room
        // after it, and never past the end of the text.
        let margin = 2.0 * s;
        let shift = self
            .shift
            .min(end + margin - room)
            .max(x + margin - room)
            .min(x)
            .max(0.0)
            .round();
        if shift != self.shift {
            self.shift = shift;
            let to = v3(-shift, self.query_y);
            let ease = &anim.decelerate;
            anim.vector3(&self.query.visual, "Offset", to, CARET_GLIDE_MS, 0, ease)?;
        }
        let to = v3((x - shift).round(), self.caret_y);
        anim.vector3(
            &self.caret,
            "Offset",
            to,
            CARET_GLIDE_MS,
            0,
            &anim.decelerate,
        )?;
        // Held back while the heading's on its way into the field.
        let hold = std::mem::take(&mut self.caret_hold);
        self.blink(anim, hold)?;

        let empty = text.is_empty();
        if empty != self.empty {
            self.empty = empty;
            let to = if empty { 1.0 } else { 0.0 };
            let ph = &self.placeholder.visual;
            anim.scalar(ph, "Opacity", to, PLACEHOLDER_MS, 0, &anim.decelerate)?;
        }
        Ok(())
    }

    /// Shows `items` (each a word, what it's a form of, how many of its
    /// first characters to bold as typed, and the code of its language when
    /// there are several) under the field, or else `note` in their place. New rows slide in, rows no longer needed fade, and
    /// the footer glides to its new place. Returns the box's new height in
    /// pixels.
    pub(crate) fn set_suggestions(
        &mut self,
        renderer: &Renderer,
        anim: &Animator,
        theme: &Theme,
        items: &[(&str, Option<&str>, usize, Option<&str>)],
        note: Option<&str>,
    ) -> Result<f32> {
        let s = self.scale;
        let count = items.len().min(self.rows.len());
        let x = self.field_at.X + (TEXT_X * s).round();
        let room = self.field_size.X - (TEXT_X + TEXT_PAD_RIGHT) * s;
        let row_h = (ROW_H * s).round();
        let was_noted = self.noted;
        self.noted = count == 0 && note.is_some();
        let v = &self.note.visual;
        match note.filter(|_| self.noted) {
            Some(note) => {
                let layout = renderer.layout_line(note, Face::Menu, Ink::Tertiary, room / s)?;
                let size = self.note.show(renderer, &layout, s, theme)?;
                let rest = v3(x, ((row_h - size.Y) / 2.0).round());
                Animator::stop(v, &["Offset"]);
                if was_noted {
                    v.SetOffset(rest)?;
                } else {
                    // Like a row: a short slide down from under the field.
                    Animator::stop(v, &["Opacity"]);
                    v.SetOpacity(0.0)?;
                    v.SetOffset(v3(rest.X, rest.Y - DRIFT * s))?;
                    anim.scalar(v, "Opacity", 1.0, ROW_MS, 0, &anim.decelerate)?;
                    anim.vector3(v, "Offset", rest, ROW_MS, 0, &anim.decelerate)?;
                }
            }
            None if was_noted => {
                anim.scalar(v, "Opacity", 0.0, LEAVE_MS, 0, &anim.accelerate)?;
            }
            None => {}
        }
        for (i, row) in self.rows.iter().enumerate() {
            let v = &row.visual;
            let Some((word, base, typed, badge)) = items.get(i).filter(|_| i < count) else {
                if i < self.count {
                    anim.scalar(v, "Opacity", 0.0, LEAVE_MS, 0, &anim.accelerate)?;
                }
                continue;
            };
            let layout = renderer.layout_suggestion(word, *typed, *base, *badge, room / s)?;
            let size = row.show(renderer, &layout, s, theme)?;
            let rest = v3(x, i as f32 * row_h + ((row_h - size.Y) / 2.0).round());
            self.row_rest[i] = rest;
            Animator::stop(v, &["Offset"]);
            if i < self.count {
                v.SetOffset(rest)?;
                continue;
            }
            // Appearing: a short slide down from under the field.
            let delay = (i - self.count) as u64 * ROW_STAGGER_MS;
            Animator::stop(v, &["Opacity"]);
            v.SetOpacity(0.0)?;
            v.SetOffset(v3(rest.X, rest.Y - DRIFT * s))?;
            anim.scalar(v, "Opacity", 1.0, ROW_MS, delay, &anim.decelerate)?;
            anim.vector3(v, "Offset", rest, ROW_MS, delay, &anim.decelerate)?;
        }
        // The note takes a row's room.
        let shown = count.max(self.noted as usize);
        if shown != self.count.max(was_noted as usize) {
            let to = v3(self.field_at.X, self.footer_y(shown));
            let footer = &self.footer.visual;
            anim.vector3(footer, "Offset", to, RESIZE_MS, 0, &anim.decelerate)?;
        }
        self.count = count;
        self.select(anim, None)?;
        Ok(self.height(shown))
    }

    /// The picked suggestion, if any.
    pub(crate) fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// Picks the next suggestion down (or up). Past either end the pick
    /// goes back to the field, as in a browser's address bar.
    pub(crate) fn step(&mut self, anim: &Animator, down: bool) -> Result<()> {
        if self.count == 0 {
            return Ok(());
        }
        let last = self.count - 1;
        let next = match (self.selected, down) {
            (None, true) => Some(0),
            (None, false) => Some(last),
            (Some(i), true) if i < last => Some(i + 1),
            (Some(i), false) if i > 0 => Some(i - 1),
            _ => None,
        };
        self.select(anim, next)
    }

    /// Picks the suggestion under the pointer, in card pixels; one that
    /// left the card lets go of it.
    pub(crate) fn hover(&mut self, anim: &Animator, point: Option<(f32, f32)>) -> Result<()> {
        match point {
            Some((x, y)) => match self.row_at(x, y) {
                Some(row) => self.select(anim, Some(row)),
                None => Ok(()),
            },
            None => self.select(anim, None),
        }
    }

    /// The suggestion under a point in card pixels.
    pub(crate) fn row_at(&self, x: f32, y: f32) -> Option<usize> {
        let (at, row_h) = (self.field_at, (ROW_H * self.scale).round());
        let top = self.list.Offset().ok()?.Y;
        let inside = x >= at.X && x < at.X + self.field_size.X && y >= top;
        let row = ((y - top) / row_h) as usize;
        (self.active && inside && row < self.count).then_some(row)
    }

    /// The card's heading is sinking into the field, as what's typed: the
    /// field waits for it where it rests, rather than rising into place,
    /// and what's typed shows `delay` ms from now, fading in over `ms` as
    /// the heading fades out. The caret follows once it has.
    pub(crate) fn await_heading(&mut self, anim: &Animator, delay: u64, ms: u64) -> Result<()> {
        self.caret_hold = delay + ms;
        Animator::stop(&self.field, &["Offset"]);
        self.field.SetOffset(self.field_at)?;
        let query = &self.query.visual;
        Animator::stop(query, &["Opacity"]);
        query.SetOpacity(0.0)?;
        anim.scalar(query, "Opacity", 1.0, ms, delay, &anim.fade)
    }

    /// Cross-fades the heading to `title`, the new one rising into place.
    pub(crate) fn set_title(
        &mut self,
        renderer: &Renderer,
        anim: &Animator,
        theme: &Theme,
        title: &str,
    ) -> Result<()> {
        let s = self.scale;
        // The heading on show becomes the old one.
        std::mem::swap(&mut self.title, &mut self.old_title);
        let width = CARD_W - 2.0 * PAD_X;
        let heading = renderer.layout_line(title, Face::Title, Ink::Primary, width)?;
        self.title.show(renderer, &heading, s, theme)?;
        let rest = v3((PAD_X * s).round(), (PAD_TOP * s).round());
        let (new, old) = (&self.title.visual, &self.old_title.visual);
        Animator::stop(new, &["Opacity", "Offset"]);
        new.SetOpacity(0.0)?;
        new.SetOffset(v3(rest.X, rest.Y + DRIFT * s))?;
        anim.scalar(new, "Opacity", 1.0, TITLE_MS, 0, &anim.decelerate)?;
        anim.vector3(new, "Offset", rest, TITLE_MS, 0, &anim.decelerate)?;
        anim.scalar(old, "Opacity", 0.0, TITLE_MS / 2, 0, &anim.accelerate)
    }

    /// Shakes the field, as a sign that what's typed can't be looked up.
    pub(crate) fn shake(&self, anim: &Animator) -> Result<()> {
        anim.shake(&self.field, self.field_at, self.scale)
    }

    /// Fades the box away. Returns where the word being looked up sits in
    /// the card, in pixels, and its size in DIPs, for the heading to grow
    /// out of: the picked suggestion, or else what's typed.
    pub(crate) fn leave(&mut self, anim: &Animator) -> Result<(Vector3, f32)> {
        self.active = false;
        Animator::stop(&self.caret, &["Opacity"]);
        self.caret.SetOpacity(0.0)?;
        for part in [
            self.title.visual.cast::<Visual>()?,
            self.old_title.visual.cast()?,
            self.field.cast()?,
            self.list.cast()?,
            self.footer.visual.cast()?,
        ] {
            anim.scalar(&part, "Opacity", 0.0, LEAVE_MS, 0, &anim.accelerate)?;
        }
        if let Some(row) = self.selected.filter(|&i| i < self.count) {
            let (list, rest) = (self.list.Offset()?, self.row_rest[row]);
            return Ok((v3(rest.X, list.Y + rest.Y), SUGGESTION_SIZE));
        }
        Ok((self.query_at()?, QUERY_SIZE))
    }

    /// Where what's typed starts in the card, in pixels, at its size of
    /// [`QUERY_SIZE`] DIPs.
    pub(crate) fn query_at(&self) -> Result<Vector3> {
        Ok(v3(
            self.field_at.X + self.text_host.Offset()?.X - self.shift,
            self.field_at.Y + self.query_y,
        ))
    }

    /// Picks suggestion `row` (or none), as the one to look up.
    pub(crate) fn pick(&mut self, anim: &Animator, row: Option<usize>) -> Result<()> {
        self.select(anim, row.filter(|&i| i < self.count))
    }

    /// Hides the box at once and lets go of its text surfaces.
    pub(crate) fn reset(&mut self) {
        self.active = false;
        let _ = self.visual.SetOpacity(0.0);
        Animator::stop(&self.caret, &["Opacity"]);
        for text in [
            &self.title,
            &self.old_title,
            &self.glyph,
            &self.placeholder,
            &self.query,
            &self.footer,
            &self.note,
        ]
        .into_iter()
        .chain(&self.rows)
        {
            text.clear();
        }
        (self.count, self.selected, self.noted) = (0, None, false);
        self.room = 0.0;
    }

    /// Where what the card shows under the field starts, in card pixels.
    pub(crate) fn below_field(&self) -> f32 {
        self.field_at.Y + self.field_size.Y + (LIST_GAP * self.scale).round()
    }

    /// The room made under the field, in pixels: none until some is.
    pub(crate) fn room(&self) -> f32 {
        self.room
    }

    /// Makes `room` pixels under the field for the card to show something
    /// in, the footer gliding down or up to make way. Returns the box's
    /// new height in pixels.
    pub(crate) fn make_room(&mut self, anim: &Animator, room: f32) -> Result<f32> {
        if room != self.room {
            self.room = room;
            let to = v3(self.field_at.X, self.footer_y(0));
            let footer = &self.footer.visual;
            anim.vector3(footer, "Offset", to, RESIZE_MS, 0, &anim.decelerate)?;
        }
        Ok(self.height(0))
    }

    /// Whether a point in card pixels is on the field.
    pub(crate) fn field_at(&self, x: f32, y: f32) -> bool {
        let (at, size) = (self.field_at, self.field_size);
        self.active && x >= at.X && x < at.X + size.X && y >= at.Y && y < at.Y + size.Y
    }

    /// Where the footer goes with `count` rows under the field, in card pixels.
    fn footer_y(&self, count: usize) -> f32 {
        let s = self.scale;
        let field_bottom = self.field_at.Y + self.field_size.Y;
        let list = if count > 0 {
            (LIST_GAP * s).round() + count as f32 * (ROW_H * s).round()
        } else if self.room > 0.0 {
            (LIST_GAP * s).round() + self.room
        } else {
            0.0
        };
        field_bottom + list + (FOOTER_GAP * s).round()
    }

    /// The box's height with `count` rows under the field, in pixels.
    fn height(&self, count: usize) -> f32 {
        (self.footer_y(count) + self.footer_h + PAD_BOTTOM * self.scale).ceil()
    }

    /// Moves the highlight to `row`: it glides from row to row, and fades
    /// in where it's wanted rather than sliding from where it last was.
    fn select(&mut self, anim: &Animator, row: Option<usize>) -> Result<()> {
        if row == self.selected {
            return Ok(());
        }
        let was_selected = self.selected.is_some();
        self.selected = row;
        let visual = &self.highlight;
        let Some(row) = row else {
            return anim.scalar(visual, "Opacity", 0.0, 120, 0, &anim.accelerate);
        };
        let to = v3(self.field_at.X, row as f32 * (ROW_H * self.scale).round());
        if was_selected {
            anim.vector3(visual, "Offset", to, HIGHLIGHT_MOVE_MS, 0, &anim.decelerate)?;
        } else {
            Animator::stop(visual, &["Offset"]);
            visual.SetOffset(to)?;
        }
        anim.scalar(visual, "Opacity", 1.0, 100, 0, &anim.decelerate)
    }

    /// Shows the caret, solid for now: blinking restarts with every
    /// keystroke, `delay` ms from now.
    fn blink(&self, anim: &Animator, delay: u64) -> Result<()> {
        let a = anim.compositor.CreateScalarKeyFrameAnimation()?;
        a.InsertKeyFrame(0.0, 1.0)?;
        a.InsertKeyFrame(0.49, 1.0)?;
        a.InsertKeyFrame(0.5, 0.0)?;
        a.InsertKeyFrame(0.99, 0.0)?;
        a.InsertKeyFrame(1.0, 1.0)?;
        a.SetDuration(ts(CARET_BLINK_MS))?;
        a.SetIterationBehavior(AnimationIterationBehavior::Forever)?;
        if delay > 0 {
            // Hidden until the field is in place.
            self.caret.SetOpacity(0.0)?;
            a.SetDelayTime(ts(delay))?;
        } else {
            self.caret.SetOpacity(1.0)?;
        }
        self.caret.StartAnimation(&HSTRING::from("Opacity"), &a)
    }
}
