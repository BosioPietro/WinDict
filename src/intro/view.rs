//! Building the introduction's visuals: the picture, what a step says and
//! what can be done there, the dots and the buttons under them, laid out
//! for the current scale, theme and language.

use super::*;

impl Intro {
    /// Builds the view afresh for the current scale, theme and language.
    /// With `crossfade`, or in another language, the old view fades out as
    /// the new one fades in; otherwise it goes at once.
    pub(super) fn replace_view(&mut self, crossfade: bool) -> Result<()> {
        let language = crate::i18n::current();
        let relanguage = self.view.is_some() && language != self.built_in;
        self.built_in = language;
        let crossfade = crossfade || relanguage;
        let view = self.build()?;
        let anim = &self.anim;
        if let Some(old) = self.view.replace(view) {
            if crossfade {
                anim.scalar(&old.content, "Opacity", 0.0, THEME_MS, 0, &anim.fade)?;
                self.gone.push(old.content.cast()?);
                win::set_timer(self.hwnd, TIMER_GONE, GONE_MS);
            } else {
                self.release_list()?;
                self.frame.card.Children()?.Remove(&old.content)?;
            }
        }
        if let Some(view) = &self.view {
            if crossfade {
                view.content.SetOpacity(0.0)?;
                anim.scalar(&view.content, "Opacity", 1.0, THEME_MS, 0, &anim.fade)?;
            }
            self.show_dots(&view.dots, false)?;
        }
        self.keep_focus();
        self.show_switch(false)?;
        self.show_note()?;
        self.show_picker()?;
        self.show_hover(self.hovered)?;
        self.show_focus()
    }

    fn build(&self) -> Result<View> {
        let (c, r, t, s) = (
            &self.anim.compositor,
            &self.renderer,
            &self.theme,
            self.scale,
        );
        let px = |dip: f32| (dip * s).round();
        let content = c.CreateContainerVisual()?;
        self.frame.card.Children()?.InsertAtTop(&content)?;

        // The picture, its corners rounded, with a hairline round it.
        let at = Rect {
            x: px(INSET),
            y: px(INSET),
            w: px(PANE_W),
            h: px(PANE_H),
        };
        let pane = picture_card::picture_pane(c, r, t, s, &content, at)?;
        let picture = self.build_picture(&pane)?;
        let close = picture_card::close_button(c, r, t, s, &content, CARD_W)?;

        // The step goes over the dots and the buttons, for a list open over
        // them.
        let dots = self.build_dots(&content)?;
        let (footer, buttons) = self.build_footer(&content)?;
        let step = self.build_step(&content)?;
        Ok(View {
            content,
            pane,
            picture,
            step,
            close,
            dots,
            footer,
            buttons,
        })
    }

    /// The step's picture, filling the `pane`, on top of what's there.
    pub(super) fn build_picture(&self, pane: &ContainerVisual) -> Result<SpriteVisual> {
        let (c, r, s) = (&self.anim.compositor, &self.renderer, self.scale);
        let size = v2((PANE_W * s).round(), (PANE_H * s).round());
        let step = self.steps[self.at];
        let surface = match step.scene() {
            Some(shot) => {
                scene::draw(r, shot, crate::i18n::current(), &self.theme, size.X, size.Y)?
            }
            None => r.draw_picture(step.picture(self.theme.dark), size.X, size.Y)?,
        };
        let picture = c.CreateSpriteVisual()?;
        picture.SetBrush(&c.CreateSurfaceBrushWithSurface(&surface)?)?;
        picture.SetSize(size)?;
        picture.SetCenterPoint(Vector3 {
            X: size.X / 2.0,
            Y: size.Y / 2.0,
            Z: 0.0,
        })?;
        pane.Children()?.InsertAtTop(&picture)?;
        Ok(picture)
    }

