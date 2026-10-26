//! Where a card's window goes: next to what it's for, in the middle of the
//! screen, or beside another window, and as it's zoomed.

use super::*;

/// Between the card and what it's for, and the card and the screen's
/// edges, in DIPs.
const ANCHOR_GAP: f32 = 10.0;
const SCREEN_GAP: f32 = 8.0;

/// Where the window goes for a card next to `anchor` inside the monitor's
/// `work` area, both in screen pixels, at `scale` pixels per DIP. Returns the
/// window rectangle and whether the card grows upwards from the anchor (when
/// there's more room above it than below, unless it must grow `downward`).
///
/// The window always has room for the tallest card plus the shadow margin,
/// so resizing never moves the window itself.
pub(super) fn placement(anchor: RECT, work: RECT, scale: f32, downward: bool) -> (RECT, bool) {
    let px = |dip: f32| (dip * scale).round() as i32;
    let (max_h, gap, edge) = (px(MAX_CARD_H), px(ANCHOR_GAP), px(SCREEN_GAP));

    let room_below = work.bottom - edge - (anchor.bottom + gap);
    let room_above = (anchor.top - gap) - (work.top + edge);
    let grow_up = !downward && room_below < max_h && room_above > room_below;
    let from = if grow_up {
        anchor.top - gap
    } else {
        anchor.bottom + gap
    };
    let window = window_for(anchor.left - px(PAD_X), from, grow_up, scale, work);
    (window, grow_up)
}

/// The window for a card at `s` pixels per DIP with its left edge at
/// `card_x`, and its bottom (growing up) or top at `from`, as near there as
/// the `work` area has room for the tallest card.
fn window_for(card_x: i32, from: i32, grow_up: bool, s: f32, work: RECT) -> RECT {
    let px = |dip: f32| (dip * s).round() as i32;
    let (card_w, max_h, m, edge) = (px(CARD_W), px(MAX_CARD_H), px(MARGIN), px(SCREEN_GAP));
    let card_x = card_x.clamp(
        work.left + edge,
        (work.right - edge - card_w).max(work.left + edge),
    );
    let window_h = max_h + 2 * m;
    let window_y = if grow_up {
        let card_bottom = from.min(work.bottom - edge).max(work.top + edge + max_h);
        card_bottom + m - window_h
    } else {
        let card_top = from.min(work.bottom - edge - max_h).max(work.top + edge);
        card_top - m
    };
    RECT {
        left: card_x - m,
        top: window_y,
        right: card_x + card_w + m,
        bottom: window_y + window_h,
    }
}

/// About half the empty search box's height, in DIPs: how far above the
/// screen's middle a centred one starts, so that it sits in the middle.
const CENTRED_RISE: f32 = 68.0;

/// An anchor that opens a search box in the middle of the screen `near`
/// (screen pixels) is on, for when there's no caret to open it at, on a
/// card zoomed to `zoom`.
pub fn centred_anchor(near: POINT, zoom: f32) -> RECT {
    let (work, scale) = monitor_at(near);
    centred_in(work, zoomed(work, scale, zoom))
}

/// An anchor that opens a card beside `window` (screen pixels), level with
/// its top: on its right, or on its left when there's no room there, for a
/// card zoomed to `zoom`.
pub fn anchor_beside(window: RECT, zoom: f32) -> RECT {
    let near = POINT {
        x: (window.left + window.right) / 2,
        y: window.top,
    };
    let (work, scale) = monitor_at(near);
    let s = zoomed(work, scale, zoom);
    let px = |dip: f32| (dip * s).round() as i32;
    let (card_w, gap) = (px(CARD_W), px(BESIDE_GAP));
    let card_x = if window.right + gap + card_w <= work.right - px(SCREEN_GAP) {
        window.right + gap
    } else {
        window.left - gap - card_w
    };
    // `placement` puts the card's top that far under the anchor, and its
    // left that far left of it.
    let bottom = window.top - px(ANCHOR_GAP);
    RECT {
        left: card_x + px(PAD_X),
        top: bottom - px(BESIDE_LINE),
        right: card_x + px(PAD_X) + 1,
        bottom,
    }
}

