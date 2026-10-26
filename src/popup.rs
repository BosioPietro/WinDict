//! The definition card: a borderless, non-activating, top-most window whose
//! content is a Windows.UI.Composition visual tree. Blur comes from the host
//! backdrop brush; every animation runs on the system compositor, so motion
//! stays smooth no matter what our UI thread is doing.

mod body;
mod focus;
mod parts;
mod scrolling;
mod search;

use windows::core::{Interface, Result, HSTRING};
use windows::Win32::Foundation::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::*;
use windows_numerics::{Vector2, Vector3};

use crate::i18n::tr;
use crate::render::{
    self, Body, Face, Ink, Link, ListLayout, Mark, Renderer, Target, LANGUAGES_TOP, WORD_SIZE,
};
use crate::searchbox::SearchBox;
use crate::theme::Theme;
use crate::ui::{
    self, attach, blink, monitor_at, scale, text_sprite, ts, v2, v3, Animator, Frame, Glide, List,
    Push, ScrollFade, Scrollbar, Scroller, Track, MARGIN, RESIZE_MS,
};
use crate::win::{self, create_tooltip, set_tooltip_rect, Window};

use body::Content;
use parts::{Arrival, Badge, Header, IconButton, IconSpec, Loader};
use scrolling::ScrollChrome;

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
/// The pin button's left edge: mirrors the back button's, at the right.
const PIN_X: f32 = CARD_W - BACK_X - BACK_SIZE;
/// How much narrower the heading is beside the pin button.
const PIN_ROOM: f32 = CARD_W - PAD_X - PIN_X + 6.0;
/// "Pinned" in Segoe Fluent Icons and Segoe MDL2 Assets, and "Pinned fill",
/// which fills its head in but leaves out the needle.
const PIN_GLYPH: &str = "\u{E840}";
const PIN_FILL_GLYPH: &str = "\u{E842}";
/// The translate button's left edge: left of the pin button.
const TRANSLATE_X: f32 = PIN_X - BACK_SIZE - 2.0;
/// How much narrower the heading is beside the translate and pin buttons.
const TRANSLATE_ROOM: f32 = CARD_W - PAD_X - TRANSLATE_X + 6.0;
/// "Translate" in Segoe Fluent Icons and Segoe MDL2 Assets.
const TRANSLATE_GLYPH: &str = "\u{E8C1}";
/// The swap button's tooltip's tool, after the back and translate buttons'.
const SWAP_TOOL: usize = 3;
/// The speak button's tooltip's tool, after the swap button's.
const SPEAK_TOOL: usize = 4;
/// The pin button's tooltip's tool, after the speak button's.
const PIN_TOOL: usize = 5;
/// "Volume" in Segoe Fluent Icons and Segoe MDL2 Assets.
const SPEAK_GLYPH: &str = "\u{E767}";
/// How far the speak button slides in from the left as it appears.
const SPEAK_SLIDE: f32 = 6.0;
/// How long a pronunciation takes to fade in, and how far it slides in
/// from the left as it does, in DIPs.
const PHONETIC_MS: u64 = 320;
const PHONETIC_SLIDE: f32 = 6.0;
const FOCUS_RADIUS: f32 = 4.0;
/// How long the ring takes to glide to the next link, and to fade.
const FOCUS_MS: u64 = 200;
const FOCUS_FADE_MS: u64 = 100;
/// "Copied" badge: room around its text, its corners, how far above the
/// card's bottom it sits, how long it shows, and how far and long it rises.
const BADGE_PAD_X: f32 = 14.0;
const BADGE_PAD_Y: f32 = 7.0;
const BADGE_RADIUS: f32 = 8.0;
const BADGE_BOTTOM: f32 = 14.0;
const BADGE_MS: u64 = 1100;
const BADGE_RISE: f32 = 6.0;
const BADGE_RISE_MS: u64 = 260;
/// How far its shadow spreads and drops, and how dark it is next to the card's.
const BADGE_SHADOW: f32 = 10.0;
const BADGE_SHADOW_DY: f32 = 2.0;
const BADGE_SHADOW_OPACITY: f32 = 0.8;
/// "Accept" (a check mark) in Segoe Fluent Icons and Segoe MDL2 Assets.
const COPIED_GLYPH: &str = "\u{E73E}";

