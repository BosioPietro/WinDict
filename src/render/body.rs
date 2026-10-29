//! Laying out what the card's body shows: definitions, messages, and the translation with its pickers.

use super::*;

impl Renderer {
    pub fn layout_body(&self, body: &Body, width: f32, bottom_padding: f32) -> Result<Layout> {
        let mut out = Stack::default();
        match body {
            // Room for the loading dots, which popup.rs draws.
            Body::Loading { .. } => out.y = 26.0,
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
            Body::Translation(card) => self.layout_translation(&mut out, card, width)?,
        }
        Ok(Layout {
            items: out.items,
            width,
            height: out.y + bottom_padding,
            links: out.links,
            caret: out.caret,
            list: out.list,
        })
    }

    pub(super) fn layout_message(
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
            // Where the word falls is up to the translation; the link is just it.
            let (before, after) = tr()
                .did_you_mean
                .split_once("{word}")
                .unwrap_or((tr().did_you_mean, ""));
            // DirectWrite counts UTF-16 code units.
            let range = DWRITE_TEXT_RANGE {
                startPosition: before.encode_utf16().count() as u32,
                length: word.encode_utf16().count() as u32,
            };
            let line = self.text(&format!("{before}{word}{after}"), &self.formats.body, width)?;
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
    pub(super) fn layout_language(
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
                selectable: true,
            });
        }
        out.y += h + 10.0;
        Ok(())
    }

    /// The note, then each part of speech with its numbered senses.
    pub(super) fn layout_definition(
        &self,
        out: &mut Stack,
        def: &Definition,
        width: f32,
    ) -> Result<()> {
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
            let pos = self.spaced_caps(tr().part_of_speech(&meaning.part_of_speech), width)?;
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
                self.layout_related(out, tr().similar, &meaning.synonyms, indent, width - indent)?;
            }
            if !meaning.antonyms.is_empty() {
                self.layout_related(
                    out,
                    tr().opposite,
                    &meaning.antonyms,
                    indent,
                    width - indent,
                )?;
            }
        }
        Ok(())
    }

    /// `label` ("Similar", "Opposite") and the words, each a link to its own
    /// definition.
    pub(super) fn layout_related(
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

    /// The languages of a translation, then the translation, or the list to
    /// pick one of the languages from.
    pub(super) fn layout_translation(
        &self,
        out: &mut Stack,
        card: &TranslationCard,
        width: f32,
    ) -> Result<()> {
        out.y += LANGUAGES_TOP;
        let open = card.picker.as_ref().map(|(side, p)| (*side, p));
        self.layout_direction(out, (card.from, card.to), open, width)?;
        if let Some((side, picker)) = open {
            let current = match side {
                Side::From => card.from,
                Side::To => card.to,
            };
            return self.layout_picker(out, picker, current, 0.0, width);
        }
        out.y += 16.0;
        match card.state {
            Translated::Asking => {
                let ask = fill(
                    tr().translate_ambiguous,
                    &[
                        ("word", &card.word),
                        ("from", &translate::name(card.from)),
                        ("to", &translate::name(card.to)),
                    ],
                );
                let (_, h) = out.text(
                    self.text(&ask, &self.formats.body, width)?,
                    0.0,
                    Ink::Secondary,
                );
                out.y += h + 8.0;
                let label = fill(
                    tr().translate_into,
                    &[("language", &translate::name(card.to))],
                );
                let line = self.text(&label, &self.formats.body, width)?;
                let all = DWRITE_TEXT_RANGE {
                    startPosition: 0,
                    length: u32::MAX,
                };
                unsafe {
                    let _ = line.SetUnderline(true, all);
                }
                let (w, h) = out.text(line, 0.0, Ink::Accent);
                out.links.push(Link {
                    x: -2.0,
                    y: out.y,
                    width: w + 4.0,
                    height: h,
                    target: Target::Confirm,
                });
                out.y += h;
            }
            Translated::Waiting => {}
            Translated::Loading => {
                let line = self.text(tr().translating, &self.formats.body, width)?;
                let (_, h) = out.text(line, 0.0, Ink::Tertiary);
                out.y += h;
            }
            Translated::Failed(error) => {
                self.layout_message(out, tr().translate_failed, error, None, width)?;
            }
            Translated::Done(translation) => {
                self.layout_translated(out, card, translation, width)?
            }
        }
        Ok(())
    }

    /// The boxes of a translation's languages, with `open`'s list under the
    /// one of its side, as the settings show them.
    pub fn layout_languages(
        &self,
        from: &str,
        to: &str,
        open: Option<(Side, &Picker)>,
        width: f32,
    ) -> Result<Layout> {
        let mut out = Stack::default();
        self.layout_direction(&mut out, (from, to), open, width)?;
        if let Some((side, picker)) = open {
            let current = match side {
                Side::From => from,
                Side::To => to,
            };
            self.layout_picker(&mut out, picker, current, 0.0, width)?;
        }
        Ok(Layout {
            items: out.items,
            width,
            height: out.y,
            links: out.links,
            caret: out.caret,
            list: out.list,
        })
    }

    /// The box of the language WinDict is shown in, `language`'s code,
    /// `width` wide, with `picker`'s list under it while it's open,
    /// `list_w` wide and lined up with the box's right, and what moves in
    /// it as it opens or shuts, apart.
    pub fn layout_interface(
        &self,
        language: &str,
        picker: Option<&Picker>,
        width: f32,
        list_w: f32,
    ) -> Result<(Layout, Moving)> {
        let mut out = Stack::default();
        let (chevron, name) =
            self.layout_select(&mut out, 0.0, width, Target::Interface, language, picker)?;
        // The name was laid out after the chevron, so it goes first. It has
        // room for what overhangs its box; the chevron turns about its
        // middle, so it has none.
        let name = take(&mut out.items, name, 2.0);
        let chevron = take(&mut out.items, chevron, 0.0);
        out.y += SELECT_H;
        if let Some(picker) = picker {
            self.layout_picker(&mut out, picker, language, width - list_w, list_w)?;
        }
        let layout = Layout {
            items: out.items,
            width,
            height: out.y,
            links: out.links,
            caret: out.caret,
            list: out.list,
        };
        Ok((layout, Moving { name, chevron }))
    }

    /// A tile to pick `language` by, `width` by `height`: its flag and its
    /// name in itself, outlined in the accent and ticked when it's `on`.
    pub fn layout_tile(&self, language: &str, on: bool, width: f32, height: f32) -> Result<Layout> {
        let mut out = Stack::default();
        out.items.push(Item::Shape {
            x: 0.0,
            y: 0.0,
            width,
            height,
            radius: CONTROL_RADIUS,
            fill: Some(Paint::Control),
            stroke: Some(if on {
                Paint::Focus
            } else {
                Paint::ControlStroke
            }),
        });
        let format = &self.formats.suggestion;
        let line_h = metrics(&self.text("Ag", format, width)?.layout).1;
        let line_y = ((height - line_h) / 2.0).round();
        let mut x = SELECT_PAD;
        x += self.badge(&mut out, language, x, line_y, line_h)? + 10.0;
        let mut end = width - SELECT_PAD;
        if on {
            let check = self.text(CHECK_ICON, &self.formats.icon, width)?;
            let (check_w, check_h, _) = metrics(&check);
            end -= check_w;
            out.items.push(Item::Text {
                layout: check,
                x: end,
                y: ((height - check_h) / 2.0).round(),
                ink: Ink::Accent,
                accents: Vec::new(),
                selectable: false,
            });
            end -= 8.0;
        }
        let line = self.text(&translate::name(language), format, (end - x).max(1.0))?;
        if on {
            let all = DWRITE_TEXT_RANGE {
                startPosition: 0,
                length: u32::MAX,
            };
            unsafe {
                let _ = line.SetFontWeight(DWRITE_FONT_WEIGHT_SEMI_BOLD, all);
            }
        }
        out.items.push(Item::Text {
            layout: line,
            x,
            y: line_y,
            ink: Ink::Primary,
            accents: Vec::new(),
            selectable: false,
        });
        Ok(Layout {
            items: out.items,
            width,
            height,
            links: Vec::new(),
            caret: None,
            list: None,
        })
    }

    /// A box for each language, which lists the others to pick from, and
    /// the arrows between them, which swap them. `open`'s side shows what's
    /// typed in its picker.
    pub(super) fn layout_direction(
        &self,
        out: &mut Stack,
        (from, to): (&str, &str),
        open: Option<(Side, &Picker)>,
        width: f32,
    ) -> Result<()> {
        let box_w = ((width - SWAP_W - 2.0 * SWAP_GAP) / 2.0).floor();
        let open = |side| open.filter(|(s, _)| *s == side).map(|(_, p)| p);
        let target = Target::Language;
        self.layout_select(out, 0.0, box_w, target(Side::From), from, open(Side::From))?;
        let x = box_w + SWAP_GAP;
        let icon = self.text(SWAP_ICON, &self.formats.button, SWAP_W)?;
        let (_, icon_h, _) = metrics(&icon);
        out.items.push(Item::Text {
            layout: icon,
            x,
            y: out.y + ((SELECT_H - icon_h) / 2.0).round(),
            ink: Ink::Secondary,
            accents: Vec::new(),
            selectable: false,
        });
        out.links.push(Link {
            x,
            y: out.y,
            width: SWAP_W,
            height: SELECT_H,
            target: Target::Swap,
        });
        let x = width - box_w;
        self.layout_select(out, x, box_w, target(Side::To), to, open(Side::To))?;
        out.y += SELECT_H;
        Ok(())
    }

    /// The box showing `language`, `x` from the left and `width` wide,
    /// which a click on opens to `target`. While `picker` is open on it, it
    /// shows what's typed instead. Returns where in `out`'s items its
    /// chevron and its text are.
    pub(super) fn layout_select(
        &self,
        out: &mut Stack,
        x: f32,
        width: f32,
        target: Target,
        language: &str,
        picker: Option<&Picker>,
    ) -> Result<(usize, usize)> {
        let y = out.y;
        out.items.push(Item::Shape {
            x,
            y,
            width,
            height: SELECT_H,
            radius: CONTROL_RADIUS,
            fill: Some(Paint::Control),
            stroke: Some(if picker.is_some() {
                Paint::Focus
            } else {
                Paint::ControlStroke
            }),
        });
        // The ring goes just outside the box.
        out.links.push(Link {
            x: x - 3.0,
            y: y - 3.0,
            width: width + 6.0,
            height: SELECT_H + 6.0,
            target,
        });
        let format = &self.formats.suggestion;
        let line_h = metrics(&self.text("Ag", format, width)?.layout).1;
        let line_y = y + ((SELECT_H - line_h) / 2.0).round();
        let centred = |h: f32| y + ((SELECT_H - h) / 2.0).round();

        let chevron = if picker.is_some() {
            CHEVRON_UP
        } else {
            CHEVRON_DOWN
        };
        let chevron = self.text(chevron, &self.formats.icon, width)?;
        let (chevron_w, chevron_h, _) = metrics(&chevron);
        let chevron_x = x + width - SELECT_PAD - chevron_w;
        let chevron_at = out.items.len();
        out.items.push(Item::Text {
            layout: chevron,
            x: chevron_x,
            y: centred(chevron_h),
            ink: Ink::Secondary,
            accents: Vec::new(),
            selectable: false,
        });

        let mut left = x + SELECT_PAD;
        let Some(picker) = picker else {
            if let Some(w) = self.flag(out, language, left, line_y, line_h) {
                left += w + 8.0;
            }
            let name = self.text(&translate::name(language), format, chevron_x - left)?;
            let name_at = out.items.len();
            out.items.push(Item::Text {
                layout: name,
                x: left,
                y: line_y,
                ink: Ink::Primary,
                accents: Vec::new(),
                selectable: false,
            });
            return Ok((chevron_at, name_at));
        };
        let glyph = self.text(SEARCH_ICON, &self.formats.icon, width)?;
        let (glyph_w, glyph_h, _) = metrics(&glyph);
        out.items.push(Item::Text {
            layout: glyph,
            x: left,
            y: centred(glyph_h),
            ink: Ink::Secondary,
            accents: Vec::new(),
            selectable: false,
        });
        left += glyph_w + 8.0;
        // Until something's typed, the language it is now, faintly.
        let (text, ink, at) = if picker.query.is_empty() {
            (translate::name(language), Ink::Tertiary, 0)
        } else {
            (picker.query.to_string(), Ink::Primary, picker.caret)
        };
        let line = self.text(&text, format, chevron_x - left)?;
        let (mut caret_x, mut caret_y) = (0.0, 0.0);
        let mut hit = DWRITE_HIT_TEST_METRICS::default();
        unsafe { line.HitTestTextPosition(at, false, &mut caret_x, &mut caret_y, &mut hit)? };
        let line_at = out.items.len();
        out.items.push(Item::Text {
            layout: line,
            x: left,
            y: line_y,
            ink,
            accents: Vec::new(),
            selectable: false,
        });
        out.caret = Some(Mark {
            x: left + caret_x.round(),
            y: line_y + hit.top,
            width: CARET_W,
            height: hit.height,
        });
        Ok((chevron_at, line_at))
    }

    /// The list of the languages `picker` lists, under the box it's open
    /// on, `x` from its left and `width` wide: its outline, with room for
    /// as many rows as show at once, and all of them laid out apart (see
    /// [`Renderer::layout_rows`]).
    pub(super) fn layout_picker(
        &self,
        out: &mut Stack,
        picker: &Picker,
        current: &str,
        x: f32,
        width: f32,
    ) -> Result<()> {
        out.y += LIST_GAP;
        let top = out.y;
        let height = 2.0 * LIST_PAD + picker.shown() as f32 * ROW_H;
        let row_w = width - 2.0 * LIST_PAD;
        out.list = Some(Box::new(ListLayout {
            panel: list_panel(width, height),
            rows: self.layout_rows(picker, current, row_w)?,
            frame: Mark {
                x,
                y: top,
                width,
                height,
            },
            viewport: Mark {
                x: x + LIST_PAD,
                y: top + LIST_PAD,
                width: row_w,
                height: height - 2.0 * LIST_PAD,
            },
            row_h: ROW_H,
            radius: CONTROL_RADIUS,
            highlight: picker.highlight,
        }));
        out.y += height;
        Ok(())
    }

    /// How wide a list of `codes` has to be for each one's row to show
    /// whole, ticked, with its name in English too, and the scrollbar.
    pub fn list_width(&self, codes: &[&str]) -> Result<f32> {
        let line_w = |text: &str, format| -> Result<f32> {
            Ok(metrics(&self.text(text, format, f32::from(u16::MAX))?.layout).0)
        };
        let format = &self.formats.suggestion;
        let check = line_w(CHECK_ICON, &self.formats.icon)?;
        let mut widest: f32 = 0.0;
        for code in codes {
            let name = translate::name(code);
            let mut w = line_w(&name, format)?;
            if let Some(english) = translate::english_name(code).filter(|e| *e != name) {
                w += 8.0 + line_w(english, format)?;
            }
            widest = widest.max(w);
        }
        // As [`Renderer::layout_rows`] lays a row out, and a little over,
        // as the ticked one's name is bolder.
        let row = SELECT_PAD + BADGE_W + 10.0 + widest + 8.0 + check + 8.0 + 6.0 + SELECT_PAD;
        Ok((row + 2.0 * LIST_PAD + 4.0).ceil())
    }

    /// Every language `picker` lists, a row each, `width` wide, `current`
    /// ticked, and clear of the scrollbar when they don't all fit; or a
    /// row saying none match.
    fn layout_rows(&self, picker: &Picker, current: &str, width: f32) -> Result<Layout> {
        let mut out = Stack::default();
        let format = &self.formats.suggestion;
        let line_h = metrics(&self.text("Ag", format, width)?.layout).1;
        let line_y = |y: f32| y + ((ROW_H - line_h) / 2.0).round();
        if picker.matches.is_empty() {
            let line = self.text(tr().no_languages_match, format, width)?;
            out.items.push(Item::Text {
                layout: line,
                x: SELECT_PAD,
                y: line_y(0.0),
                ink: Ink::Tertiary,
                accents: Vec::new(),
                selectable: false,
            });
        }
        let scrolls = picker.matches.len() > picker.shown();
        let right = width - SELECT_PAD - if scrolls { 6.0 } else { 0.0 };
        for (i, code) in picker.matches.iter().enumerate() {
            let y = i as f32 * ROW_H;
            let mut x = SELECT_PAD;
            self.badge(&mut out, code, x, line_y(y), line_h)?;
            x += BADGE_W + 10.0;
            let name = translate::name(code);
            let line = self.text(&name, format, width)?;
            if *code == current {
                let all = DWRITE_TEXT_RANGE {
                    startPosition: 0,
                    length: u32::MAX,
                };
                unsafe {
                    let _ = line.SetFontWeight(DWRITE_FONT_WEIGHT_SEMI_BOLD, all);
                }
            }
            let (w, _, _) = metrics(&line);
            out.items.push(Item::Text {
                layout: line,
                x,
                y: line_y(y),
                ink: Ink::Primary,
                accents: Vec::new(),
                selectable: false,
            });
            x += w + 8.0;
            let mut end = right;
            if *code == current {
                let check = self.text(CHECK_ICON, &self.formats.icon, width)?;
                let (check_w, check_h, _) = metrics(&check);
                end -= check_w;
                out.items.push(Item::Text {
                    layout: check,
                    x: end,
                    y: y + ((ROW_H - check_h) / 2.0).round(),
                    ink: Ink::Accent,
                    accents: Vec::new(),
                    selectable: false,
                });
                end -= 8.0;
            }
            // Its name in English too, where that's another.
            if let Some(english) = translate::english_name(code).filter(|e| *e != name) {
                if end - x > 30.0 {
                    let line = self.text(english, format, end - x)?;
                    out.items.push(Item::Text {
                        layout: line,
                        x,
                        y: line_y(y),
                        ink: Ink::Tertiary,
                        accents: Vec::new(),
                        selectable: false,
                    });
                }
            }
            out.links.push(Link {
                x: 0.0,
                y,
                width,
                height: ROW_H,
                target: Target::Pick(LangCode::new(*code)),
            });
        }
        Ok(Layout {
            items: out.items,
            width,
            height: picker.matches.len().max(1) as f32 * ROW_H,
            links: out.links,
            caret: None,
            list: None,
        })
    }

    /// A list of languages to tick, `width` wide, `shown` rows of it at
    /// once and the rest scrolling, the fill behind `highlight`'s row.
    /// Nothing's drawn here: the outline and the rows are its list's (see
    /// [`ListLayout`]).
    pub fn layout_ticks(
        &self,
        ticks: &[Tick],
        highlight: Option<usize>,
        shown: usize,
        width: f32,
    ) -> Result<Layout> {
        let shown = shown.clamp(1, ticks.len().max(1));
        let height = 2.0 * LIST_PAD + shown as f32 * TICK_ROW_H;
        let row_w = width - 2.0 * LIST_PAD;
        let list = ListLayout {
            panel: list_panel(width, height),
            rows: self.layout_tick_rows(ticks, ticks.len() > shown, row_w)?,
            frame: Mark {
                x: 0.0,
                y: 0.0,
                width,
                height,
            },
            viewport: Mark {
                x: LIST_PAD,
                y: LIST_PAD,
                width: row_w,
                height: height - 2.0 * LIST_PAD,
            },
            row_h: TICK_ROW_H,
            radius: CONTROL_RADIUS,
            highlight,
        };
        Ok(Layout {
            items: Vec::new(),
            width,
            height,
            links: Vec::new(),
            caret: None,
            list: Some(Box::new(list)),
        })
    }

    /// Every language `ticks` lists, a row each, `width` wide: its box,
    /// its flag, its name in itself and in English, and what's said at its
    /// right, clear of the scrollbar when it `scrolls`.
    fn layout_tick_rows(&self, ticks: &[Tick], scrolls: bool, width: f32) -> Result<Layout> {
        let mut out = Stack::default();
        let format = &self.formats.suggestion;
        let line_h = metrics(&self.text("Ag", format, width)?.layout).1;
        let right = width - SELECT_PAD - if scrolls { 6.0 } else { 0.0 };
        for (i, tick) in ticks.iter().enumerate() {
            let y = i as f32 * TICK_ROW_H;
            let line_y = y + ((TICK_ROW_H - line_h) / 2.0).round();
            let box_y = y + ((TICK_ROW_H - TICK_BOX) / 2.0).round();
            let mut x = SELECT_PAD;
            // Ticked, it's filled with the accent, or dimmed when it has to be.
            let fill = match (tick.on, tick.fixed) {
                (true, true) => Paint::Disabled,
                (true, false) => Paint::Accent,
                (false, _) => Paint::Sunken,
            };
            out.items.push(Item::Shape {
                x,
                y: box_y,
                width: TICK_BOX,
                height: TICK_BOX,
                radius: TICK_RADIUS,
                fill: Some(fill),
                stroke: (!tick.on).then_some(Paint::StrongStroke),
            });
            if tick.on {
                let check = self.text(CHECK_ICON, &self.formats.icon, width)?;
                let (check_w, check_h, _) = metrics(&check);
                out.items.push(Item::Text {
                    layout: check,
                    x: x + ((TICK_BOX - check_w) / 2.0).round(),
                    y: box_y + ((TICK_BOX - check_h) / 2.0).round(),
                    ink: Ink::OnAccent,
                    accents: Vec::new(),
                    selectable: false,
                });
            }
            x += TICK_BOX + 12.0;
            x += self.badge(&mut out, tick.code, x, line_y, line_h)? + 10.0;

            let mut end = right;
            if !tick.aside.is_empty() {
                let aside = self.text(tick.aside, &self.formats.footer, width)?;
                let (aside_w, aside_h, _) = metrics(&aside);
                end -= aside_w;
                out.items.push(Item::Text {
                    layout: aside,
                    x: end,
                    y: y + ((TICK_ROW_H - aside_h) / 2.0).round(),
                    ink: Ink::Secondary,
                    accents: Vec::new(),
                    selectable: false,
                });
                end -= 8.0;
            }
            let ink = if tick.fixed {
                Ink::Secondary
            } else {
                Ink::Primary
            };
            let name = translate::name(tick.code);
            let line = self.text(&name, format, (end - x).max(1.0))?;
            let (w, _, _) = metrics(&line);
            out.items.push(Item::Text {
                layout: line,
                x,
                y: line_y,
                ink,
                accents: Vec::new(),
                selectable: false,
            });
            x += w + 8.0;
            // Its name in English too, where that's another.
            if let Some(english) = translate::english_name(tick.code).filter(|e| *e != name) {
                if end - x > 30.0 {
                    let line = self.text(english, format, end - x)?;
                    out.items.push(Item::Text {
                        layout: line,
                        x,
                        y: line_y,
                        ink: Ink::Tertiary,
                        accents: Vec::new(),
                        selectable: false,
                    });
                }
            }
            out.links.push(Link {
                x: 0.0,
                y,
                width,
                height: TICK_ROW_H,
                target: Target::Pick(LangCode::new(tick.code)),
            });
        }
        Ok(Layout {
            items: out.items,
            width,
            height: ticks.len().max(1) as f32 * TICK_ROW_H,
            links: out.links,
            caret: None,
            list: None,
        })
    }

    /// The translation, the definition it was asked with in its language,
    /// the other translations by part of speech, and where it came from.
    pub(super) fn layout_translated(
        &self,
        out: &mut Stack,
        card: &TranslationCard,
        translation: &Translation,
        width: f32,
    ) -> Result<()> {
        // A passage reads at the size of the body: as a heading it'd fill the card.
        let format = if translate::is_passage(&translation.text) {
            &self.formats.body
        } else {
            &self.formats.heading
        };
        let main = self.text(&capitalize(&translation.text), format, width)?;
        let (_, h) = out.text(main, 0.0, Ink::Primary);
        out.y += h;
        if let Some(context) = &translation.context {
            out.y += 4.0;
            let quoted = format!("\u{201C}{}\u{201D}", capitalize(context));
            let line = self.text(&quoted, &self.formats.example, width)?;
            let (_, h) = out.text(line, 0.0, Ink::Secondary);
            out.y += h;
        }
        for group in &translation.groups {
            out.y += 10.0;
            out.items.push(Item::Rule {
                y: out.y,
                x: 0.0,
                width,
            });
            out.y += 14.0;
            let label = match group.part_of_speech.as_str() {
                "" => tr().also,
                pos => tr().part_of_speech(pos),
            };
            let (_, h) = out.text(self.spaced_caps(label, width)?, 0.0, Ink::Accent);
            out.y += h + 6.0;
            for term in &group.terms {
                let text = self.text(&capitalize(&term.text), &self.formats.body, width)?;
                let (w, h, baseline) = metrics(&text);
                out.text(text, 0.0, Ink::Primary);
                if term.back.is_empty() {
                    out.y += h + 3.0;
                    continue;
                }
                // What it means back, beside it when there's room, else under it.
                let gap = 10.0;
                let beside = width - w - gap >= 140.0;
                let (x, room) = if beside {
                    (w + gap, width - w - gap)
                } else {
                    out.y += h;
                    (0.0, width)
                };
                let back =
                    self.text(&capitalize(&term.back.join(", ")), &self.formats.meta, room)?;
                let (_, back_h, back_baseline) = metrics(&back);
                let y = if beside {
                    out.y + baseline - back_baseline
                } else {
                    out.y
                };
                out.items.push(Item::Text {
                    layout: back,
                    x,
                    y,
                    ink: Ink::Tertiary,
                    accents: Vec::new(),
                    selectable: true,
                });
                out.y = if beside {
                    (out.y + h).max(y + back_h)
                } else {
                    out.y + back_h
                };
                out.y += 3.0;
            }
        }
        out.y += 14.0;
        let source = translation.source;
        if source == translate::GOOGLE {
            let url = format!(
                "https://translate.google.com/?sl={}&tl={}&text={}&op=translate",
                card.from,
                card.to,
                percent_encode(card.word)
            );
            let (w, h) = self.external_link(out, source, 0.0, width)?;
            out.links.push(Link {
                x: -4.0,
                y: out.y - 4.0,
                width: w + 8.0,
                height: h + 8.0,
                target: Target::Url(url),
            });
            out.y += h;
        } else {
            let label = self.text(source, &self.formats.footer, width)?;
            let (_, h) = out.text(label, 0.0, Ink::Accent);
            out.y += h;
        }
        Ok(())
    }

    /// The footer: the sources, each linking to the word's page there.
    pub(super) fn layout_sources(
        &self,
        out: &mut Stack,
        def: &Definition,
        width: f32,
    ) -> Result<()> {
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
}

/// A list's outline, `width` by `height`, from its top left.
fn list_panel(width: f32, height: f32) -> Layout {
    Layout {
        items: vec![Item::Shape {
            x: 0.0,
            y: 0.0,
            width,
            height,
            radius: LIST_RADIUS,
            fill: Some(Paint::List),
            stroke: Some(Paint::ControlStroke),
        }],
        width,
        height,
        links: Vec::new(),
        caret: None,
        list: None,
    }
}

/// Takes the text at `i` out of `items`, to draw apart: it, from its own top
/// left with `room` to spare at its right, and where it was.
fn take(items: &mut Vec<Item>, i: usize, room: f32) -> Placed {
    let mut item = items.remove(i);
    let (mut x, mut y, mut size) = (0.0, 0.0, (0.0, 0.0));
    if let Item::Text {
        layout,
        x: at_x,
        y: at_y,
        ..
    } = &mut item
    {
        let (w, h, _) = metrics(layout);
        (x, y, size) = (*at_x, *at_y, (w, h));
        (*at_x, *at_y) = (0.0, 0.0);
    }
    Placed {
        layout: Layout {
            items: vec![item],
            width: size.0 + room,
            height: size.1,
            links: Vec::new(),
            caret: None,
            list: None,
        },
        x,
        y,
    }
}
