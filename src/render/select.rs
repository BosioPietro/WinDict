//! Selecting a layout's text with the mouse: where a point falls in it,
//! what's between two such places, and the rectangles that cover it.

use windows::core::BOOL;

use super::*;

/// A place in a layout's selectable text: the text it's in, and how far
/// into it, in UTF-16 units. Texts are laid out in reading order, so
/// places compare as they read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TextPoint {
    item: usize,
    at: u32,
}

impl Layout {
    /// The selectable texts, with their index in the items and top left.
    fn texts(&self) -> impl Iterator<Item = (usize, &Text, f32, f32)> {
        self.items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| match item {
                Item::Text {
                    layout,
                    x,
                    y,
                    selectable: true,
                    ..
                } => Some((i, layout, *x, *y)),
                _ => None,
            })
    }

    /// Whether (`x`, `y`), in DIPs, is over selectable text.
    pub fn over_text(&self, x: f32, y: f32) -> bool {
        self.texts().any(|(_, text, tx, ty)| {
            let (w, h, _) = metrics(text);
            (tx..=tx + w).contains(&x) && (ty..=ty + h).contains(&y)
        })
    }

    /// The place in the selectable text nearest (`x`, `y`), in DIPs: in
    /// the text on that height nearest it across, or else in the nearest
    /// text up or down. `None` if there's none.
    pub fn text_point(&self, x: f32, y: f32) -> Option<TextPoint> {
        let gap = |from: f32, to: f32, at: f32| (from - at).max(at - to).max(0.0);
        let (item, text, tx, ty) = self.texts().min_by(|a, b| {
            let distance = |&(_, text, tx, ty): &(usize, &Text, f32, f32)| {
                let (w, h, _) = metrics(text);
                (gap(ty, ty + h, y), gap(tx, tx + w, x))
            };
            distance(a)
                .partial_cmp(&distance(b))
                .unwrap_or(std::cmp::Ordering::Equal)
        })?;
        let (mut trailing, mut inside) = (BOOL(0), BOOL(0));
        let mut hit = DWRITE_HIT_TEST_METRICS::default();
        unsafe {
            text.HitTestPoint(x - tx, y - ty, &mut trailing, &mut inside, &mut hit)
                .ok()?;
        }
        let at = hit.textPosition + if trailing.as_bool() { hit.length } else { 0 };
        Some(TextPoint { item, at })
    }

    /// The word around `point`: the run of letters and digits it's in or
    /// next to, or the character after it if it's in neither.
    pub fn word_at(&self, point: TextPoint) -> (TextPoint, TextPoint) {
        let Some(Item::Text { layout: text, .. }) = self.items.get(point.item) else {
            return (point, point);
        };
        let wide = &text.wide;
        // Characters with where they start, in UTF-16 units.
        let mut chars = Vec::new();
        let mut at = 0;
        for c in char::decode_utf16(wide.iter().copied()) {
            let c = c.unwrap_or(char::REPLACEMENT_CHARACTER);
            chars.push((at as u32, c));
            at += c.len_utf16();
        }
        let is_word = |c: char| c.is_alphanumeric() || c == '\'' || c == '’' || c == '-';
        let mut i = chars.partition_point(|&(start, _)| start < point.at);
        // On the word's far edge, the word is the one just before.
        if (i == chars.len() || !is_word(chars[i].1)) && i > 0 && is_word(chars[i - 1].1) {
            i -= 1;
        }
        let Some(&(start, c)) = chars.get(i) else {
            return (point, point);
        };
        let end_of = |i: usize| chars.get(i + 1).map_or(wide.len() as u32, |&(s, _)| s);
        if !is_word(c) {
            let end = end_of(i);
            return (
                TextPoint { at: start, ..point },
                TextPoint { at: end, ..point },
            );
        }
        let (mut first, mut last) = (i, i);
        while first > 0 && is_word(chars[first - 1].1) {
            first -= 1;
        }
        while last + 1 < chars.len() && is_word(chars[last + 1].1) {
            last += 1;
        }
        let from = TextPoint {
            at: chars[first].0,
            ..point
        };
        (
            from,
            TextPoint {
                at: end_of(last),
                ..point
            },
        )
    }

    /// Each selectable text's part between `a` and `b`, with its top left.
    fn spans(&self, a: TextPoint, b: TextPoint) -> Vec<(&Text, f32, f32, u32, u32)> {
        let (from, to) = if a <= b { (a, b) } else { (b, a) };
        self.texts()
            .filter(|(i, ..)| (from.item..=to.item).contains(i))
            .filter_map(|(i, text, x, y)| {
                let start = if i == from.item { from.at } else { 0 };
                let end = if i == to.item {
                    to.at
                } else {
                    text.wide.len() as u32
                };
                (start < end).then_some((text, x, y, start, end))
            })
            .collect()
    }

    /// The rectangles covering the text between `a` and `b`, in DIPs.
    pub fn selection_rects(&self, a: TextPoint, b: TextPoint) -> Vec<Mark> {
        self.spans(a, b)
            .into_iter()
            .flat_map(|(text, x, y, start, end)| {
                let range = DWRITE_TEXT_RANGE {
                    startPosition: start,
                    length: end - start,
                };
                hit_rects(text, range).into_iter().map(move |r| Mark {
                    x: x + r.left,
                    y: y + r.top,
                    width: r.width,
                    height: r.height,
                })
            })
            .collect()
    }

    /// The text between `a` and `b`: texts side by side are joined with a
    /// space, and those one under another with a line break.
    pub fn selected_text(&self, a: TextPoint, b: TextPoint) -> String {
        let mut out = String::new();
        let mut last: Option<(f32, f32)> = None;
        for (text, _, y, start, end) in self.spans(a, b) {
            let (_, h, _) = metrics(text);
            if let Some((top, bottom)) = last {
                let beside = y < bottom && y + h > top;
                out.push(if beside { ' ' } else { '\n' });
            }
            last = Some((y, y + h));
            out.push_str(&String::from_utf16_lossy(
                &text.wide[start as usize..end as usize],
            ));
        }
        out
    }
}
