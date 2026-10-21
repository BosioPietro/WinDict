//! Text layout (DirectWrite) and drawing (Direct2D) into composition surfaces.

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
use windows::Win32::System::WinRT::Composition::*;
use windows::UI::Composition::{CompositionDrawingSurface, CompositionGraphicsDevice, Compositor};
use windows_numerics::{Matrix3x2, Vector2};

use crate::dictionary::{self, Definition};
use crate::dicts;
use crate::theme::Theme;

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
    /// A language's flag.
    Flag {
        flag: Flag,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    },
}

/// The flags drawn for the languages WinDict knows.
#[derive(Clone, Copy)]
enum Flag {
    UnitedKingdom,
    Italy,
}

impl Flag {
    fn of(language: &str) -> Option<Flag> {
        match language {
            "en" => Some(Flag::UnitedKingdom),
            "it" => Some(Flag::Italy),
            _ => None,
        }
    }
}

/// Laid-out content, in DIPs.
pub struct Layout {
    items: Vec<Item>,
    pub width: f32,
    pub height: f32,
    /// Clickable areas, in DIPs relative to the layout.
    pub links: Vec<Link>,
}

impl Layout {
    /// The bottom of the first line of text, in DIPs.
    pub fn first_line_bottom(&self) -> f32 {
        self.items
            .iter()
            .find_map(|item| match item {
                Item::Text { layout, y, .. } => Some(y + first_line_height(layout)),
                Item::Rule { .. } | Item::Key { .. } | Item::Flag { .. } => None,
            })
            .unwrap_or(0.0)
    }
}

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
}

/// The word's size in the heading, and the search box's text size, in DIPs.
pub const WORD_SIZE: f32 = 26.0;
pub const QUERY_SIZE: f32 = 17.0;
/// The size of a suggestion under the search box, in DIPs.
pub const SUGGESTION_SIZE: f32 = 14.0;

/// Room for the sense numbers, in DIPs.
const SENSE_INDENT: f32 = 22.0;
/// "Open in new window", after links that open the browser.
const EXTERNAL_ICON: &str = "\u{E8A7}";

/// A layout being built top to bottom.
#[derive(Default)]
struct Stack {
    items: Vec<Item>,
    links: Vec<Link>,
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
    /// The search box's heading.
    Title,
    /// What's typed in the search box.
    Query,
    /// The settings window's heading.
    Heading,
    /// A setting's name.
    Setting,
    /// A number shown by a setting.
    Value,
    /// A small symbol, such as the close button's cross.
    Chrome,
}

/// What the body of the card shows.
pub enum Body<'a> {
    Loading,
    Definition(&'a Definition),
    /// `suggestion`, if any, is offered after `detail` as a link to its own
    /// definition: "Did you mean embody?"
    Message {
        title: &'a str,
        detail: &'a str,
        suggestion: Option<&'a str>,
    },
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
    heading: IDWriteTextFormat,
    setting: IDWriteTextFormat,
    value: IDWriteTextFormat,
    chrome: IDWriteTextFormat,
}

pub struct Renderer {
    d3d: ID3D11Device,
    _d2d_device: ID2D1Device,
    d2d: ID2D1Factory1,
    dwrite: IDWriteFactory,
    formats: Formats,
    pub graphics: CompositionGraphicsDevice,
}

impl Renderer {
    pub fn new(compositor: &Compositor) -> Result<Renderer> {
        unsafe {
            let d3d = create_d3d_device()?;
            let dxgi: IDXGIDevice = d3d.cast()?;
            let factory: ID2D1Factory1 =
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let d2d_device = factory.CreateDevice(&dxgi)?;
            let interop: ICompositorInterop = compositor.cast()?;
            let graphics = interop.CreateGraphicsDevice(&d2d_device)?;
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let formats = create_formats(&dwrite)?;
            Ok(Renderer {
                d3d,
                _d2d_device: d2d_device,
                d2d: factory,
                dwrite,
                formats,
                graphics,
            })
        }
    }

