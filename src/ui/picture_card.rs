//! What the introduction and the setup have alike: a window opened larger
//! than its size in DIPs, its card a picture on the left and what it says
//! on the right, a close button at the top right, and a title after a
//! symbol on a tinted square, the text sliding in line after line. Each
//! window keeps its own state; these are the pieces they're built of.

use windows::core::Result;
use windows::Win32::Foundation::RECT;
use windows::UI::Color;
use windows::UI::Composition::*;
use windows_numerics::Vector3;

use super::{container, outline, rounded, v2, v3, Animator, Button, Frame, Rect, Text, MARGIN};
use crate::render::{Face, Ink, Renderer};
use crate::theme::{argb, Theme};

// Geometry, in DIPs.
/// The picture, on the left, as far from the card's edges all round.
pub const INSET: f32 = 12.0;
const PICTURE_RADIUS: f32 = 8.0;
/// Between the picture and the text, and between the text and the right.
pub const PAD_X: f32 = 36.0;
/// The strip along the top that drags the window, over the picture too.
pub const TOP_H: f32 = 48.0;
const CLOSE_SIZE: f32 = 32.0;
/// The close button's distance from the card's top and right edges.
const CLOSE_INSET: f32 = 10.0;
const CLOSE_RADIUS: f32 = 5.0;
/// Where the title starts.
pub const TITLE_Y: f32 = 64.0;
/// The symbol before the title, on a square tinted with the accent, and
/// the space after it.
const BADGE: f32 = 36.0;
const BADGE_RADIUS: f32 = 8.0;
const BADGE_GAP: f32 = 14.0;
/// Between the title and what it says.
const TITLE_GAP: f32 = 14.0;
/// From the buttons to the bottom of the card.
pub const PAD_BOTTOM: f32 = 28.0;
pub const BUTTON_GAP: f32 = 8.0;
/// How far the text slides as it goes and comes.
pub const SLIDE: f32 = 24.0;
/// The least room kept between the window and the screen's edges.
const SCREEN_GAP: f32 = 16.0;
/// How much larger than its size in DIPs the window opens, all of it alike.
const ENLARGE: f32 = 1.25;

// Durations, in ms.
pub const SHOW_MS: u64 = 360;
pub const LEAVE_MS: u64 = 150;
pub const ENTER_MS: u64 = 300;
/// Each line of the text comes in this long after the one above it.
const STAGGER_MS: u64 = 40;
pub const HOVER_MS: u64 = 100;
const PRESS_MS: u64 = 90;
pub const FOCUS_MS: u64 = 150;

/// "Cancel" in Segoe Fluent Icons and Segoe MDL2 Assets.
const CLOSE_GLYPH: &str = "\u{E8BB}";

/// The close button at the card's top right.
pub struct Close {
    pub root: ContainerVisual,
    pub at: Rect,
    /// The soft fill that lights under the pointer.
    pub hover: SpriteVisual,
}

/// The scale of a window with a `card_w` × `card_h` card on a monitor of
/// `scale` pixels per DIP, enlarged by `ENLARGE`: smaller, should it not
/// fit in the `work` area.
pub fn fitted(work: RECT, scale: f32, card_w: f32, card_h: f32) -> f32 {
    let w = (work.right - work.left) as f32 / (card_w + 2.0 * SCREEN_GAP);
    let h = (work.bottom - work.top) as f32 / (card_h + 2.0 * SCREEN_GAP);
    (scale * ENLARGE).min(w).min(h)
}

/// Sizes `frame`'s `card_w` × `card_h` card for `s` pixels per DIP.
/// Returns the window's size in pixels: the card, and room for its shadow.
pub fn fit(frame: &Frame, anim: &Animator, s: f32, card_w: f32, card_h: f32) -> Result<(i32, i32)> {
    let m = (MARGIN * s).round();
    let (w, h) = ((card_w * s).round(), (card_h * s).round());
    frame.resize(anim, w, h, v3(m, m), s, false)?;
    Ok(((w + 2.0 * m) as i32, (h + 2.0 * m) as i32))
}

/// The middle of a `card_w` × `card_h` card at `s` pixels per DIP, in the
/// window: what it grows from.
pub fn centre(s: f32, card_w: f32, card_h: f32) -> Vector3 {
    let m = (MARGIN * s).round();
    Vector3 {
        X: m + (card_w * s).round() / 2.0,
        Y: m + (card_h * s).round() / 2.0,
        Z: 0.0,
    }
}

/// A point in window pixels, relative to the card's top left, at `s`
/// pixels per DIP.
pub fn card_point(s: f32, x: i32, y: i32) -> (f32, f32) {
    let m = (MARGIN * s).round();
    (x as f32 - m, y as f32 - m)
}

/// Brings each line of `text` in from `from` away, a beat after the one
/// above it, starting after `delay`.
pub fn bring_in(anim: &Animator, text: &ContainerVisual, from: Vector3, delay: u64) -> Result<()> {
    for (i, line) in text.Children()?.into_iter().enumerate() {
        let rest = line.Offset()?;
        let delay = delay + i as u64 * STAGGER_MS;
        Animator::stop(&line, &["Opacity", "Offset"]);
        line.SetOpacity(0.0)?;
        line.SetOffset(rest + from)?;
        anim.scalar(&line, "Opacity", 1.0, ENTER_MS, delay, &anim.decelerate)?;
        anim.vector3(&line, "Offset", rest, ENTER_MS, delay, &anim.decelerate)?;
    }
    Ok(())
}

