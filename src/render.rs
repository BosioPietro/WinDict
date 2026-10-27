//! Text layout (DirectWrite) and drawing (Direct2D) into composition surfaces.
//!
//! This file has the layout types and the [`Renderer`] itself; `text` lays out
//! lines and fields, `body` the card's contents, and `draw` paints layouts
//! and shapes into surfaces.

mod body;
mod draw;
mod flags;
mod text;

use std::cell::{Cell, RefCell};

use windows::core::{w, Interface, Result, HSTRING};
use windows::Foundation::Size;
use windows::Graphics::DirectX::{DirectXAlphaMode, DirectXPixelFormat};
use windows::Win32::Foundation::{E_POINTER, HMODULE, POINT};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::System::WinRT::Composition::*;
use windows::UI::Composition::{CompositionDrawingSurface, CompositionGraphicsDevice, Compositor};
use windows_numerics::{Matrix3x2, Vector2};

use crate::dictionary::{self, percent_encode, Definition};
use crate::dicts;
use crate::i18n::{fill, tr};
use crate::lang::LangCode;
use crate::theme::{to_d2d, Theme};
use crate::translate::{self, Translation};
use flags::Flag;

#[derive(Clone, Copy)]
pub enum Ink {
    Primary,
    Secondary,
    Tertiary,
    Accent,
    /// Says something went wrong.
    Critical,
}

/// Where a line of text sits in the width it's given.
#[derive(Clone, Copy)]
pub enum Align {
    Center,
    Right,
}

enum Item {
    Text {
        layout: IDWriteTextLayout,
        x: f32,
        y: f32,
        ink: Ink,
        /// Ranges drawn in the accent color instead.
        accents: Vec<DWRITE_TEXT_RANGE>,
    },
    Rule {
        y: f32,
        x: f32,
        width: f32,
    },
    /// The rounded cap behind a key's name.
    Key {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    },
    /// The Windows logo, `size` square, in the primary ink.
    Logo {
        x: f32,
        y: f32,
        size: f32,
    },
    /// A language's flag.
    Flag {
        flag: Flag,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    },
    /// A rounded rectangle, filled, outlined with a hairline, or both.
    Shape {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        radius: f32,
        fill: Option<Paint>,
        stroke: Option<Paint>,
    },
}

/// The colors of an [`Item::Shape`].
#[derive(Clone, Copy)]
enum Paint {
    /// A control's face, and its outline.
    Control,
    ControlStroke,
    /// The list a language is picked from.
    Field,
    /// The accent, as thick as the rings round focused controls.
    Focus,
}

/// Laid-out content, in DIPs.
pub struct Layout {
    items: Vec<Item>,
    pub width: f32,
    pub height: f32,
    /// Clickable areas, in DIPs relative to the layout.
    pub links: Vec<Link>,
    /// The caret of the language box being typed in, which the consumer
    /// draws itself so that it can blink.
    pub caret: Option<Mark>,
    /// The list a language is being picked from, which the consumer shows
    /// itself so that it can scroll: only its outline is drawn.
    pub list: Option<Box<ListLayout>>,
}

/// A list that scrolls in a layout: all its rows, laid out apart, and the
/// band of the layout they show through.
pub struct ListLayout {
    /// From the top left of the first row.
    pub rows: Layout,
    /// The list's outline, and the band in it the rows show through, in
    /// DIPs in the layout.
    pub frame: Mark,
    pub viewport: Mark,
    pub row_h: f32,
    /// The corners of the fill behind the row Enter picks.
    pub radius: f32,
    /// Which row Enter picks.
    pub highlight: Option<usize>,
}

/// The card's heading, laid out: the word, and its pronunciation apart from
/// it, so that it can fade in once it's known.
pub struct Heading {
    /// The word, from the heading's top left.
    pub word: Layout,
    /// The size the word takes, which tells how it wraps.
    pub word_size: (f32, f32),
    /// The pronunciation, and where it goes in the heading.
    pub phonetic: Option<Placed>,
    /// Where the button after the pronunciation goes: its left edge, and
    /// the middle of the line it's on.
    pub button: Option<(f32, f32)>,
    /// The whole heading's.
    pub height: f32,
}

/// A layout, and where it goes in another, in DIPs.
pub struct Placed {
    pub layout: Layout,
    pub x: f32,
    pub y: f32,
}

/// The room between the heading's text and the button after it, in DIPs.
const HEADING_BUTTON_GAP: f32 = 2.0;

