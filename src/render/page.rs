//! Laying out the pages the intro's pictures show behind the card: a book's
//! or a mail's lines, set as the app showing them would, with what the card
//! is about selected.

use super::*;

/// A face a page is set in.
pub struct Font {
    pub family: &'static str,
    pub size: f32,
    pub bold: bool,
}

/// A line on a page, in DIPs. A justified one fills `width`, its words
/// spread apart; a line that isn't starts at `x`.
pub struct Line<'a> {
    pub x: f32,
    /// The top of its line box.
    pub y: f32,
    pub width: f32,
    /// Its line box's, its text in the middle, as a browser has it.
    pub height: f32,
    pub font: &'a Font,
    pub color: u32,
    pub justify: bool,
    pub text: &'a str,
    /// What follows `text` selected, if anything.
    pub picked: &'a str,
}

/// A rounded rectangle on a page, in `color`.
pub struct Patch {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub radius: f32,
    pub color: u32,
}

impl Renderer {
    /// `lines` and `patches` on a page `width` by `height`. What's selected
    /// shows on the accent, as a selection does.
    pub fn layout_page(
        &self,
        lines: &[Line],
        patches: &[Patch],
        width: f32,
        height: f32,
    ) -> Result<Layout> {
        let mut items: Vec<Item> = (patches.iter())
            .map(|p| Item::Shape {
                x: p.x,
                y: p.y,
                width: p.width,
                height: p.height,
                radius: p.radius,
                fill: Some(Paint::Rgb(p.color)),
                stroke: None,
            })
            .collect();
        for line in lines {
            let format = self.page_format(line.font)?;
            let measure = |text: &str| -> Result<(Text, f32, f32)> {
                let layout = self.text(text, &format, 10_000.0)?;
                let (w, h, _) = metrics(&layout);
                Ok((layout, w, h))
            };
            // The words, then what's selected as one.
            let mut words: Vec<(&str, bool)> = if line.justify {
                line.text.split_whitespace().map(|w| (w, false)).collect()
            } else if line.text.is_empty() {
                Vec::new()
            } else {
                vec![(line.text, false)]
            };
            if !line.picked.is_empty() {
                words.push((line.picked, true));
            }
            let laid: Vec<_> = (words.iter())
                .map(|&(text, picked)| Ok((measure(text)?, picked)))
                .collect::<Result<_>>()?;
            // A line that isn't justified keeps its own spaces, each run's
            // width taking in those it ends with.
            let gap = match (line.justify, laid.len()) {
                (true, n) if n > 1 => {
                    let used: f32 = laid.iter().map(|((_, w, _), _)| w).sum();
                    (line.width - used) / (n - 1) as f32
                }
                _ => 0.0,
            };
            let mut x = line.x;
            for ((layout, w, h), picked) in laid {
                let y = line.y + (line.height - h) / 2.0;
                if picked {
                    items.push(Item::Shape {
                        x,
                        y: line.y,
                        width: w,
                        height: line.height,
                        radius: 0.0,
                        fill: Some(Paint::Accent),
                        stroke: None,
                    });
                }
                items.push(Item::Text {
                    layout,
                    x,
                    y,
                    ink: if picked {
                        Ink::OnAccent
                    } else {
                        Ink::Rgb(line.color)
                    },
                    accents: Vec::new(),
                    selectable: false,
                });
                x += w + gap;
            }
        }
        Ok(Layout {
            items,
            width,
            height,
            links: Vec::new(),
            caret: None,
            list: None,
        })
    }

    /// How wide `text` is, set in `font`, in DIPs.
    pub fn page_width(&self, font: &Font, text: &str) -> Result<f32> {
        let layout = self.text(text, &self.page_format(font)?, 10_000.0)?;
        Ok(metrics(&layout).0)
    }

    fn page_format(&self, font: &Font) -> Result<IDWriteTextFormat> {
        let weight = if font.bold {
            DWRITE_FONT_WEIGHT_BOLD
        } else {
            DWRITE_FONT_WEIGHT_NORMAL
        };
        unsafe {
            let format = self.dwrite.CreateTextFormat(
                &HSTRING::from(font.family),
                None,
                weight,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                font.size,
                w!("en-us"),
            )?;
            format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            Ok(format)
        }
    }
}
