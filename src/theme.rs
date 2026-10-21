//! Colors that follow the Windows light/dark setting and accent color.

use windows::core::w;
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::System::Registry::*;
use windows::UI::Color;
use windows::UI::ViewManagement::{UIColorType, UISettings};

use crate::config::ThemeMode;

#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub dark: bool,
    pub transparency: bool,
    pub accent: Color,
}

impl Theme {
    pub fn current(mode: ThemeMode) -> Theme {
        let dark = match mode {
            ThemeMode::Dark => true,
            ThemeMode::Light => false,
            ThemeMode::System => read_personalize_dword("AppsUseLightTheme").is_none_or(|v| v == 0),
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
        }
    }

    /// Color laid over the blurred backdrop.
    pub fn tint(&self) -> Color {
        match (self.dark, self.transparency) {
            (true, true) => argb(0xB8, 0x20, 0x20, 0x20),
            (true, false) => rgb(0x2B, 0x2B, 0x2B),
            (false, true) => argb(0xC4, 0xF6, 0xF6, 0xF6),
            (false, false) => rgb(0xF6, 0xF6, 0xF6),
        }
    }

    pub fn border(&self) -> Color {
        if self.dark {
            argb(0x26, 0xFF, 0xFF, 0xFF)
        } else {
            argb(0x1A, 0x00, 0x00, 0x00)
        }
    }

    /// The soft fill behind whatever is under the pointer.
    pub fn highlight(&self) -> Color {
        if self.dark {
            argb(0x17, 0xFF, 0xFF, 0xFF)
        } else {
            argb(0x0F, 0x00, 0x00, 0x00)
        }
    }

    /// The ring around what the keyboard is on, as on Windows 11 controls.
    pub fn focus(&self) -> Color {
        if self.dark {
            rgb(0xFF, 0xFF, 0xFF)
        } else {
            argb(0xE4, 0x00, 0x00, 0x00)
        }
    }

    /// The search field's background, as on a focused Windows 11 text box.
    pub fn field(&self) -> Color {
        if self.dark {
            argb(0xB3, 0x1E, 0x1E, 0x1E)
        } else {
            rgb(0xFF, 0xFF, 0xFF)
        }
    }

    /// A group of settings, a step up from the card it's on.
    pub fn group(&self) -> Color {
        if self.dark {
            argb(0x0D, 0xFF, 0xFF, 0xFF)
        } else {
            argb(0xB3, 0xFF, 0xFF, 0xFF)
        }
    }

    pub fn group_stroke(&self) -> Color {
        if self.dark {
            argb(0x33, 0x00, 0x00, 0x00)
        } else {
            argb(0x12, 0x00, 0x00, 0x00)
        }
    }

    /// The line between two settings in a group.
    pub fn separator(&self) -> Color {
        if self.dark {
            argb(0x15, 0xFF, 0xFF, 0xFF)
        } else {
            argb(0x0F, 0x00, 0x00, 0x00)
        }
    }

    /// The scrollbar's thumb.
    pub fn thumb(&self) -> Color {
        if self.dark {
            argb(0x60, 0xFF, 0xFF, 0xFF)
        } else {
            argb(0x50, 0x00, 0x00, 0x00)
        }
    }

    /// The line between groups of commands in the tray menu.
    pub fn menu_separator(&self) -> Color {
        if self.dark {
            argb(0x24, 0xFF, 0xFF, 0xFF)
        } else {
            argb(0x1A, 0x00, 0x00, 0x00)
        }
    }

    /// A button's face, as on Windows 11 controls.
    pub fn control(&self) -> Color {
        if self.dark {
            argb(0x0F, 0xFF, 0xFF, 0xFF)
        } else {
            argb(0xB3, 0xFF, 0xFF, 0xFF)
        }
    }

    pub fn control_stroke(&self) -> Color {
        if self.dark {
            argb(0x12, 0xFF, 0xFF, 0xFF)
        } else {
            argb(0x17, 0x00, 0x00, 0x00)
        }
    }

    /// The groove behind a switch that's off, or a choice of options.
    pub fn sunken(&self) -> Color {
        if self.dark {
            argb(0x1F, 0x00, 0x00, 0x00)
        } else {
            argb(0x0A, 0x00, 0x00, 0x00)
        }
    }

    /// The chosen one of a choice of options, raised out of the groove.
    pub fn raised(&self) -> Color {
        if self.dark {
            argb(0x1F, 0xFF, 0xFF, 0xFF)
        } else {
            rgb(0xFF, 0xFF, 0xFF)
        }
    }

    /// The outline of a switch that's off.
    pub fn strong_stroke(&self) -> Color {
        if self.dark {
            argb(0x8B, 0xFF, 0xFF, 0xFF)
        } else {
            argb(0x72, 0x00, 0x00, 0x00)
        }
    }

    /// A switch's knob, off and on.
    pub fn knob(&self, on: bool) -> Color {
        match (on, self.dark) {
            (false, true) => argb(0xC5, 0xFF, 0xFF, 0xFF),
            (false, false) => argb(0x9E, 0x00, 0x00, 0x00),
            (true, true) => rgb(0x00, 0x00, 0x00),
            (true, false) => rgb(0xFF, 0xFF, 0xFF),
        }
    }

    pub fn shadow_opacity(&self) -> f32 {
        if self.dark {
            0.55
        } else {
            0.22
        }
    }

    pub fn text(&self) -> D2D1_COLOR_F {
        self.ink(if self.dark { 0.96 } else { 0.90 })
    }

    pub fn text_secondary(&self) -> D2D1_COLOR_F {
        self.ink(if self.dark { 0.68 } else { 0.62 })
    }

    pub fn text_tertiary(&self) -> D2D1_COLOR_F {
        self.ink(if self.dark { 0.46 } else { 0.45 })
    }

    pub fn divider(&self) -> D2D1_COLOR_F {
        self.ink(if self.dark { 0.09 } else { 0.08 })
    }

    /// The edge around a language's flag, so its white parts don't melt
    /// into the card.
    pub fn flag_edge(&self) -> D2D1_COLOR_F {
        self.ink(if self.dark { 0.16 } else { 0.28 })
    }

    /// The cap behind a key's name.
    pub fn key(&self) -> D2D1_COLOR_F {
        self.ink(if self.dark { 0.08 } else { 0.05 })
    }

    pub fn accent_d2d(&self) -> D2D1_COLOR_F {
        to_d2d(self.accent)
    }

    /// Text that says something went wrong.
    pub fn critical(&self) -> D2D1_COLOR_F {
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
