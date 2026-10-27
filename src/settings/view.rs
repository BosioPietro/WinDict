//! Building the settings window's visuals: the header, the groups of
//! settings with their controls, and the footer, laid out for the current
//! scale and theme. Where things can be clicked or focused is worked out
//! here too, as they're placed.

use windows::core::{Interface, Result};
use windows::Win32::Foundation::{E_UNEXPECTED, RECT};
use windows::UI::Composition::*;
use windows_numerics::Vector3;

use crate::i18n;
use crate::ui::RESIZE_MS;

use super::widgets::{container, outline, resizable_outline, rounded};
use super::*;

impl Settings {
    /// Builds the view afresh for the current scale and theme, showing the
    /// current state at rest. With `crossfade`, the old view fades out as
    /// the new one fades in; otherwise it goes at once.
    pub(super) fn replace_view(&mut self, crossfade: bool) -> Result<()> {
        let children = self.frame.card.Children()?;
        if let Some(old) = self.fading.take() {
            self.release_list()?;
            children.Remove(&old)?;
        }
        let view = self.build()?;
        if let Some(old) = self.view.replace(view) {
            if crossfade {
                let anim = &self.anim;
                anim.scalar(&old.content, "Opacity", 0.0, THEME_MS, 0, &anim.fade)?;
                self.fading = Some(old.content);
            } else {
                self.release_list()?;
                children.Remove(&old.content)?;
            }
        }
        self.restore()?;
        if crossfade {
            if let Some(view) = &self.view {
                let anim = &self.anim;
                view.content.SetOpacity(0.0)?;
                anim.scalar(&view.content, "Opacity", 1.0, THEME_MS, 0, &anim.fade)?;
            }
        }
        Ok(())
    }

    /// Builds the view afresh for what the default languages show to have
    /// changed. The parts that moved glide from where they were as the card
    /// resizes, and the old view fades away over the new one, so that what
    /// stayed the same stays put.
    pub(super) fn slide_view(&mut self) -> Result<()> {
        let children = self.frame.card.Children()?;
        if let Some(old) = self.fading.take() {
            self.release_list()?;
            children.Remove(&old)?;
        }
        let view = self.build()?;
        let old = self.view.replace(view);
        self.restore()?;
        let (Some(old), Some(view)) = (old, &self.view) else {
            return Ok(());
        };
        let anim = &self.anim;
        for ((part, rest), (_, was)) in view.reveal.iter().zip(&old.reveal) {
            if (was.X, was.Y) != (rest.X, rest.Y) {
                Animator::stop(part, &["Offset"]);
                part.SetOffset(*was)?;
                anim.vector3(part, "Offset", *rest, EXPAND_MS, 0, &anim.decelerate)?;
            }
        }
        children.Remove(&old.content)?;
        children.InsertAtTop(&old.content)?;
        anim.scalar(
            &old.content,
            "Opacity",
            0.0,
            SLIDE_FADE_MS,
            0,
            &anim.decelerate,
        )?;
        self.fading = Some(old.content);
        Ok(())
    }

    /// Shows the default languages' boxes afresh in the view as it is, when
    /// they keep their height: the rest of the window isn't touched, so it
    /// can't flicker. Returns whether it could.
    pub(super) fn refresh_boxes(&mut self) -> Result<bool> {
        let (layout, slack) = self.layout_languages()?;
        let (s, c) = (self.scale, &self.anim.compositor);
        let Some(view) = &mut self.view else {
            return Ok(false);
        };
        let boxes = &view.boxes;
        if layout.caret.is_some() != boxes.caret.is_some() {
            return Ok(false);
        }
        boxes.text.show(&self.renderer, &layout, s, &self.theme)?;
        if let (Some(m), Some(caret)) = (layout.caret, &boxes.caret) {
            caret.SetOffset(v3((m.x * s).round(), (m.y * s).round()))?;
            caret.SetSize(v2((m.width * s).round().max(1.0), (m.height * s).round()))?;
            ui::blink(c, caret)?;
        }
        let moved = view.slack != slack;
        view.slack = slack;
        if moved {
            self.show_slack(true)?;
        }
        Ok(true)
    }

    /// Shows the box of the language WinDict is shown in afresh, in the
    /// view as it is: it keeps its height, its list covering the rows
    /// under it.
    pub(super) fn refresh_interface(&self) -> Result<()> {
        let (layout, moving) = self.layout_interface()?;
        let Some(view) = &self.view else {
            return Ok(());
        };
        let v = &view.interface;
        let (r, s, t) = (&self.renderer, self.scale, &self.theme);
        (v.morph).show(&self.anim, r, &v.text, &layout, Some(&moving), s, t)?;
        self.show_caret(&v.caret, &layout)
    }

    /// Puts `caret` where `layout` has it, blinking, or hides it.
    fn show_caret(&self, caret: &SpriteVisual, layout: &Layout) -> Result<()> {
        let s = self.scale;
        let Some(m) = layout.caret else {
            Animator::stop(caret, &["Opacity"]);
            return caret.SetOpacity(0.0);
        };
        caret.SetOffset(v3((m.x * s).round(), (m.y * s).round()))?;
        caret.SetSize(v2((m.width * s).round().max(1.0), (m.height * s).round()))?;
        ui::blink(&self.anim.compositor, caret)
    }

    /// Takes the list out of the views on their way out, before they're let
    /// go of, keeping it only in the current view's boxes.
    fn release_list(&self) -> Result<()> {
        let keep = match &self.view {
            Some(view) => vec![
                view.interface.text.visual.cast()?,
                view.boxes.text.visual.cast()?,
            ],
            None => Vec::new(),
        };
        self.list.detach_unless(&keep)
    }

