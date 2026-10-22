//! Bringing the visuals up to date with the state, with or without animation.

use super::*;

impl Settings {
    // -----------------------------------------------------------------------
    // Showing the state

    /// Slides the knob over and fades the switch's fill to match its value.
    pub(super) fn show_switch(&self, sw: Switch, animate: bool) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let (v, anim, on) = (&view.switches[sw as usize], &self.anim, self.switch(sw));
        let rest = v.rest[on as usize];
        let (fill, color) = (if on { 1.0 } else { 0.0 }, self.theme.knob(on));
        if animate {
            anim.vector3(&v.knob, "Offset", rest, KNOB_MS, 0, &anim.overshoot)?;
            anim.scalar(&v.on, "Opacity", fill, SWITCH_FILL_MS, 0, &anim.decelerate)?;
            let ms = SWITCH_FILL_MS;
            anim.scalar(&v.off, "Opacity", 1.0 - fill, ms, 0, &anim.decelerate)?;
            anim.color(&v.knob_brush, v.knob_brush.Color()?, color, ms)
        } else {
            Animator::stop(&v.knob, &["Offset"]);
            Animator::stop(&v.on, &["Opacity"]);
            Animator::stop(&v.off, &["Opacity"]);
            Animator::stop(&v.knob_brush, &["Color"]);
            v.knob.SetOffset(rest)?;
            v.on.SetOpacity(fill)?;
            v.off.SetOpacity(1.0 - fill)?;
            v.knob_brush.SetColor(color)
        }
    }

    /// The knob grows under the pointer, and stretches while pressed.
    pub(super) fn show_knob(&self, sw: Switch) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let hit = Some(Hit::Switch(sw));
        let k = if self.pressed == hit {
            Vector3 {
                X: KNOB_PRESSED / KNOB,
                Y: KNOB_HOVER / KNOB,
                Z: 1.0,
            }
        } else if self.hovered == hit {
            scale(KNOB_HOVER / KNOB)
        } else {
            scale(1.0)
        };
        let (anim, knob) = (&self.anim, &view.switches[sw as usize].knob);
        anim.vector3(knob, "Scale", k, KNOB_MS / 2, 0, &anim.decelerate)
    }

    /// Glides `choice`'s pill to the chosen option, and brightens its name.
    pub(super) fn show_choice(&self, choice: Choice, animate: bool) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let (v, anim) = (&view.choices[choice as usize], &self.anim);
        let chosen = self.chosen(choice);
        let rest = v.rest[chosen];
        match (animate, self.pill_from) {
            (true, Some((c, from))) if c == choice && from != chosen => {
                anim.vector3(&v.pill, "Offset", rest, PILL_MS, 0, &anim.overshoot)?;
                // It stretches as it goes, and settles as it arrives.
                let stretch = anim.compositor.CreateVector3KeyFrameAnimation()?;
                let wide = Vector3 {
                    X: 1.12,
                    Y: 0.92,
                    Z: 1.0,
                };
                stretch.InsertKeyFrameWithEasingFunction(0.35, wide, &anim.decelerate)?;
                stretch.InsertKeyFrameWithEasingFunction(1.0, scale(1.0), &anim.decelerate)?;
                stretch.SetDuration(ui::span(PILL_MS))?;
                v.pill.StartAnimation(&HSTRING::from("Scale"), &stretch)?;
            }
            _ => {
                Animator::stop(&v.pill, &["Offset", "Scale"]);
                v.pill.SetOffset(rest)?;
                v.pill.SetScale(scale(1.0))?;
            }
        }
        self.show_choice_labels(choice)
    }

    pub(super) fn show_choice_labels(&self, choice: Choice) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let (anim, chosen) = (&self.anim, self.chosen(choice));
        for (i, label) in view.choices[choice as usize].labels.iter().enumerate() {
            let opacity = if i == chosen {
                1.0
            } else if self.hovered == Some(Hit::Choice(choice, i)) {
                0.85
            } else {
                0.6
            };
            anim.scalar(label, "Opacity", opacity, LABEL_MS, 0, &anim.decelerate)?;
        }
        Ok(())
    }

    /// Shows a counter's number. With `roll`, the new one rolls in from
    /// below as the old leaves upwards (or the other way, counting down).
    pub(super) fn show_count(&mut self, counter: Counter, roll: Option<bool>) -> Result<()> {
        let n = self.count(counter);
        let ends = [false, true].map(|up| self.next(counter, up).is_none());
        let (r, anim, t, s) = (&self.renderer, &self.anim, &self.theme, self.scale);
        let Some(view) = self.view.as_mut() else {
            return Ok(());
        };
        let v = &mut view.counters[counter as usize];
        std::mem::swap(&mut v.value, &mut v.old);
        let text = counter.text(n);
        let w = VALUE_W;
        let line = r.layout_aligned(&text, Face::Value, Ink::Primary, w, Align::Center)?;
        let size = v.value.show(r, &line, s, t)?;
        let rest = v3(0.0, ((v.host_h - size.Y) / 2.0).round());
        let (new, old) = (&v.value.visual, &v.old.visual);
        Animator::stop(new, &["Opacity", "Offset"]);
        match roll {
            Some(up) => {
                let d = if up { ROLL * s } else { -ROLL * s };
                new.SetOpacity(0.0)?;
                new.SetOffset(v3(rest.X, rest.Y + d))?;
                anim.scalar(new, "Opacity", 1.0, ROLL_MS, 0, &anim.decelerate)?;
                anim.vector3(new, "Offset", rest, ROLL_MS, 0, &anim.overshoot)?;
                let away = v3(rest.X, rest.Y - d);
                anim.scalar(old, "Opacity", 0.0, ROLL_MS / 2, 0, &anim.accelerate)?;
                anim.vector3(old, "Offset", away, ROLL_MS, 0, &anim.decelerate)?;
            }
            None => {
                new.SetOpacity(1.0)?;
                new.SetOffset(rest)?;
                Animator::stop(old, &["Opacity", "Offset"]);
                old.SetOpacity(0.0)?;
                v.old.clear();
            }
        }
        // A button that can't count any further fades back.
        for (button, end) in v.buttons.iter().zip(ends) {
            let opacity = if end { 0.35 } else { 1.0 };
            let glyph = &button.glyph;
            anim.scalar(glyph, "Opacity", opacity, LABEL_MS, 0, &anim.decelerate)?;
        }
        Ok(())
    }

    /// Shows `shortcut`'s keys, or while it's recorded, the modifiers held
    /// so far. With `rise`, the new keys rise into place as the old fade.
    pub(super) fn show_caps(&mut self, shortcut: Shortcut, rise: bool) -> Result<()> {
        let (r, anim, t, s) = (&self.renderer, &self.anim, &self.theme, self.scale);
        let layout: Layout = if self.recording == Some(shortcut) {
            let held = hotkey::modifier_names(self.preview);
            if held.is_empty() {
                r.layout_aligned(
                    tr().recording_hint,
                    Face::Menu,
                    Ink::Accent,
                    160.0,
                    Align::Right,
                )?
            } else {
                r.layout_caps(&held)?
            }
        } else {
            let names = key_names(self.values.config.hotkey(shortcut));
            r.layout_caps(&names.iter().map(String::as_str).collect::<Vec<_>>())?
        };
        let Some(view) = self.view.as_mut() else {
            return Ok(());
        };
        let v = &mut view.shortcuts[shortcut as usize];
        std::mem::swap(&mut v.caps, &mut v.old_caps);
        let size = v.caps.show(r, &layout, s, t)?;
        let rest = v3(
            (v.caps_right - size.X).round(),
            (v.caps_middle - size.Y / 2.0).round(),
        );
        v.caps_rest = rest;
        let (new, old) = (&v.caps.visual, &v.old_caps.visual);
        Animator::stop(new, &["Opacity", "Offset"]);
        if rise {
            new.SetOpacity(0.0)?;
            new.SetOffset(v3(rest.X, rest.Y + DRIFT * s))?;
            anim.scalar(new, "Opacity", 1.0, CAPS_MS, 0, &anim.decelerate)?;
            anim.vector3(new, "Offset", rest, CAPS_MS, 0, &anim.overshoot)?;
            anim.scalar(old, "Opacity", 0.0, CAPS_MS / 2, 0, &anim.accelerate)
        } else {
            new.SetOpacity(1.0)?;
            new.SetOffset(rest)?;
            Animator::stop(old, &["Opacity", "Offset"]);
            old.SetOpacity(0.0)?;
            v.old_caps.clear();
            Ok(())
        }
    }

    pub(super) fn shake_caps(&self, shortcut: Shortcut) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let v = &view.shortcuts[shortcut as usize];
        self.anim.shake(&v.caps.visual, v.caps_rest, self.scale)
    }

    /// Changes a setting's description, cross-fading to the new one.
    pub(super) fn set_note(&mut self, note: Note, text: &str, error: bool) -> Result<()> {
        let now = &self.notes[note as usize];
        if (text, error) == (now.0.as_str(), now.1) {
            return Ok(());
        }
        self.notes[note as usize] = (text.to_owned(), error);
        self.show_note(note, text, error, true)
    }

    pub(super) fn show_note(
        &mut self,
        note: Note,
        text: &str,
        error: bool,
        animate: bool,
    ) -> Result<()> {
        let (r, anim, t, s) = (&self.renderer, &self.anim, &self.theme, self.scale);
        let Some(view) = self.view.as_mut() else {
            return Ok(());
        };
        let v = &mut view.notes[note as usize];
        std::mem::swap(&mut v.text, &mut v.old);
        let ink = if error { Ink::Critical } else { Ink::Secondary };
        let line = r.layout_line(text, Face::Small, ink, v.w)?;
        v.text.show(r, &line, s, t)?;
        let (new, old, rest) = (&v.text.visual, &v.old.visual, v.at);
        Animator::stop(new, &["Opacity", "Offset"]);
        if animate {
            new.SetOpacity(0.0)?;
            new.SetOffset(v3(rest.X, rest.Y + DRIFT * s / 2.0))?;
            anim.scalar(new, "Opacity", 1.0, DETAIL_MS, 0, &anim.decelerate)?;
            anim.vector3(new, "Offset", rest, DETAIL_MS, 0, &anim.decelerate)?;
            anim.scalar(old, "Opacity", 0.0, DETAIL_MS / 2, 0, &anim.accelerate)
        } else {
            new.SetOpacity(1.0)?;
            new.SetOffset(rest)?;
            Animator::stop(old, &["Opacity"]);
            old.SetOpacity(0.0)?;
            v.old.clear();
            Ok(())
        }
    }

    /// Shows what's typed in the email field, or its placeholder, scrolled
    /// as little as keeps the caret in view.
    pub(super) fn show_field(&mut self) -> Result<()> {
        let (r, t, s) = (&self.renderer, &self.theme, self.scale);
        let (text, ink, at) = match self.email.text() {
            "" => (EMAIL_PLACEHOLDER, Ink::Tertiary, 0),
            text => (text, Ink::Primary, self.email.caret_utf16()),
        };
        let (layout, w, caret) = r.layout_field(text, ink, at)?;
        let Some(view) = &self.view else {
            return Ok(());
        };
        let v = &view.field;
        let size = v.text.show(r, &layout, s, t)?;
        let (x, w, room) = (caret.x * s, w * s, v.at.w);
        let caret_w = (CARET_W * s).round().max(1.0);
        let mut shift = self.email_shift;
        if x - shift > room - caret_w {
            shift = x - room + caret_w;
        }
        shift = shift.min(x).min(w + caret_w - room).max(0.0).round();
        self.email_shift = shift;
        let y = ((v.at.h - size.Y) / 2.0).round();
        v.text.visual.SetOffset(v3(-shift, y))?;
        v.caret.SetSize(v2(caret_w, (caret.height * s).round()))?;
        v.caret
            .SetOffset(v3((x - shift).round(), y + (caret.top * s).round()))?;
        if self.editing() {
            self.blink()?;
        }
        Ok(())
    }

    /// Whether what's typed goes in the email field.
    pub(super) fn editing(&self) -> bool {
        self.visible && self.active && self.focus == Some(Stop::Email)
    }

    /// The accent line under the email field, and its caret, show while
    /// it's typed in.
    pub(super) fn show_editing(&self) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let (anim, v, editing) = (&self.anim, &view.field, self.editing());
        let line = if editing { 1.0 } else { 0.0 };
        anim.scalar(&v.accent, "Opacity", line, HOVER_MS, 0, &anim.decelerate)?;
        if editing {
            self.blink()
        } else {
            Animator::stop(&v.caret, &["Opacity"]);
            v.caret.SetOpacity(0.0)
        }
    }

    /// Shows the email field's caret, solid for now: blinking restarts
    /// with every keystroke.
    pub(super) fn blink(&self) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        ui::blink(&self.anim.compositor, &view.field.caret)
    }

    /// Whether `hit` has a tooltip.
    pub(super) fn has_tip(&self, hit: Option<Hit>) -> bool {
        let tips = self.view.as_ref().map(|view| &view.tips);
        tips.is_some_and(|tips| tips.iter().any(|(h, _)| Some(*h) == hit))
    }

    /// Shows the tooltip of what's under the pointer, `hit`, under it,
    /// rising in a moment after the pointer settles there, and sends any
    /// other away.
    pub(super) fn show_tip(&self, hit: Option<Hit>) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        for (over, tip) in &view.tips {
            self.show_one_tip(view, *over, tip, hit == Some(*over))?;
        }
        Ok(())
    }

    /// Shows `tip`, under `over`, or sends it away.
    pub(super) fn show_one_tip(
        &self,
        view: &View,
        over: Hit,
        tip: &TipView,
        on: bool,
    ) -> Result<()> {
        let (anim, s) = (&self.anim, self.scale);
        Animator::stop(&tip.root, &["Opacity", "Offset"]);
        let info = view
            .hits(self.open)
            .find(|(hit, _)| *hit == over)
            .map(|(_, rect)| rect)
            // Not while it's under the header.
            .filter(|rect| rect.y >= self.scroll);
        let (true, Some(info)) = (on, info) else {
            return anim.scalar(&tip.root, "Opacity", 0.0, TIP_HIDE_MS, 0, &anim.accelerate);
        };
        let (w, h) = (tip.size.X, tip.size.Y);
        let gap = (TIP_GAP * s).round();
        let margin = (GROUP_X * s).round();
        let top = view.header_h + info.y - self.scroll;
        let below = top + info.h + gap;
        let y = if below + h <= self.card.1 - margin {
            below
        } else {
            top - gap - h
        };
        let x = (info.x + (info.w - w) / 2.0)
            .min(self.card.0 - margin - w)
            .max(margin);
        let rest = v3(x, y);
        let drift = (TIP_DRIFT * s).round();
        if tip.root.Opacity()? < 0.05 {
            tip.root.SetOffset(v3(x, y + drift))?;
        }
        anim.scalar(
            &tip.root,
            "Opacity",
            1.0,
            TIP_MS,
            TIP_DELAY_MS,
            &anim.decelerate,
        )?;
        anim.vector3(
            &tip.root,
            "Offset",
            rest,
            TIP_MS,
            TIP_DELAY_MS,
            &anim.decelerate,
        )
    }

    /// Fills the boxes of the languages used, and empties the others'.
    pub(super) fn show_checks(&self, animate: bool) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let anim = &self.anim;
        for (i, item) in view.languages.items.iter().enumerate() {
            let fill = if self.ticked(i) { 1.0 } else { 0.0 };
            if animate {
                let ms = SWITCH_FILL_MS;
                anim.scalar(&item.on, "Opacity", fill, ms, 0, &anim.decelerate)?;
                anim.scalar(&item.off, "Opacity", 1.0 - fill, ms, 0, &anim.decelerate)?;
                // The tick pops in.
                if fill == 1.0 && item.on.Opacity()? < 0.5 {
                    item.on.SetScale(scale(0.7))?;
                }
                let k = scale(1.0);
                anim.vector3(&item.on, "Scale", k, KNOB_MS, 0, &anim.overshoot)?;
            } else {
                Animator::stop(&item.on, &["Opacity", "Scale"]);
                Animator::stop(&item.off, &["Opacity"]);
                item.on.SetOpacity(fill)?;
                item.on.SetScale(scale(1.0))?;
                item.off.SetOpacity(1.0 - fill)?;
            }
        }
        Ok(())
    }

    /// Shows how the download of the language at `i` is going, if it is.
    pub(super) fn show_status(&self, i: usize) -> Result<()> {
        let (r, t, s) = (&self.renderer, &self.theme, self.scale);
        let Some(view) = &self.view else {
            return Ok(());
        };
        let item = &view.languages.items[i];
        let Some((text, error)) = &self.downloads[i] else {
            item.status.clear();
            return Ok(());
        };
        let ink = if *error {
            Ink::Critical
        } else {
            Ink::Secondary
        };
        let line = r.layout_aligned(text, Face::Small, ink, STATUS_W, Align::Right)?;
        let size = item.status.show(r, &line, s, t)?;
        let at = v3(
            (item.status_right - size.X).round(),
            (item.status_middle - size.Y / 2.0).round(),
        );
        item.status.visual.SetOffset(at)
    }

    /// Opens the list under the languages row, or shuts it: the group grows
    /// or shrinks with it, what's below moving down or up, and the chevron
    /// turns over.
    pub(super) fn show_open(&self, animate: bool) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let (v, anim) = (&view.languages, &self.anim);
        let grow = if self.open { v.list_h } else { 0.0 };
        let h = v.group_h + grow;
        let (clip, outline) = (v2(v.group_w, h), v2(v.group_w - 1.0, h - 1.0));
        let after = v3(0.0, grow);
        let (opacity, angle) = if self.open { (1.0, 180.0) } else { (0.0, 0.0) };
        if animate {
            let ease = &anim.decelerate;
            anim.vector2(&v.clip, "Size", clip, EXPAND_MS)?;
            anim.vector2(&v.outline, "Size", outline, EXPAND_MS)?;
            anim.vector3(&v.after, "Offset", after, EXPAND_MS, 0, ease)?;
            anim.scalar(&v.list, "Opacity", opacity, EXPAND_MS, 0, ease)?;
            anim.scalar(
                &v.chevron,
                "RotationAngleInDegrees",
                angle,
                EXPAND_MS,
                0,
                ease,
            )
        } else {
            Animator::stop(&v.clip, &["Size"]);
            Animator::stop(&v.outline, &["Size"]);
            Animator::stop(&v.after, &["Offset"]);
            Animator::stop(&v.list, &["Opacity"]);
            Animator::stop(&v.chevron, &["RotationAngleInDegrees"]);
            v.clip.SetSize(clip)?;
            v.outline.SetSize(outline)?;
            v.after.SetOffset(after)?;
            v.list.SetOpacity(opacity)?;
            v.chevron.SetRotationAngleInDegrees(angle)
        }
    }

    /// The ring round the shortcut being recorded breathes; the others fade.
    pub(super) fn show_recording(&self) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        for shortcut in SHORTCUTS {
            let ring = &view.shortcuts[shortcut as usize].ring;
            self.show_ring(ring, self.recording == Some(shortcut))?;
        }
        Ok(())
    }

    fn show_ring(&self, ring: &ShapeVisual, recording: bool) -> Result<()> {
        let anim = &self.anim;
        if !recording {
            return anim.scalar(ring, "Opacity", 0.0, 150, 0, &anim.accelerate);
        }
        if !ui::motion() {
            Animator::stop(ring, &["Opacity"]);
            return ring.SetOpacity(1.0);
        }
        let breathe = anim.compositor.CreateScalarKeyFrameAnimation()?;
        breathe.InsertKeyFrame(0.0, 1.0)?;
        breathe.InsertKeyFrameWithEasingFunction(0.5, 0.35, &anim.fade)?;
        breathe.InsertKeyFrameWithEasingFunction(1.0, 1.0, &anim.fade)?;
        breathe.SetDuration(ts(BREATHE_MS))?;
        breathe.SetIterationBehavior(AnimationIterationBehavior::Forever)?;
        ring.SetOpacity(1.0)?;
        ring.StartAnimation(&HSTRING::from("Opacity"), &breathe)
    }

    /// Lights up what's under the pointer, or puts out what no longer is.
    pub(super) fn show_hover(&self, hit: Option<Hit>) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let anim = &self.anim;
        let on = hit.is_some() && hit == self.hovered;
        let fade = |visual: &SpriteVisual| {
            let opacity = if on { 1.0 } else { 0.0 };
            anim.scalar(visual, "Opacity", opacity, HOVER_MS, 0, &anim.decelerate)
        };
        match hit {
            Some(Hit::Close) => fade(&view.close_hover),
            Some(Hit::Shortcut(shortcut)) => fade(&view.shortcuts[shortcut as usize].hover),
            Some(Hit::Switch(sw)) => {
                fade(&view.switches[sw as usize].hover)?;
                self.show_knob(sw)
            }
            Some(Hit::Choice(choice, _)) => self.show_choice_labels(choice),
            Some(Hit::Step(counter, up)) => {
                fade(&view.counters[counter as usize].buttons[up as usize].hover)
            }
            Some(Hit::Languages) => fade(&view.languages.hover),
            Some(Hit::Language(i)) => fade(&view.languages.items[i].hover),
            Some(Hit::Email) => fade(&view.field.hover),
            Some(Hit::Swap) => fade(&view.boxes.swap_hover),
            Some(Hit::Info | Hit::Select(_) | Hit::Pick(_) | Hit::Config) | None => Ok(()),
        }
    }

    /// Presses in, or lets go of, what the button went down on.
    pub(super) fn show_pressed(&self, hit: Option<Hit>) -> Result<()> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let (anim, down) = (&self.anim, hit.is_some() && hit == self.pressed);
        match hit {
            Some(Hit::Switch(sw)) => self.show_knob(sw),
            Some(Hit::Step(counter, up)) => {
                let button = &view.counters[counter as usize].buttons[up as usize].visual;
                let (k, ease) = if down {
                    (0.9, &anim.decelerate)
                } else {
                    (1.0, &anim.overshoot)
                };
                let ms = if down { PRESS_MS } else { PRESS_MS * 3 };
                anim.vector3(button, "Scale", scale(k), ms, 0, ease)
            }
            _ => Ok(()),
        }
    }

    /// Moves the focus ring to the focused control, gliding there when it
    /// was showing already, or hides it.
    pub(super) fn show_focus(&mut self, animate: bool) -> Result<()> {
        self.show_editing()?;
        let Some(view) = &self.view else {
            return Ok(());
        };
        let anim = &self.anim;
        let stop = self.focus.filter(|_| self.ring).and_then(|focus| {
            view.stops(self.open)
                .find(|(stop, _, _)| *stop == focus)
                .map(|(_, rect, radius)| (rect, radius))
        });
        let Some((rect, radius)) = stop else {
            self.ring_shown = false;
            Animator::stop(&view.ring, &["Opacity"]);
            return view.ring.SetOpacity(0.0);
        };
        // The stroke is drawn inside the mask, so the ring is the stop grown
        // by half of it, to be centred on its edge.
        let stroke = (self.scale * ui::FOCUS_STROKE).max(1.0);
        let rect = rect.outset(stroke / 2.0);
        let radius = radius + stroke / 2.0;
        if view.ring_radius.get() != radius {
            // The corners take exactly the radius, so a pill, whose radius is
            // half its height, doesn't have them overlap and squash.
            let side = (2.0 * radius).ceil() + 1.0;
            let mask = self
                .renderer
                .draw_rounded(side, side, radius, Some(stroke))?;
            view.ring_source.SetSurface(&mask)?;
            view.ring_grid.SetInsets(radius)?;
            view.ring_radius.set(radius);
        }
        let ring = &view.ring;
        if animate && self.ring_shown {
            anim.vector3(ring, "Offset", rect.at(), FOCUS_MS, 0, &anim.decelerate)?;
            anim.vector2(ring, "Size", rect.size(), FOCUS_MS)?;
        } else {
            Animator::stop(ring, &["Offset", "Size"]);
            ring.SetOffset(rect.at())?;
            ring.SetSize(rect.size())?;
        }
        self.ring_shown = true;
        anim.scalar(&view.ring, "Opacity", 1.0, HOVER_MS, 0, &anim.decelerate)
    }
}
