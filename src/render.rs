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

use crate::dictionary::Definition;
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
    },
    Rule {
        y: f32,
        x: f32,
        width: f32,
    },
}

/// Laid-out content, in DIPs.
pub struct Layout {
    items: Vec<Item>,
    pub width: f32,
    pub height: f32,
}

/// What the body of the card shows.
pub enum Body<'a> {
    Loading,
    Definition(&'a Definition),
    Message { title: &'a str, detail: &'a str },
}

struct Formats {
    word: IDWriteTextFormat,
    phonetic: IDWriteTextFormat,
    part_of_speech: IDWriteTextFormat,
    number: IDWriteTextFormat,
    body: IDWriteTextFormat,
    example: IDWriteTextFormat,
    meta: IDWriteTextFormat,
    footer: IDWriteTextFormat,
    title: IDWriteTextFormat,
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

    /// The word and its pronunciation.
    pub fn layout_header(&self, word: &str, phonetic: Option<&str>, width: f32) -> Result<Layout> {
        let mut items = Vec::new();
        let title = self.text(word, &self.formats.word, width)?;
        let (title_w, title_h, baseline) = metrics(&title);
        let mut height = title_h;
        if let Some(p) = phonetic {
            let gap = 10.0;
            let phon = self.text(p, &self.formats.phonetic, width)?;
            let (phon_w, phon_h, phon_baseline) = metrics(&phon);
            if title_w + gap + phon_w <= width {
                // Same line, sharing the baseline.
                items.push(Item::Text {
                    layout: phon,
                    x: title_w + gap,
                    y: baseline - phon_baseline,
                    ink: Ink::Secondary,
                });
            } else {
                items.push(Item::Text {
                    layout: phon,
                    x: 0.0,
                    y: title_h + 2.0,
                    ink: Ink::Secondary,
                });
                height += 2.0 + phon_h;
            }
        }
        items.insert(
            0,
            Item::Text {
                layout: title,
                x: 0.0,
                y: 0.0,
                ink: Ink::Primary,
            },
        );
        Ok(Layout {
            items,
            width,
            height,
        })
    }

    pub fn layout_body(&self, body: &Body, width: f32, bottom_padding: f32) -> Result<Layout> {
        let mut items = Vec::new();
        let mut y = 0.0;
        match body {
            Body::Loading => y = 26.0,
            Body::Message { title, detail } => {
                let t = self.text(title, &self.formats.title, width)?;
                y += metrics(&t).1 + 4.0;
                items.push(Item::Text {
                    layout: t,
                    x: 0.0,
                    y: 0.0,
                    ink: Ink::Primary,
                });
                if !detail.is_empty() {
                    let d = self.text(detail, &self.formats.body, width)?;
                    let h = metrics(&d).1;
                    items.push(Item::Text {
                        layout: d,
                        x: 0.0,
                        y,
                        ink: Ink::Secondary,
                    });
                    y += h;
                }
            }
            Body::Definition(def) => {
                let indent = 22.0;
                for (i, meaning) in def.meanings.iter().enumerate() {
                    if i > 0 {
                        y += 6.0;
                        items.push(Item::Rule { y, x: 0.0, width });
                        y += 14.0;
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
                    let h = metrics(&pos).1;
                    items.push(Item::Text {
                        layout: pos,
                        x: 0.0,
                        y,
                        ink: Ink::Accent,
                    });
                    y += h + 6.0;

                    for (n, sense) in meaning.senses.iter().enumerate() {
                        let number =
                            self.text(&format!("{}", n + 1), &self.formats.number, indent)?;
                        items.push(Item::Text {
                            layout: number,
                            x: 0.0,
                            y,
                            ink: Ink::Tertiary,
                        });
                        let text = self.text(&sense.text, &self.formats.body, width - indent)?;
                        let h = metrics(&text).1;
                        items.push(Item::Text {
                            layout: text,
                            x: indent,
                            y,
                            ink: Ink::Primary,
                        });
                        y += h;
                        if let Some(example) = &sense.example {
                            y += 3.0;
                            let ex = self.text(
                                &format!("\u{201C}{example}\u{201D}"),
                                &self.formats.example,
                                width - indent,
                            )?;
                            let h = metrics(&ex).1;
                            items.push(Item::Text {
                                layout: ex,
                                x: indent,
                                y,
                                ink: Ink::Secondary,
                            });
                            y += h;
                        }
                        y += 9.0;
                    }
                    if !meaning.synonyms.is_empty() {
                        let label = "Similar  ";
                        let line = format!("{label}{}", meaning.synonyms.join(" \u{00B7} "));
                        let syn = self.text(&line, &self.formats.meta, width - indent)?;
                        unsafe {
                            let range = DWRITE_TEXT_RANGE {
                                startPosition: 0,
                                length: label.trim_end().len() as u32,
                            };
                            let _ = syn.SetFontWeight(DWRITE_FONT_WEIGHT_SEMI_BOLD, range);
                        }
                        let h = metrics(&syn).1;
                        items.push(Item::Text {
                            layout: syn,
                            x: indent,
                            y,
                            ink: Ink::Secondary,
                        });
                        y += h + 6.0;
                    }
                }
                y += 8.0;
                let footer = self.text(
                    "Wiktionary \u{00B7} dictionaryapi.dev",
                    &self.formats.footer,
                    width,
                )?;
                let h = metrics(&footer).1;
                items.push(Item::Text {
                    layout: footer,
                    x: 0.0,
                    y,
                    ink: Ink::Tertiary,
                });
                y += h;
            }
        }
        Ok(Layout {
            items,
            width,
            height: y + bottom_padding,
        })
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
            for item in &layout.items {
                match item {
                    Item::Text { layout, x, y, ink } => {
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
    Ok(Formats {
        word: make(&display, 26.0, DWRITE_FONT_WEIGHT_SEMI_BOLD, normal, 0.0)?,
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
        footer: make(&small, 11.0, DWRITE_FONT_WEIGHT_NORMAL, normal, 0.0)?,
        title: make(&text, 15.0, DWRITE_FONT_WEIGHT_SEMI_BOLD, normal, 0.0)?,
    })
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