/// A rectangle in a layout, in DIPs.
#[derive(Clone, Copy)]
pub struct Mark {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Layout {
    /// The bottom of the first line of text, in DIPs.
    pub fn first_line_bottom(&self) -> f32 {
        self.items
            .iter()
            .find_map(|item| match item {
                Item::Text { layout, y, .. } => Some(y + first_line_height(layout)),
                Item::Rule { .. }
                | Item::Key { .. }
                | Item::Logo { .. }
                | Item::Flag { .. }
                | Item::Shape { .. } => None,
            })
            .unwrap_or(0.0)
    }

    /// How far down the middle of the capitals on its first line is, in
    /// DIPs: what a symbol beside the text lines up with.
    pub fn cap_middle(&self) -> f32 {
        self.items
            .iter()
            .find_map(|item| match item {
                Item::Text { layout, y, .. } => {
                    let (_, _, baseline) = metrics(layout);
                    // The format's size: the layout's own asks where in the text.
                    let format: &IDWriteTextFormat = layout;
                    let size = unsafe { format.GetFontSize() };
                    Some(y + baseline - size * CAP_HEIGHT / 2.0)
                }
                _ => None,
            })
            .unwrap_or(self.height / 2.0)
    }
}

/// How tall Segoe UI's capitals are, in ems.
const CAP_HEIGHT: f32 = 0.7;

#[derive(Clone, Debug)]
pub struct Link {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub target: Target,
}

/// What a click on a link does.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// Opens a web page.
    Url(String),
    /// Looks the word up in the same card.
    Word(String),
    /// The card's translate button: turns it into the word's translation.
    Translate,
    /// The card's speak button: says the word out loud.
    Speak,
    /// The card's back button.
    Back,
    /// The card's pin button: keeps it open, or lets it go.
    Pin,
    /// Lists the languages to translate from, or into.
    Language(Side),
    /// Lists the languages WinDict can be shown in.
    Interface,
    /// Translates the other way.
    Swap,
    /// Translates from, or into, the language with this code.
    Pick(LangCode),
    /// Translates the way the card offers.
    Confirm,
}

/// Either language of a translation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    From,
    To,
}

/// The card turned into the word's translation.
pub struct TranslationCard<'a> {
    pub word: &'a str,
    /// The languages' codes.
    pub from: &'a str,
    pub to: &'a str,
    pub state: Translated<'a>,
    /// The list a language is being picked from, in place of the
    /// translation, and the side it's picked for.
    pub picker: Option<(Side, Picker<'a>)>,
}

/// The languages to pick one from, listed under a box, which takes what's
/// typed to narrow them down.
pub struct Picker<'a> {
    pub query: &'a str,
    /// Where the caret is in `query`, in UTF-16 units.
    pub caret: u32,
    /// The codes of the languages that match.
    pub matches: &'a [&'static str],
    /// Which of `matches` Enter picks.
    pub highlight: Option<usize>,
}

impl Picker<'_> {
    /// How many rows the list shows at once: one, saying none match, if
    /// none do.
    fn shown(&self) -> usize {
        self.matches.len().clamp(1, PICKER_ROWS)
    }

    /// How much shorter the list is than with all its rows shown, in DIPs.
    pub fn spare(&self) -> f32 {
        (PICKER_ROWS - self.shown()) as f32 * ROW_H
    }
}

/// How many languages the list shows at once; the rest scroll.
pub const PICKER_ROWS: usize = 7;

