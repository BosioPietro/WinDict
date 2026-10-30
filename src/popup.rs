//! The definition card: a borderless, non-activating, top-most window whose
//! content is a Windows.UI.Composition visual tree. Blur comes from the host
//! backdrop brush; every animation runs on the system compositor, so motion
//! stays smooth no matter what our UI thread is doing.

mod body;
mod focus;
mod parts;
mod placement;
mod scrolling;
mod search;
mod selecting;

use std::rc::Rc;

use windows::core::{Interface, Result, HSTRING};
use windows::Win32::Foundation::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::*;
use windows_numerics::{Vector2, Vector3};

use crate::i18n::tr;
use crate::render::{
    self, Body, Face, Ink, Link, ListLayout, Mark, Renderer, Target, TextPoint, LANGUAGES_TOP,
    WORD_SIZE,
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
pub use placement::{anchor_beside, anchor_where, centred_anchor};
use placement::{placement, rescaled, zoomed};
use scrolling::ScrollChrome;

// Geometry, in DIPs.
pub(crate) const CARD_W: f32 = 384.0;
const MAX_CARD_H: f32 = 460.0;
pub(crate) const PAD_X: f32 = 22.0;
pub(crate) const PAD_TOP: f32 = 18.0;
const HEADER_GAP: f32 = 12.0;
const PAD_BOTTOM: f32 = 18.0;
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
/// "Pinned" in Segoe Fluent Icons and Segoe MDL2 Assets, and "Pinned fill",
/// which fills its head in but leaves out the needle.
const PIN_GLYPH: &str = "\u{E840}";
const PIN_FILL_GLYPH: &str = "\u{E842}";
/// How far each button left of the pin button is from the next.
const BUTTON_STEP: f32 = BACK_SIZE + 2.0;
/// "Translate" in Segoe Fluent Icons and Segoe MDL2 Assets.
const TRANSLATE_GLYPH: &str = "\u{E8C1}";
/// The swap button's tooltip's tool, after the back and translate buttons'.
const SWAP_TOOL: usize = 3;
/// The speak button's tooltip's tool, after the swap button's.
const SPEAK_TOOL: usize = 4;
/// The pin button's tooltip's tool, after the speak button's.
const PIN_TOOL: usize = 5;
/// The scan button's tooltip's tool, after the pin button's.
const SCAN_TOOL: usize = 6;
/// The Wikipedia button's tooltip's tool, after the scan button's.
const WIKIPEDIA_TOOL: usize = 7;
/// Wikipedia's W, in a serif face as on its logo.
const WIKIPEDIA_GLYPH: &str = "W";
/// "Text recognition" in Segoe Fluent Icons: an A in a dashed frame.
const SCAN_GLYPH: &str = "\u{F683}";
/// "Screen clipping", scissors over a screen, for Segoe MDL2 Assets, which
/// has no text recognition.
const SCAN_GLYPH_MDL2: &str = "\u{F406}";
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
/// How long a body that changed in place takes to fade over.
const CROSSFADE_MS: u64 = 240;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Transition {
    /// No animation (the card is appearing).
    Instant,
    /// Different content: the body fades in, the card resizes.
    Replace,
    /// The same entry, refined: no fade, scroll position kept.
    Refine,
    /// The same entry with something added, like a link or a picture
    /// that came later: it cross-fades in, scroll position kept.
    Reveal,
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
    renderer: Rc<Renderer>,
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
    /// Shows the word's Wikipedia article: between the translate and pin
    /// buttons, once it's known there's one.
    wikipedia: IconButton,
    /// Says the word: after its pronunciation, and moving with the heading.
    speak: IconButton,
    /// Keeps the card open, or lets it go again.
    pin: IconButton,
    /// Reads text off the screen into the search box: there only with it.
    scan: IconButton,
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
    /// The left button, down on the body's text.
    selecting: Option<selecting::Selecting>,
    last_press: Option<selecting::Press>,
    pub visible: bool,
    /// Pinned open: a window of its own, dragged by its heading.
    pinned: bool,
    /// Last, so it goes after everything shown on it.
    _window: Window,
}

/// One of the buttons on the card.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CardButton {
    Speak,
    Translate,
    Wikipedia,
    Pin,
    Scan,
    Back,
}

