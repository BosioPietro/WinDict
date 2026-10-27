//! A list that scrolls inside what scrolls: the languages a translation's
//! is picked from. Its rows are drawn once, all of them, and glide past a
//! clipped band as the card's content does, faded towards the ends more of
//! them are past, with a scrollbar of its own and a fill behind the row
//! Enter picks that fades from row to row.

use windows::core::{IUnknown, Interface, Result};
use windows::UI::Composition::*;

use super::{
    max_scroll, v2, v3, Animator, Push, ScrollFade, Scrollbar, Scroller, Text, Track, WHEEL_ROWS,
};
use crate::render::{Link, ListLayout, Mark, Renderer, Target};
use crate::theme::Theme;

/// Room above and below the thumb, in DIPs: it ends where the list's
/// corners start to curve.
const THUMB_ENDS: (f32, f32) = (4.0, 4.0);
/// How long a scroll by a notch of the wheel, or to show a row, takes, and
/// how long the fill takes to fade from one row to the next, in ms.
const SCROLL_MS: u64 = 140;
const FILL_MS: u64 = 100;

pub struct List {
    /// At the left of the list's outline, level with the top of the band
    /// its rows show through; everything else is in here.
    root: ContainerVisual,
    /// The clipped band the rows show through.
    host: ContainerVisual,
    fade: ScrollFade,
    /// What scrolls: the fills, and the rows over them.
    content: ContainerVisual,
    rows: Text,
    /// Behind the row Enter picks: two, so that one can fade out as the
    /// other fades in. `lit` is the one that's showing, or last showed.
    fills: [SpriteVisual; 2],
    lit: usize,
    fill_brush: CompositionColorBrush,
    fill_source: CompositionSurfaceBrush,
    fill_grid: CompositionNineGridBrush,
    bar: Scrollbar,
    thumb_brush: CompositionColorBrush,
    /// The row the fill is behind.
    highlight: Option<usize>,
    /// The rows' links, in DIPs from the first row's top left.
    links: Vec<Link>,
    /// The list's outline, and the band in it the rows show through, in
    /// DIPs in the parent; none while the list is hidden.
    frame: Option<(Mark, Mark)>,
    /// A row's height, in DIPs, and pixels per DIP.
    row_h: f32,
    s: f32,
    /// The height of all the rows, and of the band, in pixels.
    content_h: f32,
    viewport_h: f32,
    scroller: Scroller,
}

impl List {
    /// A hidden list, to be attached to what it shows in.
    pub fn new(c: &Compositor) -> Result<List> {
        let root = c.CreateContainerVisual()?;
        root.SetIsVisible(false)?;
        let host = c.CreateContainerVisual()?;
        host.SetClip(&c.CreateInsetClip()?)?;
        let content = c.CreateContainerVisual()?;
        host.Children()?.InsertAtTop(&content)?;

        // Drawn pixel for pixel and shown through a mask, as shapes are
        // jagged in what scrolls.
        let fill_brush = c.CreateColorBrush()?;
        let fill_source = c.CreateSurfaceBrush()?;
        fill_source.SetStretch(CompositionStretch::Fill)?;
        let fill_grid = c.CreateNineGridBrush()?;
        fill_grid.SetSource(&fill_source)?;
        let mask = c.CreateMaskBrush()?;
        mask.SetSource(&fill_brush)?;
        mask.SetMask(&fill_grid)?;
        let fill = || -> Result<SpriteVisual> {
            let fill = c.CreateSpriteVisual()?;
            fill.SetBrush(&mask)?;
            fill.SetOpacity(0.0)?;
            content.Children()?.InsertAtTop(&fill)?;
            Ok(fill)
        };
        let fills = [fill()?, fill()?];
        let rows = Text::new(c, &content)?;

        let fade = ScrollFade::new(c, &root, &host)?;
        let thumb_brush = c.CreateColorBrush()?;
        let bar = Scrollbar::new(c, &root, &thumb_brush.cast()?)?;
        Ok(List {
            root,
            host,
            fade,
            content,
            rows,
            fills,
            lit: 0,
            fill_brush,
            fill_source,
            fill_grid,
            bar,
            thumb_brush,
            highlight: None,
            links: Vec::new(),
            frame: None,
            row_h: 0.0,
            s: 1.0,
            content_h: 0.0,
            viewport_h: 0.0,
            scroller: Scroller::default(),
        })
    }