/// How far a translation has got.
pub enum Translated<'a> {
    /// The word is in both languages: which way is up to the user.
    Asking,
    /// On its way, too briefly yet to say so.
    Waiting,
    Loading,
    Done(&'a Translation),
    Failed(&'a str),
}

/// How far down a translation's body its languages start, in DIPs: room
/// for their rings, which reach above them.
pub const LANGUAGES_TOP: f32 = 4.0;

/// The word's size in the heading, and the search box's text size, in DIPs.
pub const WORD_SIZE: f32 = 26.0;
pub const QUERY_SIZE: f32 = 17.0;
/// The size of a suggestion under the search box, in DIPs.
pub const SUGGESTION_SIZE: f32 = 14.0;
/// The stroke of the ring around what Tab, or typing, has focused, as on
/// Windows 11 controls, in DIPs.
pub const FOCUS_STROKE: f32 = 2.0;

/// Room for the sense numbers, in DIPs.
const SENSE_INDENT: f32 = 22.0;
/// "Open in new window", after links that open the browser.
const EXTERNAL_ICON: &str = "\u{E8A7}";
/// "Switch", between the languages of a translation, and the chevrons after
/// them.
const SWAP_ICON: &str = "\u{E8AB}";
const CHEVRON_DOWN: &str = "\u{E70D}";
const CHEVRON_UP: &str = "\u{E70E}";
const SEARCH_ICON: &str = "\u{E721}";
const CHECK_ICON: &str = "\u{E73E}";
// A translation's languages, in DIPs: a box for each, the swap button
// between them, and the list under the one being picked.
pub const SELECT_H: f32 = 36.0;
const SELECT_PAD: f32 = 10.0;
const CONTROL_RADIUS: f32 = 5.0;
const SWAP_W: f32 = 32.0;
const SWAP_GAP: f32 = 6.0;
const LIST_GAP: f32 = 6.0;
const LIST_PAD: f32 = 4.0;
const LIST_RADIUS: f32 = 8.0;
const ROW_H: f32 = 32.0;
/// Room for a flag or a code's cap, so the names line up.
const BADGE_W: f32 = 24.0;
const CARET_W: f32 = 1.5;
/// The Windows logo on a shortcut's cap, in DIPs: four panes, a gap apart.
const LOGO_SIZE: f32 = 11.0;
const LOGO_GAP: f32 = 1.0;
/// The paragraph face's size, and its caps for keys within the line: how
/// far they reach above the baseline and below it, in ems.
const PARAGRAPH_SIZE: f32 = 14.0;
/// The names on those caps, a little smaller than the text around them.
const INLINE_KEY_SIZE: f32 = 12.0;
const INLINE_CAP_RISE: f32 = 1.0;
const INLINE_CAP_DROP: f32 = 0.35;

/// A layout being built top to bottom.
#[derive(Default)]
struct Stack {
    items: Vec<Item>,
    links: Vec<Link>,
    caret: Option<Mark>,
    list: Option<Box<ListLayout>>,
    /// Top of the current line.
    y: f32,
}

impl Stack {
    /// Places text on the current line, `x` from the left. Returns its width
    /// and height; moving down is up to the caller.
    fn text(&mut self, layout: IDWriteTextLayout, x: f32, ink: Ink) -> (f32, f32) {
        self.text_with_accents(layout, x, ink, Vec::new())
    }

    /// Like [`Stack::text`], with `accents` drawn in the accent color.
    fn text_with_accents(
        &mut self,
        layout: IDWriteTextLayout,
        x: f32,
        ink: Ink,
        accents: Vec<DWRITE_TEXT_RANGE>,
    ) -> (f32, f32) {
        let (w, h, _) = metrics(&layout);
        self.items.push(Item::Text {
            layout,
            x,
            y: self.y,
            ink,
            accents,
        });
        (w, h)
    }
}

/// The typeface of a line of menu text.
#[derive(Clone, Copy)]
pub enum Face {
    Menu,
    /// Smaller, for the menu's heading.
    Small,
    /// Symbol glyphs, such as the check mark.
    Icon,
    /// A larger symbol, centred in its width, for a button.
    Button,
    /// Larger still, for a symbol with fine detail.
    LargeButton,
    /// The search box's heading.
    Title,
    /// What's typed in the search box.
    Query,
    /// The settings window's heading.
    Heading,
    /// The title of a step of the introduction.
    Display,
    /// A setting's name.
    Setting,
    /// A number shown by a setting.
    Value,
    /// A small symbol, such as the close button's cross.
    Chrome,
}

/// What the body of the card shows.
pub enum Body<'a> {
    /// On its way; with `status`, the heading says what's happening
    /// rather than naming a word, and shows muted.
    Loading {
        status: bool,
    },
    Definition(&'a Definition),
    /// `suggestion`, if any, is offered after `detail` as a link to its own
    /// definition: "Did you mean embody?"
    Message {
        title: &'a str,
        detail: &'a str,
        suggestion: Option<&'a str>,
    },
    Translation(TranslationCard<'a>),
}

/// Where the caret goes in a laid-out text box, in DIPs.
pub struct Caret {
    pub x: f32,
    pub top: f32,
    pub height: f32,
}

struct Formats {
    word: IDWriteTextFormat,
    /// The search box's text, on a single line.
    query: IDWriteTextFormat,
    /// A suggestion under the search box, on a single line.
    suggestion: IDWriteTextFormat,
    phonetic: IDWriteTextFormat,
    part_of_speech: IDWriteTextFormat,
    number: IDWriteTextFormat,
    body: IDWriteTextFormat,
    example: IDWriteTextFormat,
    meta: IDWriteTextFormat,
    footer: IDWriteTextFormat,
    title: IDWriteTextFormat,
    menu: IDWriteTextFormat,
    icon: IDWriteTextFormat,
    /// [`EXTERNAL_ICON`], sized for the footer.
    external: IDWriteTextFormat,
    button: IDWriteTextFormat,
    large_button: IDWriteTextFormat,
    heading: IDWriteTextFormat,
    display: IDWriteTextFormat,
    /// Wrapped text, such as what a step of the introduction says.
    paragraph: IDWriteTextFormat,
    setting: IDWriteTextFormat,
    value: IDWriteTextFormat,
    chrome: IDWriteTextFormat,
}

thread_local! {
    /// A drawing failed because the GPU went away: a driver update, a
    /// reset, a remote session.
    static DEVICE_LOST: Cell<bool> = const { Cell::new(false) };
}

/// Whether a drawing failed because the GPU went away since this was last
/// asked: what every window shows is gone, and is to be drawn again.
pub fn take_device_lost() -> bool {
    DEVICE_LOST.take()
}

pub struct Renderer {
    d3d: RefCell<ID3D11Device>,
    d2d_device: RefCell<ID2D1Device>,
    d2d: ID2D1Factory1,
    dwrite: IDWriteFactory,
    formats: Formats,
    pub graphics: CompositionGraphicsDevice,
    /// How many times the devices were made again, the GPU gone.
    generation: Cell<u32>,
}

impl Renderer {
    pub fn new(compositor: &Compositor) -> Result<Renderer> {
        unsafe {
            let d3d = create_d3d_device()?;
            let factory: ID2D1Factory1 =
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let d2d_device = factory.CreateDevice(&d3d.cast::<IDXGIDevice>()?)?;
            let interop: ICompositorInterop = compositor.cast()?;
            let graphics = interop.CreateGraphicsDevice(&d2d_device)?;
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let formats = create_formats(&dwrite)?;
            Ok(Renderer {
                d3d: RefCell::new(d3d),
                d2d_device: RefCell::new(d2d_device),
                d2d: factory,
                dwrite,
                formats,
                graphics,
                generation: Cell::new(0),
            })
        }
    }

    /// Hands unused GPU memory back to the driver while we sit idle.
    pub fn trim(&self) {
        let d3d = self.d3d.borrow();
        if let Ok(dxgi) = d3d.cast::<IDXGIDevice3>() {
            unsafe {
                // As Trim asks, nothing the context holds keeps memory.
                if let Ok(context) = d3d.GetImmediateContext() {
                    context.ClearState();
                }
                dxgi.Trim();
            }
        }
    }

    /// The widest and tallest surface the GPU draws, in pixels.
    pub fn max_px(&self) -> f32 {
        let level = unsafe { self.d3d.borrow().GetFeatureLevel() };
        if level.0 >= D3D_FEATURE_LEVEL_11_0.0 {
            16_384.0
        } else if level.0 >= D3D_FEATURE_LEVEL_10_0.0 {
            8_192.0
        } else {
            4_096.0
        }
    }

    /// Whether the GPU drawn with went away.
    pub fn lost(&self) -> bool {
        unsafe { self.d3d.borrow().GetDeviceRemovedReason() }.is_err()
    }

    /// Draws with a GPU again, if the one drawn with went away: the
    /// surfaces stay, emptied, for the caller to draw everything again.
    /// Returns whether it had gone.
    pub fn recover(&self) -> Result<bool> {
        if !self.lost() {
            return Ok(false);
        }
        unsafe {
            let d3d = create_d3d_device()?;
            let d2d_device = self.d2d.CreateDevice(&d3d.cast::<IDXGIDevice>()?)?;
            let interop: ICompositionGraphicsDeviceInterop = self.graphics.cast()?;
            interop.SetRenderingDevice(&d2d_device)?;
            *self.d3d.borrow_mut() = d3d;
            *self.d2d_device.borrow_mut() = d2d_device;
        }
        self.generation.set(self.generation.get() + 1);
        Ok(true)
    }

    /// Changes when the devices are made again: what was drawn before is
    /// gone.
    pub fn generation(&self) -> u32 {
        self.generation.get()
    }

    fn text(
        &self,
        text: &str,
        format: &IDWriteTextFormat,
        width: f32,
    ) -> Result<IDWriteTextLayout> {
        let wide: Vec<u16> = text.encode_utf16().collect();
        unsafe {
            self.dwrite
                .CreateTextLayout(&wide, format, width.max(1.0), 10_000.0)
        }
    }
}

fn create_d3d_device() -> Result<ID3D11Device> {
    let mut result = Err(windows::core::Error::empty());
    for driver in [D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP] {
        let mut device = None;
        result = unsafe {
            D3D11CreateDevice(
                None,
                driver,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                None,
            )
        }
        .and_then(|_| device.ok_or_else(|| windows::core::Error::from(E_POINTER)));
        if result.is_ok() {
            break;
        }
    }
    result
}

fn font_exists(dwrite: &IDWriteFactory, name: &HSTRING) -> bool {
    unsafe {
        let mut collection = None;
        if dwrite
            .GetSystemFontCollection(&mut collection, false)
            .is_err()
        {
            return false;
        }
        let Some(collection) = collection else {
            return false;
        };
        let mut index = 0;
        let mut exists = windows::core::BOOL(0);
        collection
            .FindFamilyName(name, &mut index, &mut exists)
            .is_ok()
            && exists.as_bool()
    }
}

fn create_formats(dwrite: &IDWriteFactory) -> Result<Formats> {
    // Windows 11 ships the variable Segoe UI; Windows 10 has the classic one.
    let pick = |preferred: &str| {
        let name = HSTRING::from(preferred);
        if font_exists(dwrite, &name) {
            name
        } else {
            HSTRING::from("Segoe UI")
        }
    };
    let display = pick("Segoe UI Variable Display");
    let text = pick("Segoe UI Variable Text");
    let small = pick("Segoe UI Variable Small");

    let make = |family: &HSTRING,
                size: f32,
                weight: DWRITE_FONT_WEIGHT,
                style: DWRITE_FONT_STYLE,
                line: f32| unsafe {
        let format = dwrite.CreateTextFormat(
            family,
            None,
            weight,
            style,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!("en-us"),
        )?;
        format.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
        if line > 0.0 {
            let spacing = size * line;
            format.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, spacing, spacing * 0.8)?;
        }
        Ok::<_, windows::core::Error>(format)
    };
    let normal = DWRITE_FONT_STYLE_NORMAL;
    let menu = make(&text, 13.0, DWRITE_FONT_WEIGHT_NORMAL, normal, 0.0)?;
    unsafe { menu.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)? };
    // Windows 11 ships Segoe Fluent Icons; Windows 10 has Segoe MDL2 Assets.
    let fluent = HSTRING::from("Segoe Fluent Icons");
    let symbols = if font_exists(dwrite, &fluent) {
        fluent
    } else {
        HSTRING::from("Segoe MDL2 Assets")
    };
    let icon = make(&symbols, 11.0, DWRITE_FONT_WEIGHT_NORMAL, normal, 0.0)?;
    let external = make(&symbols, 9.5, DWRITE_FONT_WEIGHT_NORMAL, normal, 0.0)?;
    let button = make(&symbols, 14.0, DWRITE_FONT_WEIGHT_NORMAL, normal, 0.0)?;
    unsafe { button.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)? };
    let large_button = make(&symbols, 17.0, DWRITE_FONT_WEIGHT_NORMAL, normal, 0.0)?;
    unsafe { large_button.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)? };
    let query = make(&text, QUERY_SIZE, DWRITE_FONT_WEIGHT_NORMAL, normal, 0.0)?;
    unsafe { query.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)? };
    let suggestion = make(
        &text,
        SUGGESTION_SIZE,
        DWRITE_FONT_WEIGHT_NORMAL,
        normal,
        0.0,
    )?;
    unsafe { suggestion.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)? };
    let setting = make(&text, 14.0, DWRITE_FONT_WEIGHT_NORMAL, normal, 0.0)?;
    unsafe { setting.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)? };
    let value = make(&text, 14.0, DWRITE_FONT_WEIGHT_SEMI_BOLD, normal, 0.0)?;
    unsafe { value.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)? };
    let chrome = make(&symbols, 10.0, DWRITE_FONT_WEIGHT_NORMAL, normal, 0.0)?;
    unsafe { chrome.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)? };
    Ok(Formats {
        word: make(
            &display,
            WORD_SIZE,
            DWRITE_FONT_WEIGHT_SEMI_BOLD,
            normal,
            0.0,
        )?,
        query,
        suggestion,
        phonetic: make(&text, 15.0, DWRITE_FONT_WEIGHT_NORMAL, normal, 0.0)?,
        part_of_speech: make(&small, 11.5, DWRITE_FONT_WEIGHT_SEMI_BOLD, normal, 0.0)?,
        number: make(&text, 13.5, DWRITE_FONT_WEIGHT_SEMI_BOLD, normal, 1.45)?,
        body: make(&text, 14.5, DWRITE_FONT_WEIGHT_NORMAL, normal, 1.4)?,
        example: make(
            &text,
            13.5,
            DWRITE_FONT_WEIGHT_NORMAL,
            DWRITE_FONT_STYLE_ITALIC,
            1.4,
        )?,
        meta: make(&text, 13.0, DWRITE_FONT_WEIGHT_NORMAL, normal, 1.4)?,
        footer: make(&small, 12.0, DWRITE_FONT_WEIGHT_NORMAL, normal, 0.0)?,
        title: make(&text, 15.0, DWRITE_FONT_WEIGHT_SEMI_BOLD, normal, 0.0)?,
        menu,
        icon,
        external,
        button,
        large_button,
        heading: make(&display, 20.0, DWRITE_FONT_WEIGHT_SEMI_BOLD, normal, 0.0)?,
        display: make(&display, 24.0, DWRITE_FONT_WEIGHT_SEMI_BOLD, normal, 0.0)?,
        paragraph: make(
            &text,
            PARAGRAPH_SIZE,
            DWRITE_FONT_WEIGHT_NORMAL,
            normal,
            1.55,
        )?,
        setting,
        value,
        chrome,
    })
}

