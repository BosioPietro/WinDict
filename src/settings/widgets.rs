//! The settings window's controls, each built in its row's group: switches,
//! choices, text fields, counters and tooltips, and the rounded shapes and
//! outlines they're drawn with.

use windows::core::Result;
use windows::UI::Color;
use windows::UI::Composition::*;
use windows_numerics::Vector3;

use crate::theme::rgb;

use super::*;

impl Settings {
    /// A switch ending at `right` and centred on `middle`, in `group`.
    /// Returns it (without its row's hover) and its track.
    pub(super) fn build_switch(
        &self,
        group: &ContainerVisual,
        right: f32,
        middle: f32,
    ) -> Result<(SwitchView, Rect)> {
        let (c, r, t, s) = (
            &self.anim.compositor,
            &self.renderer,
            &self.theme,
            self.scale,
        );
        let px = |dip: f32| (dip * s).round();
        let (w, h) = (px(SWITCH_W), px(SWITCH_H));
        let track = Rect {
            x: right - w,
            y: (middle - h / 2.0).round(),
            w,
            h,
        };
        let root = container(c, group, track.at())?;
        let face = Rect {
            w,
            h,
            ..Rect::default()
        };
        let off = container(c, &root, v3(0.0, 0.0))?;
        rounded(c, r, &off, face, h / 2.0, t.sunken())?;
        outline(c, r, &off, face, h / 2.0, t.strong_stroke())?;
        let (on, _) = rounded(c, r, &root, face, h / 2.0, t.accent)?;
        let knob = px(KNOB);
        let dot = Rect {
            w: knob,
            h: knob,
            ..Rect::default()
        };
        let (knob_visual, knob_brush) = rounded(c, r, &root, dot, knob / 2.0, t.knob(false))?;
        knob_visual.SetCenterPoint(Vector3 {
            X: knob / 2.0,
            Y: knob / 2.0,
            Z: 0.0,
        })?;
        let inset = ((h - knob) / 2.0).round();
        let view = SwitchView {
            on,
            off,
            knob: knob_visual,
            knob_brush,
            rest: [v3(inset, inset), v3(w - inset - knob, inset)],
            hover: c.CreateSpriteVisual()?,
        };
        Ok((view, track))
    }

    /// `choice`'s options side by side, ending at `right` and centred on
    /// `middle`, in `group`. Returns it, its bounds and each option's.
    pub(super) fn build_choice(
        &self,
        choice: Choice,
        group: &ContainerVisual,
        right: f32,
        middle: f32,
    ) -> Result<(ChoiceView, Rect, Vec<Rect>)> {
        let (c, r, t, s) = (
            &self.anim.compositor,
            &self.renderer,
            &self.theme,
            self.scale,
        );
        let px = |dip: f32| (dip * s).round();
        let (segment_w, h) = (choice.segment_w(), px(SEGMENT_H));
        let segment = px(segment_w);
        let names = choice.names();
        let n = names.len() as f32;
        let at = Rect {
            x: right - n * segment,
            y: (middle - h / 2.0).round(),
            w: n * segment,
            h,
        };
        let root = container(c, group, at.at())?;
        let face = Rect {
            w: at.w,
            h,
            ..Rect::default()
        };
        rounded(c, r, &root, face, px(SEGMENT_RADIUS), t.sunken())?;
        outline(c, r, &root, face, px(SEGMENT_RADIUS), t.control_stroke())?;

        let inset = px(PILL_INSET);
        let pill_face = Rect {
            w: segment - 2.0 * inset,
            h: h - 2.0 * inset,
            ..Rect::default()
        };
        let pill = container(c, &root, v3(inset, inset))?;
        pill.SetSize(pill_face.size())?;
        pill.SetCenterPoint(Vector3 {
            X: pill_face.w / 2.0,
            Y: pill_face.h / 2.0,
            Z: 0.0,
        })?;
        rounded(c, r, &pill, pill_face, px(PILL_RADIUS), t.raised())?;
        outline(c, r, &pill, pill_face, px(PILL_RADIUS), t.control_stroke())?;

        let mut labels = Vec::new();
        let mut segments = Vec::new();
        let mut rest = Vec::new();
        for (i, name) in names.into_iter().enumerate() {
            let x = i as f32 * segment;
            let label = Text::new(c, &root)?;
            let line =
                r.layout_aligned(name, Face::Menu, Ink::Primary, segment_w, Align::Center)?;
            let size = label.show(r, &line, s, t)?;
            label
                .visual
                .SetOffset(v3(x, ((h - size.Y) / 2.0).round()))?;
            labels.push(label.visual);
            segments.push(Rect {
                x: at.x + x,
                y: at.y,
                w: segment,
                h,
            });
            rest.push(v3(x + inset, inset));
        }
        Ok((ChoiceView { pill, labels, rest }, at, segments))
    }