    /// Shows the list of languages under the box being picked in, or takes
    /// it away, fading as the view it was in does.
    pub(super) fn show_list(&mut self) -> Result<()> {
        let open = self.picking.as_ref().map(|p| p.on);
        let (layout, parent) = match (open, &self.view) {
            (Some(Select::Interface), Some(view)) => (
                self.layout_interface()?.0,
                view.interface.text.visual.cast()?,
            ),
            (Some(Select::Translate(_)), Some(view)) => {
                (self.layout_languages()?.0, view.boxes.text.visual.cast()?)
            }
            _ => return self.list.hide(&self.anim, SLIDE_FADE_MS),
        };
        let (anim, r, t, s) = (&self.anim, &self.renderer, &self.theme, self.scale);
        match layout.list.as_deref() {
            Some(list) => {
                self.list.attach(&parent)?;
                self.list.show(anim, r, t, list, s)
            }
            None => self.list.hide(anim, SLIDE_FADE_MS),
        }
    }

    /// Shows the rows after the languages as far up as the list of matches
    /// being short leaves them.
    pub(super) fn show_slack(&self, animate: bool) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let (spare, slack) = (&view.spare, view.slack);
        let shift = v3(0.0, -slack);
        let size = v2(spare.size.X, spare.size.Y - slack);
        let edge = v2(size.X - 1.0, size.Y - 1.0);
        let anim = &self.anim;
        if animate {
            for part in [&spare.tail, &spare.below] {
                anim.vector3(part, "Offset", shift, RESIZE_MS, 0, &anim.decelerate)?;
            }
            anim.vector2(&spare.bg, "Size", size, RESIZE_MS)?;
            anim.vector2(&spare.clip, "Size", size, RESIZE_MS)?;
            anim.vector2(&spare.border, "Size", edge, RESIZE_MS)?;
        } else {
            for part in [&spare.tail, &spare.below] {
                Animator::stop(part, &["Offset"]);
                part.SetOffset(shift)?;
            }
            Animator::stop(&spare.bg, &["Size"]);
            spare.bg.SetSize(size)?;
            Animator::stop(&spare.clip, &["Size"]);
            spare.clip.SetSize(size)?;
            Animator::stop(&spare.border, &["Size"]);
            spare.border.SetSize(edge)?;
        }
        Ok(())
    }

    /// Sizes the card to its content, as far as the `work` area allows.
    pub(super) fn fit(&mut self, work: RECT) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let s = self.scale;
        let room = ((work.bottom - work.top) as f32 - 2.0 * (SCREEN_GAP * s).round())
            .min((MAX_CARD_H * s).round());
        let w = (CARD_W * s).round();
        let h = (view.header_h + view.height(&self.open)).min(room).ceil();
        self.card = (w, h);
        self.room = room;
        let m = (MARGIN * s).round();
        self.frame.resize(&self.anim, w, h, v3(m, m), s, false)?;
        view.body.SetSize(v2(w, h - view.header_h))?;
        self.scroller.at = self.scroller.at.min(self.max_scroll());
        if let Some(view) = &self.view {
            Animator::stop(&view.scroller, &["Offset"]);
            view.scroller.SetOffset(v3(0.0, -self.scroller.at))?;
        }
        self.show_scroll(false)
    }

    /// Shows the state on a freshly built view, everything at rest.
    fn restore(&mut self) -> Result<()> {
        for sw in SWITCHES {
            self.show_switch(sw, false)?;
            self.show_knob(sw)?;
        }
        let from = self.pill_from.take();
        for choice in CHOICES {
            self.show_choice(choice, false)?;
        }
        // The theme changed with a click on the pill's way over: it carries on here.
        if let (Some((choice, from)), Some(view)) = (from, &self.view) {
            let v = &view.choices[choice as usize];
            v.pill.SetOffset(v.rest[from])?;
            self.pill_from = Some((choice, from));
            self.show_choice(choice, true)?;
            self.pill_from = None;
        }
        for counter in COUNTERS {
            self.show_count(counter, None)?;
        }
        self.email_shift = 0.0;
        self.show_field()?;
        for shortcut in SHORTCUTS {
            self.show_caps(shortcut, false)?;
        }
        for note in NOTES {
            let (text, error) = self.notes[note as usize].clone();
            self.show_note(note, &text, error, false)?;
        }
        self.show_checks(false)?;
        for i in 0..self.downloads.len() {
            self.show_status(i)?;
        }
        self.show_open(false)?;
        self.show_recording()?;
        self.show_slack(false)?;
        self.show_list()?;
        self.show_hover(self.hovered)?;
        self.ring_shown = false;
        self.show_focus(false)?;
        if let Some(view) = &self.view {
            view.scroller.SetOffset(v3(0.0, -self.scroller.at))?;
        }
        self.show_scroll(false)
    }

    /// Each part rises into place a beat after the one above it.
    pub(super) fn reveal(&self) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let (anim, drift) = (&self.anim, DRIFT * self.scale);
        for (i, (part, rest)) in view.reveal.iter().enumerate() {
            let delay = REVEAL_DELAY_MS + i as u64 * REVEAL_STAGGER_MS;
            Animator::stop(part, &["Opacity", "Offset"]);
            part.SetOpacity(0.0)?;
            part.SetOffset(v3(rest.X, rest.Y + drift))?;
            anim.scalar(part, "Opacity", 1.0, REVEAL_MS, delay, &anim.decelerate)?;
            anim.vector3(part, "Offset", *rest, REVEAL_MS, delay, &anim.decelerate)?;
        }
        Ok(())
    }

    fn build(&self) -> Result<View> {
        let (c, r, t, s) = (
            &self.anim.compositor,
            &self.renderer,
            &self.theme,
            self.scale,
        );
        let px = |dip: f32| (dip * s).round();
        let card_w = px(CARD_W);
        let content = c.CreateContainerVisual()?;
        self.frame.card.Children()?.InsertAtTop(&content)?;
        let mut reveal: Vec<(Visual, Vector3)> = Vec::new();

        let (header_h, close_at, close_hover) = self.build_header(&content, &mut reveal)?;

        // The groups of settings scroll under the header, should they not fit.
        let body = c.CreateContainerVisual()?;
        body.SetClip(&c.CreateInsetClip()?)?;
        body.SetOffset(v3(0.0, header_h))?;
        // As tall as the card allows; `fit` sizes it again when that changes.
        body.SetSize(v2(card_w, (self.card.1 - header_h).max(0.0)))?;
        let fade = ScrollFade::new(c, &content, &body)?;
        fade.set_scale(c, s)?;
        let scroller = container(c, &body, v3(0.0, 0.0))?;

        let (group_x, group_w, row_h) = (px(GROUP_X), card_w - 2.0 * px(GROUP_X), px(ROW_H));
        let (languages_layout, slack) = self.layout_languages()?;
        let translate_h =
            px(TRANSLATE_HEAD) + (languages_layout.height * s).ceil() + slack + px(TRANSLATE_FOOT);
        let missing = || windows::core::Error::from(E_UNEXPECTED);
        let mut parts = Parts {
            // Over the groups, where the box of WinDict's language goes.
            overlay: Some(container(c, &scroller, v3(0.0, 0.0))?),
            ..Parts::default()
        };
        let mut folds: [Option<FoldView>; FOLDS.len()] = Default::default();
        let mut languages = None;
        let mut y = 0.0;
        // What the groups go in: after a fold's list, what slides down as it
        // opens; after the default languages' row, what slides up.
        let mut parent = scroller.clone();
        let mut spare = None;
        let mut slack_y = 0.0;
        for (i, (name, rows)) in SECTIONS.iter().enumerate() {
            let rows: Vec<&Row> = rows.iter().filter(|row| row.shown(&self.values)).collect();
            let name = name(i18n::tr());
            y += if i == 0 { px(2.0) } else { px(SECTION_GAP) };
            let label = Text::new(c, &parent)?;
            let caps = r.layout_label(name, Ink::Accent, 300.0)?;
            let label_size = label.show(r, &caps, s, t)?;
            let at = v3(px(LABEL_X), y);
            label.visual.SetOffset(at)?;
            reveal.push((label.visual.cast()?, at));
            y += label_size.Y + px(LABEL_GAP);

            let group_at = v3(group_x, y);
            let heights: Vec<f32> = rows
                .iter()
                .map(|row| match row.control {
                    Control::Translate => translate_h,
                    Control::Selection => px(SELECTION_H),
                    _ => row_h,
                })
                .collect();
            let group_h: f32 = heights.iter().sum();
            // The group with a row that folds has room for the list under
            // it, which the clip and outline show as it opens.
            let fold = rows.iter().find_map(|row| match row.control {
                Control::Fold(fold) => Some(fold),
                _ => None,
            });
            let list_h = match fold {
                Some(Fold::Shortcuts) => SHORTCUT_ROWS.len() as f32 * row_h,
                Some(Fold::Languages) => dicts::languages().count() as f32 * px(LANGUAGE_H),
                None => 0.0,
            };
            let group = container(c, &parent, group_at)?;
            let bounds = Rect {
                w: group_w,
                h: group_h,
                ..Rect::default()
            };
            let open = Rect {
                h: group_h + list_h,
                ..bounds
            };
            group.SetSize(open.size())?;
            let clip = c.CreateRoundedRectangleGeometry()?;
            clip.SetSize(bounds.size())?;
            clip.SetCornerRadius(v2(px(GROUP_RADIUS), px(GROUP_RADIUS)))?;
            group.SetClip(&c.CreateGeometricClipWithGeometry(&clip)?)?;
            let (bg, _) = rounded(c, r, &group, open, 0.0, t.group())?;
            let mut row_y = 0.0;
            let mut tail: Option<ContainerVisual> = None;
            for (j, spec) in rows.iter().enumerate() {
                let row = Rect {
                    x: 0.0,
                    y: row_y,
                    w: group_w,
                    h: heights[j],
                };
                row_y += heights[j];
                let host = tail.as_ref().unwrap_or(&group);
                self.build_row(&mut parts, host, group_at, spec, row, &languages_layout)?;
                if matches!(spec.control, Control::Translate) {
                    slack_y = group_at.Y + row.y + row.h;
                    tail = Some(container(c, &group, v3(0.0, 0.0))?);
                }
            }
            let list = match fold {
                Some(Fold::Shortcuts) => Some(self.build_shortcuts(
                    &mut parts,
                    &group,
                    group_at,
                    group_h,
                    &languages_layout,
                )?),
                Some(Fold::Languages) => {
                    let (list, items) =
                        self.build_list(&mut parts, &group, group_at, group_h, group_w)?;
                    languages = Some(items);
                    Some(list)
                }
                None => None,
            };
            let (_, border) =
                resizable_outline(c, &group, open, px(GROUP_RADIUS), t.group_stroke())?;
            border.SetSize(v2(bounds.w - 1.0, bounds.h - 1.0))?;
            if let Some(tail) = tail {
                let below = container(c, &parent, v3(0.0, 0.0))?;
                parent = below.clone();
                spare = Some(SpareView {
                    tail,
                    below,
                    bg,
                    clip: clip.clone(),
                    border: border.clone(),
                    size: bounds.size(),
                });
            }
            if let (Some(fold), Some(list)) = (fold, list) {
                let after = container(c, &parent, v3(0.0, 0.0))?;
                parent = after.clone();
                let (hover, chevron) = parts.folds[fold as usize].take().ok_or_else(missing)?;
                folds[fold as usize] = Some(FoldView {
                    hover,
                    chevron,
                    clip,
                    outline: border,
                    group_w,
                    group_h,
                    list,
                    list_y: group_at.Y + group_h,
                    list_h,
                    after,
                });
            }
            reveal.push((group.cast()?, group_at));
            if rows
                .iter()
                .any(|row| matches!(row.control, Control::Interface))
            {
                let v = parts.interface.as_ref().ok_or_else(missing)?;
                reveal.push((v.text.visual.cast()?, v.at));
            }
            y += group_h;
        }

        // The footer: a way to the file behind it all.
        y += px(FOOTER_GAP);
        let footer_at = v3(0.0, y);
        let footer = container(c, &parent, footer_at)?;
        let link = Text::new(c, &footer)?;
        let link_size = link.show(r, &r.layout_link(i18n::tr().config_link)?, s, t)?;
        let link_at = Rect {
            x: px(LABEL_X),
            y: 0.0,
            w: link_size.X,
            h: link_size.Y,
        };
        link.visual.SetOffset(link_at.at())?;
        let link_at = link_at.offset(footer_at);
        parts.hits.push((Hit::Config, link_at.outset(px(4.0))));
        parts
            .stops
            .push((Stop::Config, link_at.outset(px(FOCUS_OUTSET)), px(4.0)));
        reveal.push((footer.cast()?, footer_at));
        y += link_size.Y + px(PAD_BOTTOM);
        let content_h = y;

        // Over everything else that scrolls, but the ring.
        if let Some(overlay) = &parts.overlay {
            let children = scroller.Children()?;
            children.Remove(overlay)?;
            children.InsertAtTop(overlay)?;
        }
        let (ring, ring_source, ring_grid) = self.build_ring(&scroller)?;

        let (c, t) = (&self.anim.compositor, &self.theme);
        let bar = Scrollbar::new(
            c,
            &content,
            &c.CreateColorBrushWithColor(t.thumb())?.cast()?,
        )?;
        let tips = vec![
            (Hit::Info, self.build_tip(&content, i18n::tr().email_tip)?),
            (Hit::Swap, self.build_tip(&content, i18n::tr().swap_tip)?),
        ];

        Ok(View {
            content,
            header_h,
            content_h,
            close_at,
            close_hover,
            body,
            fade,
            scroller,
            bar,
            tips,
            reveal,
            shortcuts: parts
                .shortcuts
                .into_iter()
                .collect::<Option<_>>()
                .ok_or_else(missing)?,
            // Some rows don't show.
            switches: parts.switches.into(),
            choices: parts
                .choices
                .into_iter()
                .collect::<Option<_>>()
                .ok_or_else(missing)?,
            notes: parts.notes.into(),
            counters: parts
                .counters
                .into_iter()
                .collect::<Option<_>>()
                .ok_or_else(missing)?,
            field: parts.field.ok_or_else(missing)?,
            translate_at: parts.translate.ok_or_else(missing)?,
            spare: spare.ok_or_else(missing)?,
            boxes: parts.boxes.ok_or_else(missing)?,
            interface: parts.interface.ok_or_else(missing)?,
            hits: parts.hits,
            stops: parts.stops,
            ring,
            ring_source,
            ring_grid,
            ring_radius: Cell::new(-1.0),
            folds: folds
                .into_iter()
                .collect::<Option<_>>()
                .ok_or_else(missing)?,
            languages: languages.ok_or_else(missing)?,
            slack,
            slack_y,
        })
    }

    /// The heading, the version under it, and the close button, added to
    /// `content`. Returns the header's height, and the close button's place
    /// and hover.
    fn build_header(
        &self,
        content: &ContainerVisual,
        reveal: &mut Vec<(Visual, Vector3)>,
    ) -> Result<(f32, Rect, SpriteVisual)> {
        let (c, r, t, s) = (
            &self.anim.compositor,
            &self.renderer,
            &self.theme,
            self.scale,
        );
        let px = |dip: f32| (dip * s).round();
        let card_w = px(CARD_W);
        let title = Text::new(c, content)?;
        let heading = r.layout_line(i18n::tr().settings, Face::Heading, Ink::Primary, 300.0)?;
        let title_size = title.show(r, &heading, s, t)?;
        let title_at = v3(px(LABEL_X), px(PAD_TOP));
        title.visual.SetOffset(title_at)?;
        reveal.push((title.visual.cast()?, title_at));
        let version = Text::new(c, content)?;
        let name = format!("WinDict {}", env!("CARGO_PKG_VERSION"));
        let line = r.layout_line(&name, Face::Small, Ink::Tertiary, 300.0)?;
        let version_size = version.show(r, &line, s, t)?;
        let version_at = v3(title_at.X, title_at.Y + title_size.Y);
        version.visual.SetOffset(version_at)?;
        reveal.push((version.visual.cast()?, version_at));
        let header_h = version_at.Y + version_size.Y + px(HEADER_GAP);

        let size = px(CLOSE_SIZE);
        let close_at = Rect {
            x: card_w - px(CLOSE_INSET) - size,
            y: px(CLOSE_INSET),
            w: size,
            h: size,
        };
        let close = container(c, content, close_at.at())?;
        let face = Rect {
            w: size,
            h: size,
            ..Rect::default()
        };
        let (close_hover, _) = rounded(c, r, &close, face, px(CLOSE_RADIUS), t.highlight())?;
        close_hover.SetOpacity(0.0)?;
        let glyph = Text::new(c, &close)?;
        let cross = r.layout_line(CLOSE_GLYPH, Face::Chrome, Ink::Primary, CLOSE_SIZE)?;
        let glyph_size = glyph.show(r, &cross, s, t)?;
        glyph
            .visual
            .SetOffset(v3(0.0, ((size - glyph_size.Y) / 2.0).round()))?;
        reveal.push((close.cast()?, close_at.at()));
        Ok((header_h, close_at, close_hover))
    }

    /// The ring around what the keyboard is on, in the `scroller`, and the
    /// brushes that shape it as it glides.
    fn build_ring(
        &self,
        scroller: &ContainerVisual,
    ) -> Result<(
        SpriteVisual,
        CompositionSurfaceBrush,
        CompositionNineGridBrush,
    )> {
        let c = &self.anim.compositor;
        // It glides from stop to stop.
        // Drawn through a mask, like the controls: a shape would be jagged
        // in the scroller. The mask is a nine-grid, to stretch as it glides.
        let ring = c.CreateSpriteVisual()?;
        ring.SetOpacity(0.0)?;
        let ring_source = c.CreateSurfaceBrush()?;
        ring_source.SetStretch(CompositionStretch::Fill)?;
        let ring_grid = c.CreateNineGridBrush()?;
        ring_grid.SetSource(&ring_source)?;
        let ring_brush = c.CreateColorBrushWithColor(self.theme.focus())?;
        let ring_mask = c.CreateMaskBrush()?;
        ring_mask.SetSource(&ring_brush)?;
        ring_mask.SetMask(&ring_grid)?;
        ring.SetBrush(&ring_mask)?;
        scroller.Children()?.InsertAtTop(&ring)?;
        Ok((ring, ring_source, ring_grid))
    }

    /// The shortcut row's edit symbol and keys, and the ring that breathes
    /// around the row while a new shortcut is awaited. `row` is in `group`,
    /// and its control ends at `right`, centred on `middle`.
    fn build_shortcut(
        &self,
        group: &ContainerVisual,
        row: Rect,
        right: f32,
        middle: f32,
        hover: Option<SpriteVisual>,
    ) -> Result<ShortcutView> {
        let (c, r, t, s) = (
            &self.anim.compositor,
            &self.renderer,
            &self.theme,
            self.scale,
        );
        let px = |dip: f32| (dip * s).round();
        let edit = Text::new(c, group)?;
        let pen = r.layout_line(EDIT_GLYPH, Face::Icon, Ink::Secondary, EDIT_W)?;
        let size = edit.show(r, &pen, s, t)?;
        let x = right - px(EDIT_W);
        edit.visual
            .SetOffset(v3(x, (middle - size.Y / 2.0).round()))?;
        let old_caps = Text::new(c, group)?;
        let caps = Text::new(c, group)?;
        // Rings the row, breathing, while a new shortcut is awaited.
        let ring = c.CreateShapeVisual()?;
        ring.SetOffset(row.at())?;
        ring.SetSize(row.size())?;
        ring.SetOpacity(0.0)?;
        let inset = px(RECORD_INSET);
        let geometry = c.CreateRoundedRectangleGeometry()?;
        geometry.SetOffset(v2(inset, inset))?;
        geometry.SetSize(v2(row.w - 2.0 * inset, row.h - 2.0 * inset))?;
        geometry.SetCornerRadius(v2(px(RECORD_RADIUS), px(RECORD_RADIUS)))?;
        let shape = c.CreateSpriteShapeWithGeometry(&geometry)?;
        shape.SetStrokeBrush(&c.CreateColorBrushWithColor(t.accent)?)?;
        shape.SetStrokeThickness((RECORD_STROKE * s).max(1.0))?;
        ring.Shapes()?.Append(&shape)?;
        group.Children()?.InsertAtTop(&ring)?;

        Ok(ShortcutView {
            hover: hover.ok_or_else(|| windows::core::Error::from(E_UNEXPECTED))?,
            caps,
            old_caps,
            caps_right: x - px(CAPS_GAP),
            caps_middle: middle,
            caps_rest: v3(0.0, 0.0),
            ring,
        })
    }

    /// The chevron at the right of a row that folds, which turns as the
    /// list opens.
    fn build_chevron(
        &self,
        group: &ContainerVisual,
        right: f32,
        middle: f32,
    ) -> Result<SpriteVisual> {
        let (c, r, t, s) = (
            &self.anim.compositor,
            &self.renderer,
            &self.theme,
            self.scale,
        );
        let chevron = Text::new(c, group)?;
        let line = r.layout_line(CHEVRON_GLYPH, Face::Icon, Ink::Secondary, CHEVRON_W)?;
        let size = chevron.show(r, &line, s, t)?;
        let at = v3(right - size.X, (middle - size.Y / 2.0).round());
        chevron.visual.SetOffset(at)?;
        chevron.visual.SetCenterPoint(Vector3 {
            X: size.X / 2.0,
            Y: size.Y / 2.0,
            Z: 0.0,
        })?;
        Ok(chevron.visual)
    }

    /// The shortcuts, each a row like the others, in the list `top` down
    /// `group` (at `group_at` in the scroller). Returns the list.
    fn build_shortcuts(
        &self,
        parts: &mut Parts,
        group: &ContainerVisual,
        group_at: Vector3,
        top: f32,
        languages: &Layout,
    ) -> Result<ContainerVisual> {
        let (c, r, t, s) = (
            &self.anim.compositor,
            &self.renderer,
            &self.theme,
            self.scale,
        );
        let list = container(c, group, v3(0.0, top))?;
        list.SetOpacity(0.0)?;
        let at = v3(group_at.X, group_at.Y + top);
        let (w, h) = (group.Size()?.X, (ROW_H * s).round());
        // The line over the first, under the row the list opens out of.
        let line = Rect {
            w,
            h: s.round().max(1.0),
            ..Rect::default()
        };
        rounded(c, r, &list, line, 0.0, t.separator())?;
        for (i, spec) in SHORTCUT_ROWS.iter().enumerate() {
            let row = Rect {
                x: 0.0,
                y: i as f32 * h,
                w,
                h,
            };
            self.build_row(parts, &list, at, spec, row, languages)?;
        }
        Ok(list)
    }

    /// The list of languages, each with a box to tick, `top` down `group`
    /// (which is `w` wide, at `group_at` in the scroller). Returns it and
    /// its languages.
    fn build_list(
        &self,
        parts: &mut Parts,
        group: &ContainerVisual,
        group_at: Vector3,
        top: f32,
        w: f32,
    ) -> Result<(ContainerVisual, Vec<LanguageView>)> {
        let (c, r, t, s) = (
            &self.anim.compositor,
            &self.renderer,
            &self.theme,
            self.scale,
        );
        let px = |dip: f32| (dip * s).round();
        let list = container(c, group, v3(0.0, top))?;
        let at = v3(group_at.X, group_at.Y + top);
        list.SetOpacity(0.0)?;
        // The tick, in the color that stands out on the accent.
        let inverse = t.on_accent();
        let (h, b) = (px(LANGUAGE_H), px(CHECK_SIZE));
        let outset = px(FOCUS_OUTSET);
        let mut items = Vec::new();
        for (i, (_, name)) in dicts::languages().enumerate() {
            let row = Rect {
                x: 0.0,
                y: i as f32 * h,
                w,
                h,
            };
            let line = Rect {
                h: s.round().max(1.0),
                ..row
            };
            rounded(c, r, &list, line, 0.0, t.separator())?;
            let (hover, _) = rounded(c, r, &list, row, 0.0, t.highlight())?;
            hover.SetOpacity(0.0)?;
            let middle = row.y + h / 2.0;
            let check = Rect {
                x: px(ROW_PAD),
                y: (middle - b / 2.0).round(),
                w: b,
                h: b,
            };
            let face = Rect {
                w: b,
                h: b,
                ..Rect::default()
            };
            let radius = px(CHECK_RADIUS);
            let off = container(c, &list, check.at())?;
            rounded(c, r, &off, face, radius, t.sunken())?;
            outline(c, r, &off, face, radius, t.strong_stroke())?;
            let on = container(c, &list, check.at())?;
            on.SetCenterPoint(Vector3 {
                X: b / 2.0,
                Y: b / 2.0,
                Z: 0.0,
            })?;
            rounded(c, r, &on, face, radius, t.accent)?;
            let tick = Text::new(c, &on)?;
            let glyph = r.layout_aligned(
                CHECK_GLYPH,
                Face::Icon,
                Ink::Primary,
                CHECK_SIZE,
                Align::Center,
            )?;
            let size = tick.show(r, &glyph, s, &inverse)?;
            tick.visual
                .SetOffset(v3(0.0, ((b - size.Y) / 2.0).round()))?;

            let x = check.x + b + px(CHECK_GAP);
            let label = Text::new(c, &list)?;
            let text = r.layout_line(name, Face::Setting, Ink::Primary, 200.0)?;
            let size = label.show(r, &text, s, t)?;
            label
                .visual
                .SetOffset(v3(x, (middle - size.Y / 2.0).round()))?;

            let place = |rect: Rect| rect.offset(at);
            parts.hits.push((Hit::Language(i), place(row)));
            parts.stops.push((
                Stop::Language(i),
                place(check).outset(outset),
                radius + outset,
            ));
            items.push(LanguageView {
                on,
                off,
                hover,
                status: Text::new(c, &list)?,
                status_right: w - px(ROW_PAD),
                status_middle: middle,
            });
        }
        Ok((list, items))
    }

    /// One setting: its name and description on the left, its control on
    /// the right, and a line above unless it comes first (`row.y` is 0). `row`
    /// is its place in the `group`, which sits at `group_at` in the scroller.
    fn build_row(
        &self,
        parts: &mut Parts,
        group: &ContainerVisual,
        group_at: Vector3,
        spec: &Row,
        row: Rect,
        languages: &Layout,
    ) -> Result<()> {
        let (c, r, t, s) = (
            &self.anim.compositor,
            &self.renderer,
            &self.theme,
            self.scale,
        );
        let px = |dip: f32| (dip * s).round();
        if row.y > 0.0 {
            let line = Rect {
                h: s.round().max(1.0),
                ..row
            };
            rounded(c, r, group, line, 0.0, t.separator())?;
        }
        match spec.control {
            Control::Translate => {
                return self.build_translate(parts, group, group_at, spec, row, languages);
            }
            Control::Selection => return self.build_selection(parts, group, group_at, spec, row),
            _ => {}
        }
        // Rows that are clicked as a whole light up under the pointer.
        let hover = match spec.control {
            Control::Shortcut(_) | Control::Switch(_) | Control::Fold(_) => {
                let (hover, _) = rounded(c, r, group, row, 0.0, t.highlight())?;
                hover.SetOpacity(0.0)?;
                Some(hover)
            }
            Control::Choice(_)
            | Control::Counter(_)
            | Control::Interface
            | Control::Email
            | Control::Translate
            | Control::Selection => None,
        };

        let room = GROUP_W - ROW_PAD - TEXT_X - spec.control.width() - CONTROL_GAP;
        let title = Text::new(c, group)?;
        let name = r.layout_line(spec.title(), Face::Setting, Ink::Primary, room)?;
        let title_size = title.show(r, &name, s, t)?;
        let detail = Text::new(c, group)?;
        let line = r.layout_line(spec.detail(), Face::Small, Ink::Secondary, room)?;
        let detail_size = detail.show(r, &line, s, t)?;
        let icon = Text::new(c, group)?;
        let glyph = r.layout_line(spec.icon, Face::LargeButton, Ink::Primary, ICON_W)?;
        let icon_size = icon.show(r, &glyph, s, t)?;
        let icon_y = row.y + ((row.h - icon_size.Y) / 2.0).round();
        icon.visual.SetOffset(v3(px(ROW_PAD), icon_y))?;
        let block = title_size.Y + px(DETAIL_GAP) + detail_size.Y;
        let text_y = row.y + ((row.h - block) / 2.0).round();
        title.visual.SetOffset(v3(px(TEXT_X), text_y))?;
        if matches!(spec.control, Control::Email) {
            // The info symbol follows the name, wherever that ends.
            let (_, name_w, _) = r.layout_field(spec.title(), Ink::Primary, 0)?;
            let info = Text::new(c, group)?;
            let glyph = r.layout_line(INFO_GLYPH, Face::Icon, Ink::Secondary, INFO_W)?;
            let size = info.show(r, &glyph, s, t)?;
            let x = px(TEXT_X) + (name_w * s).round() + px(INFO_GAP);
            let y = text_y + ((title_size.Y - size.Y) / 2.0).round();
            info.visual.SetOffset(v3(x, y))?;
            let at = Rect {
                x,
                y,
                w: px(INFO_W),
                h: size.Y,
            };
            parts.hits.push((Hit::Info, at.offset(group_at)));
        }
        let detail_at = v3(px(TEXT_X), text_y + title_size.Y + px(DETAIL_GAP));
        detail.visual.SetOffset(detail_at)?;
        // A description that changes cross-fades from the one before.
        let note = match spec.control {
            Control::Shortcut(shortcut) => Some(Note::of(shortcut)),
            Control::Fold(Fold::Shortcuts) => Some(Note::Shortcuts),
            Control::Fold(Fold::Languages) => Some(Note::Language),
            Control::Switch(Switch::UiAccess) => Some(Note::UiAccess),
            _ => None,
        };
        if let Some(note) = note {
            parts.notes[note as usize] = Some(NoteView {
                text: detail,
                old: Text::new(c, group)?,
                at: detail_at,
                w: room,
            });
        }

        let right = row.w - px(ROW_PAD);
        let middle = row.y + row.h / 2.0;
        let place = |rect: Rect| rect.offset(group_at);
        let outset = px(FOCUS_OUTSET);
        match spec.control {
            Control::Shortcut(shortcut) => {
                let view = self.build_shortcut(group, row, right, middle, hover)?;
                let inset = px(RECORD_INSET);
                parts.hits.push((Hit::Shortcut(shortcut), place(row)));
                let ring_at = place(row).outset(-inset);
                parts
                    .stops
                    .push((Stop::Shortcut(shortcut), ring_at, px(RECORD_RADIUS)));
                parts.shortcuts[shortcut as usize] = Some(view);
            }
            Control::Switch(sw) => {
                let (view, track) = self.build_switch(group, right, middle)?;
                parts.hits.push((Hit::Switch(sw), place(row)));
                let radius = track.h / 2.0 + outset;
                parts
                    .stops
                    .push((Stop::Switch(sw), place(track).outset(outset), radius));
                parts.switches[sw as usize] = Some(SwitchView {
                    hover: hover.ok_or_else(|| windows::core::Error::from(E_UNEXPECTED))?,
                    ..view
                });
            }
            Control::Choice(choice) => {
                let (view, at, segments) = self.build_choice(choice, group, right, middle)?;
                for (i, segment) in segments.into_iter().enumerate() {
                    parts.hits.push((Hit::Choice(choice, i), place(segment)));
                }
                let radius = px(SEGMENT_RADIUS) + outset;
                parts
                    .stops
                    .push((Stop::Choice(choice), place(at).outset(outset), radius));
                parts.choices[choice as usize] = Some(view);
            }
            Control::Interface => {
                let at = v3(
                    group_at.X + right - px(INTERFACE_W),
                    group_at.Y + (middle - px(SELECT_H) / 2.0).round(),
                );
                parts.interface = Some(self.build_interface(parts, at)?);
            }
            Control::Email => {
                let (view, at) = self.build_field(group, right, middle)?;
                parts.hits.push((Hit::Email, place(at)));
                let radius = px(FIELD_RADIUS) + outset;
                parts
                    .stops
                    .push((Stop::Email, place(at).outset(outset), radius));
                parts.field = Some(FieldView {
                    at: place(view.at),
                    ..view
                });
            }
            Control::Counter(counter) => {
                let (view, at, buttons) = self.build_counter(group, right, middle)?;
                for (up, button) in [false, true].into_iter().zip(buttons) {
                    parts.hits.push((Hit::Step(counter, up), place(button)));
                }
                let radius = px(STEP_RADIUS) + outset;
                parts
                    .stops
                    .push((Stop::Counter(counter), place(at).outset(outset), radius));
                parts.counters[counter as usize] = Some(view);
            }
            Control::Translate | Control::Selection => {}
            Control::Fold(fold) => {
                let chevron = self.build_chevron(group, right, middle)?;
                parts.hits.push((Hit::Fold(fold), place(row)));
                let ring_at = place(row).outset(-px(RECORD_INSET));
                parts
                    .stops
                    .push((Stop::Fold(fold), ring_at, px(RECORD_RADIUS)));
                let hover = hover.ok_or_else(|| windows::core::Error::from(E_UNEXPECTED))?;
                parts.folds[fold as usize] = Some((hover, chevron));
            }
        }
        Ok(())
    }

    /// The ways of reading the selection: the setting's name over its
    /// options, side by side across the row, and what the chosen one does
    /// under them.
    fn build_selection(
        &self,
        parts: &mut Parts,
        group: &ContainerVisual,
        group_at: Vector3,
        spec: &Row,
        row: Rect,
    ) -> Result<()> {
        let (c, r, t, s) = (
            &self.anim.compositor,
            &self.renderer,
            &self.theme,
            self.scale,
        );
        let px = |dip: f32| (dip * s).round();
        let room = GROUP_W - ROW_PAD - TEXT_X;
        let title = Text::new(c, group)?;
        let name = r.layout_line(spec.title(), Face::Setting, Ink::Primary, room)?;
        let title_size = title.show(r, &name, s, t)?;
        let top = row.y + px(TRANSLATE_PAD);
        title.visual.SetOffset(v3(px(TEXT_X), top))?;
        let icon = Text::new(c, group)?;
        let glyph = r.layout_line(spec.icon, Face::LargeButton, Ink::Primary, ICON_W)?;
        let icon_size = icon.show(r, &glyph, s, t)?;
        let icon_y = top + ((title_size.Y - icon_size.Y) / 2.0).round();
        icon.visual.SetOffset(v3(px(ROW_PAD), icon_y))?;

        let choice = Choice::Selection;
        let right = row.w - px(ROW_PAD);
        let middle = row.y + px(SELECTION_HEAD) + px(SEGMENT_H) / 2.0;
        let (view, at, segments) = self.build_choice(choice, group, right, middle)?;
        let place = |rect: Rect| rect.offset(group_at);
        for (i, segment) in segments.into_iter().enumerate() {
            parts.hits.push((Hit::Choice(choice, i), place(segment)));
        }
        let outset = px(FOCUS_OUTSET);
        let radius = px(SEGMENT_RADIUS) + outset;
        parts
            .stops
            .push((Stop::Choice(choice), place(at).outset(outset), radius));
        parts.choices[choice as usize] = Some(view);

        // Filled in, and cross-faded as the choice changes, by the note.
        let detail = Text::new(c, group)?;
        let detail_at = v3(px(TEXT_X), at.y + at.h + px(SELECTION_GAP));
        detail.visual.SetOffset(detail_at)?;
        parts.notes[Note::Selection as usize] = Some(NoteView {
            text: detail,
            old: Text::new(c, group)?,
            at: detail_at,
            w: room,
        });
        Ok(())
    }

    /// The box of the language WinDict is shown in, at `at` in the
    /// scroller, over the groups, and where it can be clicked and focused.
    fn build_interface(&self, parts: &mut Parts, at: Vector3) -> Result<InterfaceView> {
        let (c, r, t, s) = (
            &self.anim.compositor,
            &self.renderer,
            &self.theme,
            self.scale,
        );
        let overlay = parts
            .overlay
            .as_ref()
            .ok_or_else(|| windows::core::Error::from(E_UNEXPECTED))?;
        let (layout, moving) = self.layout_interface()?;
        let text = Text::new(c, overlay)?;
        text.visual.SetOffset(at)?;
        let morph = Morph::new(c, &text, layout.caret.is_some())?;
        morph.show(&self.anim, r, &text, &layout, Some(&moving), s, t)?;
        let parent = text.visual.cast::<ContainerVisual>()?;
        let (caret, _) = rounded(c, r, &parent, Rect::default(), 0.0, t.accent)?;
        self.show_caret(&caret, &layout)?;
        let radius = (SELECT_RING_RADIUS * s).round();
        for link in layout
            .links
            .iter()
            .filter(|l| l.target == Target::Interface)
        {
            let rect = Rect {
                x: at.X + link.x * s,
                y: at.Y + link.y * s,
                w: link.width * s,
                h: link.height * s,
            };
            parts.hits.push((Hit::Select(Select::Interface), rect));
            parts
                .stops
                .push((Stop::Select(Select::Interface), rect, radius));
        }
        Ok(InterfaceView {
            text,
            morph,
            caret,
            at,
        })
    }

    /// The default languages: the setting's name and description over the
    /// translate card's boxes (the `languages` layout), the swap button
    /// between them and, when one is open, the list to pick from.
    fn build_translate(
        &self,
        parts: &mut Parts,
        group: &ContainerVisual,
        group_at: Vector3,
        spec: &Row,
        row: Rect,
        languages: &Layout,
    ) -> Result<()> {
        let (c, r, t, s) = (
            &self.anim.compositor,
            &self.renderer,
            &self.theme,
            self.scale,
        );
        let px = |dip: f32| (dip * s).round();
        let room = GROUP_W - ROW_PAD - TEXT_X;
        let title = Text::new(c, group)?;
        let name = r.layout_line(spec.title(), Face::Setting, Ink::Primary, room)?;
        let title_size = title.show(r, &name, s, t)?;
        let top = row.y + px(TRANSLATE_PAD);
        title.visual.SetOffset(v3(px(TEXT_X), top))?;
        let detail = Text::new(c, group)?;
        let line = r.layout_line(spec.detail(), Face::Small, Ink::Secondary, room)?;
        let detail_size = detail.show(r, &line, s, t)?;
        let detail_at = v3(px(TEXT_X), top + title_size.Y + px(DETAIL_GAP));
        detail.visual.SetOffset(detail_at)?;
        let icon = Text::new(c, group)?;
        let glyph = r.layout_line(spec.icon, Face::LargeButton, Ink::Primary, ICON_W)?;
        let icon_size = icon.show(r, &glyph, s, t)?;
        let block = title_size.Y + px(DETAIL_GAP) + detail_size.Y;
        let icon_y = top + ((block - icon_size.Y) / 2.0).round();
        icon.visual.SetOffset(v3(px(ROW_PAD), icon_y))?;

        let at = v3(px(ROW_PAD), row.y + px(TRANSLATE_HEAD));
        // Under the boxes, so the arrows show through it.
        let swap = languages.links.iter().find(|l| l.target == Target::Swap);
        let swap_at = swap.map_or(Rect::default(), |l| Rect {
            x: at.X + (l.x * s).round(),
            y: at.Y + (l.y * s).round(),
            w: (l.width * s).round(),
            h: (l.height * s).round(),
        });
        let (swap_hover, _) = rounded(c, r, group, swap_at, px(SWAP_RING_RADIUS), t.highlight())?;
        swap_hover.SetOpacity(0.0)?;
        let boxes = Text::new(c, group)?;
        boxes.show(r, languages, s, t)?;
        boxes.visual.SetOffset(at)?;
        // The list of languages scrolls in the box it's open under, over
        // its outline.
        self.list.apply_theme(t)?;
        let mut blinker = None;
        if let Some(m) = languages.caret {
            let caret = rounded(
                c,
                r,
                &boxes.visual.cast::<ContainerVisual>()?,
                Rect {
                    x: (m.x * s).round(),
                    y: (m.y * s).round(),
                    w: (m.width * s).round().max(1.0),
                    h: (m.height * s).round(),
                },
                0.0,
                t.accent,
            )?;
            ui::blink(c, &caret.0)?;
            blinker = Some(caret.0);
        }
        parts.boxes = Some(BoxesView {
            text: boxes,
            caret: blinker,
            swap_hover,
            at,
            group_at,
        });
        let place = |rect: Rect| rect.offset(group_at);
        for link in &languages.links {
            let rect = place(Rect {
                x: at.X + link.x * s,
                y: at.Y + link.y * s,
                w: link.width * s,
                h: link.height * s,
            });
            match &link.target {
                Target::Language(side) => {
                    let select = Select::Translate(*side);
                    parts.hits.push((Hit::Select(select), rect));
                    let stop = (Stop::Select(select), rect, px(SELECT_RING_RADIUS));
                    parts.stops.push(stop);
                }
                Target::Swap => {
                    parts.hits.push((Hit::Swap, rect));
                    let stop = (Stop::Swap, rect, px(SWAP_RING_RADIUS));
                    parts.stops.push(stop);
                }
                _ => {}
            }
        }
        parts.translate = Some(place(row));
        Ok(())
    }
}

/// The parts of a view as its rows are built.
#[derive(Default)]
struct Parts {
    shortcuts: [Option<ShortcutView>; SHORTCUTS.len()],
    translate: Option<Rect>,
    switches: [Option<SwitchView>; SWITCHES.len()],
    choices: [Option<ChoiceView>; CHOICES.len()],
    notes: [Option<NoteView>; NOTES.len()],
    counters: [Option<CounterView>; COUNTERS.len()],
    field: Option<FieldView>,
    /// The hover and chevron of each row that folds, in the order of [`FOLDS`].
    folds: [Option<(SpriteVisual, SpriteVisual)>; FOLDS.len()],
    boxes: Option<BoxesView>,
    interface: Option<InterfaceView>,
    /// What the box of WinDict's language goes in.
    overlay: Option<ContainerVisual>,
    hits: Vec<(Hit, Rect)>,
    stops: Vec<(Stop, Rect, f32)>,
}
