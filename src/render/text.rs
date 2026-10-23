//! Laying out a line, a label or a field of text.

use super::*;

/// How far below its middle the speaker glyph's ink reaches in a heading
/// button, in DIPs: in Segoe Fluent Icons and Segoe MDL2 Assets alike, it
/// spans 161 to 1887 of the 2048 units of a 14 DIP em, around its middle.
const BUTTON_INK_BELOW: f32 = (1887.0 - 161.0) / 2.0 / 2048.0 * 14.0;

impl Renderer {
    /// The word and its pronunciation, `indent` DIPs from the left, with
    /// room after them for a button `button` DIPs wide, if there's one.
    /// A `muted` heading says what the card is doing rather than naming a word.
    pub fn layout_header(
        &self,
        word: &str,
        phonetic: Option<&str>,
        width: f32,
        indent: f32,
        button: Option<f32>,
        muted: bool,
    ) -> Result<Heading> {
        let room = width - indent;
        let button_room = button.map_or(0.0, |w| HEADING_BUTTON_GAP + w);
        // The button follows the pronunciation, or else the word.
        let word_room = match phonetic {
            Some(_) => room,
            None => room - button_room,
        };
        let word = capitalize(word);
        let title = self.text(&word, &self.formats.word, word_room)?;
        let (title_w, title_h, baseline) = metrics(&title);
        let mut height = title_h;
        // Where what the button follows ends, and where the button's middle
        // goes so that its glyph rests on the last line's baseline, as one
        // more letter would.
        let (end_x, base) = line_end(&title, &word);
        let mut end = (indent + end_x, base - BUTTON_INK_BELOW);
        let mut pronunciation = None;
        if let Some(p) = phonetic {
            let gap = 10.0;
            let phon = self.text(p, &self.formats.phonetic, room - button_room)?;
            let (phon_w, phon_h, phon_baseline) = metrics(&phon);
            let wrapped = title_w + gap + phon_w + button_room > room;
            let (x, y) = if wrapped {
                height += 2.0 + phon_h;
                (indent, title_h + 2.0)
            } else {
                // Same line, sharing the baseline.
                (indent + title_w + gap, baseline - phon_baseline)
            };
            let (end_x, base) = line_end(&phon, p);
            end = (x + end_x, y + base - BUTTON_INK_BELOW);
            pronunciation = Some(Placed {
                layout: Layout {
                    items: vec![Item::Text {
                        layout: phon,
                        x: 0.0,
                        y: 0.0,
                        ink: Ink::Secondary,
                        accents: Vec::new(),
                    }],
                    // A little over, so the last glyph's overhang isn't cut.
                    width: phon_w.ceil() + 1.0,
                    height: phon_h,
                    links: Vec::new(),
                    caret: None,
                    list: None,
                },
                x,
                y,
            });
        }
        let word = Layout {
            items: vec![Item::Text {
                layout: title,
                x: indent,
                y: 0.0,
                ink: if muted { Ink::Tertiary } else { Ink::Primary },
                accents: Vec::new(),
            }],
            width,
            height: title_h,
            links: Vec::new(),
            caret: None,
            list: None,
        };
        Ok(Heading {
            word,
            word_size: (title_w, title_h),
            phonetic: pronunciation,
            button: button.map(|_| (end.0 + HEADING_BUTTON_GAP, end.1)),
            height,
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
            caret: None,
            list: None,
        };
        Ok((layout, w, caret))
    }

    /// What's typed in a settings field, on one line in `ink`, its width,
    /// and where the caret goes when it's `at` UTF-16 units in.
    pub fn layout_field(&self, text: &str, ink: Ink, at: u32) -> Result<(Layout, f32, Caret)> {
        let line = self.text(text, &self.formats.setting, 0.0)?;
        let (w, height, _) = metrics(&line);
        let (mut x, mut y) = (0.0, 0.0);
        let mut hit = DWRITE_HIT_TEST_METRICS::default();
        unsafe { line.HitTestTextPosition(at, false, &mut x, &mut y, &mut hit)? };
        let caret = Caret {
            x,
            top: hit.top,
            height: hit.height,
        };
        let mut out = Stack::default();
        out.text(line, 0.0, ink);
        let layout = Layout {
            items: out.items,
            width: w + 4.0,
            height,
            links: Vec::new(),
            caret: None,
            list: None,
        };
        Ok((layout, w, caret))
    }

    /// Where the caret goes in a settings field holding `text` when it's
    /// clicked `x` DIPs in, in UTF-16 units.
    pub fn field_position(&self, text: &str, x: f32) -> Result<u32> {
        let line = self.text(text, &self.formats.setting, 0.0)?;
        let (mut trailing, mut inside) = (false.into(), false.into());
        let mut hit = DWRITE_HIT_TEST_METRICS::default();
        unsafe { line.HitTestPoint(x, 0.0, &mut trailing, &mut inside, &mut hit)? };
        Ok(hit.textPosition + if trailing.as_bool() { hit.length } else { 0 })
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
        if let Some(code) = badge {
            let line_h = metrics(&self.text("Ag", &self.formats.suggestion, width)?).1;
            x = self.badge(&mut out, code, 0.0, 0.0, line_h)? + 8.0;
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
            caret: None,
            list: None,
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
            caret: None,
            list: None,
        })
    }