    /// Shows the list in `parent`, on top of what's there, taking it out of
    /// wherever it was.
    pub fn attach(&self, parent: &ContainerVisual) -> Result<()> {
        if let Ok(old) = self.root.Parent() {
            if &old == parent {
                return Ok(());
            }
            old.Children()?.Remove(&self.root)?;
        }
        parent.Children()?.InsertAtTop(&self.root)
    }

    /// Takes the list out of wherever it is, unless that's one of `keep`,
    /// as it must be before a view it's in is let go of: a visual goes
    /// closed with the tree it's in.
    pub fn detach_unless(&self, keep: &[ContainerVisual]) -> Result<()> {
        let Ok(parent) = self.root.Parent() else {
            return Ok(());
        };
        // The same visual can come back through another pointer: only
        // IUnknown tells them apart.
        let at: IUnknown = parent.cast()?;
        for v in keep {
            if v.cast::<IUnknown>()? == at {
                return Ok(());
            }
        }
        parent.Children()?.Remove(&self.root)
    }

    pub fn apply_theme(&self, t: &Theme) -> Result<()> {
        self.fill_brush.SetColor(t.highlight())?;
        self.thumb_brush.SetColor(t.thumb())
    }

    /// Shows `list` at `s` pixels per DIP. The same rows as before keep
    /// their place, gliding to show the row Enter picks, and the fill fades
    /// over to it; others start from the top, showing it at once.
    pub fn show(
        &mut self,
        anim: &Animator,
        renderer: &Renderer,
        theme: &Theme,
        list: &ListLayout,
        s: f32,
    ) -> Result<()> {
        let same = self.frame.is_some()
            && self.s == s
            && self.links.len() == list.rows.links.len()
            && self
                .links
                .iter()
                .zip(&list.rows.links)
                .all(|(a, b)| a.target == b.target);
        let (frame, viewport) = (list.frame, list.viewport);
        (self.frame, self.row_h, self.s) = (Some((frame, viewport)), list.row_h, s);
        self.links = list.rows.links.clone();
        let px = |dip: f32| (dip * s).round();
        self.content_h = self.rows.show(renderer, &list.rows, s, theme)?.Y;
        self.viewport_h = px(viewport.height);
        self.root.SetOffset(v3(px(frame.x), px(viewport.y)))?;
        self.root.SetSize(v2(px(frame.width), self.viewport_h))?;
        self.host.SetOffset(v3(px(viewport.x - frame.x), 0.0))?;
        self.host.SetSize(v2(px(viewport.width), self.viewport_h))?;
        self.fade.set_scale(&anim.compositor, s)?;

        let radius = list.radius * s;
        let side = (2.0 * radius).ceil() + 1.0;
        let mask = renderer.draw_rounded(side, side, radius, None)?;
        self.fill_source.SetSurface(&mask)?;
        self.fill_grid.SetInsets(radius)?;
        for fill in &self.fills {
            fill.SetSize(v2(px(viewport.width), px(list.row_h)))?;
        }
        Animator::stop(&self.root, &["Opacity"]);
        self.root.SetOpacity(1.0)?;
        self.root.SetIsVisible(true)?;

        if !same {
            self.scroller.stop_at(0.0);
            Animator::stop(&self.content, &["Offset"]);
            self.content.SetOffset(v3(0.0, 0.0))?;
            // Whatever the fills were behind is gone.
            for fill in &self.fills {
                Animator::stop(fill, &["Opacity"]);
                fill.SetOpacity(0.0)?;
            }
            self.highlight = None;
        }
        self.place_highlight(anim, list.highlight, same)?;
        if !self.reveal(anim, same)? {
            let running = self.scroller.running().is_some();
            self.show_scroll(anim, running)?;
        }
        Ok(())
    }