    /// The step's symbol, its title, what it says and what can be done
    /// there, each a visual of its own in the text returned, in `content`.
    pub(super) fn build_step(&self, content: &ContainerVisual) -> Result<StepView> {
        let (c, r, t, s) = (
            &self.anim.compositor,
            &self.renderer,
            &self.theme,
            self.scale,
        );
        let px = |dip: f32| (dip * s).round();
        let step = self.steps[self.at];
        let text = container(c, content, v3(0.0, 0.0))?;
        let x = px(TEXT_X);

        let (title, glyph) = (step.title(), step.glyph());
        let mut y = picture_card::heading(c, r, t, s, &text, TEXT_X, TEXT_W, title, glyph, false)?;

        let detail = Text::new(c, &text)?;
        let (said, keys) = step.detail(&self.shortcuts);
        let paragraph = r.layout_paragraph(said, &keys, Ink::Secondary, TEXT_W)?;
        let size = detail.show(r, &paragraph, s, t)?;
        detail.visual.SetOffset(v3(x, y))?;
        let picks = matches!(step, Step::Languages | Step::Dictionaries);
        y += size.Y + px(if picks { PICKER_GAP } else { ACTIONS_GAP });

        let mut view = StepView {
            text,
            controls: Vec::new(),
            switch: None,
            note: None,
            picker: None,
        };
        match step {
            // A tile for each language, in rows across.
            Step::Languages => {
                let w =
                    ((TEXT_W - (TILE_COLUMNS - 1) as f32 * TILE_GAP) / TILE_COLUMNS as f32).floor();
                for (i, language) in Self::tiles().into_iter().enumerate() {
                    let (row, column) = (i / TILE_COLUMNS, i % TILE_COLUMNS);
                    let on = language == self.options.interface;
                    let layout = r.layout_tile(language.code(), on, w, TILE_H)?;
                    let tile = Text::new(c, &view.text)?;
                    tile.show(r, &layout, s, t)?;
                    let at = Rect {
                        x: x + px(column as f32 * (w + TILE_GAP)),
                        y: y + px(row as f32 * (TILE_H + TILE_GAP)),
                        w: px(w),
                        h: px(TILE_H),
                    };
                    tile.visual.SetOffset(v3(at.x, at.y))?;
                    let button = self.build_stop(&view.text, at, TILE_RADIUS)?;
                    view.controls.push((Hit::Tile(language), button));
                }
            }
            // The dictionaries, a list as long as leaves room for a note,
            // the rest of it scrolling.
            Step::Dictionaries => {
                let top = y / s;
                let rows = Tick::rows_in(CARD_H - PAD_BOTTOM - BUTTON_H - NOTE_ROOM - top);
                let layout = self.layout_picker(rows)?;
                let picker = Text::new(c, &view.text)?;
                let at = v3(x, y);
                picker.visual.SetOffset(at)?;
                let morph = Morph::new(c, &picker, false)?;
                morph.show(&self.anim, r, &picker, &layout, None, s, t)?;
                let face = Rect {
                    x,
                    y,
                    w: px(TEXT_W),
                    h: (layout.height * s).round(),
                };
                let button = self.build_stop(&view.text, face, LIST_RADIUS)?;
                view.controls.push((Hit::List, button));
                let note = Text::new(c, &view.text)?;
                note.visual.SetOffset(v3(x, y + face.h + px(NOTE_GAP)))?;
                view.note = Some(note);
                view.picker = Some(PickerView {
                    text: picker,
                    morph,
                    at,
                    rows,
                });
            }
            Step::Lookup | Step::Translate => {
                let taken = step == Step::Lookup && self.options.shortcut_taken;
                if taken {
                    let warning = Text::new(c, &view.text)?;
                    let line = r.layout_line(
                        tr().intro_shortcut_taken,
                        Face::Menu,
                        Ink::Critical,
                        TEXT_W,
                    )?;
                    let size = warning.show(r, &line, s, t)?;
                    warning.visual.SetOffset(v3(x, y))?;
                    y += size.Y + px(WARNING_GAP);
                }
                let mut offers = Vec::new();
                if taken {
                    offers.push((Hit::ChooseShortcut, tr().intro_choose_shortcut));
                }
                // Translations need the internet.
                if step == Step::Lookup || self.options.online {
                    offers.push((Hit::ShowMe, tr().intro_show_me));
                }
                let mut left = x;
                for (hit, label) in offers {
                    let w = button_width(r, label)?;
                    let at = Rect {
                        x: left,
                        y,
                        w: px(w),
                        h: px(BUTTON_H),
                    };
                    left += at.w + px(BUTTON_GAP);
                    let button = self.build_button(&view.text, at, label, false)?;
                    view.controls.push((hit, button));
                }
            }
            Step::AboveStart | Step::Ready => {
                let label = if step == Step::AboveStart {
                    tr().intro_ui_access
                } else {
                    tr().start_with_windows
                };
                let row = Rect {
                    x,
                    y,
                    w: px(TEXT_W),
                    h: px(ROW_H),
                };
                let (button, switch) = self.build_switch_row(&view.text, row, label)?;
                view.controls.push((Hit::Switch, button));
                view.switch = Some(switch);
                if step == Step::AboveStart {
                    let note = Text::new(c, &view.text)?;
                    note.visual.SetOffset(v3(x, y + row.h + px(NOTE_GAP)))?;
                    view.note = Some(note);
                }
            }
        }
        Ok(view)
    }

