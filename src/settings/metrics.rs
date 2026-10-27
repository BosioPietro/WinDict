//! The settings window's sizes, durations and glyphs.

use crate::{popup, ui};

// Geometry, in DIPs.
pub(super) const CARD_W: f32 = 460.0;
/// Where the heading, the section labels and the footer start.
pub(super) const LABEL_X: f32 = popup::PAD_X;
pub(super) const PAD_TOP: f32 = 18.0;
pub(super) const HEADER_GAP: f32 = 12.0;
pub(super) const CLOSE_SIZE: f32 = 32.0;
/// The close button's distance from the card's top and right edges.
pub(super) const CLOSE_INSET: f32 = 10.0;
pub(super) const CLOSE_RADIUS: f32 = 5.0;
pub(super) const GROUP_X: f32 = 16.0;
pub(super) const GROUP_W: f32 = CARD_W - 2.0 * GROUP_X;
pub(super) const GROUP_RADIUS: f32 = 8.0;
pub(super) const ROW_H: f32 = 60.0;
pub(super) const ROW_PAD: f32 = 16.0;
/// The icon before each name, and the space after it.
pub(super) const ICON_W: f32 = 20.0;
pub(super) const ICON_GAP: f32 = 16.0;
/// Where a row's name and description start.
pub(super) const TEXT_X: f32 = ROW_PAD + ICON_W + ICON_GAP;
/// Between a setting's name and its description.
pub(super) const DETAIL_GAP: f32 = 1.0;
/// Between a setting's description and its control.
pub(super) const CONTROL_GAP: f32 = 12.0;
pub(super) const SECTION_GAP: f32 = 20.0;
pub(super) const LABEL_GAP: f32 = 8.0;
pub(super) const FOOTER_GAP: f32 = 16.0;
pub(super) const PAD_BOTTOM: f32 = 18.0;
pub(super) const SCREEN_GAP: f32 = 16.0;
/// The tallest the card grows before it scrolls.
pub(super) const MAX_CARD_H: f32 = 580.0;
pub(super) const SWITCH_W: f32 = 40.0;
pub(super) const SWITCH_H: f32 = 20.0;
pub(super) const KNOB: f32 = 12.0;
/// The knob grows under the pointer, and stretches while pressed.
pub(super) const KNOB_HOVER: f32 = 14.0;
pub(super) const KNOB_PRESSED: f32 = 17.0;
pub(super) const SEGMENT_W: f32 = 70.0;
pub(super) const SEGMENT_H: f32 = 30.0;
pub(super) const SEGMENT_RADIUS: f32 = 6.0;
pub(super) const PILL_INSET: f32 = 3.0;
pub(super) const PILL_RADIUS: f32 = 4.0;
pub(super) const STEP_SIZE: f32 = 30.0;
pub(super) const STEP_RADIUS: f32 = 6.0;
pub(super) const VALUE_W: f32 = 40.0;
/// The box of the language WinDict is shown in.
pub(super) const INTERFACE_W: f32 = 150.0;
/// The text field.
pub(super) const FIELD_W: f32 = 190.0;
pub(super) const FIELD_H: f32 = 32.0;
pub(super) const FIELD_RADIUS: f32 = 4.0;
/// Between the field's sides and its text.
pub(super) const FIELD_PAD: f32 = 10.0;
/// The line along its bottom, which thickens into the accent while typing.
pub(super) const FIELD_LINE: f32 = 2.0;
pub(super) const CARET_W: f32 = 1.0;
/// Between a setting's name and the info symbol after it.
pub(super) const INFO_GAP: f32 = 6.0;
pub(super) const INFO_W: f32 = 16.0;
/// How wide the info tooltip grows before wrapping.
pub(super) const TIP_W: f32 = 300.0;
pub(super) const TIP_PAD: f32 = 10.0;
pub(super) const TIP_RADIUS: f32 = 6.0;
/// How far the tooltip's shadow spreads, and how far it drops.
pub(super) const TIP_SHADOW: f32 = 12.0;
pub(super) const TIP_SHADOW_DY: f32 = 3.0;
/// Between the info symbol and its tooltip.
pub(super) const TIP_GAP: f32 = 6.0;
/// How far the tooltip rises from as it appears.
pub(super) const TIP_DRIFT: f32 = 4.0;
/// The default languages' row: the room for its name and description above
/// the boxes, and under them.
pub(super) const TRANSLATE_PAD: f32 = 12.0;
pub(super) const TRANSLATE_HEAD: f32 = 64.0;
pub(super) const TRANSLATE_FOOT: f32 = 16.0;
/// The way of reading the selection's row: its name, then the options from
/// `SELECTION_HEAD` down, then two lines of what the chosen one does.
pub(super) const SELECTION_HEAD: f32 = 42.0;
pub(super) const SELECTION_GAP: f32 = 8.0;
pub(super) const SELECTION_H: f32 = 124.0;
/// Corner radii of the rings round the boxes and the swap button.
pub(super) const SELECT_RING_RADIUS: f32 = 8.0;
pub(super) const SWAP_RING_RADIUS: f32 = 6.0;
/// A language in the list under the languages row.
pub(super) const LANGUAGE_H: f32 = 40.0;
pub(super) const CHECK_SIZE: f32 = 20.0;
pub(super) const CHECK_RADIUS: f32 = 4.0;
/// Between a language's box and its name.
pub(super) const CHECK_GAP: f32 = 12.0;
/// The room for how a language's download is going, on its right.
pub(super) const STATUS_W: f32 = 260.0;
pub(super) const CHEVRON_W: f32 = 16.0;
/// How wide each control is, for the description beside it to stay clear.
pub(super) const SHORTCUT_W: f32 = 150.0;
pub(super) const EDIT_W: f32 = 16.0;
pub(super) const CAPS_GAP: f32 = 8.0;
pub(super) const FOCUS_OUTSET: f32 = 3.0;
/// The ring round the shortcut while it's being recorded.
pub(super) const RECORD_INSET: f32 = 3.0;
pub(super) const RECORD_RADIUS: f32 = 6.0;
pub(super) const RECORD_STROKE: f32 = 1.5;
/// How far parts rise from as they appear.
pub(super) const DRIFT: f32 = 8.0;
/// How far a number rolls as it changes.
pub(super) const ROLL: f32 = 14.0;
pub(super) const WHEEL_STEP: f32 = 48.0;