/// "serendipity" → "Serendipity". Only the first letter changes.
pub fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// (width, height, first baseline) of a laid-out text.
fn metrics(layout: &IDWriteTextLayout) -> (f32, f32, f32) {
    unsafe {
        let mut m = DWRITE_TEXT_METRICS::default();
        let _ = layout.GetMetrics(&mut m);
        let mut lines = vec![DWRITE_LINE_METRICS::default(); m.lineCount.max(1) as usize];
        let mut count = 0;
        let _ = layout.GetLineMetrics(Some(&mut lines), &mut count);
        (
            m.widthIncludingTrailingWhitespace,
            m.height,
            lines[0].baseline,
        )
    }
}

/// The rectangles covering `range` of a laid-out text, one per line it spans.
fn hit_rects(layout: &IDWriteTextLayout, range: DWRITE_TEXT_RANGE) -> Vec<DWRITE_HIT_TEST_METRICS> {
    let (start, length) = (range.startPosition, range.length);
    unsafe {
        // The first call only counts them.
        let mut count = 0;
        let _ = layout.HitTestTextRange(start, length, 0.0, 0.0, None, &mut count);
        let mut rects = vec![DWRITE_HIT_TEST_METRICS::default(); count as usize];
        if layout
            .HitTestTextRange(start, length, 0.0, 0.0, Some(&mut rects), &mut count)
            .is_err()
        {
            return Vec::new();
        }
        rects.truncate(count as usize);
        rects
    }
}

