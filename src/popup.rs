//! The definition card: a borderless, non-activating, top-most window whose
//! content is a Windows.UI.Composition visual tree. Blur comes from the host
//! backdrop brush; every animation runs on the system compositor, so motion
//! stays smooth no matter what our UI thread is doing.

use windows::core::{Interface, Result, HSTRING, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::*;
use windows_numerics::Vector3;

use crate::render::{Body, Face, Ink, Link, Mark, Renderer, Target, WORD_SIZE};
use crate::searchbox::SearchBox;
use crate::theme::Theme;
use crate::ui::{
    self, attach, blink, monitor_at, scale, text_sprite, ts, v2, v3, Animator, Bursts, Frame, Glide, Push,
    ScrollFade, MARGIN, RESIZE_MS,
};
use crate::win::Window;

// Geometry, in DIPs.
pub(crate) const CARD_W: f32 = 384.0;
const MAX_CARD_H: f32 = 460.0;
pub(crate) const PAD_X: f32 = 22.0;
pub(crate) const PAD_TOP: f32 = 18.0;
const HEADER_GAP: f32 = 12.0;
const PAD_BOTTOM: f32 = 18.0;
const ANCHOR_GAP: f32 = 10.0;
const SCREEN_GAP: f32 = 8.0;
const BACK_SIZE: f32 = 28.0;
/// The back button's left edge: it reaches a little into the padding, so
/// its arrow lines up with the text below.
const BACK_X: f32 = PAD_X - 7.0;
/// How far the word moves right to make room for the back button.
const BACK_INDENT: f32 = BACK_X + BACK_SIZE + 6.0 - PAD_X;
const BACK_RADIUS: f32 = 5.0;
/// How far the back button slides in from as it appears.
const BACK_SLIDE: f32 = 10.0;
/// "Back" in Segoe Fluent Icons and Segoe MDL2 Assets.
const BACK_GLYPH: &str = "\u{E72B}";
const BACK_TIP: &str = "Back (Backspace)";
/// The translate button's left edge: mirrors the back button's, at the right.
const TRANSLATE_X: f32 = CARD_W - BACK_X - BACK_SIZE;
/// How much narrower the heading is beside the translate button.
const TRANSLATE_ROOM: f32 = CARD_W - PAD_X - TRANSLATE_X + 6.0;
/// "Translate" in Segoe Fluent Icons and Segoe MDL2 Assets.
const TRANSLATE_GLYPH: &str = "\u{E8C1}";
const TRANSLATE_TIP: &str = "Translate (Ctrl+T)";
const SWAP_TIP: &str = "Swap languages";
/// The swap button's tooltip's tool, after the back and translate buttons'.
const SWAP_TOOL: usize = 3;
/// The ring around the link Tab moved to, as on Windows 11 controls.
const FOCUS_STROKE: f32 = 2.0;
const FOCUS_RADIUS: f32 = 4.0;
/// How far, in pixels, rings can reach past the definition's edges.
const RING_ROOM: f32 = 16.0;
/// Room kept above and below a link Tab scrolls into view, clear of the
/// body's fades.
const FOCUS_MARGIN: f32 = 20.0;

// Durations, in ms.
pub const SHOW_MS: u64 = 340;
const FADE_IN_MS: u64 = 160;
const SCROLL_MS: u64 = 140;
/// How long what the card showed takes to fade out as the search box comes in.
const MAKE_WAY_MS: u64 = 120;

/// Where the window goes for a card next to `anchor` inside the monitor's
/// `work` area, both in screen pixels, at `scale` pixels per DIP. Returns the
/// window rectangle and whether the card grows upwards from the anchor (when
/// there's more room above it than below, unless it must grow `downward`).
///
/// The window always has room for the tallest card plus the shadow margin,
/// so resizing never moves the window itself.
fn placement(anchor: RECT, work: RECT, scale: f32, downward: bool) -> (RECT, bool) {
    let px = |dip: f32| (dip * scale).round() as i32;
    let (card_w, max_h, m) = (px(CARD_W), px(MAX_CARD_H), px(MARGIN));
    let (gap, edge) = (px(ANCHOR_GAP), px(SCREEN_GAP));

    let card_x = (anchor.left - px(PAD_X)).clamp(
        work.left + edge,
        (work.right - edge - card_w).max(work.left + edge),
    );
    let room_below = work.bottom - edge - (anchor.bottom + gap);
    let room_above = (anchor.top - gap) - (work.top + edge);
    let grow_up = !downward && room_below < max_h && room_above > room_below;

    let window_h = max_h + 2 * m;
    let window_y = if grow_up {
        let card_bottom = (anchor.top - gap).max(work.top + edge + max_h);
        card_bottom + m - window_h
    } else {
        let card_top = (anchor.bottom + gap)
            .min(work.bottom - edge - max_h)
            .max(work.top + edge);
        card_top - m
    };
    let window = RECT {
        left: card_x - m,
        top: window_y,
        right: card_x + card_w + m,
        bottom: window_y + window_h,
    };
    (window, grow_up)
}

/// About half the empty search box's height, in DIPs: how far above the
/// screen's middle a centred one starts, so that it sits in the middle.
const CENTRED_RISE: f32 = 68.0;

/// An anchor that opens a search box in the middle of the screen `near`
/// (screen pixels) is on, for when there's no caret to open it at.
pub fn centred_anchor(near: POINT) -> RECT {
    let (work, scale) = monitor_at(near);
    centred_in(work, scale)
}

fn centred_in(work: RECT, s: f32) -> RECT {
    let px = |dip: f32| (dip * s).round() as i32;
    // `placement` puts the card's text at the anchor's left, and its top
    // just under the anchor.
    let left = (work.left + work.right) / 2 - px(CARD_W) / 2 + px(PAD_X);
    let bottom = (work.top + work.bottom) / 2 - px(CENTRED_RISE) - px(ANCHOR_GAP);
    RECT {
        left,
        top: bottom - 1,
        right: left + 1,
        bottom,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Transition {
    /// No animation (the card is appearing).
    Instant,
    /// Different content: the body fades in, the card resizes.
    Replace,
    /// The same entry, refined: no fade, scroll position kept.
    Refine,
}

pub struct Popup {
    pub hwnd: HWND,
    _target: DesktopWindowTarget,
    renderer: Renderer,
    anim: Animator,

    /// Everything on the window; moves and fades as one when showing and hiding.
    root: ContainerVisual,
    frame: Frame,
    header: Header,
    content: Content,
    loader: Loader,
    scroll_chrome: ScrollChrome,
    back: IconButton,
    translate: IconButton,
    search: SearchBox,

    theme: Theme,
    scale: f32,
    grow_up: bool,
    /// Window position and size, physical pixels.
    window: RECT,
    /// Card height and header area height, physical pixels.
    card_h: f32,
    header_h: f32,
    /// How far the word is indented for the back button, in DIPs.
    header_indent: f32,
    /// Keeps the clickable area at its largest while the card's height
    /// keeps changing, as setting it repaints the window.
    region_hold: bool,
    /// The clickable area last set, so an unchanged one isn't set again,
    /// which repaints the whole window.
    region: std::cell::Cell<(i32, i32, i32, i32, i32)>,
    /// What the heading was last drawn from, to tell a refinement that
    /// leaves it alone.
    header_key: (String, Option<String>, u32),
    body_h: f32,
    scroll: f32,
    scroll_anim: Option<Glide>,
    wheel_bursts: Bursts,
    /// Where the pointer last was on the window, in window pixels.
    pointer: Option<(i32, i32)>,
    pub visible: bool,
    /// Last, so it goes after everything shown on it.
    _window: Window,
}

// ---------------------------------------------------------------------------
// Parts of the card

/// The word and its pronunciation. `back` holds the previous rendering
/// while it cross-fades to the new one.
struct Header {
    visual: SpriteVisual,
    brush: CompositionSurfaceBrush,
    back: SpriteVisual,
    back_brush: CompositionSurfaceBrush,
    surface: Option<CompositionDrawingSurface>,
}

impl Header {
    fn new(c: &Compositor, card: &ContainerVisual) -> Result<Header> {
        let (back, back_brush) = text_sprite(c)?;
        back.SetOpacity(0.0)?;
        card.Children()?.InsertAtTop(&back)?;
        let (visual, brush) = text_sprite(c)?;
        card.Children()?.InsertAtTop(&visual)?;
        Ok(Header {
            visual,
            brush,
            back,
            back_brush,
            surface: None,
        })
    }
}

/// The scrolling part of the card: a clipped host below the header, and the
/// rendered definition inside it.
struct Content {
    host: ContainerVisual,
    /// Shows `host`, faded at the ends it scrolls towards.
    fade: ScrollFade,
    visual: SpriteVisual,
    brush: CompositionSurfaceBrush,
    links: Vec<Link>,
    /// Rings the link Tab moved to; inside `visual`, so it scrolls with it.
    focus: ShapeVisual,
    focus_brush: CompositionColorBrush,
    /// The caret in the language box being typed in; inside `visual` too.
    caret: SpriteVisual,
    caret_brush: CompositionColorBrush,
    /// The first of the focused link's rectangles in `links`.
    focused: Option<usize>,
    /// Lightens the button swapping a translation's languages under the
    /// pointer; inside `visual` too.
    swap: ShapeVisual,
    swap_geometry: CompositionRoundedRectangleGeometry,
    swap_brush: CompositionColorBrush,
    swap_hovered: bool,
    /// Says what that button does. Owned by the card's window.
    swap_tip: Option<HWND>,
    /// The area of the window it shows over, last set.
    swap_tip_rect: RECT,
}

impl Content {
    fn new(c: &Compositor, card: &ContainerVisual, hwnd: HWND) -> Result<Content> {
        let host = c.CreateContainerVisual()?;
        host.SetClip(&c.CreateInsetClip()?)?;
        let (visual, brush) = text_sprite(c)?;
        host.Children()?.InsertAtTop(&visual)?;
        let swap = c.CreateShapeVisual()?;
        swap.SetRelativeSizeAdjustment(v2(1.0, 1.0))?;
        swap.SetOpacity(0.0)?;
        let swap_geometry = c.CreateRoundedRectangleGeometry()?;
        let swap_shape = c.CreateSpriteShapeWithGeometry(&swap_geometry)?;
        let swap_brush = c.CreateColorBrush()?;
        swap_shape.SetFillBrush(&swap_brush)?;
        swap.Shapes()?.Append(&swap_shape)?;
        visual.Children()?.InsertAtTop(&swap)?;
        // Reaches past the definition's edges, which would clip the rings
        // of links at them.
        let focus = c.CreateShapeVisual()?;
        focus.SetRelativeSizeAdjustment(v2(1.0, 1.0))?;
        focus.SetOffset(v3(-RING_ROOM, -RING_ROOM))?;
        focus.SetSize(v2(2.0 * RING_ROOM, 2.0 * RING_ROOM))?;
        visual.Children()?.InsertAtTop(&focus)?;
        let caret = c.CreateSpriteVisual()?;
        let caret_brush = c.CreateColorBrush()?;
        caret.SetBrush(&caret_brush)?;
        caret.SetOpacity(0.0)?;
        visual.Children()?.InsertAtTop(&caret)?;
        let fade = ScrollFade::new(c, card, &host)?;
        Ok(Content {
            host,
            fade,
            visual,
            brush,
            links: Vec::new(),
            focus,
            focus_brush: c.CreateColorBrush()?,
            caret,
            caret_brush,
            focused: None,
            swap,
            swap_geometry,
            swap_brush,
            swap_hovered: false,
            swap_tip: create_tooltip(hwnd, SWAP_TIP, SWAP_TOOL),
            swap_tip_rect: RECT::default(),
        })
    }

    /// The swap button's link, if the card has one.
    fn swap_link(&self) -> Option<&Link> {
        self.links.iter().find(|l| l.target == Target::Swap)
    }

    /// Puts the caret at `mark`, blinking, or takes it away.
    fn show_caret(&self, c: &Compositor, mark: Option<Mark>, s: f32) -> Result<()> {
        Animator::stop(&self.caret, &["Opacity"]);
        let Some(m) = mark else {
            return self.caret.SetOpacity(0.0);
        };
        self.caret
            .SetSize(v2((m.width * s).round().max(1.0), (m.height * s).round()))?;
        self.caret
            .SetOffset(v3((m.x * s).round(), (m.y * s).round()))?;
        blink(c, &self.caret)
    }

    /// The rectangles of the link starting at `start`: more than one when
    /// a synonym wraps.
    fn rects(&self, start: usize) -> impl Iterator<Item = &Link> {
        let target = &self.links[start].target;
        self.links[start..]
            .iter()
            .take_while(move |l| &l.target == target)
    }

    /// Where each link starts in `links`, in reading order.
    fn stops(&self) -> Vec<usize> {
        (0..self.links.len())
            .filter(|&i| i == 0 || self.links[i - 1].target != self.links[i].target)
            .collect()
    }
}

/// A glyph button level with the word: the arrow left of it, shown once a
/// synonym has been followed, and the translate button right of it.
struct IconButton {
    visual: ContainerVisual,
    /// Inside `visual`, which keeps the button's place: slides in and out.
    slide: ContainerVisual,
    hover: SpriteVisual,
    hover_clip: CompositionRoundedRectangleGeometry,
    hover_brush: CompositionColorBrush,
    glyph: SpriteVisual,
    glyph_brush: CompositionSurfaceBrush,
    /// Rings the button while Tab is on it.
    ring: ShapeVisual,
    ring_shape: CompositionSpriteShape,
    ring_clip: CompositionRoundedRectangleGeometry,
    ring_brush: CompositionColorBrush,
    /// Owned by the card's window, which destroys it.
    tooltip: Option<HWND>,
    /// The tooltip's tool, unique among the card's buttons.
    tool: usize,
    icon: &'static str,
    face: Face,
    ink: Ink,
    /// Left edge in the card, in DIPs.
    x: f32,
    /// Where it slides in from, in DIPs right of its place.
    slide_from: f32,
    shown: bool,
    hovered: bool,
    focused: bool,
    /// Top of the button in the card, physical pixels.
    top: f32,
}

impl IconButton {
    #[allow(clippy::too_many_arguments)]
    fn new(
        c: &Compositor,
        card: &ContainerVisual,
        hwnd: HWND,
        icon: &'static str,
        face: Face,
        ink: Ink,
        tip: &str,
        tool: usize,
        x: f32,
        slide_from: f32,
    ) -> Result<IconButton> {
        let visual = c.CreateContainerVisual()?;
        visual.SetOpacity(0.0)?;
        card.Children()?.InsertAtTop(&visual)?;
        let slide = c.CreateContainerVisual()?;
        visual.Children()?.InsertAtTop(&slide)?;
        let hover = c.CreateSpriteVisual()?;
        let hover_clip = c.CreateRoundedRectangleGeometry()?;
        hover.SetClip(&c.CreateGeometricClipWithGeometry(&hover_clip)?)?;
        let hover_brush = c.CreateColorBrush()?;
        hover.SetBrush(&hover_brush)?;
        hover.SetOpacity(0.0)?;
        slide.Children()?.InsertAtTop(&hover)?;
        let (glyph, glyph_brush) = text_sprite(c)?;
        slide.Children()?.InsertAtTop(&glyph)?;
        let ring = c.CreateShapeVisual()?;
        let ring_clip = c.CreateRoundedRectangleGeometry()?;
        let ring_shape = c.CreateSpriteShapeWithGeometry(&ring_clip)?;
        let ring_brush = c.CreateColorBrush()?;
        ring_shape.SetStrokeBrush(&ring_brush)?;
        ring.Shapes()?.Append(&ring_shape)?;
        ring.SetOpacity(0.0)?;
        slide.Children()?.InsertAtTop(&ring)?;
        Ok(IconButton {
            visual,
            slide,
            hover,
            hover_clip,
            hover_brush,
            glyph,
            glyph_brush,
            ring,
            ring_shape,
            ring_clip,
            ring_brush,
            tooltip: create_tooltip(hwnd, tip, tool),
            tool,
            icon,
            face,
            ink,
            x,
            slide_from,
            shown: false,
            hovered: false,
            focused: false,
            top: 0.0,
        })
    }

    fn apply_theme(&self, renderer: &Renderer, t: &Theme, s: f32) -> Result<()> {
        self.hover_brush.SetColor(t.highlight())?;
        let size = v2(BACK_SIZE * s, BACK_SIZE * s);
        self.visual.SetSize(size)?;
        self.hover.SetSize(size)?;
        self.hover_clip.SetSize(size)?;
        self.hover_clip
            .SetCornerRadius(v2(BACK_RADIUS * s, BACK_RADIUS * s))?;
        // Inside the button, the stroke centred on the ring's edge.
        let stroke = (FOCUS_STROKE * s).round();
        self.ring_brush.SetColor(t.focus())?;
        self.ring.SetSize(size)?;
        self.ring_shape.SetStrokeThickness(stroke)?;
        self.ring_clip.SetOffset(v2(stroke / 2.0, stroke / 2.0))?;
        self.ring_clip
            .SetSize(v2(BACK_SIZE * s - stroke, BACK_SIZE * s - stroke))?;
        self.ring_clip
            .SetCornerRadius(v2(BACK_RADIUS * s, BACK_RADIUS * s))?;
        let layout = renderer.layout_line(self.icon, self.face, self.ink, BACK_SIZE)?;
        self.glyph_brush
            .SetSurface(&renderer.draw(&layout, s, t)?)?;
        self.glyph
            .SetSize(v2(layout.width * s, layout.height * s))?;
        self.glyph
            .SetOffset(v3(0.0, ((BACK_SIZE - layout.height) / 2.0 * s).round()))
    }

    /// Fades the button in, sliding from [`IconButton::slide_from`], or out.
    fn set_shown(&mut self, anim: &Animator, shown: bool, s: f32) -> Result<()> {
        if shown == self.shown {
            return Ok(());
        }
        self.shown = shown;
        let away = v3(self.slide_from * s, 0.0);
        if shown {
            // Slides in alongside the word making room for it.
            Animator::stop(&self.slide, &["Offset"]);
            self.slide.SetOffset(away)?;
            anim.scalar(&self.visual, "Opacity", 1.0, RESIZE_MS, 0, &anim.fade)?;
            let home = v3(0.0, 0.0);
            anim.vector3(&self.slide, "Offset", home, RESIZE_MS, 0, &anim.decelerate)?;
        } else {
            // Gone before the word slides over where it was.
            anim.scalar(&self.visual, "Opacity", 0.0, 100, 0, &anim.decelerate)?;
            anim.vector3(&self.slide, "Offset", away, 150, 0, &anim.accelerate)?;
            self.set_hovered(anim, false)?;
            self.set_focused(false)?;
        }
        Ok(())
    }

    fn set_hovered(&mut self, anim: &Animator, hovered: bool) -> Result<()> {
        if hovered == self.hovered {
            return Ok(());
        }
        self.hovered = hovered;
        let opacity = if hovered { 1.0 } else { 0.0 };
        anim.scalar(&self.hover, "Opacity", opacity, 100, 0, &anim.decelerate)
    }

    fn set_focused(&mut self, focused: bool) -> Result<()> {
        self.focused = focused;
        self.ring.SetOpacity(if focused { 1.0 } else { 0.0 })
    }

    /// Back at rest, shown or not without animating, for a new card.
    fn reset(&mut self) -> Result<()> {
        Animator::stop(&self.visual, &["Opacity"]);
        Animator::stop(&self.hover, &["Opacity"]);
        Animator::stop(&self.slide, &["Offset"]);
        self.visual.SetOpacity(if self.shown { 1.0 } else { 0.0 })?;
        self.hover.SetOpacity(0.0)?;
        self.slide.SetOffset(v3(0.0, 0.0))?;
        self.hovered = false;
        self.set_focused(false)
    }

    /// Moves the tooltip's area to `rect`, in window pixels; an empty one turns it off.
    fn set_tooltip_rect(&self, owner: HWND, rect: RECT) {
        if let Some(tooltip) = self.tooltip {
            set_tooltip_rect(tooltip, owner, self.tool, rect);
        }
    }
}

/// Moves the area of `owner` that `tooltip`'s `tool` shows over to `rect`,
/// in client pixels. An empty one hides it.
pub fn set_tooltip_rect(tooltip: HWND, owner: HWND, tool: usize, rect: RECT) {
    let info = TTTOOLINFOW {
        cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
        hwnd: owner,
        uId: tool,
        rect,
        ..Default::default()
    };
    unsafe {
        SendMessageW(
            tooltip,
            TTM_NEWTOOLRECTW,
            None,
            Some(LPARAM(&info as *const _ as isize)),
        );
        if rect.right <= rect.left {
            SendMessageW(tooltip, TTM_POP, None, None);
        }
    }
}

fn contains(r: RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

/// A system tooltip showing `text` over an area of `owner`, initially empty.
/// `tool` tells it apart from the owner's other tooltips.
pub fn create_tooltip(owner: HWND, text: &str, tool: usize) -> Option<HWND> {
    unsafe {
        let controls = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_WIN95_CLASSES,
        };
        let _ = InitCommonControlsEx(&controls);
        // Always: the card never becomes the active window.
        let tooltip = CreateWindowExW(
            WS_EX_TOPMOST,
            TOOLTIPS_CLASSW,
            None,
            WINDOW_STYLE(TTS_ALWAYSTIP | TTS_NOPREFIX),
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            Some(owner),
            None,
            None,
            None,
        )
        .ok()?;
        // The tooltip keeps its own copy.
        let mut wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        let info = TTTOOLINFOW {
            cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
            // Watches the owner's mouse messages itself.
            uFlags: TTF_SUBCLASS,
            hwnd: owner,
            uId: tool,
            lpszText: PWSTR(wide.as_mut_ptr()),
            ..Default::default()
        };
        SendMessageW(
            tooltip,
            TTM_ADDTOOLW,
            None,
            Some(LPARAM(&info as *const _ as isize)),
        );
        Some(tooltip)
    }
}

/// Three pulsing dots, shown while an online lookup runs.
struct Loader {
    visual: ContainerVisual,
    dots: Vec<SpriteVisual>,
    brush: CompositionColorBrush,
    running: bool,
}

impl Loader {
    fn new(c: &Compositor, host: &ContainerVisual, theme: &Theme) -> Result<Loader> {
        let visual = c.CreateContainerVisual()?;
        host.Children()?.InsertAtTop(&visual)?;
        let brush = c.CreateColorBrushWithColor(theme.accent)?;
        let mut dots = Vec::new();
        for _ in 0..3 {
            let dot = c.CreateSpriteVisual()?;
            dot.SetBrush(&brush)?;
            let ellipse = c.CreateEllipseGeometry()?;
            dot.SetClip(&c.CreateGeometricClipWithGeometry(&ellipse)?)?;
            visual.Children()?.InsertAtTop(&dot)?;
            dots.push(dot);
        }
        Ok(Loader {
            visual,
            dots,
            brush,
            running: false,
        })
    }

    fn start(&self, anim: &Animator, s: f32) -> Result<()> {
        let size = 7.0 * s;
        Animator::stop(&self.visual, &["Opacity"]);
        self.visual.SetOpacity(1.0)?;
        self.visual.SetOffset(v3(PAD_X * s, 6.0 * s))?;
        for (i, dot) in self.dots.iter().enumerate() {
            dot.SetSize(v2(size, size))?;
            dot.SetOffset(v3(i as f32 * size * 1.9, 0.0))?;
            dot.SetCenterPoint(Vector3 {
                X: size / 2.0,
                Y: size / 2.0,
                Z: 0.0,
            })?;
            if let Ok(clip) = dot.Clip()?.cast::<CompositionGeometricClip>() {
                if let Ok(ellipse) = clip.Geometry()?.cast::<CompositionEllipseGeometry>() {
                    ellipse.SetCenter(v2(size / 2.0, size / 2.0))?;
                    ellipse.SetRadius(v2(size / 2.0, size / 2.0))?;
                }
            }
            if !ui::motion() {
                dot.SetOpacity(0.6)?;
                dot.SetScale(scale(1.0))?;
                continue;
            }
            let pulse = anim.compositor.CreateScalarKeyFrameAnimation()?;
            pulse.InsertKeyFrame(0.0, 0.25)?;
            pulse.InsertKeyFrameWithEasingFunction(0.3, 1.0, &anim.decelerate)?;
            pulse.InsertKeyFrameWithEasingFunction(0.7, 0.25, &anim.decelerate)?;
            pulse.InsertKeyFrame(1.0, 0.25)?;
            pulse.SetDuration(ts(1100))?;
            pulse.SetDelayTime(ts(i as u64 * 160))?;
            pulse.SetIterationBehavior(AnimationIterationBehavior::Forever)?;
            dot.SetOpacity(0.25)?;
            dot.StartAnimation(&HSTRING::from("Opacity"), &pulse)?;

            let bounce = anim.compositor.CreateVector3KeyFrameAnimation()?;
            let small = scale(0.8);
            bounce.InsertKeyFrame(0.0, small)?;
            bounce.InsertKeyFrameWithEasingFunction(0.3, scale(1.0), &anim.decelerate)?;
            bounce.InsertKeyFrameWithEasingFunction(0.7, small, &anim.decelerate)?;
            bounce.InsertKeyFrame(1.0, small)?;
            bounce.SetDuration(ts(1100))?;
            bounce.SetDelayTime(ts(i as u64 * 160))?;
            bounce.SetIterationBehavior(AnimationIterationBehavior::Forever)?;
            dot.SetScale(small)?;
            dot.StartAnimation(&HSTRING::from("Scale"), &bounce)?;
        }
        Ok(())
    }

    fn stop(&self) {
        for dot in &self.dots {
            Animator::stop(dot, &["Opacity", "Scale"]);
        }
        let _ = self.visual.SetOpacity(0.0);
    }
}

/// The scrollbar thumb, shown only when the content scrolls.
struct ScrollChrome {
    thumb: SpriteVisual,
    thumb_clip: CompositionRoundedRectangleGeometry,
    thumb_brush: CompositionColorBrush,
}

impl ScrollChrome {
    fn new(c: &Compositor, card: &ContainerVisual) -> Result<ScrollChrome> {
        let thumb = c.CreateSpriteVisual()?;
        let thumb_clip = c.CreateRoundedRectangleGeometry()?;
        thumb.SetClip(&c.CreateGeometricClipWithGeometry(&thumb_clip)?)?;
        let thumb_brush = c.CreateColorBrush()?;
        thumb.SetBrush(&thumb_brush)?;
        thumb.SetOpacity(0.0)?;
        card.Children()?.InsertAtTop(&thumb)?;

        Ok(ScrollChrome {
            thumb,
            thumb_clip,
            thumb_brush,
        })
    }

    fn apply_theme(&self, t: &Theme) -> Result<()> {
        self.thumb_brush.SetColor(t.thumb())
    }
}

// ---------------------------------------------------------------------------

impl Popup {
    pub fn new(window: Window, compositor: Compositor, theme: Theme) -> Result<Popup> {
        let hwnd = window.hwnd();
        let target = attach(hwnd, &compositor)?;
        let renderer = Renderer::new(&compositor)?;
        let c = &compositor;

        // Creation order is stacking order: later visuals draw on top.
        let root = c.CreateContainerVisual()?;
        target.SetRoot(&root)?;
        let frame = Frame::new(c, &root, &theme)?;
        let header = Header::new(c, &frame.card)?;
        let back = IconButton::new(
            c,
            &frame.card,
            hwnd,
            BACK_GLYPH,
            Face::Button,
            Ink::Primary,
            BACK_TIP,
            1,
            BACK_X,
            -BACK_SLIDE,
        )?;
        // Its glyph's detail wants it larger than the arrow, and it's
        // colored like the links, as it leads somewhere too.
        let translate = IconButton::new(
            c,
            &frame.card,
            hwnd,
            TRANSLATE_GLYPH,
            Face::LargeButton,
            Ink::Accent,
            TRANSLATE_TIP,
            2,
            TRANSLATE_X,
            0.0,
        )?;
        let content = Content::new(c, &frame.card, hwnd)?;
        let loader = Loader::new(c, &content.host, &theme)?;
        let scroll_chrome = ScrollChrome::new(c, &frame.card)?;
        let search = SearchBox::new(c, &frame.card)?;

        Ok(Popup {
            hwnd,
            _target: target,
            renderer,
            anim: Animator::new(compositor)?,
            root,
            frame,
            header,
            content,
            loader,
            scroll_chrome,
            back,
            translate,
            search,
            theme,
            scale: 1.0,
            grow_up: false,
            window: RECT::default(),
            card_h: 0.0,
            header_h: 0.0,
            header_indent: 0.0,
            region_hold: false,
            region: Default::default(),
            header_key: Default::default(),
            body_h: 0.0,
            scroll: 0.0,
            scroll_anim: None,
            wheel_bursts: Bursts::default(),
            pointer: None,
            visible: false,
            _window: window,
        })
    }

    // -----------------------------------------------------------------------
    // Public API

    /// Positions the card next to `anchor` (screen pixels) and animates it in.
    pub fn show(
        &mut self,
        anchor: RECT,
        theme: Theme,
        word: &str,
        phonetic: Option<&str>,
        body: Body,
    ) -> Result<()> {
        self.prepare(anchor, theme, false)?;
        self.set_content(word, phonetic, body, Transition::Instant)?;
        self.appear()
    }

    /// Shows the search box under `title`. An open card turns into it in
    /// place; otherwise a new one appears next to `anchor`.
    pub fn show_search(
        &mut self,
        anchor: RECT,
        theme: Theme,
        title: &str,
        placeholder: &str,
    ) -> Result<()> {
        if self.visible {
            return self.enter_search(title, placeholder, true);
        }
        // Its top stays put, so the field doesn't move as suggestions come and go.
        self.prepare(anchor, theme, true)?;
        self.enter_search(title, placeholder, false)?;
        self.appear()
    }

    /// Shows `text` in the search box, the caret `at` UTF-16 units in.
    pub fn set_query(&mut self, text: &str, at: u32) -> Result<()> {
        self.search
            .set_query(&self.renderer, &self.anim, &self.theme, text, at)
    }

    /// Changes the search box's heading.
    pub fn set_search_title(&mut self, title: &str) -> Result<()> {
        self.search
            .set_title(&self.renderer, &self.anim, &self.theme, title)
    }

    /// Shakes the search field: what's typed can't be looked up.
    pub fn shake_search(&self) -> Result<()> {
        self.search.shake(&self.anim)
    }

    /// Whether a point in window pixels is on the search field.
    pub fn search_at(&self, x: i32, y: i32) -> bool {
        let (x, y) = self.card_point(x, y);
        self.search.field_at(x, y)
    }

    /// Shows `items` (each a word, what it's a form of, how many of its
    /// first characters to bold as typed, and the code of its language when
    /// there are several) under the search field, or else `note`, resizing
    /// the card to fit.
    pub fn set_suggestions(
        &mut self,
        items: &[(&str, Option<&str>, usize, Option<&str>)],
        note: Option<&str>,
    ) -> Result<()> {
        let (renderer, anim, theme) = (&self.renderer, &self.anim, &self.theme);
        let h = self
            .search
            .set_suggestions(renderer, anim, theme, items, note)?;
        self.header_h = h;
        if (h - self.card_h).abs() > 0.5 {
            self.resize(h, true)?;
        }
        Ok(())
    }

    /// Picks the next suggestion down, or up.
    pub fn step_suggestion(&mut self, down: bool) -> Result<()> {
        self.search.step(&self.anim, down)
    }

    /// Picks suggestion `row`, or none, as the one to look up.
    pub fn pick_suggestion(&mut self, row: Option<usize>) -> Result<()> {
        self.search.pick(&self.anim, row)
    }

    /// The picked suggestion, if any.
    pub fn suggestion(&self) -> Option<usize> {
        self.search.selected()
    }

    /// The suggestion under a point in window pixels.
    pub fn suggestion_at(&self, x: i32, y: i32) -> Option<usize> {
        let (x, y) = self.card_point(x, y);
        self.search.row_at(x, y)
    }

    /// Fades the card out. The caller hides the window after [`crate::ui::HIDE_MS`].
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
        // Release the text surfaces while idle.
        let _ = self.header.brush.SetSurface(None::<&ICompositionSurface>);
        let _ = self.content.brush.SetSurface(None::<&ICompositionSurface>);
        for button in [&self.back, &self.translate] {
            let _ = button.glyph_brush.SetSurface(None::<&ICompositionSurface>);
        }
        let _ = self
            .header
            .back_brush
            .SetSurface(None::<&ICompositionSurface>);
        self.header.surface = None;
        self.loader.stop();
        self.search.reset();
        self.renderer.trim();
    }

    /// Replaces the card's content.
    pub fn set_content(
        &mut self,
        word: &str,
        phonetic: Option<&str>,
        body: Body,
        transition: Transition,
    ) -> Result<()> {
        let rise_from = match (self.search.active, transition) {
            (false, _) => None,
            (true, Transition::Instant) => {
                self.search.reset();
                None
            }
            (true, _) => Some(self.search.leave(&self.anim)?),
        };
        let s = self.scale;
        let content_w = CARD_W - 2.0 * PAD_X;
        let indent = if self.back.shown { BACK_INDENT } else { 0.0 };
        let room = if self.translate.shown {
            TRANSLATE_ROOM
        } else {
            0.0
        };
        let header_layout =
            self.renderer
                .layout_header(word, phonetic, content_w - room, indent)?;
        let body_layout = self.renderer.layout_body(&body, content_w, PAD_BOTTOM)?;
        let key = (
            word.to_owned(),
            phonetic.map(str::to_owned),
            (content_w - room + indent).to_bits(),
        );
        let header_same = std::mem::replace(&mut self.header_key, key.clone()) == key;
        // An unchanged heading keeps the surface it has, so it can't flicker.
        let header_surface = match self.header.surface.clone().filter(|_| header_same) {
            Some(surface) => surface,
            None => self.renderer.draw(&header_layout, s, &self.theme)?,
        };
        let body_surface = self.renderer.draw(&body_layout, s, &self.theme)?;

        let header_h = (PAD_TOP + header_layout.height + HEADER_GAP) * s;
        let body_h = body_layout.height * s;
        let max_h = MAX_CARD_H * s;
        let card_h = (header_h + body_h.min(max_h - header_h)).ceil();
        let transition = if self.card_h > 0.0 {
            transition
        } else {
            Transition::Instant
        };
        let previous_header_h = self.header_h;
        let previous_indent = std::mem::replace(&mut self.header_indent, indent);
        self.header_h = header_h;
        self.body_h = body_h;
        // A refinement keeps the reader's place, and so the focused link.
        let kept = match transition {
            Transition::Refine => self.focused_link().cloned(),
            _ => None,
        };
        self.content.links = body_layout.links.clone();
        if kept == Some(Target::Translate) && self.translate.shown {
            self.focus_translate()?;
        } else {
            let links = &self.content.links;
            let focused = kept.and_then(|t| links.iter().position(|l| l.target == t));
            self.focus_link(focused)?;
        }

        // Header: on a refinement, cross-fade from the old rendering so only
        // what changed (typically the pronunciation) appears to fade in.
        let anim = &self.anim;
        let header = &mut self.header;
        let old_header = header.surface.replace(header_surface.clone());
        Animator::stop(&header.visual, &["Opacity"]);
        Animator::stop(&header.back, &["Opacity"]);
        header.back.SetOpacity(0.0)?;
        if let (Transition::Refine, Some(old), false) = (transition, old_header, header_same) {
            header.back_brush.SetSurface(&old)?;
            header.back.SetSize(header.visual.Size()?)?;
            header.back.SetOffset(v3(PAD_X * s, PAD_TOP * s))?;
            header.back.SetOpacity(1.0)?;
            header.visual.SetOpacity(0.0)?;
            anim.scalar(&header.back, "Opacity", 0.0, 240, 0, &anim.decelerate)?;
            anim.scalar(&header.visual, "Opacity", 1.0, 240, 0, &anim.decelerate)?;
        } else {
            header.visual.SetOpacity(1.0)?;
        }
        header.brush.SetSurface(&header_surface)?;
        header
            .visual
            .SetSize(v2(content_w * s, header_layout.height * s))?;
        let rest = v3(PAD_X * s, PAD_TOP * s);
        let full = scale(1.0);
        Animator::stop(&header.visual, &["Offset", "Scale"]);
        header.visual.SetScale(full)?;
        if let Some((from, size)) = rise_from {
            // The typed word grows into the heading from where it was typed.
            let k = size / WORD_SIZE;
            header.visual.SetScale(Vector3 { X: k, Y: k, Z: 1.0 })?;
            header.visual.SetOffset(from)?;
            header.visual.SetOpacity(0.0)?;
            let ease = &anim.decelerate;
            anim.vector3(&header.visual, "Offset", rest, RESIZE_MS, 0, ease)?;
            anim.vector3(&header.visual, "Scale", full, RESIZE_MS, 0, ease)?;
            anim.scalar(&header.visual, "Opacity", 1.0, 160, 0, ease)?;
        } else if transition != Transition::Instant && previous_indent != indent {
            // The word glides over to make room for the back button, or
            // to take its place.
            // Moving left, it waits a moment for the arrow to fade first.
            let from = PAD_X + previous_indent - indent;
            let delay = if indent < previous_indent { 60 } else { 0 };
            header.visual.SetOffset(v3(from * s, PAD_TOP * s))?;
            anim.vector3(
                &header.visual,
                "Offset",
                rest,
                RESIZE_MS,
                delay,
                &anim.decelerate,
            )?;
        } else {
            header.visual.SetOffset(rest)?;
        }

        // The buttons sit level with the word.
        let line = header_layout.first_line_bottom();
        let top = ((PAD_TOP + (line - BACK_SIZE) / 2.0) * s).round();
        for button in [&mut self.back, &mut self.translate] {
            button.top = top;
            button.visual.SetOffset(v3((button.x * s).round(), top))?;
        }

        let content = &self.content;
        content.brush.SetSurface(&body_surface)?;
        content.visual.SetSize(v2(content_w * s, body_h))?;
        content.show_caret(&self.anim.compositor, body_layout.caret, s)?;
        Animator::stop(&content.visual, &["Offset", "Opacity"]);
        Animator::stop(&content.host, &["Offset"]);

        let was_loading = self.loader.running;
        self.loader.running = matches!(body, Body::Loading);
        if self.loader.running {
            self.loader.start(&self.anim, s)?;
        } else if was_loading {
            let anim = &self.anim;
            anim.scalar(
                &self.loader.visual,
                "Opacity",
                0.0,
                120,
                0,
                &anim.accelerate,
            )?;
        }

        let anim = &self.anim;
        let content = &self.content;
        match transition {
            Transition::Instant => {
                self.scroll = 0.0;
                self.scroll_anim = None;
                content.visual.SetOpacity(1.0)?;
                content.visual.SetOffset(v3(PAD_X * s, 0.0))?;
                content.host.SetOffset(v3(0.0, header_h))?;
                self.resize(card_h, false)?;
            }
            Transition::Replace => {
                // New body slides up a touch and fades in while the card resizes.
                self.scroll = 0.0;
                self.scroll_anim = None;
                content.visual.SetOpacity(0.0)?;
                content.visual.SetOffset(v3(PAD_X * s, 8.0 * s))?;
                anim.scalar(&content.visual, "Opacity", 1.0, 220, 80, &anim.decelerate)?;
                let rest = v3(PAD_X * s, 0.0);
                anim.vector3(
                    &content.visual,
                    "Offset",
                    rest,
                    RESIZE_MS,
                    80,
                    &anim.decelerate,
                )?;
                self.move_content_host(previous_header_h)?;
                self.resize(card_h, true)?;
            }
            Transition::Refine => {
                // Same entry with more detail: keep the reader's place.
                content.visual.SetOpacity(1.0)?;
                self.resize(card_h, true)?;
                self.scroll = self.scroll.min(self.max_scroll());
                self.scroll_anim = None;
                self.content
                    .visual
                    .SetOffset(v3(PAD_X * s, -self.scroll.round()))?;
                self.move_content_host(previous_header_h)?;
            }
        }
        self.update_scroll_chrome(false)?;
        self.sync_tooltip();
        self.sync_swap()
    }

    /// Shows or hides the back button. The word moves to make room for it
    /// with the next [`Popup::set_content`].
    pub fn set_back(&mut self, shown: bool) -> Result<()> {
        self.back.set_shown(&self.anim, shown, self.scale)?;
        self.sync_tooltip();
        Ok(())
    }

    /// Shows or hides the translate button. The word makes room for it with
    /// the next [`Popup::set_content`].
    pub fn set_translate(&mut self, shown: bool) -> Result<()> {
        self.translate.set_shown(&self.anim, shown, self.scale)?;
        self.sync_tooltip();
        Ok(())
    }

    /// Highlights the button the pointer, in window pixels, is on.
    pub fn hover(&mut self, point: Option<(i32, i32)>) -> Result<()> {
        self.pointer = point;
        if self.search.active {
            let point = point.map(|(x, y)| self.card_point(x, y));
            return self.search.hover(&self.anim, point);
        }
        let over_back = point.is_some_and(|(x, y)| self.back_at(x, y));
        let over_translate = point.is_some_and(|(x, y)| self.translate_at(x, y));
        self.back.set_hovered(&self.anim, over_back)?;
        self.translate.set_hovered(&self.anim, over_translate)?;
        self.sync_swap()
    }

    /// Lightens the button swapping a translation's languages while the
    /// pointer's on it, and keeps its tooltip over it as the body changes
    /// and scrolls.
    fn sync_swap(&mut self) -> Result<()> {
        let s = self.scale;
        let link = self.content.swap_link().cloned();
        let rect = link.as_ref().map_or(RECT::default(), |l| self.link_rect(l));
        let over = self
            .pointer
            .is_some_and(|(x, y)| self.link_at(x, y) == Some(&Target::Swap));
        let content = &mut self.content;
        if let Some(l) = &link {
            let geometry = &content.swap_geometry;
            geometry.SetOffset(v2((l.x * s).round(), (l.y * s).round()))?;
            geometry.SetSize(v2((l.width * s).round(), (l.height * s).round()))?;
            geometry.SetCornerRadius(v2(BACK_RADIUS * s, BACK_RADIUS * s))?;
        }
        if over != content.swap_hovered {
            content.swap_hovered = over;
            let opacity = if over { 1.0 } else { 0.0 };
            let anim = &self.anim;
            anim.scalar(&content.swap, "Opacity", opacity, 100, 0, &anim.decelerate)?;
        }
        if rect != content.swap_tip_rect {
            content.swap_tip_rect = rect;
            if let Some(tip) = content.swap_tip {
                set_tooltip_rect(tip, self.hwnd, SWAP_TOOL, rect);
            }
        }
        Ok(())
    }

    /// The part of the body's link `l` in view, in window pixels.
    fn link_rect(&self, l: &Link) -> RECT {
        let s = self.scale;
        let body = self.frame_y() + self.header_h;
        let left = (MARGIN + PAD_X + l.x) * s;
        let top = (body + l.y * s - self.scroll).max(body);
        let bottom = (body + (l.y + l.height) * s - self.scroll).min(self.frame_y() + self.card_h);
        if bottom <= top {
            return RECT::default();
        }
        RECT {
            left: left as i32,
            top: top as i32,
            right: (left + l.width * s) as i32,
            bottom: bottom as i32,
        }
    }

    /// Whether a point in window pixels is on the back button.
    pub fn back_at(&self, x: i32, y: i32) -> bool {
        self.back.shown && contains(self.button_rect(&self.back), x, y)
    }

    /// Whether a point in window pixels is on the translate button.
    pub fn translate_at(&self, x: i32, y: i32) -> bool {
        self.translate.shown && contains(self.button_rect(&self.translate), x, y)
    }

    /// A button in window pixels.
    fn button_rect(&self, button: &IconButton) -> RECT {
        let s = self.scale;
        let left = (MARGIN * s + (button.x * s).round()) as i32;
        let top = (self.frame_y() + button.top) as i32;
        let size = (BACK_SIZE * s).round() as i32;
        RECT {
            left,
            top,
            right: left + size,
            bottom: top + size,
        }
    }

    /// Keeps the tooltips over the buttons, and off while they're hidden.
    fn sync_tooltip(&self) {
        for button in [&self.back, &self.translate] {
            let rect = if button.shown {
                self.button_rect(button)
            } else {
                RECT::default()
            };
            button.set_tooltip_rect(self.hwnd, rect);
        }
    }

    fn move_content_host(&self, previous_header_h: f32) -> Result<()> {
        let to = v3(0.0, self.header_h);
        if (previous_header_h - self.header_h).abs() > 0.5 {
            self.anim.vector3(
                &self.content.host,
                "Offset",
                to,
                RESIZE_MS,
                0,
                &self.anim.decelerate,
            )
        } else {
            self.content.host.SetOffset(to)
        }
    }

    /// Scrolls the body by a mouse wheel delta.
    pub fn scroll_by(&mut self, wheel_delta: i32) -> Result<()> {
        let max = self.max_scroll();
        if max <= 0.0 {
            return Ok(());
        }
        let mut step = -(wheel_delta as f32 / 120.0) * 56.0 * self.scale;
        // A touchpad's scroll is smoothed already, so it follows at once.
        let precise = self.wheel_bursts.precise(wheel_delta);
        if precise {
            step = self.wheel_bursts.precise_step(step, self.scale);
        }
        if !self.wheel_bursts.take(self.scroll + step, max) {
            return Ok(());
        }
        // Past the end, it gives a little, and springs back.
        let visible = self.visible_scroll();
        let (to, push) = ui::stretch(visible, self.scroll, step, max, self.scale);
        let ms = if precise {
            self.scroll_anim.map_or(0, |glide| glide.easing_left())
        } else {
            SCROLL_MS
        };
        self.scroll_to(to, push, ms)
    }

    /// Scrolls the body smoothly to `to`, in pixels from the top, pushed
    /// past it, over `ms`.
    fn scroll_to(&mut self, to: f32, push: Push, ms: u64) -> Result<()> {
        // Start from where the content is on screen right now, not from the
        // previous target: restarting from a stale value is what made
        // scrolling feel like it lagged behind the wheel.
        let (at, max) = (self.scroll, self.max_scroll());
        self.scroll = to.round();
        let most = ui::stretch_limit(self.scale);
        let glide = Glide::new(self.scroll_anim, at, self.scroll, push, max, most, ms);
        self.scroll_anim = Some(glide);
        let x = PAD_X * self.scale;
        glide.play(&self.anim, &self.content.visual, |scroll| v3(x, -scroll))?;
        self.update_scroll_chrome(true)?;
        self.sync_swap()
    }

    /// The scroll position currently on screen, following the running animation.
    fn visible_scroll(&self) -> f32 {
        self.scroll_anim
            .filter(Glide::running)
            .map_or(self.scroll, |glide| glide.at())
    }

    /// The link under a point in window coordinates, if any.
    pub fn link_at(&self, x: i32, y: i32) -> Option<&Target> {
        let s = self.scale;
        let (x, y) = (x as f32 - MARGIN * s, y as f32 - self.frame_y());
        // Only the visible part of the body is clickable.
        if y < self.header_h || y >= self.card_h || x < 0.0 || x >= CARD_W * s {
            return None;
        }
        let (bx, by) = ((x - PAD_X * s) / s, (y - self.header_h + self.scroll) / s);
        self.content
            .links
            .iter()
            .find(|l| bx >= l.x && bx < l.x + l.width && by >= l.y && by < l.y + l.height)
            .map(|l| &l.target)
    }

    /// Whether the card has links, or the translate button, to Tab through.
    pub fn has_links(&self) -> bool {
        self.translate.shown || !self.content.links.is_empty()
    }

    /// The link, or button, Tab moved to, if any.
    pub fn focused_link(&self) -> Option<&Target> {
        static TRANSLATE: Target = Target::Translate;
        if self.translate.focused {
            return Some(&TRANSLATE);
        }
        self.content.focused.map(|i| &self.content.links[i].target)
    }

    /// Moves the keyboard focus to the next link, or the previous one,
    /// going round from the last to the first, and scrolls it into view.
    /// The translate button comes first, as it does reading the card.
    pub fn step_link(&mut self, forward: bool) -> Result<()> {
        // `None` for the translate button.
        let mut stops: Vec<Option<usize>> = Vec::new();
        if self.translate.shown {
            stops.push(None);
        }
        stops.extend(self.content.stops().into_iter().map(Some));
        if stops.is_empty() {
            return Ok(());
        }
        let n = stops.len();
        let current = if self.translate.focused {
            Some(None)
        } else {
            self.content.focused.map(Some)
        };
        let at = current.and_then(|f| stops.iter().position(|&s| s == f));
        let next = match (at, forward) {
            (None, true) => 0,
            (None, false) => n - 1,
            (Some(i), true) => (i + 1) % n,
            (Some(i), false) => (i + n - 1) % n,
        };
        match stops[next] {
            None => self.focus_translate(),
            Some(start) => {
                self.focus_link(Some(start))?;
                self.scroll_into_view(start)
            }
        }
    }

    /// Moves the keyboard focus to the link going to `target`, if the card
    /// has one.
    pub fn focus_target(&mut self, target: &Target) -> Result<()> {
        if let Some(start) = self.content.links.iter().position(|l| &l.target == target) {
            self.focus_link(Some(start))?;
            self.scroll_into_view(start)?;
        }
        Ok(())
    }

    /// Takes the ring off whatever Tab moved to.
    pub fn unfocus(&mut self) -> Result<()> {
        self.focus_link(None)
    }

    /// Rings the translate button.
    fn focus_translate(&mut self) -> Result<()> {
        self.focus_link(None)?;
        self.translate.set_focused(true)
    }

    /// Rings the link starting at `start` in the body's links, or none.
    fn focus_link(&mut self, start: Option<usize>) -> Result<()> {
        self.translate.set_focused(false)?;
        let content = &mut self.content;
        content.focused = start;
        let shapes = content.focus.Shapes()?;
        shapes.Clear()?;
        let Some(start) = start else {
            return Ok(());
        };
        let (c, s) = (&self.anim.compositor, self.scale);
        for l in content.rects(start) {
            let geometry = c.CreateRoundedRectangleGeometry()?;
            let (x, y) = ((l.x * s).round(), (l.y * s).round());
            geometry.SetOffset(v2(x + RING_ROOM, y + RING_ROOM))?;
            geometry.SetSize(v2((l.width * s).round(), (l.height * s).round()))?;
            geometry.SetCornerRadius(v2(FOCUS_RADIUS * s, FOCUS_RADIUS * s))?;
            let shape = c.CreateSpriteShapeWithGeometry(&geometry)?;
            shape.SetStrokeBrush(&content.focus_brush)?;
            shape.SetStrokeThickness((FOCUS_STROKE * s).round())?;
            shapes.Append(&shape)?;
        }
        Ok(())
    }

    /// Scrolls the body just enough to show the link starting at `start`.
    fn scroll_into_view(&mut self, start: usize) -> Result<()> {
        let max = self.max_scroll();
        if max <= 0.0 {
            return Ok(());
        }
        let (mut top, mut bottom) = (f32::MAX, f32::MIN);
        for l in self.content.rects(start) {
            top = top.min(l.y);
            bottom = bottom.max(l.y + l.height);
        }
        let s = self.scale;
        let (top, bottom) = ((top - FOCUS_MARGIN) * s, (bottom + FOCUS_MARGIN) * s);
        let view = self.card_h - self.header_h;
        let to = if top < self.scroll {
            top
        } else if bottom > self.scroll + view {
            bottom - view
        } else {
            return Ok(());
        };
        self.scroll_to(to.clamp(0.0, max), Push::None, SCROLL_MS)
    }

    /// The card's bounds in screen pixels.
    pub fn card_rect(&self) -> RECT {
        let left = self.window.left + (MARGIN * self.scale) as i32;
        let top = self.window.top + self.frame_y() as i32;
        RECT {
            left,
            top,
            right: left + (CARD_W * self.scale) as i32,
            bottom: top + self.card_h as i32,
        }
    }

    /// Shrinks the window's hit-test region to the card once a resize settles.
    pub fn settle(&mut self) {
        self.set_region(self.card_h, self.card_h);
    }

    // -----------------------------------------------------------------------
    // Internals

    /// Places and themes a card that's about to appear, everything at rest.
    fn prepare(&mut self, anchor: RECT, theme: Theme, downward: bool) -> Result<()> {
        self.theme = theme;
        self.place(anchor, downward);
        self.apply_theme()?;

        // Reset everything to its resting state before animating in.
        for v in [
            &self.root.cast::<Visual>()?,
            &self.frame.visual.cast()?,
            &self.content.visual.cast()?,
            &self.header.visual.cast()?,
        ] {
            Animator::stop(v, &["Opacity", "Offset", "Scale"]);
        }
        self.header.visual.SetScale(scale(1.0))?;
        // A new card has nowhere to go back to; whether it can be translated
        // was said before showing it.
        self.back.shown = false;
        self.back.reset()?;
        self.translate.reset()?;
        self.search.reset();
        // The pointer comes back to the card afresh.
        self.pointer = None;
        self.content.swap_hovered = false;
        Animator::stop(&self.content.swap, &["Opacity"]);
        self.content.swap.SetOpacity(0.0)?;
        self.card_h = 0.0;
        Ok(())
    }

    /// Shows the prepared card's window and animates the card in.
    fn appear(&mut self) -> Result<()> {
        let s = self.scale;
        let card_w = CARD_W * s;
        let pivot_y = if self.grow_up {
            self.frame_y() + self.card_h
        } else {
            self.frame_y()
        };
        self.root.SetCenterPoint(Vector3 {
            X: MARGIN * s + card_w / 2.0,
            Y: pivot_y,
            Z: 0.0,
        })?;
        self.root.SetOpacity(0.0)?;
        self.root.SetScale(scale(0.96))?;
        self.root
            .SetOffset(v3(0.0, if self.grow_up { 10.0 } else { -10.0 } * s))?;

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

        self.anim.fade_in(&self.root, FADE_IN_MS, SHOW_MS)
    }

    /// Turns the card into the search box. `in_place` fades out what it
    /// showed first and resizes it smoothly; otherwise it's about to appear.
    fn enter_search(&mut self, title: &str, placeholder: &str, in_place: bool) -> Result<()> {
        self.set_back(false)?;
        self.set_translate(false)?;
        self.focus_link(None)?;
        self.header_indent = 0.0;
        let anim = &self.anim;
        let shown: [Visual; 4] = [
            self.header.visual.cast()?,
            self.header.back.cast()?,
            self.content.visual.cast()?,
            self.loader.visual.cast()?,
        ];
        for v in &shown {
            if in_place {
                anim.scalar(v, "Opacity", 0.0, MAKE_WAY_MS, 0, &anim.accelerate)?;
            } else {
                Animator::stop(v, &["Opacity"]);
                v.SetOpacity(0.0)?;
            }
        }
        self.loader.running = false;
        self.content.links.clear();
        let delay = if in_place { MAKE_WAY_MS } else { 0 };
        let (renderer, theme, s) = (&self.renderer, &self.theme, self.scale);
        let h = self
            .search
            .open(renderer, anim, theme, s, title, placeholder, delay)?;
        // Nothing below the box: no body to scroll or click.
        (self.header_h, self.body_h) = (h, 0.0);
        (self.scroll, self.scroll_anim) = (0.0, None);
        self.resize(h, in_place)?;
        self.update_scroll_chrome(false)?;
        self.sync_swap()
    }

    /// A point in window pixels, relative to the card's top left.
    fn card_point(&self, x: i32, y: i32) -> (f32, f32) {
        (x as f32 - MARGIN * self.scale, y as f32 - self.frame_y())
    }

    fn frame_y(&self) -> f32 {
        let m = MARGIN * self.scale;
        if self.grow_up {
            (self.window.bottom - self.window.top) as f32 - m - self.card_h
        } else {
            m
        }
    }

    fn max_scroll(&self) -> f32 {
        ui::max_scroll(self.body_h, self.card_h - self.header_h, self.scale)
    }

    /// Picks the window position and scale for a card next to `anchor`, on
    /// the monitor the anchor is on.
    fn place(&mut self, anchor: RECT, downward: bool) {
        let center = POINT {
            x: (anchor.left + anchor.right) / 2,
            y: (anchor.top + anchor.bottom) / 2,
        };
        let (work, scale) = monitor_at(center);
        self.scale = scale;
        (self.window, self.grow_up) = placement(anchor, work, scale, downward);
    }

    fn apply_theme(&mut self) -> Result<()> {
        let t = self.theme;
        self.header_key = Default::default();
        self.frame.apply_theme(&self.renderer, &t, self.scale)?;
        self.loader.brush.SetColor(t.accent)?;
        self.content.focus_brush.SetColor(t.focus())?;
        self.content.swap_brush.SetColor(t.highlight())?;
        self.content.caret_brush.SetColor(t.accent)?;
        self.back.apply_theme(&self.renderer, &t, self.scale)?;
        self.translate.apply_theme(&self.renderer, &t, self.scale)?;
        let c = &self.anim.compositor;
        self.content.fade.set_scale(c, self.scale)?;
        self.scroll_chrome.apply_theme(&t)
    }

    fn resize(&mut self, card_h: f32, animate: bool) -> Result<()> {
        let s = self.scale;
        let w = (CARD_W * s).round();
        let old_h = self.card_h;
        self.card_h = card_h;
        let offset = v3(MARGIN * s, self.frame_y());
        let host_size = v2(w, (card_h - self.header_h).max(0.0));
        if animate && (old_h - card_h).abs() <= 0.5 {
            // The same height: any resize under way is already headed there.
        } else if animate {
            self.set_region(old_h, card_h);
            self.frame.resize(&self.anim, w, card_h, offset, s, true)?;
            self.anim
                .vector2(&self.content.host, "Size", host_size, RESIZE_MS)?;
        } else {
            self.frame.resize(&self.anim, w, card_h, offset, s, false)?;
            Animator::stop(&self.content.host, &["Size"]);
            self.content.host.SetSize(host_size)?;
            self.set_region(card_h, card_h);
        }
        Ok(())
    }

    /// Limits the window's clickable area to the card plus its shadow, so the
    /// transparent rest of the window doesn't swallow clicks.
    pub fn hold_region(&mut self, hold: bool) {
        self.region_hold = hold;
    }

    fn set_region(&self, h1: f32, h2: f32) {
        let s = self.scale;
        let m = MARGIN * s;
        let window_h = (self.window.bottom - self.window.top) as f32;
        let w = (self.window.right - self.window.left) as f32;
        let h = if self.region_hold {
            MAX_CARD_H * s
        } else {
            h1.max(h2)
        };
        let (top, bottom) = if self.grow_up {
            (window_h - 2.0 * m - h, window_h)
        } else {
            (0.0, h + 2.0 * m)
        };
        let rect = (
            top as i32,
            bottom.ceil() as i32,
            w as i32,
            self.window.left,
            self.window.top,
        );
        if self.region.replace(rect) == rect {
            return;
        }
        unsafe {
            let region = CreateRectRgn(0, top as i32, w as i32, bottom.ceil() as i32);
            if SetWindowRgn(self.hwnd, Some(region), self.visible) == 0 {
                let _ = DeleteObject(region.into());
            }
        }
    }

    fn update_scroll_chrome(&self, animate: bool) -> Result<()> {
        let s = self.scale;
        let max = self.max_scroll();
        let anim = &self.anim;
        let chrome = &self.scroll_chrome;
        self.content.fade.show(anim, self.scroll, max, s, animate)?;

        if max <= 0.0 {
            Animator::stop(&chrome.thumb, &["Opacity"]);
            chrome.thumb.SetOpacity(0.0)?;
            return Ok(());
        }
        let viewport = self.card_h - self.header_h;
        let thumb_y =
            |scroll: f32| self.header_h + ui::thumb_span(viewport, self.body_h, scroll, max, s).0;
        let (_, thumb) = ui::thumb_span(viewport, self.body_h, self.scroll, max, s);
        let y = thumb_y(self.scroll);
        let x = ui::size_thumb(&chrome.thumb, &chrome.thumb_clip, CARD_W * s, thumb, s)?;
        let glide = self.scroll_anim.filter(|glide| animate && glide.running());
        if let Some(glide) = glide {
            // Track the content exactly, from where the thumb is now.
            glide.play(anim, &chrome.thumb, |scroll| v3(x, thumb_y(scroll)))?;
        } else {
            Animator::stop(&chrome.thumb, &["Offset"]);
            chrome.thumb.SetOffset(v3(x, y))?;
        }
        anim.scalar(&chrome.thumb, "Opacity", 1.0, 200, 150, &anim.decelerate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: RECT = RECT {
        left: 0,
        top: 0,
        right: 1920,
        bottom: 1040,
    };

    fn word_at(x: i32, y: i32) -> RECT {
        RECT {
            left: x,
            top: y,
            right: x + 60,
            bottom: y + 20,
        }
    }

    #[test]
    fn places_the_card_below_the_selection() {
        let anchor = word_at(500, 100);
        let (window, grow_up) = placement(anchor, SCREEN, 1.0, false);
        assert!(!grow_up);
        // The card starts ANCHOR_GAP below the selection, aligned with its text.
        assert_eq!(
            window.top + MARGIN as i32,
            anchor.bottom + ANCHOR_GAP as i32
        );
        assert_eq!(window.left + MARGIN as i32, anchor.left - PAD_X as i32);
        let height = MAX_CARD_H as i32 + 2 * MARGIN as i32;
        assert_eq!(window.bottom - window.top, height);
    }

    #[test]
    fn grows_up_near_the_bottom_of_the_screen() {
        let anchor = word_at(500, 950);
        let (window, grow_up) = placement(anchor, SCREEN, 1.0, false);
        assert!(grow_up);
        // The card's bottom edge sits ANCHOR_GAP above the selection.
        assert_eq!(
            window.bottom - MARGIN as i32,
            anchor.top - ANCHOR_GAP as i32
        );
    }

    #[test]
    fn centres_a_search_card() {
        for scale in [1.0, 1.5, 2.0] {
            let (window, grow_up) = placement(centred_in(SCREEN, scale), SCREEN, scale, true);
            assert!(!grow_up);
            let m = (MARGIN * scale).round() as i32;
            let card_w = (CARD_W * scale).round() as i32;
            let card_left = window.left + m;
            let middle = (SCREEN.left + SCREEN.right) / 2;
            assert!((card_left + card_w / 2 - middle).abs() <= 1, "@{scale}");
            // The empty box (about twice CENTRED_RISE tall) straddles the
            // middle, unless that leaves no room below for the tallest card.
            let card_top = window.top + m;
            let rise = (CENTRED_RISE * scale).round() as i32;
            let tallest = (MAX_CARD_H * scale).round() as i32;
            let lowest = SCREEN.bottom - (SCREEN_GAP * scale).round() as i32 - tallest;
            let expected = ((SCREEN.top + SCREEN.bottom) / 2 - rise).min(lowest);
            assert!((card_top - expected).abs() <= 1, "@{scale}");
        }
    }

    #[test]
    fn search_cards_grow_down() {
        let anchor = word_at(500, 950);
        let (window, grow_up) = placement(anchor, SCREEN, 1.0, true);
        assert!(!grow_up);
        // Still on screen, with room for the tallest card.
        assert_eq!(
            window.bottom - MARGIN as i32,
            SCREEN.bottom - SCREEN_GAP as i32
        );
    }

    #[test]
    fn stays_on_screen() {
        for scale in [1.0, 1.5, 2.0] {
            for (x, y) in [(-50, 10), (1900, 500), (0, 0), (1900, 1030)] {
                let (window, _) = placement(word_at(x, y), SCREEN, scale, false);
                let m = (MARGIN * scale).round() as i32;
                let edge = (SCREEN_GAP * scale).round() as i32;
                assert!(window.left + m >= SCREEN.left + edge, "{x},{y} @{scale}");
                assert!(window.right - m <= SCREEN.right - edge, "{x},{y} @{scale}");
                assert!(window.top + m >= SCREEN.top + edge, "{x},{y} @{scale}");
                assert!(
                    window.bottom - m <= SCREEN.bottom - edge,
                    "{x},{y} @{scale}"
                );
            }
        }
    }
}