    /// A glyph in the accent color and a word of menu text beside it, as
    /// wide as they are, level along their middles.
    pub fn layout_badge(&self, glyph: &str, label: &str) -> Result<Layout> {
        let gap = 7.0;
        let glyph = self.text(glyph, &self.formats.icon, 100.0)?;
        let label = self.text(label, &self.formats.menu, 400.0)?;
        let (glyph_w, glyph_h, _) = metrics(&glyph);
        let (label_w, label_h, _) = metrics(&label);
        let height = glyph_h.max(label_h);
        let mut out = Stack {
            y: (height - glyph_h) / 2.0,
            ..Default::default()
        };
        out.text(glyph, 0.0, Ink::Accent);
        out.y = (height - label_h) / 2.0;
        out.text(label, glyph_w + gap, Ink::Primary);
        Ok(Layout {
            items: out.items,
            width: (glyph_w + gap + label_w).ceil(),
            height,
            links: Vec::new(),
            caret: None,
            list: None,
        })
    }

    /// One unwrapped line of menu text.
    pub fn layout_line(&self, text: &str, face: Face, ink: Ink, width: f32) -> Result<Layout> {
        let mut out = Stack::default();
        let (_, height) = out.text(self.text(text, self.face(face), width)?, 0.0, ink);
        Ok(Layout {
            items: out.items,
            width,
            height,
            links: Vec::new(),
            caret: None,
            list: None,
        })
    }

    /// How wide `text` is in `face`, on one line, in DIPs.
    pub fn line_width(&self, text: &str, face: Face) -> Result<f32> {
        let line = self.text(text, self.face(face), f32::from(u16::MAX))?;
        Ok(metrics(&line).0)
    }

