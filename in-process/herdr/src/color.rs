//! Helpers de color del plugin herdr.
//!
//! Única responsabilidad: traducir el acento del wallpaper de NixOS y las
//! cadenas hexadecimales al `Rgb` del núcleo, y convertirlos al `Hsla` que
//! pintan las barras.

use gpui::rgb;
use port_term_core::frame::Rgb;

/// Obtiene el color de acento del wallpaper activo en NixOS (`~/.config/mpvpaper/accent.txt`).
pub fn system_accent_color() -> Rgb {
    if let Ok(home) = std::env::var("HOME") {
        let path = std::path::Path::new(&home).join(".config/mpvpaper/accent.txt");
        if let Ok(content) = std::fs::read_to_string(path) {
            if let Some(rgb) = parse_hex_color(content.trim()) {
                return rgb;
            }
        }
    }
    // Color de acento por defecto de NixOS (#325573)
    Rgb::new(0x32, 0x55, 0x73)
}

/// Parsea una cadena hexadecimal en formato `#RRGGBB` o `RRGGBB`.
pub fn parse_hex_color(hex: &str) -> Option<Rgb> {
    let clean = hex.strip_prefix('#').unwrap_or(hex).trim();
    if clean.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&clean[0..2], 16).ok()?;
    let g = u8::from_str_radix(&clean[2..4], 16).ok()?;
    let b = u8::from_str_radix(&clean[4..6], 16).ok()?;
    Some(Rgb::new(r, g, b))
}

pub(crate) fn to_hsla(rgb_val: Rgb) -> gpui::Hsla {
    let packed = ((rgb_val.r as u32) << 16) | ((rgb_val.g as u32) << 8) | (rgb_val.b as u32);
    rgb(packed).into()
}