    /// A tooltip saying `text`, over everything and out of sight, in `parent`.
    pub(super) fn build_tip(&self, parent: &ContainerVisual, text: &str) -> Result<TipView> {
        let (c, r, t, s) = (
            &self.anim.compositor,
            &self.renderer,
            &self.theme,
            self.scale,
        );
        let px = |dip: f32| (dip * s).round();
        let pad = px(TIP_PAD);
        let layout = r.layout_tip(text, TIP_W - 2.0 * TIP_PAD)?;
        let text_size = v2((layout.width * s).ceil(), (layout.height * s).ceil());
        let face = Rect {
            w: text_size.X + 2.0 * pad,
            h: text_size.Y + 2.0 * pad,
            ..Rect::default()
        };
        let root = container(c, parent, v3(0.0, 0.0))?;
        root.SetSize(face.size())?;
        root.SetOpacity(0.0)?;
        let radius = px(TIP_RADIUS);
        let fill = if t.dark {
            rgb(0x2C, 0x2C, 0x2C)
        } else {
            rgb(0xFA, 0xFA, 0xFA)
        };
        // A soft shadow under it, stretched from a small surface.
        let spread = px(TIP_SHADOW);
        let (surface, inset) = r.draw_shadow(spread, radius, t.shadow_opacity())?;
        let source = c.CreateSurfaceBrush()?;
        source.SetStretch(CompositionStretch::Fill)?;
        source.SetSurface(&surface)?;
        let brush = c.CreateNineGridBrush()?;
        brush.SetSource(&source)?;
        brush.SetInsets(inset)?;
        let shadow = c.CreateSpriteVisual()?;
        shadow.SetBrush(&brush)?;
        shadow.SetOffset(v3(-spread, -spread + px(TIP_SHADOW_DY)))?;
        shadow.SetSize(v2(face.w + 2.0 * spread, face.h + 2.0 * spread))?;
        root.Children()?.InsertAtTop(&shadow)?;
        rounded(c, r, &root, face, radius, fill)?;
        outline(c, r, &root, face, radius, t.border())?;
        let text = Text::new(c, &root)?;
        text.show(r, &layout, s, t)?;
        text.visual.SetOffset(v3(pad, pad))?;
        Ok(TipView {
            root,
            size: face.size(),
        })
    }