    /// Hands unused GPU memory back to the driver while we sit idle.
    pub fn trim(&self) {
        if let Ok(dxgi) = self.d3d.cast::<IDXGIDevice3>() {
            unsafe { dxgi.Trim() };
        }
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

    /// The word and its pronunciation, `indent` DIPs from the left.
    pub fn layout_header(
        &self,
        word: &str,
        phonetic: Option<&str>,
        width: f32,
        indent: f32,
    ) -> Result<Layout> {
        let mut items = Vec::new();
        let room = width - indent;
        let title = self.text(&capitalize(word), &self.formats.word, room)?;
        let (title_w, title_h, baseline) = metrics(&title);
        let mut height = title_h;
        if let Some(p) = phonetic {
            let gap = 10.0;
            let phon = self.text(p, &self.formats.phonetic, room)?;
            let (phon_w, phon_h, phon_baseline) = metrics(&phon);
            if title_w + gap + phon_w <= room {
                // Same line, sharing the baseline.
                items.push(Item::Text {
                    layout: phon,
                    x: indent + title_w + gap,
                    y: baseline - phon_baseline,
                    ink: Ink::Secondary,
                    accents: Vec::new(),
                });
            } else {
                items.push(Item::Text {
                    layout: phon,
                    x: indent,
                    y: title_h + 2.0,
                    ink: Ink::Secondary,
                    accents: Vec::new(),
                });
                height += 2.0 + phon_h;
            }
        }
        items.insert(
            0,
            Item::Text {
                layout: title,
                x: indent,
                y: 0.0,
                ink: Ink::Primary,
                accents: Vec::new(),
            },
        );
        Ok(Layout {
            items,
            width,
            height,
            links: Vec::new(),
        })
    }

    /// What's typed in the search box, on one line at least `width` wide,
    /// its width, and where the caret goes when it's `at` UTF-16 units in.
    pub fn layout_query(&self, text: &str, at: u32, width: f32) -> Result<(Layout, f32, Caret)> {
        let line = self.text(text, &self.formats.query, width)?;
        let (w, height, baseline) = metrics(&line);
        let (mut x, mut y) = (0.0, 0.0);
        let mut hit = DWRITE_HIT_TEST_METRICS::default();
        unsafe { line.HitTestTextPosition(at, false, &mut x, &mut y, &mut hit)? };
        let caret = Caret {
            x,
            top: baseline - QUERY_SIZE * 0.86,
            height: QUERY_SIZE * 1.1,
        };
        let mut out = Stack::default();
        out.text(line, 0.0, Ink::Primary);
        let layout = Layout {
            items: out.items,
            width: width.max(w + 4.0),
            height,
            links: Vec::new(),
        };
        Ok((layout, w, caret))
    }

    /// A suggested word, its first `typed` characters (what was typed) in
    /// bold, and the word it's a form of after an arrow. `badge`, the code of
    /// its language, goes before it as its flag, when there are several, or
    /// on a cap, if it has none.
    pub fn layout_suggestion(
        &self,
        word: &str,
        typed: usize,
        base: Option<&str>,
        badge: Option<&str>,
        width: f32,
    ) -> Result<Layout> {
        let mut out = Stack::default();
        let mut x = 0.0;
        if let Some(flag) = badge.and_then(Flag::of) {
            // 3:2, the height of a capital.
            let (flag_w, flag_h, gap) = (18.0, 12.0, 8.0);
            let line_h = metrics(&self.text("Ag", &self.formats.suggestion, width)?).1;
            out.items.push(Item::Flag {
                flag,
                x: 0.0,
                y: ((line_h - flag_h) / 2.0).round(),
                width: flag_w,
                height: flag_h,
            });
            x = flag_w + gap;
        } else if let Some(code) = badge {
            // Wide enough for any two letters, so the words line up.
            let (pad_y, min_w, gap) = (1.0, 24.0f32, 8.0);
            let name = self.text(&code.to_uppercase(), &self.formats.footer, 100.0)?;
            let (w, h, _) = metrics(&name);
            let line_h = metrics(&self.text("Ag", &self.formats.suggestion, width)?).1;
            let (cap_w, cap_h) = (min_w.max(w + 8.0), h + 2.0 * pad_y);
            let y = ((line_h - cap_h) / 2.0).round();
            out.items.push(Item::Key {
                x: 0.0,
                y,
                width: cap_w,
                height: cap_h,
            });
            out.items.push(Item::Text {
                layout: name,
                x: ((cap_w - w) / 2.0).round(),
                y: y + pad_y,
                ink: Ink::Secondary,
                accents: Vec::new(),
            });
            x = cap_w + gap;
        }
        let width = (width - x).max(1.0);
        let line = self.text(word, &self.formats.suggestion, width)?;
        // DirectWrite counts UTF-16 code units.
        let bold = word.chars().take(typed).map(char::len_utf16).sum::<usize>();
        let range = DWRITE_TEXT_RANGE {
            startPosition: 0,
            length: bold as u32,
        };
        unsafe {
            let _ = line.SetFontWeight(DWRITE_FONT_WEIGHT_SEMI_BOLD, range);
        }
        let (w, height) = out.text(line, x, Ink::Primary);
        if let Some(base) = base {
            let form_of =
                self.text(&format!("\u{2192} {base}"), &self.formats.suggestion, width)?;
            out.text(form_of, x + w + 8.0, Ink::Tertiary);
        }
        Ok(Layout {
            items: out.items,
            width: x + width,
            height,
            links: Vec::new(),
        })
    }

    /// Keys and what they do, on one line: "[Enter] Look up  [Esc] Close".
    pub fn layout_keys(&self, keys: &[(&str, &str)]) -> Result<Layout> {
        let (pad_x, pad_y, gap, spacing) = (6.0, 2.0, 6.0, 14.0);
        let mut out = Stack {
            y: pad_y,
            ..Default::default()
        };
        let (mut x, mut height) = (0.0, 0.0f32);
        for (i, (key, action)) in keys.iter().enumerate() {
            if i > 0 {
                x += spacing;
            }
            let name = self.text(key, &self.formats.footer, 200.0)?;
            let (w, h, _) = metrics(&name);
            let (cap_w, cap_h) = (w + 2.0 * pad_x, h + 2.0 * pad_y);
            out.items.push(Item::Key {
                x,
                y: 0.0,
                width: cap_w,
                height: cap_h,
            });
            out.text(name, x + pad_x, Ink::Secondary);
            x += cap_w + gap;
            let action = self.text(action, &self.formats.footer, 200.0)?;
            let (w, _) = out.text(action, x, Ink::Tertiary);
            x += w;
            height = height.max(cap_h);
        }
        Ok(Layout {
            items: out.items,
            width: x.max(1.0),
            height,
            links: Vec::new(),
        })
    }

    /// One unwrapped line of menu text.
    pub fn layout_line(&self, text: &str, face: Face, ink: Ink, width: f32) -> Result<Layout> {
        let format = match face {
            Face::Menu => &self.formats.menu,
            Face::Small => &self.formats.footer,
            Face::Icon => &self.formats.icon,
            Face::Button => &self.formats.button,
            Face::Title => &self.formats.title,
            Face::Query => &self.formats.query,
            Face::Heading => &self.formats.heading,
            Face::Setting => &self.formats.setting,
            Face::Value => &self.formats.value,
            Face::Chrome => &self.formats.chrome,
        };
        let mut out = Stack::default();
        let (_, height) = out.text(self.text(text, format, width)?, 0.0, ink);
        Ok(Layout {
            items: out.items,
            width,
            height,
            links: Vec::new(),
        })
    }

    /// Like [`Renderer::layout_line`], `align`ed in `width`.
    pub fn layout_aligned(
        &self,
        text: &str,
        face: Face,
        ink: Ink,
        width: f32,
        align: Align,
    ) -> Result<Layout> {
        let layout = self.layout_line(text, face, ink, width)?;
        if let Some(Item::Text { layout: line, .. }) = layout.items.first() {
            let align = match align {
                Align::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
                Align::Right => DWRITE_TEXT_ALIGNMENT_TRAILING,
            };
            unsafe { line.SetTextAlignment(align)? };
        }
        Ok(layout)
    }

    /// A small heading in spaced capitals, like a part of speech.
    pub fn layout_label(&self, text: &str, ink: Ink, width: f32) -> Result<Layout> {
        let mut out = Stack::default();
        let (_, height) = out.text(self.spaced_caps(text, width)?, 0.0, ink);
        Ok(Layout {
            items: out.items,
            width,
            height,
            links: Vec::new(),
        })
    }

    /// The keys of a shortcut, each on its own cap: "[Ctrl] [Alt] [D]".
    pub fn layout_caps(&self, keys: &[&str]) -> Result<Layout> {
        let (pad_x, pad_y, gap) = (8.0, 3.0, 4.0);
        let mut out = Stack {
            y: pad_y,
            ..Default::default()
        };
        let (mut x, mut height) = (0.0, 0.0f32);
        for (i, key) in keys.iter().enumerate() {
            if i > 0 {
                x += gap;
            }
            let name = self.text(key, &self.formats.menu, 200.0)?;
            let (w, h, _) = metrics(&name);
            // At least square, so one-letter keys don't look pinched.
            let cap_h = h + 2.0 * pad_y;
            let cap_w = (w + 2.0 * pad_x).max(cap_h);
            out.items.push(Item::Key {
                x,
                y: 0.0,
                width: cap_w,
                height: cap_h,
            });
            out.text(name, x + (cap_w - w) / 2.0, Ink::Primary);
            x += cap_w;
            height = height.max(cap_h);
        }
        Ok(Layout {
            items: out.items,
            width: x.max(1.0),
            height,
            links: Vec::new(),
        })
    }

    /// A link that opens outside WinDict, as in the card's footer.
    pub fn layout_link(&self, text: &str) -> Result<Layout> {
        let mut out = Stack::default();
        let (width, height) = self.external_link(&mut out, text, 0.0, 400.0)?;
        Ok(Layout {
            items: out.items,
            width: width.ceil() + 1.0,
            height,
            links: Vec::new(),
        })
    }

    /// `text` in the accent color, underlined, then the icon for links that
    /// open the browser. Returns its width and height.
    fn external_link(&self, out: &mut Stack, text: &str, x: f32, width: f32) -> Result<(f32, f32)> {
        let label = self.text(text, &self.formats.footer, width)?;
        let all = DWRITE_TEXT_RANGE {
            startPosition: 0,
            length: u32::MAX,
        };
        unsafe {
            let _ = label.SetUnderline(true, all);
        }
        let (w, h) = out.text(label, x, Ink::Accent);
        let gap = 3.0;
        let icon = self.text(EXTERNAL_ICON, &self.formats.external, width)?;
        let (icon_w, icon_h, _) = metrics(&icon);
        out.items.push(Item::Text {
            layout: icon,
            x: x + w + gap,
            y: out.y + (h - icon_h) / 2.0,
            ink: Ink::Accent,
            accents: Vec::new(),
        });
        Ok((w + gap + icon_w, h))
    }

    /// `text` in the small spaced capitals of a part of speech.
    fn spaced_caps(&self, text: &str, width: f32) -> Result<IDWriteTextLayout> {
        let caps = self.text(&text.to_uppercase(), &self.formats.part_of_speech, width)?;
        if let Ok(caps1) = caps.cast::<IDWriteTextLayout1>() {
            let range = DWRITE_TEXT_RANGE {
                startPosition: 0,
                length: u32::MAX,
            };
            unsafe {
                let _ = caps1.SetCharacterSpacing(0.0, 0.9, 0.0, range);
            }
        }
        Ok(caps)
    }

    pub fn layout_body(&self, body: &Body, width: f32, bottom_padding: f32) -> Result<Layout> {
        let mut out = Stack::default();
        match body {
            // Room for the loading dots, which popup.rs draws.
            Body::Loading => out.y = 26.0,
            Body::Message {
                title,
                detail,
                suggestion,
            } => self.layout_message(&mut out, title, detail, *suggestion, width)?,
            Body::Definition(def) => {
                // In several languages, each under a heading of its own.
                let grouped = !def.others.is_empty();
                for (i, section) in def.sections().enumerate() {
                    if grouped {
                        self.layout_language(&mut out, section, i, &def.word, width)?;
                    }
                    self.layout_definition(&mut out, section, width)?;
                }
                self.layout_sources(&mut out, def, width)?;
            }
        }
        Ok(Layout {
            items: out.items,
            width,
            height: out.y + bottom_padding,
            links: out.links,
        })
    }

    fn layout_message(
        &self,
        out: &mut Stack,
        title: &str,
        detail: &str,
        suggestion: Option<&str>,
        width: f32,
    ) -> Result<()> {
        let (_, h) = out.text(
            self.text(title, &self.formats.title, width)?,
            0.0,
            Ink::Primary,
        );
        out.y += h + 4.0;
        if !detail.is_empty() {
            let d = self.text(detail, &self.formats.body, width)?;
            let (_, h) = out.text(d, 0.0, Ink::Secondary);
            out.y += h;
        }
        if let Some(word) = suggestion {
            out.y += 8.0;
            let ask = "Did you mean ";
            // DirectWrite counts UTF-16 code units.
            let range = DWRITE_TEXT_RANGE {
                startPosition: ask.encode_utf16().count() as u32,
                length: word.encode_utf16().count() as u32,
            };
            let line = self.text(&format!("{ask}{word}?"), &self.formats.body, width)?;
            unsafe {
                let _ = line.SetUnderline(true, range);
            }
            // Like a synonym: one rectangle per line, should it wrap.
            for r in hit_rects(&line, range) {
                out.links.push(Link {
                    x: r.left - 2.0,
                    y: out.y + r.top,
                    width: r.width + 4.0,
                    height: r.height,
                    target: Target::Word(word.to_string()),
                });
            }
            let (_, h) = out.text_with_accents(line, 0.0, Ink::Secondary, vec![range]);
            out.y += h;
        }
        Ok(())
    }

    /// The heading over a language's part of the card: its name, then the
    /// word, when it isn't the card's (the base form of what was looked up).
    fn layout_language(
        &self,
        out: &mut Stack,
        section: &Definition,
        index: usize,
        title: &str,
        width: f32,
    ) -> Result<()> {
        if index > 0 {
            out.y += 18.0;
        }
        let name = self.text(&dicts::name(&section.language), &self.formats.title, width)?;
        let (w, h) = out.text(name, 0.0, Ink::Primary);
        if section.word.to_lowercase() != title.to_lowercase() {
            let x = w + 8.0;
            let word = self.text(&section.word, &self.formats.body, (width - x).max(1.0))?;
            let (_, word_h, _) = metrics(&word);
            out.items.push(Item::Text {
                layout: word,
                x,
                y: out.y + (h - word_h) / 2.0,
                ink: Ink::Secondary,
                accents: Vec::new(),
            });
        }
        out.y += h + 10.0;
        Ok(())
    }

    /// The note, then each part of speech with its numbered senses.
    fn layout_definition(&self, out: &mut Stack, def: &Definition, width: f32) -> Result<()> {
        let indent = SENSE_INDENT;
        if let Some(note) = &def.note {
            let n = self.text(note, &self.formats.example, width)?;
            let (_, h) = out.text(n, 0.0, Ink::Tertiary);
            out.y += h + 10.0;
        }
        for (i, meaning) in def.meanings.iter().enumerate() {
            if i > 0 {
                out.y += 6.0;
                out.items.push(Item::Rule {
                    y: out.y,
                    x: 0.0,
                    width,
                });
                out.y += 14.0;
            }
            let pos = self.spaced_caps(&meaning.part_of_speech, width)?;
            let (_, h) = out.text(pos, 0.0, Ink::Accent);
            out.y += h + 6.0;

            for (n, sense) in meaning.senses.iter().enumerate() {
                let number = self.text(&format!("{}", n + 1), &self.formats.number, indent)?;
                out.text(number, 0.0, Ink::Tertiary);
                let text = self.text(&sense.text, &self.formats.body, width - indent)?;
                let (_, h) = out.text(text, indent, Ink::Primary);
                out.y += h;
                if let Some(example) = &sense.example {
                    out.y += 3.0;
                    let ex = self.text(
                        &format!("\u{201C}{example}\u{201D}"),
                        &self.formats.example,
                        width - indent,
                    )?;
                    let (_, h) = out.text(ex, indent, Ink::Secondary);
                    out.y += h;
                }
                out.y += 9.0;
            }
            if !meaning.synonyms.is_empty() {
                self.layout_related(out, "Similar", &meaning.synonyms, indent, width - indent)?;
            }
            if !meaning.antonyms.is_empty() {
                self.layout_related(out, "Opposite", &meaning.antonyms, indent, width - indent)?;
            }
        }
        Ok(())
    }

    /// `label` ("Similar", "Opposite") and the words, each a link to its own
    /// definition.
    fn layout_related(
        &self,
        out: &mut Stack,
        label: &str,
        synonyms: &[String],
        x: f32,
        width: f32,
    ) -> Result<()> {
        let separator = " \u{00B7} ";
        let mut line = format!("{label}  ");
        // DirectWrite counts UTF-16 code units.
        let utf16 = |s: &str| s.encode_utf16().count() as u32;
        let mut ranges = Vec::new();
        for (i, synonym) in synonyms.iter().enumerate() {
            if i > 0 {
                line.push_str(separator);
            }
            ranges.push(DWRITE_TEXT_RANGE {
                startPosition: utf16(&line),
                length: utf16(synonym),
            });
            line.push_str(synonym);
        }
        let syn = self.text(&line, &self.formats.meta, width)?;
        unsafe {
            let range = DWRITE_TEXT_RANGE {
                startPosition: 0,
                length: utf16(label),
            };
            let _ = syn.SetFontWeight(DWRITE_FONT_WEIGHT_SEMI_BOLD, range);
            for range in &ranges {
                let _ = syn.SetUnderline(true, *range);
            }
        }
        for (synonym, range) in synonyms.iter().zip(&ranges) {
            // One rectangle per line, should a synonym wrap.
            for r in hit_rects(&syn, *range) {
                out.links.push(Link {
                    x: x + r.left - 2.0,
                    y: out.y + r.top,
                    width: r.width + 4.0,
                    height: r.height,
                    target: Target::Word(synonym.clone()),
                });
            }
        }
        let (_, h) = out.text_with_accents(syn, x, Ink::Secondary, ranges);
        out.y += h + 6.0;
        Ok(())
    }

    /// The footer: the sources, each linking to the word's page there.
    fn layout_sources(&self, out: &mut Stack, def: &Definition, width: f32) -> Result<()> {
        out.y += 8.0;
        let mut x = 0.0;
        let mut line_h: f32 = 0.0;
        for (i, (source, word)) in def.all_sources().into_iter().enumerate() {
            if i > 0 {
                let dot = self.text(" \u{00B7} ", &self.formats.footer, width)?;
                let (w, h) = out.text(dot, x, Ink::Tertiary);
                x += w;
                line_h = line_h.max(h);
            }
            let (w, h) = match dictionary::source_url(source, word) {
                None => {
                    let label = self.text(source, &self.formats.footer, width)?;
                    out.text(label, x, Ink::Accent)
                }
                Some(url) => {
                    let (w, h) = self.external_link(out, source, x, width)?;
                    // A little slack around small text makes it easier to hit.
                    out.links.push(Link {
                        x: x - 4.0,
                        y: out.y - 4.0,
                        width: w + 8.0,
                        height: h + 8.0,
                        target: Target::Url(url),
                    });
                    (w, h)
                }
            };
            x += w;
            line_h = line_h.max(h);
        }
        out.y += line_h;
        Ok(())
    }

    /// Draws `layout` into a new surface. `scale` is pixels per DIP.
    pub fn draw(
        &self,
        layout: &Layout,
        scale: f32,
        theme: &Theme,
    ) -> Result<CompositionDrawingSurface> {
        let px_w = (layout.width * scale).ceil().max(1.0);
        let px_h = (layout.height * scale).ceil().clamp(1.0, 16_000.0);
        let surface = self.surface(px_w, px_h)?;
        self.paint(&surface, scale, |dc| unsafe {
            let brush = |c: D2D1_COLOR_F| dc.CreateSolidColorBrush(&c, None);
            let primary = brush(theme.text())?;
            let secondary = brush(theme.text_secondary())?;
            let tertiary = brush(theme.text_tertiary())?;
            let accent = brush(theme.accent_d2d())?;
            let critical = brush(theme.critical())?;
            let divider = brush(theme.divider())?;
            let key = brush(theme.key())?;
            let flag_edge = brush(theme.flag_edge())?;
            for item in &layout.items {
                match item {
                    Item::Text {
                        layout,
                        x,
                        y,
                        ink,
                        accents,
                    } => {
                        for range in accents {
                            let _ = layout.SetDrawingEffect(&accent, *range);
                        }
                        let b = match ink {
                            Ink::Primary => &primary,
                            Ink::Secondary => &secondary,
                            Ink::Tertiary => &tertiary,
                            Ink::Accent => &accent,
                            Ink::Critical => &critical,
                        };
                        dc.DrawTextLayout(
                            Vector2 { X: *x, Y: *y },
                            layout,
                            b,
                            D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT,
                        );
                    }
                    Item::Rule { y, x, width } => {
                        let rect = D2D_RECT_F {
                            left: *x,
                            top: *y,
                            right: x + width,
                            bottom: y + 1.0 / scale,
                        };
                        dc.FillRectangle(&rect, &divider);
                    }
                    Item::Key {
                        x,
                        y,
                        width,
                        height,
                    } => {
                        // Inset by half the hairline, so it lands on whole pixels.
                        let half = 0.5 / scale;
                        let cap = D2D1_ROUNDED_RECT {
                            rect: D2D_RECT_F {
                                left: x + half,
                                top: y + half,
                                right: x + width - half,
                                bottom: y + height - half,
                            },
                            radiusX: 4.0,
                            radiusY: 4.0,
                        };
                        dc.FillRoundedRectangle(&cap, &key);
                        dc.DrawRoundedRectangle(&cap, &divider, 1.0 / scale, None);
                    }
                    Item::Flag {
                        flag,
                        x,
                        y,
                        width,
                        height,
                    } => {
                        let outline = D2D1_ROUNDED_RECT {
                            rect: D2D_RECT_F {
                                left: *x,
                                top: *y,
                                right: x + width,
                                bottom: y + height,
                            },
                            radiusX: 2.0,
                            radiusY: 2.0,
                        };
                        let mask = self.d2d.CreateRoundedRectangleGeometry(&outline)?;
                        let mut layer = D2D1_LAYER_PARAMETERS1 {
                            contentBounds: outline.rect,
                            geometricMask: std::mem::ManuallyDrop::new(Some(mask.cast()?)),
                            maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                            maskTransform: Matrix3x2::identity(),
                            opacity: 1.0,
                            opacityBrush: std::mem::ManuallyDrop::new(None),
                            layerOptions: D2D1_LAYER_OPTIONS1_NONE,
                        };
                        dc.PushLayer(&layer, None);
                        let mut outer = Matrix3x2::default();
                        dc.GetTransform(&mut outer);
                        // Drawn on a 60 by 40 grid.
                        dc.SetTransform(
                            &(Matrix3x2::scale(width / 60.0, height / 40.0)
                                * Matrix3x2::translation(*x, *y)
                                * outer),
                        );
                        let result = draw_flag(dc, *flag);
                        dc.SetTransform(&outer);
                        dc.PopLayer();
                        std::mem::ManuallyDrop::drop(&mut layer.geometricMask);
                        result?;
                        let half = 0.5 / scale;
                        let edge = D2D1_ROUNDED_RECT {
                            rect: D2D_RECT_F {
                                left: x + half,
                                top: y + half,
                                right: x + width - half,
                                bottom: y + height - half,
                            },
                            ..outline
                        };
                        dc.DrawRoundedRectangle(&edge, &flag_edge, 1.0 / scale, None);
                    }
                }
            }
            Ok(())
        })?;
        Ok(surface)
    }

    /// A soft shadow for a rounded rectangle, meant to be stretched by a
    /// nine-grid brush. Returns the surface and the nine-grid inset, in pixels.
    pub fn draw_shadow(
        &self,
        margin_px: f32,
        radius_px: f32,
        opacity: f32,
    ) -> Result<(CompositionDrawingSurface, f32)> {
        let inset = (margin_px + radius_px).ceil();
        let size = (inset * 2.0 + 1.0) as u32;
        let sigma = margin_px / 2.6;
        let (lo, hi) = (margin_px, size as f32 - margin_px);
        let mut pixels = vec![0u8; (size * size * 4) as usize];
        for py in 0..size {
            for px in 0..size {
                let (x, y) = (px as f32 + 0.5, py as f32 + 0.5);
                let d = rounded_rect_distance(x, y, lo, hi, radius_px);
                let a =
                    (opacity * 0.5 * erfc(d / (sigma * std::f32::consts::SQRT_2))).clamp(0.0, 1.0);
                pixels[((py * size + px) * 4 + 3) as usize] = (a * 255.0).round() as u8;
            }
        }
        let surface = self.surface(size as f32, size as f32)?;
        self.paint(&surface, 1.0, |dc| unsafe {
            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
                colorContext: std::mem::ManuallyDrop::new(None),
            };
            let bitmap = dc.CreateBitmap(
                D2D_SIZE_U {
                    width: size,
                    height: size,
                },
                Some(pixels.as_ptr() as *const _),
                size * 4,
                &props,
            )?;
            dc.DrawBitmap(
                &bitmap,
                None,
                1.0,
                D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
                None,
                None,
            );
            Ok(())
        })?;
        Ok((surface, inset))
    }