    /// Takes the list away, fading out over `ms`, or at once, letting go of
    /// its rows' surface, to start afresh when it next shows.
    pub fn hide(&mut self, anim: &Animator, ms: u64) -> Result<()> {
        if self.frame.take().is_none() {
            return Ok(());
        }
        if ms > 0 {
            anim.scalar(&self.root, "Opacity", 0.0, ms, 0, &anim.decelerate)?;
        } else {
            self.root.SetIsVisible(false)?;
            self.rows.clear();
        }
        self.bar.release();
        self.links.clear();
        self.highlight = None;
        self.scroller.stop_at(0.0);
        Ok(())
    }

    /// Puts the fill behind `row`, or takes it away, fading from where it
    /// was.
    pub fn set_highlight(&mut self, anim: &Animator, row: Option<usize>) -> Result<()> {
        self.place_highlight(anim, row, true)
    }

    /// Puts the fill behind `row`, or takes it away, fading from where it
    /// was with `animate`, or else at once: other rows may be where it is.
    fn place_highlight(
        &mut self,
        anim: &Animator,
        row: Option<usize>,
        animate: bool,
    ) -> Result<()> {
        if animate && row == self.highlight {
            return Ok(());
        }
        let was = std::mem::replace(&mut self.highlight, row);
        let show = |fill: &SpriteVisual, to: f32| {
            if animate {
                anim.scalar(fill, "Opacity", to, FILL_MS, 0, &anim.decelerate)
            } else {
                Animator::stop(fill, &["Opacity"]);
                fill.SetOpacity(to)
            }
        };
        if was.is_some() {
            show(&self.fills[self.lit], 0.0)?;
            self.lit = 1 - self.lit;
        }
        let Some(row) = row else {
            return Ok(());
        };
        let fill = &self.fills[self.lit];
        Animator::stop(fill, &["Opacity"]);
        fill.SetOpacity(0.0)?;
        fill.SetOffset(v3(0.0, (row as f32 * self.row_h * self.s).round()))?;
        show(fill, 1.0)
    }

    /// Scrolls just enough to show the row Enter picks, gliding there with
    /// `animate`. Returns whether it scrolled.
    fn reveal(&mut self, anim: &Animator, animate: bool) -> Result<bool> {
        let Some(row) = self.highlight else {
            return Ok(false);
        };
        let row_h = self.row_h * self.s;
        let top = (row as f32 * row_h).round();
        let bottom = (top + row_h).round();
        let to = if top < self.scroller.at {
            top
        } else if bottom > self.scroller.at + self.viewport_h {
            bottom - self.viewport_h
        } else {
            return Ok(false);
        };
        let ms = if animate { SCROLL_MS } else { 0 };
        self.glide_to(anim, to, Push::None, ms)?;
        Ok(true)
    }

    fn max_scroll(&self) -> f32 {
        max_scroll(self.content_h, self.viewport_h, self.s)
    }

    /// Scrolls by a mouse wheel delta, as far as a notch goes in the
    /// card's content, giving a little past the ends.
    pub fn wheel(&mut self, anim: &Animator, delta: i32) -> Result<()> {
        let max = self.max_scroll();
        if max <= 0.0 {
            return Ok(());
        }
        let notch = WHEEL_ROWS * self.row_h * self.s;
        match self.scroller.wheel(delta, notch, max, self.s, SCROLL_MS) {
            Some((to, push, ms)) => self.glide_to(anim, to, push, ms),
            None => Ok(()),
        }
    }

    /// Scrolls to `to`, pushed past it, over `ms`, from where the rows are
    /// on screen now.
    fn glide_to(&mut self, anim: &Animator, to: f32, push: Push, ms: u64) -> Result<()> {
        if to.round() == self.scroller.at && push == Push::None {
            return Ok(());
        }
        let glide = self
            .scroller
            .glide_to(to, push, self.max_scroll(), self.s, ms);
        glide.play(anim, &self.content, |scroll| v3(0.0, -scroll))?;
        self.show_scroll(anim, glide.running())
    }

    fn track(&self) -> Track {
        Track {
            viewport: self.viewport_h,
            content_h: self.content_h,
            max: self.max_scroll(),
            s: self.s,
            ends: THUMB_ENDS,
        }
    }

    /// The list's width, in pixels.
    fn width(&self) -> f32 {
        self.frame
            .map_or(0.0, |(frame, _)| (frame.width * self.s).round())
    }

