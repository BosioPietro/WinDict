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
                                Paint::Focus => theme.accent_d2d(),
                            })
                        };
                        // Inset by half the stroke, so it lands on whole pixels.
                        let stroke_w = match stroke {
                            Some(Paint::Focus) => (crate::ui::FOCUS_STROKE * scale).round() / scale,
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
        })?;
        Ok(surface)
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
        let result = draw_flag(dc, flag);
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
        let surface = self.surface(w.max(1.0), h.max(1.0))?;
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
        let (w, h) = (w.round().max(1.0), h.round().max(1.0));
        let surface = self.surface(w, h)?;
        self.paint(&surface, 1.0, |dc| unsafe {
            let wic: IWICImagingFactory =
                CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
            let stream = wic.CreateStream()?;
            stream.InitializeFromMemory(encoded)?;
            let decoder = wic.CreateDecoderFromStream(
                &stream,
                std::ptr::null(),
                WICDecodeMetadataCacheOnDemand,
            )?;
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
            dc.DrawBitmap(
                &bitmap,
                Some(&dest),
                1.0,
                D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                Some(&source),
                None,
            );
            Ok(())
        })?;
        Ok(surface)
    }

    pub(super) fn surface(&self, w: f32, h: f32) -> Result<CompositionDrawingSurface> {
        self.graphics.CreateDrawingSurface(
            Size {
                Width: w,
                Height: h,
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
            Flag::France => {
                for (i, rgb) in [0x002395, 0xFFFFFF, 0xED2939].into_iter().enumerate() {
                    let left = i as f32 * 20.0;
                    dc.FillRectangle(&rect(left, 0.0, left + 20.0, 40.0), &brush(rgb)?);
                }
            }
            Flag::Germany => {
                let height = 40.0 / 3.0;
                for (i, rgb) in [0x000000, 0xDD0000, 0xFFCE00].into_iter().enumerate() {
                    let top = i as f32 * height;
                    dc.FillRectangle(&rect(0.0, top, 60.0, top + height), &brush(rgb)?);
                }
            }
            Flag::Spain => {
                let red = brush(0xAA151B)?;
                for (top, bottom, rgb) in [
                    (0.0, 10.0, 0xAA151B),
                    (10.0, 30.0, 0xF1BF00),
                    (30.0, 40.0, 0xAA151B),
                ] {
                    dc.FillRectangle(&rect(0.0, top, 60.0, bottom), &brush(rgb)?);
                }
                // The arms reduced to a crowned shield between two pillars.
                let pillar = brush(0x8C8C8C)?;
                dc.FillRectangle(&rect(11.0, 15.0, 14.0, 27.0), &pillar);
                dc.FillRectangle(&rect(26.0, 15.0, 29.0, 27.0), &pillar);
                dc.FillRectangle(&rect(16.0, 11.5, 24.0, 14.0), &red);
                let shield = [
                    at(15.5, 15.0),
                    at(24.5, 15.0),
                    at(24.5, 24.0),
                    at(20.0, 28.5),
                    at(15.5, 24.0),
                ];
                polygon(dc, &shield, &red)?;
            }
            Flag::Portugal => {
                dc.FillRectangle(&rect(0.0, 0.0, 24.0, 40.0), &brush(0x006600)?);
                dc.FillRectangle(&rect(24.0, 0.0, 60.0, 40.0), &brush(0xFF0000)?);
                // The arms reduced to the gold armillary sphere behind a red-bordered
                // white shield with its five blue quinas.
                let gold = brush(0xFFE000)?;
                let sphere = D2D1_ELLIPSE {
                    point: at(24.0, 20.0),
                    radiusX: 11.0,
                    radiusY: 11.0,
                };
                dc.DrawEllipse(&sphere, &gold, 3.0, None);
                dc.DrawLine(at(13.0, 20.0), at(35.0, 20.0), &gold, 3.0, None);
                let shield = |y: f32, w: f32, h: f32| {
                    [
                        at(24.0 - w, y),
                        at(24.0 + w, y),
                        at(24.0 + w, y + h * 0.6),
                        at(24.0 + w * 0.5, y + h * 0.9),
                        at(24.0, y + h),
                        at(24.0 - w * 0.5, y + h * 0.9),
                        at(24.0 - w, y + h * 0.6),
                    ]
                };
                polygon(dc, &shield(12.0, 6.0, 16.0), &brush(0xFF0000)?)?;
                polygon(dc, &shield(14.0, 4.0, 12.0), &brush(0xFFFFFF)?)?;
                let blue = brush(0x003399)?;
                for (x, y) in [
                    (24.0, 16.5),
                    (21.5, 19.5),
                    (24.0, 19.5),
                    (26.5, 19.5),
                    (24.0, 22.5),
                ] {
                    dc.FillRectangle(&rect(x - 1.0, y - 1.2, x + 1.0, y + 1.2), &blue);
                }
            }
            Flag::Netherlands => {
                let height = 40.0 / 3.0;
                for (i, rgb) in [0xAE1C28, 0xFFFFFF, 0x21468B].into_iter().enumerate() {
                    let top = i as f32 * height;
                    dc.FillRectangle(&rect(0.0, top, 60.0, top + height), &brush(rgb)?);
                }
            }
            Flag::Sweden => {
                // Fitted from 16:10 to 3:2; the cross keeps its real width and offset.
                let (blue, yellow) = (brush(0x006AA7)?, brush(0xFECC00)?);
                dc.FillRectangle(&rect(0.0, 0.0, 60.0, 40.0), &blue);
                dc.FillRectangle(&rect(18.75, 0.0, 26.25, 40.0), &yellow);
                dc.FillRectangle(&rect(0.0, 16.0, 60.0, 24.0), &yellow);
            }
            Flag::Denmark => {
                // Fitted from 37:28 to 3:2.
                let (red, white) = (brush(0xC60C30)?, brush(0xFFFFFF)?);
                dc.FillRectangle(&rect(0.0, 0.0, 60.0, 40.0), &red);
                dc.FillRectangle(&rect(19.46, 0.0, 25.95, 40.0), &white);
                dc.FillRectangle(&rect(0.0, 17.14, 60.0, 22.86), &white);
            }
            Flag::Norway => {
                // Wider white than the real flag's, so it shows at this size.
                let (red, blue, white) = (brush(0xBA0C2F)?, brush(0x00205B)?, brush(0xFFFFFF)?);
                dc.FillRectangle(&rect(0.0, 0.0, 60.0, 40.0), &red);
                dc.FillRectangle(&rect(15.0, 0.0, 30.0, 40.0), &white);
                dc.FillRectangle(&rect(0.0, 12.5, 60.0, 27.5), &white);
                dc.FillRectangle(&rect(19.0, 0.0, 26.0, 40.0), &blue);
                dc.FillRectangle(&rect(0.0, 16.5, 60.0, 23.5), &blue);
            }
            Flag::Finland => {
                // The cross drawn like Sweden's, so the Nordic flags match.
                let (white, blue) = (brush(0xFFFFFF)?, brush(0x003580)?);
                dc.FillRectangle(&rect(0.0, 0.0, 60.0, 40.0), &white);
                dc.FillRectangle(&rect(18.75, 0.0, 26.25, 40.0), &blue);
                dc.FillRectangle(&rect(0.0, 16.0, 60.0, 24.0), &blue);
            }
            Flag::Poland => {
                for (i, rgb) in [0xFFFFFF, 0xDC143C].into_iter().enumerate() {
                    let top = i as f32 * 20.0;
                    dc.FillRectangle(&rect(0.0, top, 60.0, top + 20.0), &brush(rgb)?);
                }
            }
            Flag::Czechia => {
                dc.FillRectangle(&rect(0.0, 0.0, 60.0, 20.0), &brush(0xFFFFFF)?);
                dc.FillRectangle(&rect(0.0, 20.0, 60.0, 40.0), &brush(0xD7141A)?);
                let wedge = [at(0.0, 0.0), at(30.0, 20.0), at(0.0, 40.0)];
                polygon(dc, &wedge, &brush(0x11457E)?)?;
            }
            Flag::Romania => {
                for (i, rgb) in [0x002B7F, 0xFCD116, 0xCE1126].into_iter().enumerate() {
                    let left = i as f32 * 20.0;
                    dc.FillRectangle(&rect(left, 0.0, left + 20.0, 40.0), &brush(rgb)?);
                }
            }
            Flag::Hungary => {
                let height = 40.0 / 3.0;
                for (i, rgb) in [0xCD2A3E, 0xFFFFFF, 0x436F4D].into_iter().enumerate() {
                    let top = i as f32 * height;
                    dc.FillRectangle(&rect(0.0, top, 60.0, top + height), &brush(rgb)?);
                }
            }
            Flag::Greece => {
                // Nine stripes of 40/9, canton 10 of 27x18 units (200/9 square).
                let (blue, white) = (brush(0x0D5EAF)?, brush(0xFFFFFF)?);
                dc.FillRectangle(&rect(0.0, 0.0, 60.0, 40.0), &blue);
                for i in [1, 3, 5, 7] {
                    let top = i as f32 * 40.0 / 9.0;
                    dc.FillRectangle(&rect(0.0, top, 60.0, top + 40.0 / 9.0), &white);
                }
                dc.FillRectangle(&rect(0.0, 0.0, 200.0 / 9.0, 200.0 / 9.0), &blue);
                dc.FillRectangle(&rect(80.0 / 9.0, 0.0, 120.0 / 9.0, 200.0 / 9.0), &white);
                dc.FillRectangle(&rect(0.0, 80.0 / 9.0, 200.0 / 9.0, 120.0 / 9.0), &white);
            }
            Flag::Turkey => {
                let (red, white) = (brush(0xE30A17)?, brush(0xFFFFFF)?);
                dc.FillRectangle(&rect(0.0, 0.0, 60.0, 40.0), &red);
                let disc = |x: f32, r: f32| D2D1_ELLIPSE {
                    point: at(x, 20.0),
                    radiusX: r,
                    radiusY: r,
                };
                // Bigger and bolder than the real flag's, so they show at this size.
                dc.FillEllipse(&disc(21.0, 12.5), &white);
                dc.FillEllipse(&disc(25.0, 10.0), &red);
                star(dc, &white, 41.0, 20.0, 7.5, std::f32::consts::FRAC_PI_2)?;
            }
            Flag::Russia => {
                let height = 40.0 / 3.0;
                for (i, rgb) in [0xFFFFFF, 0x0039A6, 0xD52B1E].into_iter().enumerate() {
                    let top = i as f32 * height;
                    dc.FillRectangle(&rect(0.0, top, 60.0, top + height), &brush(rgb)?);
                }
            }
            Flag::Ukraine => {
                for (i, rgb) in [0x0057B7, 0xFFD700].into_iter().enumerate() {
                    let top = i as f32 * 20.0;
                    dc.FillRectangle(&rect(0.0, top, 60.0, top + 20.0), &brush(rgb)?);
                }
            }
            Flag::Japan => {
                dc.FillRectangle(&rect(0.0, 0.0, 60.0, 40.0), &brush(0xFFFFFF)?);
                let sun = D2D1_ELLIPSE {
                    point: at(30.0, 20.0),
                    radiusX: 12.0,
                    radiusY: 12.0,
                };
                dc.FillEllipse(&sun, &brush(0xBC002D)?);
            }
            Flag::SouthKorea => {
                let (red, blue, black) = (brush(0xCD2E3A)?, brush(0x0047A0)?, brush(0x000000)?);
                dc.FillRectangle(&rect(0.0, 0.0, 60.0, 40.0), &brush(0xFFFFFF)?);
                // Taegeuk: red disc, blue lower half, then the two lobes that make the S.
                let disc = |x: f32, r: f32| D2D1_ELLIPSE {
                    point: at(x, 20.0),
                    radiusX: r,
                    radiusY: r,
                };
                dc.FillEllipse(&disc(30.0, 10.0), &red);
                let geometry = dc.GetFactory()?.CreatePathGeometry()?;
                let sink = geometry.Open()?;
                sink.BeginFigure(at(20.0, 20.0), D2D1_FIGURE_BEGIN_FILLED);
                sink.AddArc(&D2D1_ARC_SEGMENT {
                    point: at(40.0, 20.0),
                    size: D2D_SIZE_F {
                        width: 10.0,
                        height: 10.0,
                    },
                    rotationAngle: 0.0,
                    sweepDirection: D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE,
                    arcSize: D2D1_ARC_SIZE_SMALL,
                });
                sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                sink.Close()?;
                dc.FillGeometry(&geometry, &blue, None::<&ID2D1Brush>);
                dc.FillEllipse(&disc(25.0, 5.0), &red);
                dc.FillEllipse(&disc(35.0, 5.0), &blue);
                // Trigrams, one per corner, bars across the diagonal; true = solid, false = broken.
                let trigrams = [
                    (-1.0, -1.0, [true, true, true]),
                    (1.0, -1.0, [false, true, false]),
                    (-1.0, 1.0, [true, false, true]),
                    (1.0, 1.0, [false, false, false]),
                ];
                for (sx, sy, bars) in trigrams {
                    let (cx, cy) = (30.0 + sx * 21.0, 20.0 + sy * 13.5);
                    let (ux, uy) = (sx * 0.832, sy * 0.555);
                    let (tx, ty) = (-uy, ux);
                    for (i, solid) in bars.into_iter().enumerate() {
                        let off = (i as f32 - 1.0) * 3.4;
                        let spans: &[(f32, f32)] = if solid {
                            &[(-4.5, 4.5)]
                        } else {
                            &[(-4.5, -1.0), (1.0, 4.5)]
                        };
                        for &(a, b) in spans {
                            let p = |t: f32| at(cx + ux * off + tx * t, cy + uy * off + ty * t);
                            dc.DrawLine(p(a), p(b), &black, 2.2, None);
                        }
                    }
                }
            }
            Flag::China => {
                let (red, yellow) = (brush(0xDE2910)?, brush(0xFFDE00)?);
                dc.FillRectangle(&rect(0.0, 0.0, 60.0, 40.0), &red);
                // Bigger than the real flag's, so they show at this size.
                star(dc, &yellow, 11.0, 11.0, 8.5, 0.0)?;
                // Each small star points at the big one.
                for (x, y) in [(23.0, 4.5), (28.0, 10.0), (28.0, 17.0), (23.0, 22.5)] {
                    star(dc, &yellow, x, y, 4.0, (11.0f32 - x).atan2(y - 11.0))?;
                }
            }
        }
    }
    Ok(())
}