    fn surface(&self, w: f32, h: f32) -> Result<CompositionDrawingSurface> {
        self.graphics.CreateDrawingSurface(
            Size {
                Width: w,
                Height: h,
            },
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            DirectXAlphaMode::Premultiplied,
        )
    }

    fn paint(
        &self,
        surface: &CompositionDrawingSurface,
        scale: f32,
        draw: impl FnOnce(&ID2D1DeviceContext) -> Result<()>,
    ) -> Result<()> {
        unsafe {
            let interop: ICompositionDrawingSurfaceInterop = surface.cast()?;
            let mut offset = POINT::default();
            let dc: ID2D1DeviceContext = interop.BeginDraw(None, &mut offset)?;
            dc.SetDpi(96.0 * scale, 96.0 * scale);
            dc.SetTransform(&Matrix3x2::translation(
                offset.x as f32 / scale,
                offset.y as f32 / scale,
            ));
            // Grayscale AA: ClearType needs an opaque background.
            dc.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            dc.Clear(Some(&D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            }));
            let result = draw(&dc);
            interop.EndDraw()?;
            result
        }
    }
}

/// Draws `flag` filling 0,0 to 60,40.
unsafe fn draw_flag(dc: &ID2D1DeviceContext, flag: Flag) -> Result<()> {
    let brush = |rgb: u32| unsafe {
        dc.CreateSolidColorBrush(
            &D2D1_COLOR_F {
                r: (rgb >> 16) as f32 / 255.0,
                g: (rgb >> 8 & 0xff) as f32 / 255.0,
                b: (rgb & 0xff) as f32 / 255.0,
                a: 1.0,
            },
            None,
        )
    };
    let rect = |left: f32, top: f32, right: f32, bottom: f32| D2D_RECT_F {
        left,
        top,
        right,
        bottom,
    };
    let at = |x: f32, y: f32| Vector2 { X: x, Y: y };
    unsafe {
        match flag {
            Flag::UnitedKingdom => {
                // Bolder white than the real flag's, so it shows at this size.
                let (blue, red, white) = (brush(0x012169)?, brush(0xC8102E)?, brush(0xFFFFFF)?);
                dc.FillRectangle(&rect(0.0, 0.0, 60.0, 40.0), &blue);
                for (from, to) in [
                    (at(0.0, 0.0), at(60.0, 40.0)),
                    (at(0.0, 40.0), at(60.0, 0.0)),
                ] {
                    dc.DrawLine(from, to, &white, 13.0, None);
                    dc.DrawLine(from, to, &red, 4.0, None);
                }
                dc.FillRectangle(&rect(23.0, 0.0, 37.0, 40.0), &white);
                dc.FillRectangle(&rect(0.0, 13.0, 60.0, 27.0), &white);
                dc.FillRectangle(&rect(27.0, 0.0, 33.0, 40.0), &red);
                dc.FillRectangle(&rect(0.0, 17.0, 60.0, 23.0), &red);
            }
            Flag::Italy => {
                for (i, rgb) in [0x009246, 0xF4F5F0, 0xCE2B37].into_iter().enumerate() {
                    let left = i as f32 * 20.0;
                    dc.FillRectangle(&rect(left, 0.0, left + 20.0, 40.0), &brush(rgb)?);
                }
            }
        }
    }
    Ok(())
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
        heading: make(&display, 20.0, DWRITE_FONT_WEIGHT_SEMI_BOLD, normal, 0.0)?,
        setting,
        value,
        chrome,
    })
}