impl CardButton {
    /// Every button, in the order Tab goes through them: the ones by the
    /// word as they read, and the back button last, so Tab doesn't start on
    /// it.
    const ALL: [CardButton; 6] = [
        Self::Speak,
        Self::Translate,
        Self::Wikipedia,
        Self::Pin,
        Self::Scan,
        Self::Back,
    ];

    /// What pressing the button does.
    fn target(self) -> &'static Target {
        static SPEAK: Target = Target::Speak;
        static TRANSLATE: Target = Target::Translate;
        static WIKIPEDIA: Target = Target::Wikipedia;
        static PIN: Target = Target::Pin;
        static SCAN: Target = Target::Scan;
        static BACK: Target = Target::Back;
        match self {
            Self::Speak => &SPEAK,
            Self::Translate => &TRANSLATE,
            Self::Wikipedia => &WIKIPEDIA,
            Self::Pin => &PIN,
            Self::Scan => &SCAN,
            Self::Back => &BACK,
        }
    }
}

/// Somewhere Tab stops in the card.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Stop {
    Button(CardButton),
    /// The link starting at this index in the body's links.
    Link(usize),
}

impl Stop {
    fn is_button(self) -> bool {
        matches!(self, Stop::Button(_))
    }
}

// ---------------------------------------------------------------------------