    /// A row like a setting's at `at` in `parent`, saying `label`, with a
    /// switch at its right. Returns the row, which is what's clicked, and
    /// the switch.
    fn build_switch_row(
        &self,
        parent: &ContainerVisual,
        at: Rect,
        label: &str,
    ) -> Result<(Button, SwitchView)> {
        let (c, r, t, s) = (
            &self.anim.compositor,
            &self.renderer,
            &self.theme,
            self.scale,
        );
        let px = |dip: f32| (dip * s).round();
        let root = container(c, parent, at.at())?;
        let face = Rect {
            w: at.w,
            h: at.h,
            ..Rect::default()
        };
        let radius = px(ROW_RADIUS);
        rounded(c, r, &root, face, radius, t.group())?;
        let (hover, _) = rounded(c, r, &root, face, radius, t.highlight())?;
        hover.SetOpacity(0.0)?;
        outline(c, r, &root, face, radius, t.group_stroke())?;

        let (w, h) = (px(SWITCH_W), px(SWITCH_H));
        let track_at = v3(at.w - px(ROW_PAD) - w, ((at.h - h) / 2.0).round());
        let switch = container(c, &root, track_at)?;
        let track = Rect {
            w,
            h,
            ..Rect::default()
        };
        let off = container(c, &switch, v3(0.0, 0.0))?;
        rounded(c, r, &off, track, h / 2.0, t.sunken())?;
        outline(c, r, &off, track, h / 2.0, t.strong_stroke())?;
        let (on, _) = rounded(c, r, &switch, track, h / 2.0, t.accent)?;
        let knob = px(KNOB);
        let dot = Rect {
            w: knob,
            h: knob,
            ..Rect::default()
        };
        let (knob_visual, knob_brush) = rounded(c, r, &switch, dot, knob / 2.0, t.knob(false))?;
        knob_visual.SetCenterPoint(Vector3 {
            X: knob / 2.0,
            Y: knob / 2.0,
            Z: 0.0,
        })?;
        let inset = ((h - knob) / 2.0).round();

        let text = Text::new(c, &root)?;
        let room = (TEXT_W - SWITCH_W - 3.0 * ROW_PAD).max(1.0);
        let line = r.layout_line(label, Face::Setting, Ink::Primary, room)?;
        let size = text.show(r, &line, s, t)?;
        text.visual
            .SetOffset(v3(px(ROW_PAD), ((at.h - size.Y) / 2.0).round()))?;

        let outset = px(FOCUS_OUTSET);
        let ring = ring(
            c,
            r,
            &root,
            face.outset(outset),
            radius + outset,
            ui::FOCUS_STROKE * s,
            t.focus(),
        )?;
        ring.SetOpacity(0.0)?;
        let row = Button {
            root,
            hover,
            ring,
            at,
        };
        let switch = SwitchView {
            root: switch,
            on,
            off,
            knob: knob_visual,
            knob_brush,
            rest: [v3(inset, inset), v3(w - inset - knob, inset)],
        };
        Ok((row, switch))
    }