/// Between a window and a card opened beside it, and how tall the line the
/// card's anchored to is, in DIPs.
const BESIDE_GAP: f32 = 12.0;
const BESIDE_LINE: f32 = 18.0;

/// The scale of a card zoomed to `zoom` on a monitor of `scale` pixels per
/// DIP: no larger than fits the tallest card in the `work` area, unless
/// even the unzoomed one doesn't.
pub(super) fn zoomed(work: RECT, scale: f32, zoom: f32) -> f32 {
    let room_w = (work.right - work.left) as f32 / (CARD_W + 2.0 * SCREEN_GAP);
    let room_h = (work.bottom - work.top) as f32 / (MAX_CARD_H + 2.0 * SCREEN_GAP);
    (scale * zoom).min(room_w.min(room_h).max(scale))
}

/// Where the window at `window`, its card drawn at `old` pixels per DIP,
/// goes for the card at `s`: its left edge, and the edge it grows from,
/// stay put, as far as the `work` area has room for it.
pub(super) fn rescaled(window: RECT, grow_up: bool, old: f32, s: f32, work: RECT) -> RECT {
    let old_m = (MARGIN * old).round() as i32;
    let from = if grow_up {
        window.bottom - old_m
    } else {
        window.top + old_m
    };
    window_for(window.left + old_m, from, grow_up, s, work)
}