    /// A text field ending at `right` and centred on `middle`, in `group`.
    /// Returns it, with where its text goes in `group`, and its bounds.
    pub(super) fn build_field(
        &self,
        group: &ContainerVisual,
        right: f32,
        middle: f32,
    ) -> Result<(FieldView, Rect)> {
        let (c, r, t, s) = (
            &self.anim.compositor,
            &self.renderer,
            &self.theme,
            self.scale,
        );
        let px = |dip: f32| (dip * s).round();
        let (w, h, radius) = (px(FIELD_W), px(FIELD_H), px(FIELD_RADIUS));
        let at = Rect {
            x: right - w,
            y: (middle - h / 2.0).round(),
            w,
            h,
        };
        let root = container(c, group, at.at())?;
        root.SetSize(v2(w, h))?;
        let face = Rect {
            w,
            h,
            ..Rect::default()
        };
        // No clip to round the corners off: a clip's edge is smoothed
        // differently as the field moves by fractions of a pixel in a
        // scroll, which made the lines along the bottom flicker. The shapes
        // are rounded themselves instead.
        rounded(c, r, &root, face, radius, t.control())?;
        let (hover, _) = rounded(c, r, &root, face, radius, t.highlight())?;
        hover.SetOpacity(0.0)?;
        outline(c, r, &root, face, radius, t.control_stroke())?;
        // A line `thick` tall along the bottom, thinning to nothing up the
        // corners, drawn as one shape so that it follows their curve.
        let edge = |color: Color, thick: f32| -> Result<SpriteVisual> {
            let surface = r.draw_underline(w, h, radius, thick, color)?;
            let brush = c.CreateSurfaceBrush()?;
            brush.SetStretch(CompositionStretch::Fill)?;
            brush.SetSurface(&surface)?;
            let edge = c.CreateSpriteVisual()?;
            edge.SetBrush(&brush)?;
            edge.SetSize(face.size())?;
            root.Children()?.InsertAtTop(&edge)?;
            Ok(edge)
        };
        edge(t.strong_stroke(), s.round().max(1.0))?;
        let thick = px(FIELD_LINE).max(1.0);
        let accent = edge(t.accent, thick)?;
        accent.SetOpacity(0.0)?;

        let pad = px(FIELD_PAD);
        let text_at = Rect {
            x: pad,
            y: 0.0,
            w: w - 2.0 * pad,
            h: h - thick,
        };
        let host = container(c, &root, text_at.at())?;
        host.SetSize(text_at.size())?;
        host.SetClip(&c.CreateInsetClip()?)?;
        let text = Text::new(c, &host)?;
        let (caret, _) = rounded(c, r, &host, Rect::default(), 0.0, t.accent)?;
        caret.SetOpacity(0.0)?;
        let view = FieldView {
            hover,
            accent,
            text,
            caret,
            at: text_at.offset(at.at()),
        };
        Ok((view, at))
    }

    /// A number between − and + buttons, ending at `right` and
    /// centred on `middle`, in `group`. Returns it, its bounds and its
    /// buttons'.
    pub(super) fn build_counter(
        &self,
        group: &ContainerVisual,
        right: f32,
        middle: f32,
    ) -> Result<(CounterView, Rect, Vec<Rect>)> {
        let (c, r, t, s) = (
            &self.anim.compositor,
            &self.renderer,
            &self.theme,
            self.scale,
        );
        let px = |dip: f32| (dip * s).round();
        let (b, value_w) = (px(STEP_SIZE), px(VALUE_W));
        let at = Rect {
            x: right - 2.0 * b - value_w,
            y: (middle - b / 2.0).round(),
            w: 2.0 * b + value_w,
            h: b,
        };
        let root = container(c, group, at.at())?;
        let face = Rect {
            w: b,
            h: b,
            ..Rect::default()
        };
        let mut buttons = Vec::new();
        let mut rects = Vec::new();
        for (x, glyph) in [(0.0, MINUS_GLYPH), (b + value_w, PLUS_GLYPH)] {
            let visual = container(c, &root, v3(x, 0.0))?;
            visual.SetSize(face.size())?;
            visual.SetCenterPoint(Vector3 {
                X: b / 2.0,
                Y: b / 2.0,
                Z: 0.0,
            })?;
            let radius = px(STEP_RADIUS);
            rounded(c, r, &visual, face, radius, t.control())?;
            outline(c, r, &visual, face, radius, t.control_stroke())?;
            let (hover, _) = rounded(c, r, &visual, face, radius, t.highlight())?;
            hover.SetOpacity(0.0)?;
            let text = Text::new(c, &visual)?;
            let line = r.layout_line(glyph, Face::Chrome, Ink::Primary, STEP_SIZE)?;
            let size = text.show(r, &line, s, t)?;
            text.visual
                .SetOffset(v3(0.0, ((b - size.Y) / 2.0).round()))?;
            buttons.push(StepButton {
                visual,
                hover,
                glyph: text.visual,
            });
            rects.push(Rect {
                x: at.x + x,
                y: at.y,
                w: b,
                h: b,
            });
        }
        let host = container(c, &root, v3(b, 0.0))?;
        host.SetSize(v2(value_w, b))?;
        host.SetClip(&c.CreateInsetClip()?)?;
        let old = Text::new(c, &host)?;
        let value = Text::new(c, &host)?;
        let view = CounterView {
            buttons,
            host,
            value,
            old,
            host_h: b,
        };
        Ok((view, at, rects))
    }
}