// Durations, in ms.
pub const SHOW_MS: u64 = 340;
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
        let card_bottom = (anchor.top - gap)
            .min(work.bottom - edge)
            .max(work.top + edge + max_h);
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
/// (screen pixels) is on, for when there's no caret to open it at, on a
/// card zoomed to `zoom`.
pub fn centred_anchor(near: POINT, zoom: f32) -> RECT {
    let (work, scale) = monitor_at(near);
    centred_in(work, zoomed(work, scale, zoom))
}

/// An anchor that opens a card beside `window` (screen pixels), level with
/// its top: on its right, or on its left when there's no room there, for a
/// card zoomed to `zoom`.
pub fn anchor_beside(window: RECT, zoom: f32) -> RECT {
    let near = POINT {
        x: (window.left + window.right) / 2,
        y: window.top,
    };
    let (work, scale) = monitor_at(near);
    let s = zoomed(work, scale, zoom);
    let px = |dip: f32| (dip * s).round() as i32;
    let (card_w, gap) = (px(CARD_W), px(BESIDE_GAP));
    let card_x = if window.right + gap + card_w <= work.right - px(SCREEN_GAP) {
        window.right + gap
    } else {
        window.left - gap - card_w
    };
    // `placement` puts the card's top that far under the anchor, and its
    // left that far left of it.
    let bottom = window.top - px(ANCHOR_GAP);
    RECT {
        left: card_x + px(PAD_X),
        top: bottom - px(BESIDE_LINE),
        right: card_x + px(PAD_X) + 1,
        bottom,
    }
}

/// Between a window and a card opened beside it, and how tall the line the
/// card's anchored to is, in DIPs.
const BESIDE_GAP: f32 = 12.0;
const BESIDE_LINE: f32 = 18.0;

/// The scale of a card zoomed to `zoom` on a monitor of `scale` pixels per
/// DIP: no larger than fits the tallest card in the `work` area, unless
/// even the unzoomed one doesn't.
fn zoomed(work: RECT, scale: f32, zoom: f32) -> f32 {
    let room_w = (work.right - work.left) as f32 / (CARD_W + 2.0 * SCREEN_GAP);
    let room_h = (work.bottom - work.top) as f32 / (MAX_CARD_H + 2.0 * SCREEN_GAP);
    (scale * zoom).min(room_w.min(room_h).max(scale))
}

/// Where the window at `window`, its card drawn at `old` pixels per DIP,
/// goes for the card at `s`: its left edge, and the edge it grows from,
/// stay put, as far as the `work` area has room for it.
fn rescaled(window: RECT, grow_up: bool, old: f32, s: f32, work: RECT) -> RECT {
    let px = |dip: f32| (dip * s).round() as i32;
    let (card_w, max_h, m, edge) = (px(CARD_W), px(MAX_CARD_H), px(MARGIN), px(SCREEN_GAP));
    let old_m = (MARGIN * old).round() as i32;
    let card_x = (window.left + old_m).clamp(
        work.left + edge,
        (work.right - edge - card_w).max(work.left + edge),
    );
    let window_h = max_h + 2 * m;
    let window_y = if grow_up {
        let card_bottom = (window.bottom - old_m)
            .min(work.bottom - edge)
            .max(work.top + edge + max_h);
        card_bottom + m - window_h
    } else {
        let card_top = (window.top + old_m)
            .min(work.bottom - edge - max_h)
            .max(work.top + edge);
        card_top - m
    };
    RECT {
        left: card_x - m,
        top: window_y,
        right: card_x + card_w + m,
        bottom: window_y + window_h,
    }
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
    /// The same content, drawn at another size: nothing moves.
    Rescale,
}

/// What the card shows, laid out, to be drawn again at another size.
struct Shown {
    word: String,
    phonetic: Option<String>,
    heading: render::Heading,
    body: render::Layout,
    /// How far the word is indented for the back button, in DIPs.
    indent: f32,
    loading: bool,
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
    /// Says the word: after its pronunciation, and moving with the heading.
    speak: IconButton,
    /// Keeps the card open, or lets it go again.
    pin: IconButton,
    search: SearchBox,
    glide: SpriteVisual,
    glide_source: CompositionSurfaceBrush,
    glide_grid: CompositionNineGridBrush,
    badge: Badge,

