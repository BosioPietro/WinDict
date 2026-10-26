//! Colors that follow the Windows light/dark setting and accent color,
//! or the colors of a contrast theme while one is on.

use windows::core::w;
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Registry::*;
use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows::Win32::UI::WindowsAndMessaging::{
    SystemParametersInfoW, SPI_GETHIGHCONTRAST, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
};
use windows::UI::Color;
use windows::UI::ViewManagement::{UIColorType, UISettings};

use crate::config::ThemeMode;

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Theme {
    pub dark: bool,
    pub transparency: bool,
    pub accent: Color,
    /// The contrast theme's colors, while one is on.
    pub contrast: Option<Contrast>,
}

/// A contrast theme's colors, as Windows has them: everything is drawn
/// solid in these, whatever the app's own theme.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Contrast {
    pub back: Color,
    pub text: Color,
    pub link: Color,
    pub gray: Color,
    /// What's chosen, and the text on it.
    pub chosen: Color,
    pub chosen_text: Color,
}

impl Contrast {
    fn current() -> Option<Contrast> {
        let mut hc = HIGHCONTRASTW {
            cbSize: std::mem::size_of::<HIGHCONTRASTW>() as u32,
            ..Default::default()
        };
        let on = unsafe {
            SystemParametersInfoW(
                SPI_GETHIGHCONTRAST,
                hc.cbSize,
                Some(&mut hc as *mut _ as *mut _),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
        }
        .is_ok()
            && hc.dwFlags.contains(HCF_HIGHCONTRASTON);
        let color = |index| {
            let c = unsafe { GetSysColor(index) };
            rgb(c as u8, (c >> 8) as u8, (c >> 16) as u8)
        };
        on.then(|| Contrast {
            back: color(COLOR_WINDOW),
            text: color(COLOR_WINDOWTEXT),
            link: color(COLOR_HOTLIGHT),
            gray: color(COLOR_GRAYTEXT),
            chosen: color(COLOR_HIGHLIGHT),
            chosen_text: color(COLOR_HIGHLIGHTTEXT),
        })
    }
}

impl Theme {
    pub fn current(mode: ThemeMode) -> Theme {
        if let Some(c) = Contrast::current() {
            let (r, g, b) = (c.back.R as u32, c.back.G as u32, c.back.B as u32);
            return Theme {
                dark: 299 * r + 587 * g + 114 * b < 128_000,
                transparency: false,
                accent: c.chosen,
                contrast: Some(c),
            };
        }
        let dark = match mode {
            ThemeMode::Dark => true,
            ThemeMode::Light => false,
            ThemeMode::System => read_personalize_dword("AppsUseLightTheme") == Some(0),
        };
        let transparency = read_personalize_dword("EnableTransparency") != Some(0);
        let fallback = if dark {
            rgb(0x99, 0xEB, 0xFF)
        } else {
            rgb(0x00, 0x5F, 0xB8)
        };
        let accent = UISettings::new()
            .and_then(|s| {
                s.GetColorValue(if dark {
                    UIColorType::AccentLight2
                } else {
                    UIColorType::AccentDark1
                })
            })
            .unwrap_or(fallback);
        Theme {
            dark,
            transparency,
            accent,
            contrast: None,
        }
    }

    /// The theme to draw on the accent in: the other one's colors, or a
    /// contrast theme's for what's chosen.
    pub fn on_accent(&self) -> Theme {
        Theme {
            dark: !self.dark,
            contrast: self.contrast.map(|c| Contrast {
                text: c.chosen_text,
                ..c
            }),
            ..*self
        }
    }

    /// Color laid over the blurred backdrop.
    pub fn tint(&self) -> Color {
        if let Some(c) = self.contrast {
            return c.back;
        }
        match (self.dark, self.transparency) {
            (true, true) => argb(0xB8, 0x20, 0x20, 0x20),
            (true, false) => rgb(0x2B, 0x2B, 0x2B),
            (false, true) => argb(0xC4, 0xF6, 0xF6, 0xF6),
            (false, false) => rgb(0xF6, 0xF6, 0xF6),
        }
    }

    pub fn border(&self) -> Color {
        if let Some(c) = self.contrast {
            return c.text;
        }
        if self.dark {
            argb(0x26, 0xFF, 0xFF, 0xFF)
        } else {
            argb(0x1A, 0x00, 0x00, 0x00)
        }
    }