    fn face(&self, face: Face) -> &IDWriteTextFormat {
        match face {
            Face::Menu => &self.formats.menu,
            Face::Small => &self.formats.footer,
            Face::Icon => &self.formats.icon,
            Face::Button => &self.formats.button,
            Face::LargeButton => &self.formats.large_button,
            Face::Title => &self.formats.title,
            Face::Query => &self.formats.query,
            Face::Heading => &self.formats.heading,
            Face::Display => &self.formats.display,
            Face::Setting => &self.formats.setting,
            Face::Value => &self.formats.value,
            Face::Chrome => &self.formats.chrome,
        }
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

    /// A tooltip's small text, wrapped at `width` but only as wide as its
    /// longest line.
    pub fn layout_tip(&self, text: &str, width: f32) -> Result<Layout> {
        let mut out = Stack::default();
        let line = self.text(text, &self.formats.footer, width)?;
        let (w, height) = out.text(line, 0.0, Ink::Primary);
        Ok(Layout {
            items: out.items,
            width: w.ceil().min(width),
            height,
            links: Vec::new(),
            caret: None,
            list: None,
        })
    }

    /// `text` wrapped at `width`, in the paragraph face, with each
    /// `{name}` in it that `keys` has shown as those keys, each on a cap of
    /// its own, as [`Renderer::layout_caps`] draws them, within the line.
    pub fn layout_paragraph(
        &self,
        text: &str,
        keys: &[(&str, Vec<String>)],
        ink: Ink,
        width: f32,
    ) -> Result<Layout> {
        // Each name is padded with spaces that don't break, so its cap has
        // room, and a shortcut's caps stay together on one line.
        const PAD: char = '\u{2007}';
        const GAP: char = '\u{00A0}';
        /// Room for the Windows logo, which is drawn in.
        const LOGO_ROOM: &str = "\u{2007}\u{2007}";
        let units = |s: &str| s.encode_utf16().count() as u32;
        let (mut shown, mut caps, mut rest) = (String::new(), Vec::new(), text);
        while let Some(open) = rest.find('{') {
            let Some(close) = rest[open..].find('}').map(|i| open + i) else {
                break;
            };
            let name = &rest[open + 1..close];
            let Some((_, names)) = keys.iter().find(|(n, _)| *n == name) else {
                shown.push_str(&rest[..=close]);
                rest = &rest[close + 1..];
                continue;
            };
            shown.push_str(&rest[..open]);
            for (i, key) in names.iter().enumerate() {
                if i > 0 {
                    shown.push(GAP);
                }
                let logo = key == "Win";
                let start = units(&shown);
                shown.push(PAD);
                shown.push_str(if logo { LOGO_ROOM } else { key });
                shown.push(PAD);
                let range = DWRITE_TEXT_RANGE {
                    startPosition: start,
                    length: units(&shown) - start,
                };
                caps.push((range, logo));
            }
            rest = &rest[close + 1..];
        }
        shown.push_str(rest);

        let line = self.text(&shown, &self.formats.paragraph, width)?;
        for (range, _) in &caps {
            unsafe { line.SetFontSize(INLINE_KEY_SIZE, *range)? };
        }
        let (_, _, baseline) = metrics(&line);
        let mut out = Stack::default();
        // Behind the text: each cap spans the letters, not the room between
        // the lines.
        let (rise, drop) = (
            PARAGRAPH_SIZE * INLINE_CAP_RISE,
            PARAGRAPH_SIZE * INLINE_CAP_DROP,
        );
        for (range, logo) in caps {
            for rect in hit_rects(&line, range) {
                let y = rect.top + baseline - rise;
                out.items.push(Item::Key {
                    x: rect.left,
                    y,
                    width: rect.width,
                    height: rise + drop,
                });
                if logo {
                    out.items.push(Item::Logo {
                        x: rect.left + (rect.width - LOGO_SIZE) / 2.0,
                        y: y + (rise + drop - LOGO_SIZE) / 2.0,
                        size: LOGO_SIZE,
                    });
                }
            }
        }
        let (_, height) = out.text(line, 0.0, ink);
        Ok(Layout {
            items: out.items,
            width,
            height,
            links: Vec::new(),
            caret: None,
            list: None,
        })
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
            caret: None,
            list: None,
        })
    }

    /// The keys of a shortcut, each on its own cap: "[Ctrl] [Alt] [D]".
    /// The Windows key shows its logo.
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
            let logo = *key == "Win";
            let cap_w = if logo {
                cap_h
            } else {
                (w + 2.0 * pad_x).max(cap_h)
            };
            out.items.push(Item::Key {
                x,
                y: 0.0,
                width: cap_w,
                height: cap_h,
            });
            if logo {
                out.items.push(Item::Logo {
                    x: x + (cap_w - LOGO_SIZE) / 2.0,
                    y: (cap_h - LOGO_SIZE) / 2.0,
                    size: LOGO_SIZE,
                });
            } else {
                out.text(name, x + (cap_w - w) / 2.0, Ink::Primary);
            }
            x += cap_w;
            height = height.max(cap_h);
        }
        Ok(Layout {
            items: out.items,
            width: x.max(1.0),
            height,
            links: Vec::new(),
            caret: None,
            list: None,
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
            caret: None,
            list: None,
        })
    }

    /// `text` in the accent color, underlined, then the icon for links that
    /// open the browser. Returns its width and height.
    pub(super) fn external_link(
        &self,
        out: &mut Stack,
        text: &str,
        x: f32,
        width: f32,
    ) -> Result<(f32, f32)> {
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
    pub(super) fn spaced_caps(&self, text: &str, width: f32) -> Result<IDWriteTextLayout> {
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

    /// `language`'s flag, if it has one, at `x`, centred on a line `line_h`
    /// tall at `y`. Returns its width.
    pub(super) fn flag(
        &self,
        out: &mut Stack,
        language: &str,
        x: f32,
        y: f32,
        line_h: f32,
    ) -> Option<f32> {
        let flag = Flag::of(language)?;
        // 3:2, the height of a capital.
        let (flag_w, flag_h) = (18.0, 12.0);
        out.items.push(Item::Flag {
            flag,
            x,
            y: y + ((line_h - flag_h) / 2.0).round(),
            width: flag_w,
            height: flag_h,
        });
        Some(flag_w)
    }

    /// `language`'s flag, or its code on a cap when it has none, like
    /// [`Renderer::flag`]. Returns its width.
    pub(super) fn badge(
        &self,
        out: &mut Stack,
        language: &str,
        x: f32,
        y: f32,
        line_h: f32,
    ) -> Result<f32> {
        if let Some(w) = self.flag(out, language, x, y, line_h) {
            return Ok(w);
        }
        // Wide enough for any two letters, so the words line up.
        let (pad_y, min_w) = (1.0, BADGE_W);
        let code = language.split('-').next().unwrap_or(language);
        let name = self.text(&code.to_uppercase(), &self.formats.footer, 100.0)?;
        let (w, h, _) = metrics(&name);
        let (cap_w, cap_h) = (min_w.max(w + 8.0), h + 2.0 * pad_y);
        let cap_y = y + ((line_h - cap_h) / 2.0).round();
        out.items.push(Item::Key {
            x,
            y: cap_y,
            width: cap_w,
            height: cap_h,
        });
        out.items.push(Item::Text {
            layout: name,
            x: x + ((cap_w - w) / 2.0).round(),
            y: cap_y + pad_y,
            ink: Ink::Secondary,
            accents: Vec::new(),
        });
        Ok(cap_w)
    }
}