pub(super) fn container(
    c: &Compositor,
    parent: &ContainerVisual,
    at: Vector3,
) -> Result<ContainerVisual> {
    let visual = c.CreateContainerVisual()?;
    visual.SetOffset(at)?;
    parent.Children()?.InsertAtTop(&visual)?;
    Ok(visual)
}

/// A rectangle of `color` with corners of `radius` pixels, in `parent`.
pub(super) fn rounded(
    c: &Compositor,
    r: &Renderer,
    parent: &ContainerVisual,
    rect: Rect,
    radius: f32,
    color: Color,
) -> Result<(SpriteVisual, CompositionColorBrush)> {
    let visual = c.CreateSpriteVisual()?;
    let brush = c.CreateColorBrushWithColor(color)?;
    visual.SetOffset(rect.at())?;
    visual.SetSize(rect.size())?;
    if radius > 0.0 {
        let mask = r.draw_rounded(rect.w, rect.h, radius, None)?;
        visual.SetBrush(&masked(c, &brush, &mask)?)?;
    } else {
        visual.SetBrush(&brush)?;
    }
    parent.Children()?.InsertAtTop(&visual)?;
    Ok((visual, brush))
}

/// A hairline just inside a rounded rectangle, in `parent`.
pub(super) fn outline(
    c: &Compositor,
    r: &Renderer,
    parent: &ContainerVisual,
    rect: Rect,
    radius: f32,
    color: Color,
) -> Result<SpriteVisual> {
    let visual = c.CreateSpriteVisual()?;
    let brush = c.CreateColorBrushWithColor(color)?;
    let mask = r.draw_rounded(rect.w, rect.h, radius, Some(1.0))?;
    visual.SetBrush(&masked(c, &brush, &mask)?)?;
    visual.SetOffset(rect.at())?;
    visual.SetSize(rect.size())?;
    parent.Children()?.InsertAtTop(&visual)?;
    Ok(visual)
}

/// `brush` showing only through `mask`, drawn pixel for pixel. The round
/// edges of the controls are drawn this way rather than as shapes or clips:
/// what scrolls is drawn again through a visual surface to fade its ends,
/// and that leaves shapes and clips jagged, but not what's drawn already.
fn masked(
    c: &Compositor,
    brush: &CompositionColorBrush,
    mask: &CompositionDrawingSurface,
) -> Result<CompositionMaskBrush> {
    let shape = c.CreateSurfaceBrushWithSurface(mask)?;
    shape.SetStretch(CompositionStretch::Fill)?;
    let masked = c.CreateMaskBrush()?;
    masked.SetSource(brush)?;
    masked.SetMask(&shape)?;
    Ok(masked)
}

/// A hairline just inside a rounded rectangle, in `parent`, that can be
/// resized through the geometry returned.
pub(super) fn resizable_outline(
    c: &Compositor,
    parent: &ContainerVisual,
    rect: Rect,
    radius: f32,
    color: Color,
) -> Result<(ShapeVisual, CompositionRoundedRectangleGeometry)> {
    let visual = c.CreateShapeVisual()?;
    visual.SetOffset(rect.at())?;
    visual.SetSize(rect.size())?;
    let geometry = c.CreateRoundedRectangleGeometry()?;
    geometry.SetOffset(v2(0.5, 0.5))?;
    geometry.SetSize(v2(rect.w - 1.0, rect.h - 1.0))?;
    geometry.SetCornerRadius(v2(radius - 0.5, radius - 0.5))?;
    let shape = c.CreateSpriteShapeWithGeometry(&geometry)?;
    shape.SetStrokeBrush(&c.CreateColorBrushWithColor(color)?)?;
    shape.SetStrokeThickness(1.0)?;
    visual.Shapes()?.Append(&shape)?;
    parent.Children()?.InsertAtTop(&visual)?;
    Ok((visual, geometry))
}