fn centred_in(work: RECT, s: f32) -> RECT {
    let px = |dip: f32| (dip * s).round() as i32;
    // `placement` puts the card's text at the anchor's left, and its top
    // just under the anchor.
    let left = (work.left + work.right) / 2 - px(CARD_W) / 2 + px(PAD_X);
    let bottom = (work.top + work.bottom) / 2 - px(CENTRED_RISE) - px(ANCHOR_GAP);
    RECT {
        left,
        top: bottom - 1,
        right: left + 1,
        bottom,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: RECT = RECT {
        left: 0,
        top: 0,
        right: 1920,
        bottom: 1040,
    };

    fn word_at(x: i32, y: i32) -> RECT {
        RECT {
            left: x,
            top: y,
            right: x + 60,
            bottom: y + 20,
        }
    }

    #[test]
    fn places_the_card_below_the_selection() {
        let anchor = word_at(500, 100);
        let (window, grow_up) = placement(anchor, SCREEN, 1.0, false);
        assert!(!grow_up);
        // The card starts ANCHOR_GAP below the selection, aligned with its text.
        assert_eq!(
            window.top + MARGIN as i32,
            anchor.bottom + ANCHOR_GAP as i32
        );
        assert_eq!(window.left + MARGIN as i32, anchor.left - PAD_X as i32);
        let height = MAX_CARD_H as i32 + 2 * MARGIN as i32;
        assert_eq!(window.bottom - window.top, height);
    }

    #[test]
    fn grows_up_near_the_bottom_of_the_screen() {
        let anchor = word_at(500, 950);
        let (window, grow_up) = placement(anchor, SCREEN, 1.0, false);
        assert!(grow_up);
        // The card's bottom edge sits ANCHOR_GAP above the selection.
        assert_eq!(
            window.bottom - MARGIN as i32,
            anchor.top - ANCHOR_GAP as i32
        );
    }

    #[test]
    fn grows_up_from_below_the_work_area() {
        // A selection in the taskbar: the card stays above it, on screen.
        let (window, grow_up) = placement(word_at(500, 1050), SCREEN, 1.0, false);
        assert!(grow_up);
        assert_eq!(
            window.bottom - MARGIN as i32,
            SCREEN.bottom - SCREEN_GAP as i32
        );
    }

    #[test]
    fn centres_a_search_card() {
        for scale in [1.0, 1.5, 2.0] {
            let (window, grow_up) = placement(centred_in(SCREEN, scale), SCREEN, scale, true);
            assert!(!grow_up);
            let m = (MARGIN * scale).round() as i32;
            let card_w = (CARD_W * scale).round() as i32;
            let card_left = window.left + m;
            let middle = (SCREEN.left + SCREEN.right) / 2;
            assert!((card_left + card_w / 2 - middle).abs() <= 1, "@{scale}");
            // The empty box (about twice CENTRED_RISE tall) straddles the
            // middle, unless that leaves no room below for the tallest card.
            let card_top = window.top + m;
            let rise = (CENTRED_RISE * scale).round() as i32;
            let tallest = (MAX_CARD_H * scale).round() as i32;
            let lowest = SCREEN.bottom - (SCREEN_GAP * scale).round() as i32 - tallest;
            let expected = ((SCREEN.top + SCREEN.bottom) / 2 - rise).min(lowest);
            assert!((card_top - expected).abs() <= 1, "@{scale}");
        }
    }

    #[test]
    fn zooms_only_as_far_as_the_card_fits() {
        assert_eq!(zoomed(SCREEN, 1.0, 1.5), 1.5);
        assert_eq!(zoomed(SCREEN, 1.5, 0.8), 1.2);
        // 1040 pixels tall: the tallest card fits up to about 2.2 times.
        let most = zoomed(SCREEN, 2.0, 2.0);
        assert!(most < 2.3 && most > 2.0, "{most}");
        assert!((MAX_CARD_H + 2.0 * SCREEN_GAP) * most <= 1040.0);
        // Too small a screen for the card unzoomed: as Windows has it.
        assert_eq!(zoomed(SCREEN, 3.0, 1.5), 3.0);
    }

    #[test]
    fn keeps_a_zoomed_card_in_place() {
        let (window, _) = placement(word_at(500, 100), SCREEN, 1.0, false);
        let larger = rescaled(window, false, 1.0, 1.5, SCREEN);
        let card = |w: RECT, s: f32| (w.left + (MARGIN * s) as i32, w.top + (MARGIN * s) as i32);
        assert_eq!(card(larger, 1.5), card(window, 1.0));
        assert_eq!(
            larger.right - larger.left,
            ((CARD_W + 2.0 * MARGIN) * 1.5) as i32
        );
        // Growing up, its bottom stays put.
        let (window, _) = placement(word_at(500, 950), SCREEN, 1.0, false);
        let larger = rescaled(window, true, 1.0, 1.5, SCREEN);
        assert_eq!(
            larger.bottom - (MARGIN * 1.5) as i32,
            window.bottom - MARGIN as i32
        );
        // Near an edge, it moves only as far as it must to fit.
        let (window, _) = placement(word_at(1700, 100), SCREEN, 1.0, false);
        let larger = rescaled(window, false, 1.0, 2.0, SCREEN);
        let edge = (SCREEN_GAP * 2.0) as i32;
        assert_eq!(larger.right - (MARGIN * 2.0) as i32, SCREEN.right - edge);
    }

    #[test]
    fn search_cards_grow_down() {
        let anchor = word_at(500, 950);
        let (window, grow_up) = placement(anchor, SCREEN, 1.0, true);
        assert!(!grow_up);
        // Still on screen, with room for the tallest card.
        assert_eq!(
            window.bottom - MARGIN as i32,
            SCREEN.bottom - SCREEN_GAP as i32
        );
    }

    #[test]
    fn stays_on_screen() {
        for scale in [1.0, 1.5, 2.0] {
            for (x, y) in [(-50, 10), (1900, 500), (0, 0), (1900, 1030)] {
                let (window, _) = placement(word_at(x, y), SCREEN, scale, false);
                let m = (MARGIN * scale).round() as i32;
                let edge = (SCREEN_GAP * scale).round() as i32;
                assert!(window.left + m >= SCREEN.left + edge, "{x},{y} @{scale}");
                assert!(window.right - m <= SCREEN.right - edge, "{x},{y} @{scale}");
                assert!(window.top + m >= SCREEN.top + edge, "{x},{y} @{scale}");
                assert!(
                    window.bottom - m <= SCREEN.bottom - edge,
                    "{x},{y} @{scale}"
                );
            }
        }
    }
}