/// Where `text`, laid out in `layout`, ends: right after its last
/// character, and the baseline of the line that's on.
fn line_end(layout: &IDWriteTextLayout, text: &str) -> (f32, f32) {
    let lines = unsafe {
        let mut m = DWRITE_TEXT_METRICS::default();
        let _ = layout.GetMetrics(&mut m);
        let mut lines = vec![DWRITE_LINE_METRICS::default(); m.lineCount.max(1) as usize];
        let mut count = 0;
        let _ = layout.GetLineMetrics(Some(&mut lines), &mut count);
        lines.truncate(count.max(1) as usize);
        lines
    };
    let (last, above) = lines.split_last().expect("one line at least");
    let baseline = above.iter().map(|l| l.height).sum::<f32>() + last.baseline;
    let len = text.encode_utf16().count() as u32;
    if len == 0 {
        return (0.0, baseline);
    }
    let (mut x, mut y) = (0.0, 0.0);
    let mut hit = DWRITE_HIT_TEST_METRICS::default();
    let _ = unsafe { layout.HitTestTextPosition(len - 1, true, &mut x, &mut y, &mut hit) };
    (x, baseline)
}

/// The height of the first line of a laid-out text.
fn first_line_height(layout: &IDWriteTextLayout) -> f32 {
    let mut lines = [DWRITE_LINE_METRICS::default(); 1];
    let mut count = 0;
    // Fails with "buffer too small" when there are more lines, after filling in the first.
    let _ = unsafe { layout.GetLineMetrics(Some(&mut lines), &mut count) };
    lines[0].height
}