    theme: Theme,
    scale: f32,
    /// How much larger than the monitor's scale the card is drawn.
    zoom: f32,
    /// The content last shown, unless the search box is.
    shown: Option<Shown>,
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
    /// What the word was last drawn from (the word, its size and its
    /// indent), to tell a refinement that leaves it alone.
    header_key: (String, (u32, u32), u32),
    body_h: f32,
    /// The next body glides over from where the last one is, rather than
    /// coming in afresh: the languages under the search box's field, on
    /// their way to the same under a translation's heading, or back.
    body_glides: bool,
    scroller: Scroller,
    /// Where the pointer last was on the window, in window pixels.
    pointer: Option<(i32, i32)>,
    pub visible: bool,
    /// Pinned open: a window of its own, dragged by its heading.
    pinned: bool,
    /// Last, so it goes after everything shown on it.
    _window: Window,
}

/// Somewhere Tab stops in the card.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Stop {
    Speak,
    Translate,
    Pin,
    /// The link starting at this index in the body's links.
    Link(usize),
    Back,
}

impl Stop {
    fn is_button(self) -> bool {
        !matches!(self, Stop::Link(_))
    }
}

// ---------------------------------------------------------------------------

impl Popup {
    pub fn new(window: Window, compositor: Compositor, theme: Theme, zoom: f32) -> Result<Popup> {
        let hwnd = window.hwnd();
        let target = attach(hwnd, &compositor)?;
        let renderer = Renderer::new(&compositor)?;
        let c = &compositor;

        // Creation order is stacking order: later visuals draw on top.
        let root = c.CreateContainerVisual()?;
        target.SetRoot(&root)?;
        let frame = Frame::new(c, &root, &theme)?;
        let header = Header::new(c, &frame.card)?;
        // In the heading, so that it moves with it.
        let speak = IconButton::new(
            c,
            &header.host()?,
            hwnd,
            IconSpec {
                icon: SPEAK_GLYPH,
                face: Face::Button,
                ink: Ink::Secondary,
                tip: tr().speak_tip,
                tool: SPEAK_TOOL,
                x: 0.0,
                origin: v2(PAD_X, PAD_TOP),
                slide_from: -SPEAK_SLIDE,
            },
        )?;
        let back = IconButton::new(
            c,
            &frame.card,
            hwnd,
            IconSpec {
                icon: BACK_GLYPH,
                face: Face::Button,
                ink: Ink::Primary,
                tip: tr().back_tip,
                tool: 1,
                x: BACK_X,
                origin: v2(0.0, 0.0),
                slide_from: -BACK_SLIDE,
            },
        )?;
        // Its glyph's detail wants it larger than the arrow, and it's
        // colored like the links, as it leads somewhere too.
        let translate = IconButton::new(
            c,
            &frame.card,
            hwnd,
            IconSpec {
                icon: TRANSLATE_GLYPH,
                face: Face::LargeButton,
                ink: Ink::Accent,
                tip: tr().translate_tip,
                tool: 2,
                x: TRANSLATE_X,
                origin: v2(0.0, 0.0),
                slide_from: 0.0,
            },
        )?;
        let pin = IconButton::new(
            c,
            &frame.card,
            hwnd,
            IconSpec {
                icon: PIN_GLYPH,
                face: Face::Button,
                ink: Ink::Secondary,
                tip: tr().pin_tip,
                tool: PIN_TOOL,
                x: PIN_X,
                origin: v2(0.0, 0.0),
                slide_from: 0.0,
            },
        )?;
        let content = Content::new(c, &frame.card, hwnd)?;
        let loader = Loader::new(c, content.host(), &theme)?;
        let scroll_chrome = ScrollChrome::new(c, &frame.card)?;
        // Under the heading, which moves over it to and from the field.
        let [_, lowest] = header.visuals()?;
        let search = SearchBox::new(c, &frame.card, &lowest)?;
        // A ring gliding between a button and a link, over everything: they
        // aren't in the same space, nor does one scroll and the other not.
        let glide = c.CreateSpriteVisual()?;
        glide.SetOpacity(0.0)?;
        let glide_source = c.CreateSurfaceBrush()?;
        glide_source.SetStretch(CompositionStretch::Fill)?;
        let glide_grid = c.CreateNineGridBrush()?;
        glide_grid.SetSource(&glide_source)?;
        let glide_mask = c.CreateMaskBrush()?;
        glide_mask.SetSource(content.focus_brush())?;
        glide_mask.SetMask(&glide_grid)?;
        glide.SetBrush(&glide_mask)?;
        frame.card.Children()?.InsertAtTop(&glide)?;
        let badge = Badge::new(c, &frame.card)?;

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
            speak,
            pin,
            search,
            glide,
            glide_source,
            glide_grid,
            badge,
            theme,
            scale: 1.0,
            zoom,
            shown: None,
            grow_up: false,
            window: RECT::default(),
            card_h: 0.0,
            header_h: 0.0,
            header_indent: 0.0,
            region_hold: false,
            region: Default::default(),
            header_key: Default::default(),
            body_h: 0.0,
            body_glides: false,
            scroller: Scroller::default(),
            pointer: None,
            visible: false,
            pinned: false,
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
        self.header.release();
        self.content.release(&self.anim);
        self.back.release();
        self.translate.release();
        self.speak.release();
        self.pin.release();
        self.badge.release();
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
        // The languages under the field stay, and glide up under the heading.
        self.body_glides = rise_from.is_some() && self.search.room() > 0.0;
        let content_w = CARD_W - 2.0 * PAD_X;
        let indent = if self.back.shown() { BACK_INDENT } else { 0.0 };
        // The pin button's there whenever the search box isn't.
        self.pin.set_shown(&self.anim, true, self.scale)?;
        let room = if self.translate.shown() {
            TRANSLATE_ROOM
        } else {
            PIN_ROOM
        };
        let button = self.speak.shown().then_some(BACK_SIZE);
        let muted = matches!(body, Body::Loading { status: true });
        let heading =
            self.renderer
                .layout_header(word, phonetic, content_w - room, indent, button, muted)?;
        let shown = Shown {
            word: word.to_owned(),
            phonetic: phonetic.map(str::to_owned),
            heading,
            body: self.renderer.layout_body(&body, content_w, PAD_BOTTOM)?,
            indent,
            loading: matches!(body, Body::Loading { .. }),
        };
        self.draw_content(&shown, transition, rise_from)?;
        self.shown = Some(shown);
        Ok(())
    }