/// Fills the closed outline through `points`.
unsafe fn polygon(
    dc: &ID2D1DeviceContext,
    points: &[Vector2],
    brush: &ID2D1SolidColorBrush,
) -> Result<()> {
    unsafe {
        let geometry = dc.GetFactory()?.CreatePathGeometry()?;
        let sink = geometry.Open()?;
        sink.BeginFigure(points[0], D2D1_FIGURE_BEGIN_FILLED);
        for &point in &points[1..] {
            sink.AddLine(point);
        }
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        sink.Close()?;
        dc.FillGeometry(&geometry, brush, None::<&ID2D1Brush>);
    }
    Ok(())
}

/// Fills a five-pointed star at `x`,`y`; `angle` turns its top point
/// clockwise, in radians.
unsafe fn star(
    dc: &ID2D1DeviceContext,
    brush: &ID2D1SolidColorBrush,
    x: f32,
    y: f32,
    radius: f32,
    angle: f32,
) -> Result<()> {
    let points: Vec<Vector2> = (0..10)
        .map(|i| {
            // Plumper than a true star, so it holds up when small.
            let r = if i % 2 == 0 { radius } else { radius * 0.45 };
            let a = angle + i as f32 * std::f32::consts::PI / 5.0;
            Vector2 {
                X: x + r * a.sin(),
                Y: y - r * a.cos(),
            }
        })
        .collect();
    unsafe { polygon(dc, &points, brush) }
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