    /// The soft fill behind whatever is under the pointer.
    pub fn highlight(&self) -> Color {
        if let Some(c) = self.contrast {
            return faded(c.text);
        }
        if self.dark {
            argb(0x17, 0xFF, 0xFF, 0xFF)
        } else {
            argb(0x0F, 0x00, 0x00, 0x00)
        }
    }

    /// Laid over the tint where something sits on the card: lighter in
    /// light mode, darker in dark mode.
    pub fn badge_shade(&self) -> Color {
        if let Some(c) = self.contrast {
            return c.back;
        }
        if self.dark {
            argb(0x33, 0x00, 0x00, 0x00)
        } else {
            argb(0x80, 0xFF, 0xFF, 0xFF)
        }
    }

    /// The ring around what the keyboard is on, as on Windows 11 controls.
    pub fn focus(&self) -> Color {
        if let Some(c) = self.contrast {
            return c.text;
        }
        if self.dark {
            rgb(0xFF, 0xFF, 0xFF)
        } else {
            argb(0xE4, 0x00, 0x00, 0x00)
        }
    }

    /// The search field's background, as on a focused Windows 11 text box.
    pub fn field(&self) -> Color {
        if let Some(c) = self.contrast {
            return c.back;
        }
        if self.dark {
            argb(0xB3, 0x1E, 0x1E, 0x1E)
        } else {
            rgb(0xFF, 0xFF, 0xFF)
        }
    }

    /// A group of settings, a step up from the card it's on.
    pub fn group(&self) -> Color {
        if let Some(c) = self.contrast {
            return c.back;
        }
        if self.dark {
            argb(0x0D, 0xFF, 0xFF, 0xFF)
        } else {
            argb(0xB3, 0xFF, 0xFF, 0xFF)
        }
    }

    pub fn group_stroke(&self) -> Color {
        if let Some(c) = self.contrast {
            return c.text;
        }
        if self.dark {
            argb(0x33, 0x00, 0x00, 0x00)
        } else {
            argb(0x12, 0x00, 0x00, 0x00)
        }
    }

    /// The line between two settings in a group.
    pub fn separator(&self) -> Color {
        if let Some(c) = self.contrast {
            return c.gray;
        }
        if self.dark {
            argb(0x15, 0xFF, 0xFF, 0xFF)
        } else {
            argb(0x0F, 0x00, 0x00, 0x00)
        }
    }

    /// The scrollbar's thumb.
    pub fn thumb(&self) -> Color {
        if let Some(c) = self.contrast {
            return c.text;
        }
        if self.dark {
            argb(0x60, 0xFF, 0xFF, 0xFF)
        } else {
            argb(0x50, 0x00, 0x00, 0x00)
        }
    }

    /// The line between groups of commands in the tray menu.
    pub fn menu_separator(&self) -> Color {
        if let Some(c) = self.contrast {
            return c.gray;
        }
        if self.dark {
            argb(0x24, 0xFF, 0xFF, 0xFF)
        } else {
            argb(0x1A, 0x00, 0x00, 0x00)
        }
    }

    /// A button's face, as on Windows 11 controls.
    pub fn control(&self) -> Color {
        if let Some(c) = self.contrast {
            return c.back;
        }
        if self.dark {
            argb(0x0F, 0xFF, 0xFF, 0xFF)
        } else {
            argb(0xB3, 0xFF, 0xFF, 0xFF)
        }
    }

    pub fn control_stroke(&self) -> Color {
        if let Some(c) = self.contrast {
            return c.text;
        }
        if self.dark {
            argb(0x12, 0xFF, 0xFF, 0xFF)
        } else {
            argb(0x17, 0x00, 0x00, 0x00)
        }
    }

    /// The groove behind a switch that's off, or a choice of options.
    pub fn sunken(&self) -> Color {
        if let Some(c) = self.contrast {
            return c.back;
        }
        if self.dark {
            argb(0x1F, 0x00, 0x00, 0x00)
        } else {
            argb(0x0A, 0x00, 0x00, 0x00)
        }
    }

    /// The chosen one of a choice of options, raised out of the groove.
    pub fn raised(&self) -> Color {
        if let Some(c) = self.contrast {
            return faded(c.text);
        }
        if self.dark {
            argb(0x1F, 0xFF, 0xFF, 0xFF)
        } else {
            rgb(0xFF, 0xFF, 0xFF)
        }
    }