    /// Draws `shown` on the card, bringing it in as `transition` says, the
    /// word rising from the search box when it comes `rise_from` there.
    fn draw_content(
        &mut self,
        shown: &Shown,
        transition: Transition,
        rise_from: Option<(Vector3, f32)>,
    ) -> Result<()> {
        let s = self.scale;
        let content_w = CARD_W - 2.0 * PAD_X;
        let (word, phonetic, indent) = (&shown.word, shown.phonetic.as_deref(), shown.indent);
        let (heading, body_layout) = (&shown.heading, &shown.body);
        let (word_w, word_h) = heading.word_size;
        let key = (
            word.clone(),
            (word_w.to_bits(), word_h.to_bits()),
            indent.to_bits(),
        );
        let header_same = std::mem::replace(&mut self.header_key, key.clone()) == key;
        // An unchanged word keeps the surface it has, so it can't flicker.
        let header_surface = match self.header.surface().filter(|_| header_same) {
            Some(surface) => surface,
            None => self.renderer.draw(&heading.word, s, &self.theme)?,
        };
        let body_surface = self.renderer.draw(body_layout, s, &self.theme)?;

        let header_h = (PAD_TOP + heading.height + HEADER_GAP) * s;
        // Only as far as it's drawn can it scroll.
        let body_h = (body_layout.height * s).min(self.renderer.max_px());
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
            Transition::Refine | Transition::Rescale => self.focused_link().cloned(),
            Transition::Instant | Transition::Replace => None,
        };
        self.content.set_links(body_layout.links.clone());
        if kept == Some(Target::Speak) && self.speak.shown() {
            self.focus_speak(0)?;
        } else if kept == Some(Target::Translate) && self.translate.shown() {
            self.focus_translate(0)?;
        } else if kept == Some(Target::Back) && self.back.shown() {
            self.focus_back(0)?;
        } else if kept == Some(Target::Pin) {
            self.focus_pin(0)?;
        } else {
            let links = self.content.links();
            let focused = kept.and_then(|t| links.iter().position(|l| l.target == t));
            self.focus_link(focused, 0)?;
        }

