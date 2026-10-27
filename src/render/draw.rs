//! Drawing layouts, shadows and shapes into composition surfaces.

use super::*;

impl Renderer {
    /// Draws `layout` into a new surface. `scale` is pixels per DIP.
    pub fn draw(
        &self,
        layout: &Layout,
        scale: f32,
        theme: &Theme,
    ) -> Result<CompositionDrawingSurface> {
        let surface = self.surface(
            (layout.width * scale).ceil(),
            (layout.height * scale).ceil(),
        )?;
        self.paint(&surface, scale, |dc| unsafe {
            self.draw_items(dc, layout, scale, theme)
        })?;
        Ok(surface)
    }

    /// Draws `layout`'s items on `dc`, whose transform places them; `scale`
    /// is pixels per DIP.
    unsafe fn draw_items(
        &self,
        dc: &ID2D1DeviceContext,
        layout: &Layout,
        scale: f32,
        theme: &Theme,
    ) -> Result<()> {
        let brush = |c: D2D1_COLOR_F| dc.CreateSolidColorBrush(&c, None);
        let primary = brush(theme.text())?;
        let secondary = brush(theme.text_secondary())?;
        let tertiary = brush(theme.text_tertiary())?;
        let accent = brush(theme.accent_d2d())?;
        let critical = brush(theme.critical())?;
        let on_accent = brush(to_d2d(theme.knob(true)))?;
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
                    let own;
                    let b = match ink {
                        Ink::Rgb(c) => {
                            own = brush(hex(*c))?;
                            &own
                        }
                        Ink::Primary => &primary,
                        Ink::Secondary => &secondary,
                        Ink::Tertiary => &tertiary,
                        Ink::Accent => &accent,
                        Ink::Critical => &critical,
                        Ink::OnAccent => &on_accent,
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
                Item::Logo { x, y, size } => {
                    // On whole pixels, so the panes stay crisp.
                    let snap = |v: f32| (v * scale).round() / scale;
                    let gap = (LOGO_GAP * scale).round().max(1.0) / scale;
                    let pane = snap((size - gap) / 2.0);
                    let (left, top) = (snap(*x), snap(*y));
                    for (dx, dy) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
                        let (l, t) = (left + dx * (pane + gap), top + dy * (pane + gap));
                        let rect = D2D_RECT_F {
                            left: l,
                            top: t,
                            right: l + pane,
                            bottom: t + pane,
                        };
                        dc.FillRectangle(&rect, &primary);
                    }
                }
                Item::Flag {
                    flag,
                    x,
                    y,
                    width,
                    height,
                } => {
                    let rect = D2D_RECT_F {
                        left: *x,
                        top: *y,
                        right: x + width,
                        bottom: y + height,
                    };
                    self.draw_flag_item(dc, *flag, rect, scale, &flag_edge)?;
                }
                Item::Shape {
                    x,
                    y,
                    width,
                    height,
                    radius,
                    fill,
                    stroke,
                } => {
                    let paint = |p: Paint| {
                        brush(match p {
                            Paint::Control => to_d2d(theme.control()),
                            Paint::ControlStroke => to_d2d(theme.control_stroke()),
                            Paint::Field => to_d2d(theme.field()),
                            Paint::Focus | Paint::Accent => theme.accent_d2d(),
                            Paint::Sunken => to_d2d(theme.sunken()),
                            Paint::StrongStroke => to_d2d(theme.strong_stroke()),
                            Paint::Disabled => theme.text_tertiary(),
                            Paint::Rgb(c) => hex(c),
                        })
                    };
                    // Inset by half the stroke, so it lands on whole pixels.
                    let stroke_w = match stroke {
                        Some(Paint::Focus) => (FOCUS_STROKE * scale).round() / scale,
                        _ => 1.0 / scale,
                    };
                    let half = stroke_w / 2.0;
                    let shape = D2D1_ROUNDED_RECT {
                        rect: D2D_RECT_F {
                            left: x + half,
                            top: y + half,
                            right: x + width - half,
                            bottom: y + height - half,
                        },
                        radiusX: *radius,
                        radiusY: *radius,
                    };
                    if let Some(p) = fill {
                        dc.FillRoundedRectangle(&shape, &paint(*p)?);
                    }
                    if let Some(p) = stroke {
                        dc.DrawRoundedRectangle(&shape, &paint(*p)?, stroke_w, None);
                    }
                }
            }
        }
        Ok(())
    }

    /// Draws `flag` clipped to the rounded `rect`, with a hairline edge.
    unsafe fn draw_flag_item(
        &self,
        dc: &ID2D1DeviceContext,
        flag: Flag,
        rect: D2D_RECT_F,
        scale: f32,
        edge_brush: &ID2D1SolidColorBrush,
    ) -> Result<()> {
        let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
        let outline = D2D1_ROUNDED_RECT {
            rect,
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
                * Matrix3x2::translation(rect.left, rect.top)
                * outer),
        );
        let result = flags::draw_flag(dc, flag);
        dc.SetTransform(&outer);
        dc.PopLayer();
        std::mem::ManuallyDrop::drop(&mut layer.geometricMask);
        result?;
        let half = 0.5 / scale;
        let edge = D2D1_ROUNDED_RECT {
            rect: D2D_RECT_F {
                left: rect.left + half,
                top: rect.top + half,
                right: rect.right - half,
                bottom: rect.bottom - half,
            },
            ..outline
        };
        dc.DrawRoundedRectangle(&edge, edge_brush, 1.0 / scale, None);
        Ok(())
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

    /// The underline of a `w` by `h` pixel field with corners of `radius`:
    /// the rounded rectangle less itself raised by `thick`, so that the line
    /// is `thick` tall along the bottom and thins to nothing up the corners.
    pub fn draw_underline(
        &self,
        w: f32,
        h: f32,
        radius: f32,
        thick: f32,
        color: windows::UI::Color,
    ) -> Result<CompositionDrawingSurface> {
        let surface = self.surface(w, h)?;
        self.paint(&surface, 1.0, |dc| unsafe {
            let shape = |dy: f32| D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: 0.0,
                    top: dy,
                    right: w,
                    bottom: h + dy,
                },
                radiusX: radius,
                radiusY: radius,
            };
            let whole = self.d2d.CreateRoundedRectangleGeometry(&shape(0.0))?;
            let raised = self.d2d.CreateRoundedRectangleGeometry(&shape(-thick))?;
            let line = self.d2d.CreatePathGeometry()?;
            let sink = line.Open()?;
            whole.CombineWithGeometry(
                &raised,
                D2D1_COMBINE_MODE_EXCLUDE,
                None,
                0.1,
                &sink.cast::<ID2D1SimplifiedGeometrySink>()?,
            )?;
            sink.Close()?;
            let brush = dc.CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: color.R as f32 / 255.0,
                    g: color.G as f32 / 255.0,
                    b: color.B as f32 / 255.0,
                    a: color.A as f32 / 255.0,
                },
                None,
            )?;
            dc.FillGeometry(&line, &brush, None::<&ID2D1Brush>);
            Ok(())
        })?;
        Ok(surface)
    }

    /// A `w` by `h` pixel rounded rectangle with corners of `radius`, in
    /// white, smoothed at its edges: filled, or with `stroke` a line that
    /// thick just inside its edge. Meant as a mask, tinted by a color brush.
    pub fn draw_rounded(
        &self,
        w: f32,
        h: f32,
        radius: f32,
        stroke: Option<f32>,
    ) -> Result<CompositionDrawingSurface> {
        let surface = self.surface(w, h)?;
        self.paint(&surface, 1.0, |dc| unsafe {
            let white = dc.CreateSolidColorBrush(
                &D2D1_COLOR_F {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 1.0,
                },
                None,
            )?;
            let half = stroke.map_or(0.0, |s| s / 2.0);
            let shape = D2D1_ROUNDED_RECT {
                rect: D2D_RECT_F {
                    left: half,
                    top: half,
                    right: w - half,
                    bottom: h - half,
                },
                radiusX: (radius - half).max(0.0),
                radiusY: (radius - half).max(0.0),
            };
            match stroke {
                Some(thick) => dc.DrawRoundedRectangle(&shape, &white, thick, None),
                None => dc.FillRoundedRectangle(&shape, &white),
            }
            Ok(())
        })?;
        Ok(surface)
    }

    /// The picture `encoded` (a PNG or JPEG) filling a `w` by `h` pixel
    /// surface, cropped evenly from both sides of whichever way it's too
    /// long, like a wallpaper set to fill.
    pub fn draw_picture(
        &self,
        encoded: &[u8],
        w: f32,
        h: f32,
    ) -> Result<CompositionDrawingSurface> {
        self.draw_scene(encoded, w, h, None)
    }

    /// The picture `encoded` as [`Renderer::draw_picture`] has it, with
    /// `scene` drawn over it in its theme.
    pub fn draw_scene(
        &self,
        encoded: &[u8],
        w: f32,
        h: f32,
        scene: Option<(&Scene, &Theme)>,
    ) -> Result<CompositionDrawingSurface> {
        let (w, h) = (w.round().max(1.0), h.round().max(1.0));
        let surface = self.surface(w, h)?;
        self.paint(&surface, 1.0, |dc| unsafe {
            self.paint_scene(dc, encoded, w, h, scene)
        })?;
        Ok(surface)
    }

    /// What [`Renderer::draw_scene`] draws, on `dc`, in pixels.
    unsafe fn paint_scene(
        &self,
        dc: &ID2D1DeviceContext,
        encoded: &[u8],
        w: f32,
        h: f32,
        scene: Option<(&Scene, &Theme)>,
    ) -> Result<()> {
        let wic: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let stream = wic.CreateStream()?;
        stream.InitializeFromMemory(encoded)?;
        let decoder =
            wic.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)?;
        let frame = decoder.GetFrame(0)?;
        let converter = wic.CreateFormatConverter()?;
        converter.Initialize(
            &frame,
            &GUID_WICPixelFormat32bppPBGRA,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeMedianCut,
        )?;
        let bitmap = dc.CreateBitmapFromWicBitmap(&converter, None)?;
        let size = bitmap.GetPixelSize();
        let (pw, ph) = (size.width as f32, size.height as f32);
        let k = (w / pw).max(h / ph);
        let (cw, ch) = (w / k, h / k);
        let source = D2D_RECT_F {
            left: (pw - cw) / 2.0,
            top: (ph - ch) / 2.0,
            right: (pw + cw) / 2.0,
            bottom: (ph + ch) / 2.0,
        };
        let dest = D2D_RECT_F {
            left: 0.0,
            top: 0.0,
            right: w,
            bottom: h,
        };
        let picture = || {
            dc.DrawBitmap(
                &bitmap,
                Some(&dest),
                1.0,
                D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                Some(&source),
                None,
            )
        };
        picture();
        let Some((scene, theme)) = scene else {
            return Ok(());
        };
        let mut outer = Matrix3x2::default();
        dc.GetTransform(&mut outer);
        // The picture's own pixels, from here on.
        let own =
            Matrix3x2::translation(-source.left, -source.top) * Matrix3x2::scale(k, k) * outer;
        let draw = |over: &Over| {
            dc.SetTransform(
                &(Matrix3x2::scale(over.scale, over.scale)
                    * Matrix3x2::translation(over.x, over.y)
                    * own),
            );
            self.draw_items(dc, over.layout, k * over.scale, theme)
        };
        for over in &scene.under {
            draw(over)?;
        }
        if let Some(card) = scene.card {
            // The card, as it was, over what's under it.
            dc.SetTransform(&own);
            let mask = self.d2d.CreateRoundedRectangleGeometry(&card)?;
            let mut layer = D2D1_LAYER_PARAMETERS1 {
                contentBounds: card.rect,
                geometricMask: std::mem::ManuallyDrop::new(Some(mask.cast()?)),
                maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                maskTransform: Matrix3x2::identity(),
                opacity: 1.0,
                opacityBrush: std::mem::ManuallyDrop::new(None),
                layerOptions: D2D1_LAYER_OPTIONS1_NONE,
            };
            dc.PushLayer(&layer, None);
            dc.SetTransform(&outer);
            picture();
            dc.PopLayer();
            std::mem::ManuallyDrop::drop(&mut layer.geometricMask);
        }
        for over in &scene.on {
            draw(over)?;
        }
        dc.SetTransform(&outer);
        Ok(())
    }

    /// A surface `w` by `h` pixels, at least one and at most `max_px`:
    /// what's drawn past that is cut off.
    pub(super) fn surface(&self, w: f32, h: f32) -> Result<CompositionDrawingSurface> {
        let most = self.max_px();
        self.graphics.CreateDrawingSurface(
            Size {
                Width: w.clamp(1.0, most),
                Height: h.clamp(1.0, most),
            },
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            DirectXAlphaMode::Premultiplied,
        )
    }

    pub(super) fn paint(
        &self,
        surface: &CompositionDrawingSurface,
        scale: f32,
        draw: impl FnOnce(&ID2D1DeviceContext) -> Result<()>,
    ) -> Result<()> {
        unsafe {
            let interop: ICompositionDrawingSurfaceInterop = surface.cast()?;
            let mut offset = POINT::default();
            let dc: ID2D1DeviceContext = interop
                .BeginDraw(None, &mut offset)
                .inspect_err(|_| self.note_if_lost())?;
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
            interop.EndDraw().inspect_err(|_| self.note_if_lost())?;
            result
        }
    }

    /// A drawing failed: if it's because the GPU went away, the app is told
    /// to draw everything again.
    fn note_if_lost(&self) {
        if self.lost() {
            super::DEVICE_LOST.set(true);
        }
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

/// 0xRRGGBB, as Direct2D has colors.
fn hex(c: u32) -> D2D1_COLOR_F {
    let [_, r, g, b] = c.to_be_bytes();
    to_d2d(crate::theme::rgb(r, g, b))
}

#[cfg(test)]
impl Renderer {
    /// What [`Renderer::draw_scene`] draws, as rows of BGRA bytes, premultiplied.
    pub fn scene_pixels(
        &self,
        encoded: &[u8],
        w: u32,
        h: u32,
        scene: Option<(&Scene, &Theme)>,
    ) -> Result<Vec<u8>> {
        unsafe {
            let dc =
                (self.d2d_device.borrow()).CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            let format = D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            };
            let props = |options| D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: format,
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: options,
                colorContext: std::mem::ManuallyDrop::new(None),
            };
            let size = D2D_SIZE_U {
                width: w,
                height: h,
            };
            let target = dc.CreateBitmap(
                size,
                None,
                0,
                &props(D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW),
            )?;
            dc.SetTarget(&target);
            dc.BeginDraw();
            dc.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            dc.Clear(None);
            let drawn = self.paint_scene(&dc, encoded, w as f32, h as f32, scene);
            dc.EndDraw(None, None)?;
            drawn?;
            let read = dc.CreateBitmap(
                size,
                None,
                0,
                &props(D2D1_BITMAP_OPTIONS_CPU_READ | D2D1_BITMAP_OPTIONS_CANNOT_DRAW),
            )?;
            read.CopyFromBitmap(None, &target, None)?;
            let mapped = read.Map(D2D1_MAP_OPTIONS_READ)?;
            let mut out = Vec::with_capacity((w * h * 4) as usize);
            for row in 0..h as usize {
                let p = mapped.bits.add(row * mapped.pitch as usize);
                out.extend_from_slice(std::slice::from_raw_parts(p, w as usize * 4));
            }
            read.Unmap()?;
            Ok(out)
        }
    }
}