/// Dims `button` while it's held `down`, or brings it back.
pub fn show_pressed(anim: &Animator, button: &Button, down: bool) -> Result<()> {
    let opacity = if down { 0.8 } else { 1.0 };
    anim.scalar(
        &button.root,
        "Opacity",
        opacity,
        PRESS_MS,
        0,
        &anim.decelerate,
    )
}

/// The pane at `at` in `parent` that holds the picture, its corners
/// rounded, with a hairline round it.
pub fn picture_pane(
    c: &Compositor,
    r: &Renderer,
    t: &Theme,
    s: f32,
    parent: &ContainerVisual,
    at: Rect,
) -> Result<ContainerVisual> {
    let radius = (PICTURE_RADIUS * s).round();
    let pane = container(c, parent, at.at())?;
    pane.SetSize(at.size())?;
    let corners = c.CreateRoundedRectangleGeometry()?;
    corners.SetSize(at.size())?;
    corners.SetCornerRadius(v2(radius, radius))?;
    pane.SetClip(&c.CreateGeometricClipWithGeometry(&corners)?)?;
    // A shade darker or lighter than the picture, whatever it shows.
    let edge = if t.dark {
        argb(0x24, 0xFF, 0xFF, 0xFF)
    } else {
        argb(0x24, 0x00, 0x00, 0x00)
    };
    outline(c, r, parent, at, radius, edge)?;
    Ok(pane)
}

/// The close button at the top right of a card `card_w` DIPs wide, in
/// `parent`.
pub fn close_button(
    c: &Compositor,
    r: &Renderer,
    t: &Theme,
    s: f32,
    parent: &ContainerVisual,
    card_w: f32,
) -> Result<Close> {
    let px = |dip: f32| (dip * s).round();
    let size = px(CLOSE_SIZE);
    let at = Rect {
        x: px(card_w) - px(CLOSE_INSET) - size,
        y: px(CLOSE_INSET),
        w: size,
        h: size,
    };
    let root = container(c, parent, at.at())?;
    let face = Rect {
        w: size,
        h: size,
        ..Rect::default()
    };
    let (hover, _) = rounded(c, r, &root, face, px(CLOSE_RADIUS), t.highlight())?;
    hover.SetOpacity(0.0)?;
    let glyph = Text::new(c, &root)?;
    let cross = r.layout_line(CLOSE_GLYPH, Face::Chrome, Ink::Primary, CLOSE_SIZE)?;
    let glyph_size = glyph.show(r, &cross, s, t)?;
    glyph
        .visual
        .SetOffset(v3(0.0, ((size - glyph_size.Y) / 2.0).round()))?;
    Ok(Close { root, at, hover })
}

/// The `title`, `x` DIPs from the card's left and at most `width` wide,
/// after the `glyph` on a square tinted with the accent, or in red when it
/// says something went wrong, in `text`. Returns where what goes under it
/// starts, in pixels.
// The drawing context (compositor, renderer, theme, scale) every builder in
// `ui` takes, plus where the title goes and what it says.
#[allow(clippy::too_many_arguments)]
pub fn heading(
    c: &Compositor,
    r: &Renderer,
    t: &Theme,
    s: f32,
    text: &ContainerVisual,
    x: f32,
    width: f32,
    title: &str,
    glyph: &str,
    critical: bool,
) -> Result<f32> {
    let px = |dip: f32| (dip * s).round();
    let y = px(TITLE_Y);
    // The symbol, centred on the capitals of the title's first line.
    let indent = BADGE + BADGE_GAP;
    let line = r.layout_line(title, Face::Display, Ink::Primary, width - indent)?;
    let b = px(BADGE);
    let badge = container(
        c,
        text,
        v3(px(x), y + (line.cap_middle() * s - b / 2.0).round()),
    )?;
    let face = Rect {
        w: b,
        h: b,
        ..Rect::default()
    };
    let (hue, ink) = if critical {
        let red = t.critical();
        let channel = |v: f32| (v * 255.0).round() as u8;
        let red = Color {
            A: 0xFF,
            R: channel(red.r),
            G: channel(red.g),
            B: channel(red.b),
        };
        (red, Ink::Critical)
    } else {
        (t.accent, Ink::Accent)
    };
    let tint = argb(if t.dark { 0x33 } else { 0x24 }, hue.R, hue.G, hue.B);
    rounded(c, r, &badge, face, px(BADGE_RADIUS), tint)?;
    let symbol = Text::new(c, &badge)?;
    let shape = r.layout_line(glyph, Face::LargeButton, ink, BADGE)?;
    let size = symbol.show(r, &shape, s, t)?;
    symbol
        .visual
        .SetOffset(v3(0.0, ((b - size.Y) / 2.0).round()))?;

    let heading = Text::new(c, text)?;
    let size = heading.show(r, &line, s, t)?;
    heading.visual.SetOffset(v3(px(x + indent), y))?;
    Ok(y + size.Y.max(b) + px(TITLE_GAP))
}
