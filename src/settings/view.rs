//! Building the settings window's visuals: the header, the groups of
//! settings with their controls, and the footer, laid out for the current
//! scale and theme. Where things can be clicked or focused is worked out
//! here too, as they're placed.

use windows::core::{Interface, Result};
use windows::Win32::Foundation::{E_UNEXPECTED, RECT};
use windows::UI::Color;
use windows::UI::Composition::*;
use windows_numerics::Vector3;

use super::*;

impl Settings {
    /// Builds the view afresh for the current scale and theme, showing the
    /// current state at rest. With `crossfade`, the old view fades out as
    /// the new one fades in; otherwise it goes at once.
    pub(super) fn replace_view(&mut self, crossfade: bool) -> Result<()> {
        let children = self.frame.card.Children()?;
        if let Some(old) = self.fading.take() {
            children.Remove(&old)?;
        }
        let view = self.build()?;
        if let Some(old) = self.view.replace(view) {
            if crossfade {
                let anim = &self.anim;
                anim.scalar(&old.content, "Opacity", 0.0, THEME_MS, 0, &anim.fade)?;
                self.fading = Some(old.content);
            } else {
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

    /// Sizes the card to its content, as far as the `work` area allows.
    pub(super) fn fit(&mut self, work: RECT) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let s = self.scale;
        let room = (work.bottom - work.top) as f32 - 2.0 * (SCREEN_GAP * s).round();
        let w = (CARD_W * s).round();
        let h = (view.header_h + view.content_h).min(room).ceil();
        self.card = (w, h);
        let m = (MARGIN * s).round();
        self.frame.resize(&self.anim, w, h, v3(m, m), s, false)?;
        view.body.SetSize(v2(w, h - view.header_h))?;
        self.scroll = self.scroll.min(self.max_scroll());
        if let Some(view) = &self.view {
            Animator::stop(&view.scroller, &["Offset"]);
            view.scroller.SetOffset(v3(0.0, -self.scroll))?;
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
        self.show_theme(false)?;
        // The theme changed with a click on the pill's way over: it carries on here.
        if let (Some(from), Some(view)) = (from, &self.view) {
            let pill = &view.theme.pill;
            pill.SetOffset(view.theme.rest[from])?;
            self.pill_from = Some(from);
            self.show_theme(true)?;
            self.pill_from = None;
        }
        for counter in COUNTERS {
            self.show_count(counter, None)?;
        }
        self.show_caps(false)?;
        let (text, error) = self.detail.clone();
        self.show_detail(&text, error, false)?;
        self.show_recording()?;
        self.show_hover(self.hovered)?;
        self.ring_shown = false;
        self.show_focus(false)?;
        if let Some(view) = &self.view {
            view.scroller.SetOffset(v3(0.0, -self.scroll))?;
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

        // The header: the heading, the version under it, and the close button.
        let title = Text::new(c, &content)?;
        let heading = r.layout_line("Settings", Face::Heading, Ink::Primary, 300.0)?;
        let title_size = title.show(r, &heading, s, t)?;
        let title_at = v3(px(LABEL_X), px(PAD_TOP));
        title.visual.SetOffset(title_at)?;
        reveal.push((title.visual.cast()?, title_at));
        let version = Text::new(c, &content)?;
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
        let close = container(c, &content, close_at.at())?;
        let face = Rect {
            w: size,
            h: size,
            ..Rect::default()
        };
        let (close_hover, _) = rounded(c, &close, face, px(CLOSE_RADIUS), t.highlight())?;
        close_hover.SetOpacity(0.0)?;
        let glyph = Text::new(c, &close)?;
        let cross = r.layout_line(CLOSE_GLYPH, Face::Chrome, Ink::Primary, CLOSE_SIZE)?;
        let glyph_size = glyph.show(r, &cross, s, t)?;
        glyph
            .visual
            .SetOffset(v3(0.0, ((size - glyph_size.Y) / 2.0).round()))?;
        reveal.push((close.cast()?, close_at.at()));

        // The groups of settings scroll under the header, should they not fit.
        let body = c.CreateContainerVisual()?;
        body.SetClip(&c.CreateInsetClip()?)?;
        body.SetOffset(v3(0.0, header_h))?;
        // As tall as the card allows; `fit` sizes it again when that changes.
        body.SetSize(v2(card_w, (self.card.1 - header_h).max(0.0)))?;
        content.Children()?.InsertAtTop(&body)?;
        let scroller = container(c, &body, v3(0.0, 0.0))?;

        let (group_x, group_w, row_h) = (px(GROUP_X), card_w - 2.0 * px(GROUP_X), px(ROW_H));
        let mut parts = Parts::default();
        let mut y = 0.0;
        for (i, (name, rows)) in SECTIONS.iter().enumerate() {
            y += if i == 0 { px(2.0) } else { px(SECTION_GAP) };
            let label = Text::new(c, &scroller)?;
            let caps = r.layout_label(name, Ink::Accent, 300.0)?;
            let label_size = label.show(r, &caps, s, t)?;
            let at = v3(px(LABEL_X), y);
            label.visual.SetOffset(at)?;
            reveal.push((label.visual.cast()?, at));
            y += label_size.Y + px(LABEL_GAP);

            let group_at = v3(group_x, y);
            let group_h = rows.len() as f32 * row_h;
            let group = container(c, &scroller, group_at)?;
            let bounds = Rect {
                w: group_w,
                h: group_h,
                ..Rect::default()
            };
            group.SetSize(bounds.size())?;
            let clip = c.CreateRoundedRectangleGeometry()?;
            clip.SetSize(bounds.size())?;
            clip.SetCornerRadius(v2(px(GROUP_RADIUS), px(GROUP_RADIUS)))?;
            group.SetClip(&c.CreateGeometricClipWithGeometry(&clip)?)?;
            rounded(c, &group, bounds, 0.0, t.group())?;
            for (j, spec) in rows.iter().enumerate() {
                let row = Rect {
                    x: 0.0,
                    y: j as f32 * row_h,
                    w: group_w,
                    h: row_h,
                };
                self.build_row(&mut parts, &group, group_at, spec, row, j > 0)?;
            }
            outline(c, &group, bounds, px(GROUP_RADIUS), t.group_stroke())?;
            reveal.push((group.cast()?, group_at));
            y += group_h;
        }

        // The footer: a way to the file behind it all.
        y += px(FOOTER_GAP);
        let footer_at = v3(0.0, y);
        let footer = container(c, &scroller, footer_at)?;
        let link = Text::new(c, &footer)?;
        let link_size = link.show(r, &r.layout_link(CONFIG_LINK)?, s, t)?;
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

        // The ring around what the keyboard is on, gliding from stop to stop.
        let ring = c.CreateShapeVisual()?;
        ring.SetSize(v2(card_w, content_h))?;
        ring.SetOpacity(0.0)?;
        let ring_geometry = c.CreateRoundedRectangleGeometry()?;
        let shape = c.CreateSpriteShapeWithGeometry(&ring_geometry)?;
        shape.SetStrokeBrush(&c.CreateColorBrushWithColor(t.focus())?)?;
        shape.SetStrokeThickness(px(FOCUS_STROKE).max(1.0))?;
        ring.Shapes()?.Append(&shape)?;
        scroller.Children()?.InsertAtTop(&ring)?;

        // The divider under the header and the thumb, shown when it scrolls.
        let line = Rect {
            x: 0.0,
            y: header_h - 1.0,
            w: card_w,
            h: 1.0,
        };
        let (divider, _) = rounded(c, &content, line, 0.0, t.separator())?;
        divider.SetOpacity(0.0)?;
        let thumb = c.CreateSpriteVisual()?;
        thumb.SetBrush(&c.CreateColorBrushWithColor(t.thumb())?)?;
        let thumb_clip = c.CreateRoundedRectangleGeometry()?;
        thumb.SetClip(&c.CreateGeometricClipWithGeometry(&thumb_clip)?)?;
        thumb.SetOpacity(0.0)?;
        content.Children()?.InsertAtTop(&thumb)?;

        let missing = || windows::core::Error::from(E_UNEXPECTED);
        Ok(View {
            content,
            header_h,
            content_h,
            close_at,
            close_hover,
            body,
            scroller,
            divider,
            thumb,
            thumb_clip,
            reveal,
            shortcut: parts.shortcut.ok_or_else(missing)?,
            switches: parts
                .switches
                .into_iter()
                .collect::<Option<_>>()
                .ok_or_else(missing)?,
            theme: parts.theme.ok_or_else(missing)?,
            counters: parts
                .counters
                .into_iter()
                .collect::<Option<_>>()
                .ok_or_else(missing)?,
            hits: parts.hits,
            stops: parts.stops,
            ring,
            ring_geometry,
        })
    }

    /// One setting: its name and description on the left, its control on
    /// the right, and a line above unless it comes first. `row` is its place
    /// in the `group`, which sits at `group_at` in the scroller.
    fn build_row(
        &self,
        parts: &mut Parts,
        group: &ContainerVisual,
        group_at: Vector3,
        spec: &Row,
        row: Rect,
        divided: bool,
    ) -> Result<()> {
        let (c, r, t, s) = (
            &self.anim.compositor,
            &self.renderer,
            &self.theme,
            self.scale,
        );
        let px = |dip: f32| (dip * s).round();
        if divided {
            let line = Rect {
                h: s.round().max(1.0),
                ..row
            };
            rounded(c, group, line, 0.0, t.separator())?;
        }
        // Rows that are clicked as a whole light up under the pointer.
        let hover = match spec.control {
            Control::Shortcut | Control::Switch(_) => {
                let (hover, _) = rounded(c, group, row, 0.0, t.highlight())?;
                hover.SetOpacity(0.0)?;
                Some(hover)
            }
            Control::Theme | Control::Counter(_) => None,
        };

        let room = GROUP_W - 2.0 * ROW_PAD - spec.control.width() - CONTROL_GAP;
        let title = Text::new(c, group)?;
        let name = r.layout_line(spec.title, Face::Setting, Ink::Primary, room)?;
        let title_size = title.show(r, &name, s, t)?;
        let detail = Text::new(c, group)?;
        let line = r.layout_line(spec.detail, Face::Small, Ink::Secondary, room)?;
        let detail_size = detail.show(r, &line, s, t)?;
        let block = title_size.Y + px(DETAIL_GAP) + detail_size.Y;
        let text_y = row.y + ((row.h - block) / 2.0).round();
        title.visual.SetOffset(v3(px(ROW_PAD), text_y))?;
        let detail_at = v3(px(ROW_PAD), text_y + title_size.Y + px(DETAIL_GAP));
        detail.visual.SetOffset(detail_at)?;

        let right = row.w - px(ROW_PAD);
        let middle = row.y + row.h / 2.0;
        let place = |rect: Rect| rect.offset(group_at);
        let outset = px(FOCUS_OUTSET);
        match spec.control {
            Control::Shortcut => {
                let edit = Text::new(c, group)?;
                let pen = r.layout_line(EDIT_GLYPH, Face::Icon, Ink::Secondary, EDIT_W)?;
                let size = edit.show(r, &pen, s, t)?;
                let x = right - px(EDIT_W);
                edit.visual
                    .SetOffset(v3(x, (middle - size.Y / 2.0).round()))?;
                let old_detail = Text::new(c, group)?;
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

                parts.hits.push((Hit::Shortcut, place(row)));
                let ring_at = place(row).outset(-inset);
                parts
                    .stops
                    .push((Stop::Shortcut, ring_at, px(RECORD_RADIUS)));
                parts.shortcut = Some(ShortcutView {
                    hover: hover.ok_or_else(|| windows::core::Error::from(E_UNEXPECTED))?,
                    caps,
                    old_caps,
                    caps_right: x - px(CAPS_GAP),
                    caps_middle: middle,
                    caps_rest: v3(0.0, 0.0),
                    detail,
                    old_detail,
                    detail_at,
                    detail_w: room,
                    ring,
                });
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
            Control::Theme => {
                let (view, at, segments) = self.build_theme(group, right, middle)?;
                for (i, segment) in segments.into_iter().enumerate() {
                    parts.hits.push((Hit::Theme(i), place(segment)));
                }
                let radius = px(SEGMENT_RADIUS) + outset;
                parts
                    .stops
                    .push((Stop::Theme, place(at).outset(outset), radius));
                parts.theme = Some(view);
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
        }
        Ok(())
    }

    /// A switch ending at `right` and centred on `middle`, in `group`.
    /// Returns it (without its row's hover) and its track.
    fn build_switch(
        &self,
        group: &ContainerVisual,
        right: f32,
        middle: f32,
    ) -> Result<(SwitchView, Rect)> {
        let (c, t, s) = (&self.anim.compositor, &self.theme, self.scale);
        let px = |dip: f32| (dip * s).round();
        let (w, h) = (px(SWITCH_W), px(SWITCH_H));
        let track = Rect {
            x: right - w,
            y: (middle - h / 2.0).round(),
            w,
            h,
        };
        let root = container(c, group, track.at())?;
        let face = Rect {
            w,
            h,
            ..Rect::default()
        };
        let off = container(c, &root, v3(0.0, 0.0))?;
        rounded(c, &off, face, h / 2.0, t.sunken())?;
        outline(c, &off, face, h / 2.0, t.strong_stroke())?;
        let (on, _) = rounded(c, &root, face, h / 2.0, t.accent)?;
        let knob = px(KNOB);
        let dot = Rect {
            w: knob,
            h: knob,
            ..Rect::default()
        };
        let (knob_visual, knob_brush) = rounded(c, &root, dot, knob / 2.0, t.knob(false))?;
        knob_visual.SetCenterPoint(Vector3 {
            X: knob / 2.0,
            Y: knob / 2.0,
            Z: 0.0,
        })?;
        let inset = ((h - knob) / 2.0).round();
        let view = SwitchView {
            on,
            off,
            knob: knob_visual,
            knob_brush,
            rest: [v3(inset, inset), v3(w - inset - knob, inset)],
            hover: c.CreateSpriteVisual()?,
        };
        Ok((view, track))
    }

    /// The choice of themes ending at `right` and centred on `middle`, in
    /// `group`. Returns it, its bounds and each option's.
    fn build_theme(
        &self,
        group: &ContainerVisual,
        right: f32,
        middle: f32,
    ) -> Result<(ThemeView, Rect, Vec<Rect>)> {
        let (c, r, t, s) = (
            &self.anim.compositor,
            &self.renderer,
            &self.theme,
            self.scale,
        );
        let px = |dip: f32| (dip * s).round();
        let (segment, h) = (px(SEGMENT_W), px(SEGMENT_H));
        let n = THEMES.len() as f32;
        let at = Rect {
            x: right - n * segment,
            y: (middle - h / 2.0).round(),
            w: n * segment,
            h,
        };
        let root = container(c, group, at.at())?;
        let face = Rect {
            w: at.w,
            h,
            ..Rect::default()
        };
        rounded(c, &root, face, px(SEGMENT_RADIUS), t.sunken())?;
        outline(c, &root, face, px(SEGMENT_RADIUS), t.control_stroke())?;

        let inset = px(PILL_INSET);
        let pill_face = Rect {
            w: segment - 2.0 * inset,
            h: h - 2.0 * inset,
            ..Rect::default()
        };
        let pill = container(c, &root, v3(inset, inset))?;
        pill.SetSize(pill_face.size())?;
        pill.SetCenterPoint(Vector3 {
            X: pill_face.w / 2.0,
            Y: pill_face.h / 2.0,
            Z: 0.0,
        })?;
        rounded(c, &pill, pill_face, px(PILL_RADIUS), t.raised())?;
        outline(c, &pill, pill_face, px(PILL_RADIUS), t.control_stroke())?;

        let mut labels = Vec::new();
        let mut segments = Vec::new();
        let mut rest = Vec::new();
        for (i, (_, name)) in THEMES.iter().enumerate() {
            let x = i as f32 * segment;
            let label = Text::new(c, &root)?;
            let line =
                r.layout_aligned(name, Face::Menu, Ink::Primary, SEGMENT_W, Align::Center)?;
            let size = label.show(r, &line, s, t)?;
            label
                .visual
                .SetOffset(v3(x, ((h - size.Y) / 2.0).round()))?;
            labels.push(label.visual);
            segments.push(Rect {
                x: at.x + x,
                y: at.y,
                w: segment,
                h,
            });
            rest.push(v3(x + inset, inset));
        }
        Ok((ThemeView { pill, labels, rest }, at, segments))
    }

    /// A number between − and + buttons, ending at `right` and centred on
    /// `middle`, in `group`. Returns it, its bounds and its buttons'.
    fn build_counter(
        &self,
        group: &ContainerVisual,
        right: f32,
        middle: f32,
    ) -> Result<(CounterView, Rect, Vec<Rect>)> {
        let (c, r, t, s) = (
            &self.anim.compositor,
            &self.renderer,
            &self.theme,
            self.scale,
        );
        let px = |dip: f32| (dip * s).round();
        let (b, value_w) = (px(STEP_SIZE), px(VALUE_W));
        let at = Rect {
            x: right - 2.0 * b - value_w,
            y: (middle - b / 2.0).round(),
            w: 2.0 * b + value_w,
            h: b,
        };
        let root = container(c, group, at.at())?;
        let face = Rect {
            w: b,
            h: b,
            ..Rect::default()
        };
        let mut buttons = Vec::new();
        let mut rects = Vec::new();
        for (x, glyph) in [(0.0, MINUS_GLYPH), (b + value_w, PLUS_GLYPH)] {
            let visual = container(c, &root, v3(x, 0.0))?;
            visual.SetSize(face.size())?;
            visual.SetCenterPoint(Vector3 {
                X: b / 2.0,
                Y: b / 2.0,
                Z: 0.0,
            })?;
            let radius = px(STEP_RADIUS);
            rounded(c, &visual, face, radius, t.control())?;
            outline(c, &visual, face, radius, t.control_stroke())?;
            let (hover, _) = rounded(c, &visual, face, radius, t.highlight())?;
            hover.SetOpacity(0.0)?;
            let text = Text::new(c, &visual)?;
            let line = r.layout_line(glyph, Face::Chrome, Ink::Primary, STEP_SIZE)?;
            let size = text.show(r, &line, s, t)?;
            text.visual
                .SetOffset(v3(0.0, ((b - size.Y) / 2.0).round()))?;
            buttons.push(StepButton {
                visual,
                hover,
                glyph: text.visual,
            });
            rects.push(Rect {
                x: at.x + x,
                y: at.y,
                w: b,
                h: b,
            });
        }
        let host = container(c, &root, v3(b, 0.0))?;
        host.SetSize(v2(value_w, b))?;
        host.SetClip(&c.CreateInsetClip()?)?;
        let old = Text::new(c, &host)?;
        let value = Text::new(c, &host)?;
        let view = CounterView {
            buttons,
            host,
            value,
            old,
            host_h: b,
        };
        Ok((view, at, rects))
    }
}

fn container(c: &Compositor, parent: &ContainerVisual, at: Vector3) -> Result<ContainerVisual> {
    let visual = c.CreateContainerVisual()?;
    visual.SetOffset(at)?;
    parent.Children()?.InsertAtTop(&visual)?;
    Ok(visual)
}

/// A rectangle of `color` with corners of `radius` pixels, in `parent`.
fn rounded(
    c: &Compositor,
    parent: &ContainerVisual,
    rect: Rect,
    radius: f32,
    color: Color,
) -> Result<(SpriteVisual, CompositionColorBrush)> {
    let visual = c.CreateSpriteVisual()?;
    let brush = c.CreateColorBrushWithColor(color)?;
    visual.SetBrush(&brush)?;
    visual.SetOffset(rect.at())?;
    visual.SetSize(rect.size())?;
    if radius > 0.0 {
        let clip = c.CreateRoundedRectangleGeometry()?;
        clip.SetSize(rect.size())?;
        clip.SetCornerRadius(v2(radius, radius))?;
        visual.SetClip(&c.CreateGeometricClipWithGeometry(&clip)?)?;
    }
    parent.Children()?.InsertAtTop(&visual)?;
    Ok((visual, brush))
}

/// A hairline just inside a rounded rectangle, in `parent`.
fn outline(
    c: &Compositor,
    parent: &ContainerVisual,
    rect: Rect,
    radius: f32,
    color: Color,
) -> Result<ShapeVisual> {
    let visual = c.CreateShapeVisual()?;
    visual.SetOffset(rect.at())?;
    visual.SetSize(rect.size())?;
    let geometry = c.CreateRoundedRectangleGeometry()?;
    geometry.SetOffset(v2(0.5, 0.5))?;
    geometry.SetSize(v2(rect.w - 1.0, rect.h - 1.0))?;
    geometry.SetCornerRadius(v2(radius - 0.5, radius - 0.5))?;
    let shape = c.CreateSpriteShapeWithGeometry(&geometry)?;
    shape.SetStrokeBrush(&c.CreateColorBrushWithColor(color)?)?;
    shape.SetStrokeThickness(1.0)?;
    visual.Shapes()?.Append(&shape)?;
    parent.Children()?.InsertAtTop(&visual)?;
    Ok(visual)
}

/// The parts of a view as its rows are built.
#[derive(Default)]
struct Parts {
    shortcut: Option<ShortcutView>,
    switches: [Option<SwitchView>; 4],
    theme: Option<ThemeView>,
    counters: [Option<CounterView>; 2],
    hits: Vec<(Hit, Rect)>,
    stops: Vec<(Stop, Rect, f32)>,
}
