//! Text layout (DirectWrite) and drawing (Direct2D) into composition surfaces.

use windows::core::{w, Interface, Result, HSTRING};
use windows::Foundation::Size;
use windows::Graphics::DirectX::{DirectXAlphaMode, DirectXPixelFormat};
use windows::Win32::Foundation::{HMODULE, POINT};
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
use crate::theme::Theme;

#[derive(Clone, Copy)]
pub enum Ink {
    Primary,
    Secondary,
    Tertiary,
    Accent,
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
                Item::Rule { .. } | Item::Key { .. } => None,
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
}

/// What the body of the card shows.
pub enum Body<'a> {
    Loading,
    Definition(&'a Definition),
    Message { title: &'a str, detail: &'a str },
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
}

pub struct Renderer {
    d3d: ID3D11Device,
    _d2d_device: ID2D1Device,
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
    /// and where the caret goes after it.
    pub fn layout_query(&self, text: &str, width: f32) -> Result<(Layout, Caret)> {
        let line = self.text(text, &self.formats.query, width)?;
        let (w, height, baseline) = metrics(&line);
        let caret = Caret {
            x: w,
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
        Ok((layout, caret))
    }

    /// A suggested word, its first `typed` characters (what was typed) in
    /// bold, and the word it's a form of after an arrow.
    pub fn layout_suggestion(
        &self,
        word: &str,
        typed: usize,
        base: Option<&str>,
        width: f32,
    ) -> Result<Layout> {
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
        let mut out = Stack::default();
        let (w, height) = out.text(line, 0.0, Ink::Primary);
        if let Some(base) = base {
            let form_of =
                self.text(&format!("\u{2192} {base}"), &self.formats.suggestion, width)?;
            out.text(form_of, w + 8.0, Ink::Tertiary);
        }
        Ok(Layout {
            items: out.items,
            width,
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

    pub fn layout_body(&self, body: &Body, width: f32, bottom_padding: f32) -> Result<Layout> {
        let mut out = Stack::default();
        match body {
            // Room for the loading dots, which popup.rs draws.
            Body::Loading => out.y = 26.0,
            Body::Message { title, detail } => {
                self.layout_message(&mut out, title, detail, width)?
            }
            Body::Definition(def) => {
                self.layout_definition(&mut out, def, width)?;
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

    fn layout_message(&self, out: &mut Stack, title: &str, detail: &str, width: f32) -> Result<()> {
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
            let pos = self.text(
                &meaning.part_of_speech.to_uppercase(),
                &self.formats.part_of_speech,
                width,
            )?;
            if let Ok(pos1) = pos.cast::<IDWriteTextLayout1>() {
                let range = DWRITE_TEXT_RANGE {
                    startPosition: 0,
                    length: u32::MAX,
                };
                unsafe {
                    let _ = pos1.SetCharacterSpacing(0.0, 0.9, 0.0, range);
                }
            }
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
                self.layout_synonyms(out, &meaning.synonyms, indent, width - indent)?;
            }
        }
        Ok(())
    }

    /// "Similar" and the synonyms, each a link to its own definition.
    fn layout_synonyms(
        &self,
        out: &mut Stack,
        synonyms: &[String],
        x: f32,
        width: f32,
    ) -> Result<()> {
        let label = "Similar  ";
        let separator = " \u{00B7} ";
        let mut line = String::from(label);
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
                length: label.trim_end().len() as u32,
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
        for (i, source) in def.sources.iter().enumerate() {
            if i > 0 {
                let dot = self.text(" \u{00B7} ", &self.formats.footer, width)?;
                let (w, h) = out.text(dot, x, Ink::Tertiary);
                x += w;
                line_h = line_h.max(h);
            }
            let label = self.text(source, &self.formats.footer, width)?;
            let url = dictionary::source_url(source, &def.word);
            if url.is_some() {
                let all = DWRITE_TEXT_RANGE {
                    startPosition: 0,
                    length: u32::MAX,
                };
                unsafe {
                    let _ = label.SetUnderline(true, all);
                }
            }
            let (mut w, h) = out.text(label, x, Ink::Accent);
            if let Some(url) = url {
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
                w += gap + icon_w;
                // A little slack around small text makes it easier to hit.
                out.links.push(Link {
                    x: x - 4.0,
                    y: out.y - 4.0,
                    width: w + 8.0,
                    height: h + 8.0,
                    target: Target::Url(url),
                });
            }
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
            let divider = brush(theme.divider())?;
            let key = brush(theme.key())?;
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
        .map(|_| device.unwrap());
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