impl Popup {
    pub fn new(
        window: Window,
        compositor: Compositor,
        renderer: Rc<Renderer>,
        theme: Theme,
        zoom: f32,
    ) -> Result<Popup> {
        let hwnd = window.hwnd();
        let target = attach(hwnd, &compositor)?;
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
                x: PIN_X - BUTTON_STEP,
                origin: v2(0.0, 0.0),
                slide_from: 0.0,
            },
        )?;
        // Leads somewhere too, so it's colored like the links.
        let wikipedia = IconButton::new(
            c,
            &frame.card,
            hwnd,
            IconSpec {
                icon: WIKIPEDIA_GLYPH,
                face: Face::Letter,
                ink: Ink::Accent,
                tip: tr().wikipedia_tip,
                tool: WIKIPEDIA_TOOL,
                x: PIN_X - BUTTON_STEP,
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
        // Where the pin button is, by the search box's heading instead.
        let scan = IconButton::new(
            c,
            &frame.card,
            hwnd,
            IconSpec {
                icon: if renderer.fluent_icons() {
                    SCAN_GLYPH
                } else {
                    SCAN_GLYPH_MDL2
                },
                face: Face::Button,
                ink: Ink::Secondary,
                tip: tr().scan_tip,
                tool: SCAN_TOOL,
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
            wikipedia,
            speak,
            pin,
            scan,
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
            selecting: None,
            last_press: None,
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
        self.wikipedia.release();
        self.speak.release();
        self.pin.release();
        self.scan.release();
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
        // What was selected goes with what it was selected in.
        self.selecting = None;
        self.select(None)?;
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
        self.scan.set_shown(&self.anim, false, self.scale)?;
        let [_, _, left] = buttons_x(self.wikipedia.shown(), self.translate.shown());
        let room = CARD_W - PAD_X - left + 6.0;
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
        // An unchanged word keeps the surface it has, so it can't flicker.
        let header_same = self.header_key == key;
        let header_surface = match self.header.surface().filter(|_| header_same) {
            Some(surface) => surface,
            None => self.renderer.draw(&heading.word, s, &self.theme)?,
        };
        let body_surface = self.renderer.draw(body_layout, s, &self.theme)?;
        // Only once it's drawn is it the word the surface has.
        self.header_key = key;

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
            Transition::Refine | Transition::Reveal | Transition::Rescale => {
                self.focused_link().cloned()
            }
            Transition::Instant | Transition::Replace => None,
        };
        self.content.set_links(body_layout.links.clone());
        let selection = self.content.selection();
        (self.content).select(&self.anim.compositor, selection, Some(body_layout), s)?;
        let kept_button = CardButton::ALL
            .into_iter()
            .find(|&b| kept.as_ref() == Some(b.target()) && self.button(b).shown());
        if let Some(button) = kept_button {
            self.focus_button(button, 0)?;
        } else {
            let links = self.content.links();
            let focused = kept.and_then(|t| links.iter().position(|l| l.target == t));
            self.focus_link(focused, 0)?;
        }

        // Header: on a refinement, cross-fade from the old rendering of the
        // word, should it have changed.
        let refined = matches!(transition, Transition::Refine | Transition::Reveal);
        let crossfade = refined && !header_same;
        let size = v2(content_w * s, heading.height * s);
        self.header
            .show(&self.anim, &header_surface, size, crossfade, s)?;
        // The pronunciation fades in once it's known, whatever else changes.
        let moving = transition == Transition::Replace || refined;
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
        for button in [&mut self.back, &mut self.pin] {
            button.place(top, s)?;
        }
        // The translate button moves over to make room for Wikipedia's.
        let glide = transition != Transition::Instant;
        let [wikipedia_x, translate_x, _] =
            buttons_x(self.wikipedia.shown(), self.translate.shown());
        (self.wikipedia).move_to(&self.anim, wikipedia_x, top, glide, s)?;
        (self.translate).move_to(&self.anim, translate_x, top, glide, s)?;

        let size = v2(content_w * s, body_h);
        let reveal = transition == Transition::Reveal;
        self.content.show(
            &self.anim,
            &body_surface,
            size,
            body_layout.caret,
            reveal,
            s,
        )?;
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
            Transition::Refine | Transition::Reveal => {
                // Same entry with more detail: keep the reader's place.
                self.resize(card_h, true)?;
                self.scroller
                    .stop_at(self.scroller.at.min(self.max_scroll()));
                self.content.keep_place(self.scroller.at, s)?;
                self.move_content_host(previous_header_h)?;
                if transition == Transition::Reveal {
                    self.content.fade_in(&self.anim)?;
                }
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

    /// Shows or hides the Wikipedia button. The translate button and the
    /// word make room for it with the next [`Popup::set_content`].
    pub fn set_wikipedia(&mut self, shown: bool) -> Result<()> {
        self.wikipedia.set_shown(&self.anim, shown, self.scale)?;
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
        // A pinned card stays put, so the screen can't be read from behind it.
        if pinned {
            self.scan.set_shown(&self.anim, false, self.scale)?;
            self.sync_tooltip();
        }
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
        let on_button = self.button_at(x, y).is_some();
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
        if !self.visible {
            return Ok(());
        }
        let Some(shown) = self.shown.take() else {
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
            let drawn = self
                .apply_theme()
                .and_then(|()| self.draw_content(&shown, Transition::Rescale, None));
            // Kept even so, for the card to be drawn again once it can be.
            self.shown = Some(shown);
            return drawn;
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

    /// The GPU went away, and the renderer draws with another: draws
    /// everything again.
    pub fn recover(&mut self) -> Result<()> {
        self.redraw()
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
        if let Some((x, y)) = point {
            self.drag_text(x, y)?;
        }
        // On a scrollbar, or dragging its thumb, nothing else is hovered.
        let in_body = point.and_then(|(x, y)| self.body_point(x, y));
        let on_list = self.content.list().hover_bar(&self.anim, in_body)?;
        let point = if on_list { None } else { point };
        let point = if self.hover_bar(point)? { None } else { point };
        let over = point.and_then(|(x, y)| self.button_at(x, y));
        for button in CardButton::ALL {
            let (button_ui, anim) = self.button_mut(button);
            button_ui.set_hovered(anim, over == Some(button))?;
        }
        if self.search.active {
            let card_point = point.map(|(x, y)| self.card_point(x, y));
            self.search.hover(&self.anim, card_point)?;
        }
        // The languages under the field have a swap button to lighten.
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

    /// The button showing at a point in window pixels, if any.
    pub fn button_at(&self, x: i32, y: i32) -> Option<CardButton> {
        CardButton::ALL.into_iter().find(|&b| {
            let button = self.button(b);
            button.shown() && contains(self.button_rect(button), x, y)
        })
    }

    fn button(&self, button: CardButton) -> &IconButton {
        match button {
            CardButton::Speak => &self.speak,
            CardButton::Translate => &self.translate,
            CardButton::Wikipedia => &self.wikipedia,
            CardButton::Pin => &self.pin,
            CardButton::Scan => &self.scan,
            CardButton::Back => &self.back,
        }
    }

    /// `button`, with what animates it.
    fn button_mut(&mut self, button: CardButton) -> (&mut IconButton, &Animator) {
        let ui = match button {
            CardButton::Speak => &mut self.speak,
            CardButton::Translate => &mut self.translate,
            CardButton::Wikipedia => &mut self.wikipedia,
            CardButton::Pin => &mut self.pin,
            CardButton::Scan => &mut self.scan,
            CardButton::Back => &mut self.back,
        };
        (ui, &self.anim)
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
        for button in CardButton::ALL.map(|b| self.button(b)) {
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
        self.wikipedia.reset()?;
        self.speak.reset()?;
        self.pin.reset()?;
        // Shown, sliding in, with the search box.
        self.scan.hide_at_once()?;
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
        self.wikipedia.apply_theme(&self.renderer, &t, self.scale)?;
        self.speak.apply_theme(&self.renderer, &t, self.scale)?;
        self.pin.apply_theme(&self.renderer, &t, self.scale)?;
        self.scan.apply_theme(&self.renderer, &t, self.scale)?;
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

/// The left edges of the Wikipedia and translate buttons, in DIPs, with
/// each shown or not, and the leftmost of those shown and the pin button:
/// from the pin button leftwards, Wikipedia's then translate's.
fn buttons_x(wikipedia: bool, translate: bool) -> [f32; 3] {
    let wikipedia_x = PIN_X - BUTTON_STEP;
    let translate_x = if wikipedia {
        wikipedia_x - BUTTON_STEP
    } else {
        wikipedia_x
    };
    let left = match (wikipedia, translate) {
        (_, true) => translate_x,
        (true, false) => wikipedia_x,
        (false, false) => PIN_X,
    };
    [wikipedia_x, translate_x, left]
}

/// Each layout of a card's content at its place, in DIPs, and the card's
/// height.
pub(crate) type Still = (Vec<(render::Layout, f32, f32)>, f32);

/// The card's content, still: each layout at its place on a card showing
/// `word` and `body`, in DIPs, and the card's height. With `speak` and
/// `translate`, those buttons show, as they do on a definition.
pub(crate) fn still(
    renderer: &Renderer,
    word: &str,
    body: &Body,
    speak: bool,
    translate: bool,
) -> Result<Still> {
    let content_w = CARD_W - 2.0 * PAD_X;
    let [_, translate_x, left] = buttons_x(false, translate);
    let room = CARD_W - PAD_X - left + 6.0;
    let button = speak.then_some(BACK_SIZE);
    let heading = renderer.layout_header(word, None, content_w - room, 0.0, button, false)?;
    let glyph = |icon, face, ink| renderer.layout_line(icon, face, ink, BACK_SIZE);
    let mut out = Vec::new();
    // The buttons sit level with the word.
    let top = PAD_TOP + (heading.word.first_line_bottom() - BACK_SIZE) / 2.0;
    let mut buttons = vec![(PIN_GLYPH, Face::Button, Ink::Secondary, PIN_X, top)];
    if translate {
        buttons.push((
            TRANSLATE_GLYPH,
            Face::LargeButton,
            Ink::Accent,
            translate_x,
            top,
        ));
    }
    if let Some((x, middle)) = heading.button {
        let top = PAD_TOP + middle - BACK_SIZE / 2.0;
        buttons.push((SPEAK_GLYPH, Face::Button, Ink::Secondary, PAD_X + x, top));
    }
    for (icon, face, ink, x, top) in buttons {
        let layout = glyph(icon, face, ink)?;
        let y = top + (BACK_SIZE - layout.height) / 2.0;
        out.push((layout, x, y));
    }
    let header_h = PAD_TOP + heading.height + HEADER_GAP;
    let body = renderer.layout_body(body, content_w, PAD_BOTTOM)?;
    let height = header_h + body.height;
    out.push((heading.word, PAD_X, PAD_TOP));
    out.push((body, PAD_X, header_h));
    Ok((out, height))
}