/// "serendipity" → "Serendipity". Only the first letter changes.
fn capitalize(word: &str) -> String {
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

/// The height of the first line of a laid-out text.
fn first_line_height(layout: &IDWriteTextLayout) -> f32 {
    let mut lines = [DWRITE_LINE_METRICS::default(); 1];
    let mut count = 0;
    // Fails with "buffer too small" when there are more lines, after filling in the first.
    let _ = unsafe { layout.GetLineMetrics(Some(&mut lines), &mut count) };
    lines[0].height
}

fn rounded_rect_distance(x: f32, y: f32, lo: f32, hi: f32, r: f32) -> f32 {
    let center = (lo + hi) / 2.0;
    let half = (hi - lo) / 2.0 - r;
    let dx = ((x - center).abs() - half).max(0.0);
    let dy = ((y - center).abs() - half).max(0.0);
    let outside = (dx * dx + dy * dy).sqrt() - r;
    let inside = ((x - center).abs() - half)
        .max((y - center).abs() - half)
        .min(0.0);
    outside + inside
}

/// Complementary error function (Abramowitz–Stegun 7.1.26).
fn erfc(x: f32) -> f32 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.327_591_1 * z);
    let poly = t
        * (0.254_829_6
            + t * (-0.284_496_7 + t * (1.421_413_7 + t * (-1.453_152 + t * 1.061_405_4))));
    let r = poly * (-z * z).exp();
    if x >= 0.0 {
        r
    } else {
        2.0 - r
    }
}