// Durations, in ms.
pub(super) const SHOW_MS: u64 = 360;
pub(super) const REVEAL_MS: u64 = 340;
pub(super) const REVEAL_DELAY_MS: u64 = 40;
pub(super) const REVEAL_STAGGER_MS: u64 = 30;
pub(super) const HOVER_MS: u64 = 100;
pub(super) const KNOB_MS: u64 = 300;
pub(super) const SWITCH_FILL_MS: u64 = 160;
pub(super) const PILL_MS: u64 = 340;
pub(super) const LABEL_MS: u64 = 160;
pub(super) const ROLL_MS: u64 = 260;
pub(super) const CAPS_MS: u64 = 280;
pub(super) const DETAIL_MS: u64 = 200;
pub(super) const FOCUS_MS: u64 = 200;
pub(super) const PRESS_MS: u64 = 90;
pub(super) const SCROLL_MS: u64 = 180;
/// The pointer rests on the info symbol this long before its tooltip shows.
pub(super) const TIP_DELAY_MS: u64 = 400;
pub(super) const TIP_MS: u64 = 160;
pub(super) const TIP_HIDE_MS: u64 = 100;
pub(super) const THEME_MS: u64 = 300;
/// How long the old view takes to fade away as the languages change.
pub(super) const SLIDE_FADE_MS: u64 = 180;
/// As long as the card takes to grow or shrink.
pub(super) const EXPAND_MS: u64 = ui::RESIZE_MS;
/// Once the card has shrunk, the window does.
pub(super) const SETTLE_MS: u32 = ui::RESIZE_MS as u32 + 60;
pub(super) const TIMER_SETTLE: usize = 1;
/// The next of the dots after "Waiting for Windows".
pub const TIMER_DOTS: usize = 2;
pub(super) const BREATHE_MS: u64 = 1600;

// Segoe Fluent Icons and Segoe MDL2 Assets.
pub(super) const CLOSE_GLYPH: &str = "\u{E8BB}";
pub(super) const EDIT_GLYPH: &str = "\u{E70F}";
pub(super) const MINUS_GLYPH: &str = "\u{E738}";
pub(super) const PLUS_GLYPH: &str = "\u{E710}";
pub(super) const CHEVRON_GLYPH: &str = "\u{E70D}";
pub(super) const CHECK_GLYPH: &str = "\u{E73E}";
pub(super) const INFO_GLYPH: &str = "\u{E946}";

pub(super) const EMAIL_PLACEHOLDER: &str = "name@example.com";

/// The longest an email address can be.
pub(super) const MAX_EMAIL_CHARS: usize = 254;
