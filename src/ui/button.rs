//! The push buttons the introduction and the setup are built with, as on
//! Windows 11: a face, the accent one for what goes on, the soft fill that
//! lights under the pointer, and the ring round the one with the keyboard.

use windows::core::Result;
use windows::UI::Color;
use windows::UI::Composition::*;

use super::{container, outline, rounded, v3, Rect, FOCUS_STROKE};
use crate::render::{Align, Face, Ink, Renderer};
use crate::theme::Theme;

// Geometry, in DIPs.
/// A button is at least this wide, and has this much room either side of
/// a longer label.
pub const BUTTON_W: f32 = 96.0;
pub const BUTTON_PAD: f32 = 16.0;
pub const BUTTON_H: f32 = 32.0;
const BUTTON_RADIUS: f32 = 4.0;
/// How far the focus ring is outside what it's round.
pub const FOCUS_OUTSET: f32 = 3.0;

pub struct Button {
    pub root: ContainerVisual,
    pub hover: SpriteVisual,
    /// Round it while it has the keyboard.
    pub ring: SpriteVisual,
    pub at: Rect,
}

/// How wide a button saying `label` is, in DIPs.
pub fn button_width(r: &Renderer, label: &str) -> Result<f32> {
    Ok((r.line_width(label, Face::Menu)? + 2.0 * BUTTON_PAD).max(BUTTON_W))
}

/// A button saying `label` at `at` in `parent`, drawn at `s` pixels per
/// DIP: in the accent when it's the `primary` one, as Windows draws the one
/// to go on with.
// The drawing context (compositor, renderer, theme, scale) every builder in
// `ui` takes, plus where the button goes and what it says.
#[allow(clippy::too_many_arguments)]
pub fn button(
    c: &Compositor,
    r: &Renderer,
    t: &Theme,
    s: f32,
    parent: &ContainerVisual,
    at: Rect,
    label: &str,
    primary: bool,
) -> Result<Button> {
    let px = |dip: f32| (dip * s).round();
    let root = container(c, parent, at.at())?;
    let face = Rect {
        w: at.w,
        h: at.h,
        ..Rect::default()
    };
    let radius = px(BUTTON_RADIUS);
    // The label stands out on the accent in the other theme's color.
    let inverse = Theme {
        dark: !t.dark,
        ..*t
    };
    let (fill, stroke, ink) = if primary {
        (t.accent, t.accent, inverse)
    } else {
        (t.control(), t.control_stroke(), *t)
    };
    rounded(c, r, &root, face, radius, fill)?;
    outline(c, r, &root, face, radius, stroke)?;
    let (hover, _) = rounded(c, r, &root, face, radius, t.highlight())?;
    hover.SetOpacity(0.0)?;
    let text = super::Text::new(c, &root)?;
    let line = r.layout_aligned(label, Face::Menu, Ink::Primary, at.w / s, Align::Center)?;
    let size = text.show(r, &line, s, &ink)?;
    text.visual
        .SetOffset(v3(0.0, ((at.h - size.Y) / 2.0).round()))?;
    let outset = px(FOCUS_OUTSET);
    let ring = ring(
        c,
        r,
        &root,
        face.outset(outset),
        radius + outset,
        FOCUS_STROKE * s,
        t.focus(),
    )?;
    ring.SetOpacity(0.0)?;
    Ok(Button {
        root,
        hover,
        ring,
        at,
    })
}

/// A ring `stroke` pixels thick just inside `rect`, in `parent`, as round
/// what the keyboard is on.
pub fn ring(
    c: &Compositor,
    r: &Renderer,
    parent: &ContainerVisual,
    rect: Rect,
    radius: f32,
    stroke: f32,
    color: Color,
) -> Result<SpriteVisual> {
    let shape =
        c.CreateSurfaceBrushWithSurface(&r.draw_rounded(rect.w, rect.h, radius, Some(stroke))?)?;
    shape.SetStretch(CompositionStretch::Fill)?;
    let masked = c.CreateMaskBrush()?;
    masked.SetSource(&c.CreateColorBrushWithColor(color)?)?;
    masked.SetMask(&shape)?;
    let visual = c.CreateSpriteVisual()?;
    visual.SetBrush(&masked)?;
    visual.SetOffset(rect.at())?;
    visual.SetSize(rect.size())?;
    parent.Children()?.InsertAtTop(&visual)?;
    Ok(visual)
}