    /// The outline of a switch that's off.
    pub fn strong_stroke(&self) -> Color {
        if let Some(c) = self.contrast {
            return c.text;
        }
        if self.dark {
            argb(0x8B, 0xFF, 0xFF, 0xFF)
        } else {
            argb(0x72, 0x00, 0x00, 0x00)
        }
    }

    /// A switch's knob, off and on.
    pub fn knob(&self, on: bool) -> Color {
        if let Some(c) = self.contrast {
            return if on { c.chosen_text } else { c.text };
        }
        match (on, self.dark) {
            (false, true) => argb(0xC5, 0xFF, 0xFF, 0xFF),
            (false, false) => argb(0x9E, 0x00, 0x00, 0x00),
            (true, true) => rgb(0x00, 0x00, 0x00),
            (true, false) => rgb(0xFF, 0xFF, 0xFF),
        }
    }

    pub fn shadow_opacity(&self) -> f32 {
        if self.contrast.is_some() {
            return 0.0;
        }
        if self.dark {
            0.55
        } else {
            0.22
        }
    }

    pub fn text(&self) -> D2D1_COLOR_F {
        if let Some(c) = self.contrast {
            return to_d2d(c.text);
        }
        self.ink(if self.dark { 0.96 } else { 0.90 })
    }

    pub fn text_secondary(&self) -> D2D1_COLOR_F {
        if let Some(c) = self.contrast {
            return to_d2d(c.text);
        }
        self.ink(if self.dark { 0.68 } else { 0.62 })
    }

    pub fn text_tertiary(&self) -> D2D1_COLOR_F {
        if let Some(c) = self.contrast {
            return to_d2d(c.gray);
        }
        self.ink(if self.dark { 0.46 } else { 0.45 })
    }

    pub fn divider(&self) -> D2D1_COLOR_F {
        if let Some(c) = self.contrast {
            return to_d2d(c.gray);
        }
        self.ink(if self.dark { 0.09 } else { 0.08 })
    }

    /// The edge around a language's flag, so its white parts don't melt
    /// into the card.
    pub fn flag_edge(&self) -> D2D1_COLOR_F {
        if let Some(c) = self.contrast {
            return to_d2d(c.text);
        }
        self.ink(if self.dark { 0.16 } else { 0.28 })
    }

    /// The cap behind a key's name.
    pub fn key(&self) -> D2D1_COLOR_F {
        if let Some(c) = self.contrast {
            return to_d2d(faded(c.text));
        }
        self.ink(if self.dark { 0.08 } else { 0.05 })
    }

    pub fn accent_d2d(&self) -> D2D1_COLOR_F {
        if let Some(c) = self.contrast {
            return to_d2d(c.link);
        }
        to_d2d(self.accent)
    }

    /// Text that says something went wrong.
    pub fn critical(&self) -> D2D1_COLOR_F {
        if let Some(c) = self.contrast {
            return to_d2d(c.text);
        }
        to_d2d(if self.dark {
            rgb(0xFF, 0x99, 0xA4)
        } else {
            rgb(0xC4, 0x2B, 0x1C)
        })
    }

    fn ink(&self, a: f32) -> D2D1_COLOR_F {
        let v = if self.dark { 1.0 } else { 0.0 };
        D2D1_COLOR_F {
            r: v,
            g: v,
            b: v,
            a,
        }
    }
}

/// `c` thinned, for a fill that shows what's on it.
fn faded(c: Color) -> Color {
    Color { A: 0x40, ..c }
}

pub fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color {
        A: 0xFF,
        R: r,
        G: g,
        B: b,
    }
}

pub fn argb(a: u8, r: u8, g: u8, b: u8) -> Color {
    Color {
        A: a,
        R: r,
        G: g,
        B: b,
    }
}

pub fn to_d2d(c: Color) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: c.R as f32 / 255.0,
        g: c.G as f32 / 255.0,
        b: c.B as f32 / 255.0,
        a: c.A as f32 / 255.0,
    }
}

fn read_personalize_dword(name: &str) -> Option<u32> {
    let name = windows::core::HSTRING::from(name);
    let mut value = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            &name,
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        )
        .ok()
        .ok()?;
    }
    Some(value)
}