        // Header: on a refinement, cross-fade from the old rendering of the
        // word, should it have changed.
        let crossfade = transition == Transition::Refine && !header_same;
        let size = v2(content_w * s, heading.height * s);
        self.header
            .show(&self.anim, &header_surface, size, crossfade, s)?;
        // The pronunciation fades in once it's known, whatever else changes.
        let moving = matches!(transition, Transition::Replace | Transition::Refine);
        match (phonetic, &heading.phonetic) {
            (Some(text), Some(placed)) => {
                let surface = self.renderer.draw(&placed.layout, s, &self.theme)?;
                let size = v2(placed.layout.width * s, placed.layout.height * s);
                let at = v3((placed.x * s).round(), (placed.y * s).round());
                self.header
                    .show_phonetic(&self.anim, text, &surface, size, at, moving, s)?;
            }
            _ => self.header.hide_phonetic()?,
        }
        if let Some((x, middle)) = heading.button {
            let top = ((middle - BACK_SIZE / 2.0) * s).round();
            self.speak.move_to(&self.anim, x, top, moving, s)?;
        }
        let arrival = if let Some((from, size)) = rise_from {
            // The typed word grows into the heading from where it was typed.
            Arrival::Rise {
                from,
                scale: size / WORD_SIZE,
            }
        } else if transition != Transition::Instant && previous_indent != indent {
            // The word glides over to make room for the back button, or
            // to take its place.
            // Moving left, it waits a moment for the arrow to fade first.
            Arrival::Glide {
                from_x: PAD_X + previous_indent - indent,
                delay: if indent < previous_indent { 60 } else { 0 },
            }
        } else {
            Arrival::Still
        };
        self.header.arrive(&self.anim, arrival, s)?;

        // The buttons sit level with the word.
        let line = heading.word.first_line_bottom();
        let top = ((PAD_TOP + (line - BACK_SIZE) / 2.0) * s).round();
        for button in [&mut self.back, &mut self.translate, &mut self.pin] {
            button.place(top, s)?;
        }

        let size = v2(content_w * s, body_h);
        let c = &self.anim.compositor;
        self.content
            .show(c, &body_surface, size, body_layout.caret, s)?;
        let list = body_layout.list.as_deref();
        self.content
            .show_list(&self.anim, &self.renderer, &self.theme, list, s)?;

        self.loader.set_running(&self.anim, shown.loading, s)?;
        self.sync_title();