    /// What has the keyboard at `at` in `parent`, drawn by the renderer
    /// with corners of `radius` DIPs: the ring round it then, and the fill
    /// that lights it under the pointer.
    fn build_stop(&self, parent: &ContainerVisual, at: Rect, radius: f32) -> Result<Button> {
        let (c, r, t, s) = (
            &self.anim.compositor,
            &self.renderer,
            &self.theme,
            self.scale,
        );
        let px = |dip: f32| (dip * s).round();
        let root = container(c, parent, at.at())?;
        let face = Rect {
            w: at.w,
            h: at.h,
            ..Rect::default()
        };
        let radius = px(radius);
        let (hover, _) = rounded(c, r, &root, face, radius, t.highlight())?;
        hover.SetOpacity(0.0)?;
        let outset = px(FOCUS_OUTSET);
        let ring = ring(
            c,
            r,
            &root,
            face.outset(outset),
            radius + outset,
            ui::FOCUS_STROKE * s,
            t.focus(),
        )?;
        ring.SetOpacity(0.0)?;
        Ok(Button {
            root,
            hover,
            ring,
            at,
        })
    }

    /// A dot for each step, along the bottom of `content`, level with the
    /// buttons.
    fn build_dots(&self, content: &ContainerVisual) -> Result<Vec<DotView>> {
        let (c, s) = (&self.anim.compositor, self.scale);
        let px = |dip: f32| (dip * s).round();
        let middle = px(CARD_H - PAD_BOTTOM - BUTTON_H / 2.0);
        let n = self.steps.len() as f32;
        let w = px(DOT_ON + (n - 1.0) * (DOT + DOT_GAP));
        let visual = c.CreateShapeVisual()?;
        visual.SetOffset(v3(px(TEXT_X), (middle - px(DOT) / 2.0).round()))?;
        visual.SetSize(v2(w, px(DOT)))?;
        let mut dots = Vec::new();
        for _ in &self.steps {
            let shape = c.CreateRoundedRectangleGeometry()?;
            shape.SetCornerRadius(v2(DOT * s / 2.0, DOT * s / 2.0))?;
            let brush = c.CreateColorBrush()?;
            let sprite = c.CreateSpriteShapeWithGeometry(&shape)?;
            sprite.SetFillBrush(&brush)?;
            visual.Shapes()?.Append(&sprite)?;
            dots.push(DotView { shape, brush });
        }
        content.Children()?.InsertAtTop(&visual)?;
        Ok(dots)
    }

    /// The buttons that go through the steps, at the bottom right of
    /// `content`. Returns what holds them, and the buttons.
    pub(super) fn build_footer(
        &self,
        content: &ContainerVisual,
    ) -> Result<(ContainerVisual, Vec<(Hit, Button)>)> {
        let s = self.scale;
        let px = |dip: f32| (dip * s).round();
        let footer = container(&self.anim.compositor, content, v3(0.0, 0.0))?;
        let last = self.at + 1 == self.steps.len();
        let h = px(BUTTON_H);
        let y = px(CARD_H - PAD_BOTTOM) - h;
        let mut right = px(CARD_W - PAD_X);
        let next = if last {
            tr().intro_done
        } else if self.to_download() {
            tr().intro_download
        } else {
            tr().intro_next
        };
        let specs = [(Hit::Next, next, true), (Hit::Back, tr().intro_back, false)];
        let mut buttons = Vec::new();
        for (hit, label, primary) in specs {
            if hit == Hit::Back && self.at == 0 {
                continue;
            }
            // Download may say more than fits the others' width.
            let w = px(button_width(&self.renderer, label)?);
            let at = Rect {
                x: right - w,
                y,
                w,
                h,
            };
            right = at.x - px(BUTTON_GAP);
            buttons.push((hit, self.build_button(&footer, at, label, primary)?));
        }
        Ok((footer, buttons))
    }

    /// A button saying `label` at `at` in `content`: in the accent when
    /// it's the `primary` one.
    fn build_button(
        &self,
        content: &ContainerVisual,
        at: Rect,
        label: &str,
        primary: bool,
    ) -> Result<Button> {
        let (c, r, t, s) = (
            &self.anim.compositor,
            &self.renderer,
            &self.theme,
            self.scale,
        );
        ui::button(c, r, t, s, content, at, label, primary)
    }
}
