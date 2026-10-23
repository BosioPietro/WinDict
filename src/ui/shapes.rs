//! The shapes the settings window and the introduction are drawn with:
//! rectangles of color with rounded corners, and hairlines round them.

use windows::core::Result;
use windows::UI::Color;
use windows::UI::Composition::*;
use windows_numerics::{Vector2, Vector3};

use super::{v2, v3};
use crate::render::Renderer;

/// A rectangle, in pixels.
#[derive(Clone, Copy, Default, Debug)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }

    pub fn outset(&self, d: f32) -> Rect {
        Rect {
            x: self.x - d,
            y: self.y - d,
            w: self.w + 2.0 * d,
            h: self.h + 2.0 * d,
        }
    }

    pub fn offset(&self, by: Vector3) -> Rect {
        Rect {
            x: self.x + by.X,
            y: self.y + by.Y,
            ..*self
        }
    }

    pub fn at(&self) -> Vector3 {
        v3(self.x, self.y)
    }

    pub fn size(&self) -> Vector2 {
        v2(self.w, self.h)
    }
}

pub fn container(c: &Compositor, parent: &ContainerVisual, at: Vector3) -> Result<ContainerVisual> {
    let visual = c.CreateContainerVisual()?;
    visual.SetOffset(at)?;
    parent.Children()?.InsertAtTop(&visual)?;
    Ok(visual)
}

/// A rectangle of `color` with corners of `radius` pixels, in `parent`.
pub fn rounded(
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
pub fn outline(
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