        self.enter_body(transition, card_h, previous_header_h)?;
        self.update_scroll_chrome(false)?;
        self.sync_tooltip();
        self.sync_swap()
    }

    /// Brings the new body in as `transition` says, and sizes the card to
    /// `card_h`. `previous_header_h` is what the header measured before.
    fn enter_body(
        &mut self,
        transition: Transition,
        card_h: f32,
        previous_header_h: f32,
    ) -> Result<()> {
        let s = self.scale;
        match transition {
            Transition::Instant => {
                self.scroller.stop_at(0.0);
                self.content.appear(self.header_h, s)?;
                self.resize(card_h, false)?;
            }
            Transition::Replace if std::mem::take(&mut self.body_glides) => {
                // What's the same at the top of both glides to its new place,
                // from where it was: a translation has its languages lower.
                self.scroller.stop_at(0.0);
                self.content.keep_place(0.0, s)?;
                self.content.shift_host(-LANGUAGES_TOP * s)?;
                self.move_content_host(previous_header_h)?;
                self.resize(card_h, true)?;
            }
            Transition::Replace => {
                // New body slides up a touch and fades in while the card resizes.
                self.scroller.stop_at(0.0);
                self.content.slide_in(&self.anim, s)?;
                self.move_content_host(previous_header_h)?;
                self.resize(card_h, true)?;
            }
            Transition::Refine => {
                // Same entry with more detail: keep the reader's place.
                self.resize(card_h, true)?;
                self.scroller
                    .stop_at(self.scroller.at.min(self.max_scroll()));
                self.content.keep_place(self.scroller.at, s)?;
                self.move_content_host(previous_header_h)?;
            }
            Transition::Rescale => {
                self.resize(card_h, false)?;
                self.scroller
                    .stop_at(self.scroller.at.min(self.max_scroll()));
                self.content.keep_place(self.scroller.at, s)?;
                self.content.move_host(&self.anim, self.header_h, false)?;
            }
        }
        Ok(())
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

    /// Shows or hides the button saying the word. The heading makes room
    /// for it with the next [`Popup::set_content`].
    pub fn set_speak(&mut self, shown: bool) -> Result<()> {
        self.speak.set_shown(&self.anim, shown, self.scale)?;
        self.sync_tooltip();
        Ok(())
    }

    /// Pins the card open, as a window of its own titled with its word, or
    /// lets it go again.
    pub fn set_pinned(&mut self, pinned: bool) -> Result<()> {
        if pinned == self.pinned {
            return Ok(());
        }
        self.pinned = pinned;
        let (fill, ink, tip) = if pinned {
            (Some(PIN_FILL_GLYPH), Ink::Accent, tr().unpin_tip)
        } else {
            (None, Ink::Secondary, tr().pin_tip)
        };
        self.pin.set_icon(PIN_GLYPH, fill, ink);
        self.pin.set_tip(self.hwnd, tip);
        self.pin
            .apply_theme(&self.renderer, &self.theme, self.scale)?;
        self.sync_title();
        win::set_standalone(self.hwnd, pinned);
        Ok(())
    }

    /// Titles a pinned card's window, which the taskbar and Alt+Tab show,
    /// after its word.
    fn sync_title(&self) {
        let title = match self.pinned {
            true => format!("{} - WinDict", render::capitalize(&self.header_key.0)),
            false => "WinDict".to_string(),
        };
        win::set_title(self.hwnd, &title);
    }

    /// Whether a point in window pixels is where the card is dragged from:
    /// its heading, away from the buttons. With the search box up, that's
    /// the heading above the field.
    pub fn caption_at(&self, x: i32, y: i32) -> bool {
        let (cx, cy) = self.card_point(x, y);
        let on_button = [&self.back, &self.translate, &self.speak, &self.pin]
            .into_iter()
            .any(|b| b.shown() && contains(self.button_rect(b), x, y));
        let heading_h = match self.search.active {
            true => self.search.field_top(),
            false => self.header_h,
        };
        let in_heading = cx >= 0.0 && cx < CARD_W * self.scale && cy >= 0.0 && cy < heading_h;
        in_heading && !on_button
    }

    /// Draws the card `zoom` times as large as the monitor has it, in
    /// place, from the next card on if it shows the search box.
    pub fn set_zoom(&mut self, zoom: f32) -> Result<()> {
        self.zoom = zoom;
        self.rescale()
    }

    /// Draws the card at the scale of the monitor it's on, in place: it may
    /// have been zoomed, or dragged to a monitor with another scale.
    fn rescale(&mut self) -> Result<()> {
        let Some(shown) = self.shown.take().filter(|_| self.visible) else {
            return Ok(());
        };
        let r = self.card_rect();
        let center = POINT {
            x: (r.left + r.right) / 2,
            y: (r.top + r.bottom) / 2,
        };
        let (work, scale) = monitor_at(center);
        let old = self.scale;
        self.scale = zoomed(work, scale, self.zoom);
        if self.scale != old {
            self.window = rescaled(self.window, self.grow_up, old, self.scale, work);
            let w = &self.window;
            unsafe {
                let _ = SetWindowPos(
                    self.hwnd,
                    None,
                    w.left,
                    w.top,
                    w.right - w.left,
                    w.bottom - w.top,
                    SWP_NOACTIVATE | SWP_NOZORDER,
                );
            }
            self.scroller.at *= self.scale / old;
            self.apply_theme()?;
            self.draw_content(&shown, Transition::Rescale, None)?;
        }
        self.shown = Some(shown);
        Ok(())
    }

    /// Draws the card in `theme` from now on, and what it shows again in it.
    pub fn retheme(&mut self, theme: Theme) -> Result<()> {
        if theme == self.theme {
            return Ok(());
        }
        self.theme = theme;
        self.redraw()
    }

    /// The GPU went away: draws everything again with another.
    pub fn recover(&mut self) -> Result<()> {
        if self.renderer.recover()? {
            self.redraw()?;
        }
        Ok(())
    }

    fn redraw(&mut self) -> Result<()> {
        self.apply_theme()?;
        if let Some(shown) = self.shown.take() {
            let drawn = match self.visible {
                true => self.draw_content(&shown, Transition::Rescale, None),
                false => Ok(()),
            };
            self.shown = Some(shown);
            drawn?;
        }
        Ok(())
    }

    /// The window was dragged somewhere else, maybe to a monitor with
    /// another scale.
    pub fn moved(&mut self) -> Result<()> {
        let mut r = RECT::default();
        if unsafe { GetWindowRect(self.hwnd, &mut r) }.is_ok() {
            self.window = r;
        }
        self.rescale()
    }

    /// Shows for a moment, at the bottom of the card, that something was
    /// copied.
    pub fn show_copied(&self) -> Result<()> {
        let card = v2(CARD_W * self.scale, self.card_h);
        self.badge
            .flash(&self.anim, &self.renderer, &self.theme, card, self.scale)
    }

    /// Highlights the button the pointer, in window pixels, is on.
    pub fn hover(&mut self, point: Option<(i32, i32)>) -> Result<()> {
        self.pointer = point;
        // On a scrollbar, or dragging its thumb, nothing else is hovered.
        let in_body = point.and_then(|(x, y)| self.body_point(x, y));
        let on_list = self.content.list().hover_bar(&self.anim, in_body)?;
        let point = if on_list { None } else { point };
        let point = if self.hover_bar(point)? { None } else { point };
        if self.search.active {
            let card_point = point.map(|(x, y)| self.card_point(x, y));
            self.search.hover(&self.anim, card_point)?;
            // The languages under the field have a swap button to lighten.
            return self.sync_swap();
        }
        let over_back = point.is_some_and(|(x, y)| self.back_at(x, y));
        let over_translate = point.is_some_and(|(x, y)| self.translate_at(x, y));
        let over_speak = point.is_some_and(|(x, y)| self.speak_at(x, y));
        let over_pin = point.is_some_and(|(x, y)| self.pin_at(x, y));
        self.back.set_hovered(&self.anim, over_back)?;
        self.pin.set_hovered(&self.anim, over_pin)?;
        self.translate.set_hovered(&self.anim, over_translate)?;
        self.speak.set_hovered(&self.anim, over_speak)?;
        self.sync_swap()
    }

    /// Where the pointer last was on the window, in window pixels.
    pub fn pointer(&self) -> Option<(i32, i32)> {
        self.pointer
    }

    /// Lightens the button swapping a translation's languages while the
    /// pointer's on it, and keeps its tooltip over it as the body changes
    /// and scrolls.
    fn sync_swap(&mut self) -> Result<()> {
        let link = self.content.swap_link();
        let rect = link.map_or(RECT::default(), |l| self.link_rect(l));
        let over = self
            .pointer
            .is_some_and(|(x, y)| self.link_at(x, y) == Some(&Target::Swap));
        self.content
            .sync_swap(&self.anim, self.hwnd, rect, over, self.scale)
    }

    /// The part of the body's link `l` in view, in window pixels.
    fn link_rect(&self, l: &Link) -> RECT {
        let s = self.scale;
        let body = self.frame_y() + self.header_h;
        let left = (MARGIN + PAD_X + l.x) * s;
        let top = (body + l.y * s - self.scroller.at).max(body);
        let bottom =
            (body + (l.y + l.height) * s - self.scroller.at).min(self.frame_y() + self.card_h);
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
        self.back.shown() && contains(self.button_rect(&self.back), x, y)
    }

    /// Whether a point in window pixels is on the translate button.
    pub fn translate_at(&self, x: i32, y: i32) -> bool {
        self.translate.shown() && contains(self.button_rect(&self.translate), x, y)
    }

    /// Whether a point in window pixels is on the button saying the word.
    pub fn speak_at(&self, x: i32, y: i32) -> bool {
        self.speak.shown() && contains(self.button_rect(&self.speak), x, y)
    }

    /// Whether a point in window pixels is on the pin button.
    pub fn pin_at(&self, x: i32, y: i32) -> bool {
        self.pin.shown() && contains(self.button_rect(&self.pin), x, y)
    }

    /// A button in window pixels.
    fn button_rect(&self, button: &IconButton) -> RECT {
        let s = self.scale;
        let at = button.card_offset(s);
        let left = (MARGIN * s + at.X).round() as i32;
        let top = (self.frame_y() + at.Y).round() as i32;
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
        for button in [&self.back, &self.translate, &self.speak, &self.pin] {
            let rect = if button.shown() {
                self.button_rect(button)
            } else {
                RECT::default()
            };
            button.set_tooltip_rect(self.hwnd, rect);
        }
    }

    fn move_content_host(&self, previous_header_h: f32) -> Result<()> {
        let animate = (previous_header_h - self.header_h).abs() > 0.5;
        self.content.move_host(&self.anim, self.header_h, animate)
    }

    /// The card's bounds in screen pixels.
    pub fn card_rect(&self) -> RECT {
        let left = self.window.left + (MARGIN * self.scale).round() as i32;
        let top = self.window.top + self.frame_y().round() as i32;
        RECT {
            left,
            top,
            right: left + (CARD_W * self.scale).round() as i32,
            bottom: top + self.card_h.round() as i32,
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
            &self.content.visual()?,
        ] {
            Animator::stop(v, &["Opacity", "Offset", "Scale"]);
        }
        self.header.reset()?;
        // A new card has nowhere to go back to; whether it can be translated
        // was said before showing it.
        self.back.hide_at_once()?;
        self.translate.reset()?;
        self.speak.reset()?;
        self.pin.reset()?;
        self.search.reset();
        self.badge.reset()?;
        // The pointer comes back to the card afresh.
        self.pointer = None;
        self.content.reset_swap()?;
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

        self.anim.fade_in(&self.root, ui::FADE_IN_MS, SHOW_MS)
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

    /// Picks the window position and scale for a card next to `anchor`, on
    /// the monitor the anchor is on.
    fn place(&mut self, anchor: RECT, downward: bool) {
        let center = POINT {
            x: (anchor.left + anchor.right) / 2,
            y: (anchor.top + anchor.bottom) / 2,
        };
        let (work, scale) = monitor_at(center);
        self.scale = zoomed(work, scale, self.zoom);
        (self.window, self.grow_up) = placement(anchor, work, self.scale, downward);
    }

    fn apply_theme(&mut self) -> Result<()> {
        let t = self.theme;
        self.header_key = Default::default();
        self.frame.apply_theme(&self.renderer, &t, self.scale)?;
        self.loader.apply_theme(&t)?;
        let c = &self.anim.compositor;
        self.content.apply_theme(c, &t, self.scale)?;
        self.back.apply_theme(&self.renderer, &t, self.scale)?;
        self.translate.apply_theme(&self.renderer, &t, self.scale)?;
        self.speak.apply_theme(&self.renderer, &t, self.scale)?;
        self.pin.apply_theme(&self.renderer, &t, self.scale)?;
        self.badge.apply_theme(&self.renderer, &t, self.scale)?;
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
            self.content.size_host(&self.anim, host_size, true)?;
        } else {
            self.frame.resize(&self.anim, w, card_h, offset, s, false)?;
            self.content.size_host(&self.anim, host_size, false)?;
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
        let region = RECT {
            left: 0,
            top: top as i32,
            right: w as i32,
            bottom: bottom.ceil() as i32,
        };
        win::set_region(self.hwnd, region, self.visible);
    }
}

fn contains(r: RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
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
    fn grows_up_from_below_the_work_area() {
        // A selection in the taskbar: the card stays above it, on screen.
        let (window, grow_up) = placement(word_at(500, 1050), SCREEN, 1.0, false);
        assert!(grow_up);
        assert_eq!(
            window.bottom - MARGIN as i32,
            SCREEN.bottom - SCREEN_GAP as i32
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
    fn zooms_only_as_far_as_the_card_fits() {
        assert_eq!(zoomed(SCREEN, 1.0, 1.5), 1.5);
        assert_eq!(zoomed(SCREEN, 1.5, 0.8), 1.2);
        // 1040 pixels tall: the tallest card fits up to about 2.2 times.
        let most = zoomed(SCREEN, 2.0, 2.0);
        assert!(most < 2.3 && most > 2.0, "{most}");
        assert!((MAX_CARD_H + 2.0 * SCREEN_GAP) * most <= 1040.0);
        // Too small a screen for the card unzoomed: as Windows has it.
        assert_eq!(zoomed(SCREEN, 3.0, 1.5), 3.0);
    }

    #[test]
    fn keeps_a_zoomed_card_in_place() {
        let (window, _) = placement(word_at(500, 100), SCREEN, 1.0, false);
        let larger = rescaled(window, false, 1.0, 1.5, SCREEN);
        let card = |w: RECT, s: f32| (w.left + (MARGIN * s) as i32, w.top + (MARGIN * s) as i32);
        assert_eq!(card(larger, 1.5), card(window, 1.0));
        assert_eq!(
            larger.right - larger.left,
            ((CARD_W + 2.0 * MARGIN) * 1.5) as i32
        );
        // Growing up, its bottom stays put.
        let (window, _) = placement(word_at(500, 950), SCREEN, 1.0, false);
        let larger = rescaled(window, true, 1.0, 1.5, SCREEN);
        assert_eq!(
            larger.bottom - (MARGIN * 1.5) as i32,
            window.bottom - MARGIN as i32
        );
        // Near an edge, it moves only as far as it must to fit.
        let (window, _) = placement(word_at(1700, 100), SCREEN, 1.0, false);
        let larger = rescaled(window, false, 1.0, 2.0, SCREEN);
        let edge = (SCREEN_GAP * 2.0) as i32;
        assert_eq!(larger.right - (MARGIN * 2.0) as i32, SCREEN.right - edge);
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
