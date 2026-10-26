//! The flags drawn beside language names, and how they're drawn.

use super::*;

/// The flags drawn for the languages WinDict knows.
#[derive(Clone, Copy)]
pub(super) enum Flag {
    UnitedKingdom,
    Italy,
    France,
    Germany,
    Spain,
    Portugal,
    Netherlands,
    Sweden,
    Denmark,
    Norway,
    Finland,
    Poland,
    Czechia,
    Romania,
    Hungary,
    Greece,
    Turkey,
    Russia,
    Ukraine,
    Japan,
    SouthKorea,
    China,
}

impl Flag {
    pub(super) fn of(language: &str) -> Option<Flag> {
        match language {
            "en" => Some(Flag::UnitedKingdom),
            "it" => Some(Flag::Italy),
            "fr" => Some(Flag::France),
            "de" => Some(Flag::Germany),
            "es" => Some(Flag::Spain),
            "pt" => Some(Flag::Portugal),
            "nl" => Some(Flag::Netherlands),
            "sv" => Some(Flag::Sweden),
            "da" => Some(Flag::Denmark),
            "no" => Some(Flag::Norway),
            "fi" => Some(Flag::Finland),
            "pl" => Some(Flag::Poland),
            "cs" => Some(Flag::Czechia),
            "ro" => Some(Flag::Romania),
            "hu" => Some(Flag::Hungary),
            "el" => Some(Flag::Greece),
            "tr" => Some(Flag::Turkey),
            "ru" => Some(Flag::Russia),
            "uk" => Some(Flag::Ukraine),
            "ja" => Some(Flag::Japan),
            "ko" => Some(Flag::SouthKorea),
            "zh-CN" => Some(Flag::China),
            _ => None,
        }
    }
}

/// Draws `flag` filling 0,0 to 60,40.
pub(super) unsafe fn draw_flag(dc: &ID2D1DeviceContext, flag: Flag) -> Result<()> {
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