    /// The fades at the ends, and the thumb, following the glide under way
    /// with `animate`.
    fn show_scroll(&self, anim: &Animator, animate: bool) -> Result<()> {
        let (max, s) = (self.max_scroll(), self.s);
        let thumb = self.bar.visual();
        self.fade.show(anim, self.scroller.at, max, s, animate)?;
        if max <= 0.0 {
            Animator::stop(thumb, &["Opacity"]);
            return thumb.SetOpacity(0.0);
        }
        let track = self.track();
        let (top, h) = track.thumb(self.scroller.at);
        let x = self.bar.size(self.width(), h, s)?;
        match self.scroller.running().filter(|_| animate) {
            Some(glide) => glide.play(anim, thumb, |scroll| v3(x, track.thumb(scroll).0))?,
            None => {
                Animator::stop(thumb, &["Offset"]);
                thumb.SetOffset(v3(x, top))?;
            }
        }
        Animator::stop(thumb, &["Opacity"]);
        thumb.SetOpacity(1.0)
    }

    // -----------------------------------------------------------------------
    // The pointer, at points in the parent's pixels

    /// A point from the list's root.
    fn local(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        let (frame, viewport) = self.frame?;
        let s = self.s;
        Some((x - (frame.x * s).round(), y - (viewport.y * s).round()))
    }

    /// Whether a point is on the list, outline and all.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        let Some((frame, _)) = self.frame else {
            return false;
        };
        let (x, y) = (x / self.s, y / self.s);
        x >= frame.x && x < frame.x + frame.width && y >= frame.y && y < frame.y + frame.height
    }

    /// The row under a point, where it'll be once the rows have glided to
    /// where they're going: off the scrollbar, and only in the band they
    /// show through.
    pub fn row_at(&self, x: f32, y: f32) -> Option<usize> {
        let (_, viewport) = self.frame?;
        let (x, y) = self.local(x, y)?;
        if Scrollbar::over(x, y, self.width(), self.track()) {
            return None;
        }
        let left = (viewport.x - self.frame?.0.x) * self.s;
        let inside = x >= left && x < left + viewport.width * self.s;
        if !inside || y < 0.0 || y >= self.viewport_h {
            return None;
        }
        let row = ((y + self.scroller.at) / (self.row_h * self.s)).floor() as usize;
        (row < self.links.len()).then_some(row)
    }

    /// What clicking the row under a point does.
    pub fn link_at(&self, x: f32, y: f32) -> Option<&Target> {
        self.row_at(x, y).map(|row| &self.links[row].target)
    }

    /// The left button went down at a point: on the scrollbar, it grabs the
    /// thumb, or pages towards the point. Returns whether it was on it.
    pub fn press(&mut self, anim: &Animator, x: f32, y: f32) -> Result<bool> {
        let track = self.track();
        let Some((x, y)) = self.local(x, y) else {
            return Ok(false);
        };
        if !Scrollbar::over(x, y, self.width(), track) {
            return Ok(false);
        }
        let scroll = self.scroller.visible().clamp(0.0, track.max);
        let page = self.bar.press(y, scroll, track);
        self.bar.hover(anim, true)?;
        if let Some(to) = page {
            self.glide_to(anim, to, Push::None, SCROLL_MS)?;
        }
        Ok(true)
    }

    /// The pointer at a point, or gone: widens the thumb over the scrollbar,
    /// and drags it while it's held. Returns whether the pointer's on the
    /// scrollbar, or dragging it.
    pub fn hover_bar(&mut self, anim: &Animator, point: Option<(f32, f32)>) -> Result<bool> {
        let track = self.track();
        let at = point.and_then(|(x, y)| self.local(x, y));
        let over = at.is_some_and(|(x, y)| Scrollbar::over(x, y, self.width(), track));
        self.bar.hover(anim, over)?;
        let to = at.and_then(|(_, y)| self.bar.drag(y, track));
        if let Some(to) = to {
            self.glide_to(anim, to, Push::None, 0)?;
        }
        Ok(over || self.bar.dragging())
    }

    /// The left button came up, or the window lost it. Returns whether it
    /// had gone down on the scrollbar.
    pub fn release(&mut self, anim: &Animator) -> Result<bool> {
        let held = self.bar.release();
        if held {
            self.bar.hover(anim, false)?;
        }
        Ok(held)
    }
}
