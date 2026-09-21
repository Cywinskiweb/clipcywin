//! Resolved colors for rendering.

use crate::settings::{AccentChoice, Appearance, Theme as ThemeChoice};
use crate::util::wide::WStr;
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Color {
        Color { r, g, b, a }
    }
    pub fn from_rgb_u32(rgb: u32) -> Color {
        Color::rgba(((rgb >> 16) & 0xff) as f32 / 255.0, ((rgb >> 8) & 0xff) as f32 / 255.0, (rgb & 0xff) as f32 / 255.0, 1.0)
    }
    pub fn with_alpha(self, a: f32) -> Color {
        Color { a, ..self }
    }
    pub fn mix(self, other: Color, t: f32) -> Color {
        Color::rgba(
            self.r + (other.r - self.r) * t,
            self.g + (other.g - self.g) * t,
            self.b + (other.b - self.b) * t,
            self.a + (other.a - self.a) * t,
        )
    }
    pub fn d2d(self) -> D2D1_COLOR_F {
        D2D1_COLOR_F { r: self.r, g: self.g, b: self.b, a: self.a }
    }
    pub fn luminance(self) -> f32 {
        0.2126 * self.r + 0.7152 * self.g + 0.0722 * self.b
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub dark: bool,
    pub bg: Color,
    pub bg_solid: Color,
    pub item_bg: Color,
    pub item_hover: Color,
    pub fg: Color,
    pub fg_secondary: Color,
    pub accent: Color,
    pub on_accent: Color,
    pub badge_bg: Color,
    pub badge_fg: Color,
    pub border: Color,
    pub danger: Color,
    pub radius: f32,
    pub opacity: f32,
}

fn reg_dword(subkey: &str, value: &str) -> Option<u32> {
    let mut data = 0u32;
    let mut size = 4u32;
    let ok = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            WStr::new(subkey).pcwstr(),
            WStr::new(value).pcwstr(),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut data as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    if ok.is_ok() {
        Some(data)
    } else {
        None
    }
}

pub fn system_dark() -> bool {
    reg_dword("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize", "AppsUseLightTheme").map(|v| v == 0).unwrap_or(true)
}

/// Windows accent color as 0xRRGGBB.
pub fn system_accent() -> u32 {
    // DWM AccentColor is stored as ABGR.
    match reg_dword("Software\\Microsoft\\Windows\\DWM", "AccentColor") {
        Some(abgr) => ((abgr & 0xff) << 16) | (abgr & 0xff00) | ((abgr >> 16) & 0xff),
        None => 0x0078d4,
    }
}

impl Theme {
    pub fn resolve(a: &Appearance) -> Theme {
        let dark = match a.theme {
            ThemeChoice::Dark => true,
            ThemeChoice::Light => false,
            ThemeChoice::System => system_dark(),
        };
        let accent_rgb = match a.accent {
            AccentChoice::Windows => system_accent(),
            AccentChoice::Custom(c) => c,
        };
        let accent = Color::from_rgb_u32(accent_rgb);
        let on_accent = if accent.luminance() > 0.6 { Color::rgba(0.05, 0.05, 0.05, 1.0) } else { Color::rgba(1.0, 1.0, 1.0, 1.0) };
        if dark {
            Theme {
                dark,
                bg: Color::rgba(0.11, 0.11, 0.12, a.opacity),
                bg_solid: Color::rgba(0.11, 0.11, 0.12, 1.0),
                item_bg: Color::rgba(1.0, 1.0, 1.0, 0.06),
                item_hover: Color::rgba(1.0, 1.0, 1.0, 0.12),
                fg: Color::rgba(0.95, 0.95, 0.96, 1.0),
                fg_secondary: Color::rgba(0.7, 0.7, 0.74, 1.0),
                accent,
                on_accent,
                badge_bg: Color::rgba(1.0, 1.0, 1.0, 0.14),
                badge_fg: Color::rgba(0.9, 0.9, 0.92, 1.0),
                border: Color::rgba(1.0, 1.0, 1.0, 0.1),
                danger: Color::rgba(0.95, 0.35, 0.35, 1.0),
                radius: a.corner_radius,
                opacity: a.opacity,
            }
        } else {
            Theme {
                dark,
                bg: Color::rgba(0.96, 0.96, 0.97, a.opacity),
                bg_solid: Color::rgba(0.96, 0.96, 0.97, 1.0),
                item_bg: Color::rgba(0.0, 0.0, 0.0, 0.05),
                item_hover: Color::rgba(0.0, 0.0, 0.0, 0.1),
                fg: Color::rgba(0.1, 0.1, 0.12, 1.0),
                fg_secondary: Color::rgba(0.4, 0.4, 0.45, 1.0),
                accent,
                on_accent,
                badge_bg: Color::rgba(0.0, 0.0, 0.0, 0.1),
                badge_fg: Color::rgba(0.15, 0.15, 0.18, 1.0),
                border: Color::rgba(0.0, 0.0, 0.0, 0.1),
                danger: Color::rgba(0.8, 0.2, 0.2, 1.0),
                radius: a.corner_radius,
                opacity: a.opacity,
            }
        }
    }
}
