//! Herdr Customization Plugin para PORT.
//!
//! Transforma PORT en el entorno visual y de flujo de trabajo de Herdr:
//! - Inicio limpio: al abrir la terminal es 100 % terminal sin barras invasivas.
//! - Con `Ctrl + Alt + T` se crea un nuevo Espacio (Space) y aparece la barra lateral izquierda.
//! - Barra lateral y superior completamente transparentes, compartiendo la opacidad de la terminal.
//! - Color de acento sincronizado en tiempo real con el wallpaper de NixOS (`~/.config/mpvpaper/accent.txt`).
//! - Configurable y sincronizado a través de `~/.config/port/config.md`.

use std::sync::{Arc, RwLock};

use gpui::prelude::*;
use gpui::{
    div, px, rgb, AnyElement, FontWeight, IntoElement, MouseButton, ParentElement, Styled, Window,
};
use port_plugin_api::{
    AppearanceHook, ConfigFile, InputHook, KeyAction, LayoutHook, Plugin, PluginConfig,
};
use port_term_core::frame::Rgb;
use port_term_core::input::Key;

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

fn to_hsla(rgb_val: Rgb) -> gpui::Hsla {
    let packed =
        ((rgb_val.r as u32) << 16) | ((rgb_val.g as u32) << 8) | (rgb_val.b as u32);
    rgb(packed).into()
}

fn hsla_color(r: u8, g: u8, b: u8, a: f32) -> gpui::Hsla {
    let mut h: gpui::Hsla = rgb(((r as u32) << 16) | ((g as u32) << 8) | (b as u32)).into();
    h.a = a;
    h
}

/// Definición de un espacio de trabajo en Herdr.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HerdrSpace {
    pub name: String,
    pub branch: String,
    pub custom_color: Option<Rgb>,
}

/// Definición de una pestaña de terminal en Herdr.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HerdrTab {
    pub id: usize,
    pub title: String,
}

/// Estado interno del plugin Herdr.
#[derive(Debug, Clone)]
pub struct HerdrState {
    pub sidebar_open: bool,
    pub active_space_index: usize,
    pub spaces: Vec<HerdrSpace>,
    pub active_tab_index: usize,
    pub tabs: Vec<HerdrTab>,
    pub opacity: f32,
    pub accent_mode: String,
    next_tab_id: usize,
    next_space_num: usize,
}

impl HerdrState {
    /// Resuelve el color de acento actual (leyendo el wallpaper si está en modo "auto").
    pub fn effective_accent(&self) -> Rgb {
        if self.accent_mode.to_lowercase() == "auto" {
            system_accent_color()
        } else if let Some(rgb) = parse_hex_color(&self.accent_mode) {
            rgb
        } else {
            system_accent_color()
        }
    }
}

impl Default for HerdrState {
    fn default() -> Self {
        Self {
            // Inicialmente cerrado y sin spaces por defecto: la terminal abre pura y limpia
            sidebar_open: false,
            active_space_index: 0,
            spaces: Vec::new(),
            active_tab_index: 0,
            tabs: vec![HerdrTab {
                id: 1,
                title: "terminal".to_string(),
            }],
            opacity: 0.85,
            accent_mode: "auto".to_string(),
            next_tab_id: 2,
            next_space_num: 1,
        }
    }
}

/// Plugin de personalización visual y estructural Herdr.
pub struct HerdrPlugin {
    state: Arc<RwLock<HerdrState>>,
}

impl HerdrPlugin {
    /// Crea una nueva instancia del plugin Herdr con valores por defecto.
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(HerdrState::default())),
        }
    }

    /// Crea un nuevo espacio y abre de inmediato la barra lateral izquierda.
    pub fn create_space_and_open_sidebar(&self) {
        let mut s = self.state.write().unwrap();
        let num = s.next_space_num;
        s.next_space_num += 1;

        let name = format!("space-{num}");
        let branch = "main".to_string();

        s.spaces.push(HerdrSpace {
            name,
            branch,
            custom_color: None,
        });

        s.active_space_index = s.spaces.len() - 1;
        s.sidebar_open = true;
    }

    /// Alterna la visibilidad de la barra lateral de espacios.
    pub fn toggle_sidebar(&self) -> bool {
        let mut s = self.state.write().unwrap();
        if s.spaces.is_empty() {
            // Si no hay espacios creados aún, crear el primero y abrir
            let num = s.next_space_num;
            s.next_space_num += 1;
            s.spaces.push(HerdrSpace {
                name: format!("space-{num}"),
                branch: "main".to_string(),
                custom_color: None,
            });
            s.sidebar_open = true;
        } else {
            s.sidebar_open = !s.sidebar_open;
        }
        s.sidebar_open
    }

    /// Selecciona un espacio por índice.
    pub fn select_space(&self, index: usize) {
        let mut s = self.state.write().unwrap();
        if index < s.spaces.len() {
            s.active_space_index = index;
        }
    }

    /// Crea una nueva pestaña.
    pub fn new_tab(&self, title: impl Into<String>) -> usize {
        let mut s = self.state.write().unwrap();
        let id = s.next_tab_id;
        s.next_tab_id += 1;
        s.tabs.push(HerdrTab {
            id,
            title: title.into(),
        });
        s.active_tab_index = s.tabs.len() - 1;
        id
    }

    /// Cierra la pestaña activa si hay más de una.
    pub fn close_active_tab(&self) -> bool {
        let mut s = self.state.write().unwrap();
        if s.tabs.len() > 1 {
            let active_idx = s.active_tab_index;
            s.tabs.remove(active_idx);
            if s.active_tab_index >= s.tabs.len() {
                s.active_tab_index = s.tabs.len() - 1;
            }
            true
        } else {
            false
        }
    }

    /// Selecciona una pestaña por índice.
    pub fn select_tab(&self, index: usize) {
        let mut s = self.state.write().unwrap();
        if index < s.tabs.len() {
            s.active_tab_index = index;
        }
    }

    /// Guarda la configuración actual en la ruta predeterminada (`~/.config/port/config.md`).
    pub fn save_to_default_file(&self) -> std::io::Result<()> {
        if let Some(config) = self.save_config() {
            ConfigFile::save_plugin(&ConfigFile::default_path(), self.id(), &config)?;
        }
        Ok(())
    }
}

impl Default for HerdrPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl AppearanceHook for HerdrPlugin {
    /// Fondo oscuro característico de Herdr (#0e0e16).
    fn background_tint(&self, _base: Rgb) -> Rgb {
        Rgb::new(14, 14, 22)
    }

    fn opacity(&self) -> Option<f32> {
        Some(self.state.read().unwrap().opacity)
    }
}

impl LayoutHook for HerdrPlugin {
    fn left_sidebar_width(&self) -> f32 {
        let s = self.state.read().unwrap();
        if s.sidebar_open && !s.spaces.is_empty() {
            236.0
        } else {
            0.0
        }
    }

    fn top_bar_height(&self) -> f32 {
        let s = self.state.read().unwrap();
        // Solo ocupa altura si hay pestañas múltiples o si el panel de spaces está abierto
        if s.tabs.len() > 1 || (s.sidebar_open && !s.spaces.is_empty()) {
            38.0
        } else {
            0.0
        }
    }

    fn left_sidebar(&self) -> Option<AnyElement> {
        let state = self.state.read().unwrap();
        if !state.sidebar_open || state.spaces.is_empty() {
            return None;
        }

        let active_space_idx = state.active_space_index;
        let spaces = state.spaces.clone();
        let opacity = state.opacity;
        let accent = to_hsla(state.effective_accent());
        drop(state);

        let mut spaces_list = div().flex().flex_col().gap(px(4.0));

        for (i, space) in spaces.iter().enumerate() {
            let is_active = i == active_space_idx;
            let dot_color = space.custom_color.map(to_hsla).unwrap_or(accent);

            // Fondo y borde del espacio: transparente con tinte del acento si está activo
            let (card_bg, card_border) = if is_active {
                (accent.opacity(0.20), accent.opacity(0.55))
            } else {
                (hsla_color(0, 0, 0, 0.0), hsla_color(0, 0, 0, 0.0))
            };

            let state_for_click = Arc::clone(&self.state);
            let item = div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .px(px(10.0))
                .py(px(6.0))
                .rounded(px(6.0))
                .bg(card_bg)
                .border_1()
                .border_color(card_border)
                .on_mouse_down(MouseButton::Left, move |_event, window: &mut Window, _cx| {
                    let mut s = state_for_click.write().unwrap();
                    s.active_space_index = i;
                    window.refresh();
                })
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(10.0))
                        .child(
                            div()
                                .w(px(8.0))
                                .h(px(8.0))
                                .rounded(px(4.0))
                                .bg(dot_color),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .text_size(px(13.0))
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(if is_active {
                                            rgb(0xf0f6fc)
                                        } else {
                                            rgb(0xc9d1d9)
                                        })
                                        .child(space.name.clone()),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.0))
                                        .text_color(if is_active {
                                            accent
                                        } else {
                                            rgb(0x8b949e).into()
                                        })
                                        .child(space.branch.clone()),
                                ),
                        ),
                );

            spaces_list = spaces_list.child(item);
        }

        // Sidebar con fondo 100% transparente compartiendo la opacidad de la terminal
        let sidebar = div()
            .w(px(236.0))
            .h_full()
            .bg(hsla_color(17, 17, 26, opacity))
            .border_r(px(1.0))
            .border_color(accent.opacity(0.35))
            .p(px(12.0))
            .flex()
            .flex_col()
            .justify_between()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .px(px(6.0))
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(rgb(0x8b949e))
                                    .child("SPACES"),
                            )
                            .child(
                                div()
                                    .px(px(6.0))
                                    .py(px(1.0))
                                    .rounded(px(4.0))
                                    .bg(accent.opacity(0.20))
                                    .border_1()
                                    .border_color(accent.opacity(0.40))
                                    .text_size(px(10.0))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(accent)
                                    .child(format!("{}", spaces.len())),
                            ),
                    )
                    .child(spaces_list),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .pt(px(12.0))
                    .border_t(px(1.0))
                    .border_color(accent.opacity(0.25))
                    .child(
                        div()
                            .px(px(6.0))
                            .text_size(px(11.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(0x8b949e))
                            .child("AGENTS"),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .px(px(10.0))
                            .py(px(6.0))
                            .rounded(px(6.0))
                            .bg(accent.opacity(0.12))
                            .border_1()
                            .border_color(accent.opacity(0.30))
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap(px(8.0))
                                    .child(
                                        div()
                                            .w(px(6.0))
                                            .h(px(6.0))
                                            .rounded(px(3.0))
                                            .bg(accent),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.0))
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(rgb(0xf0f6fc))
                                            .child("herdr"),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(accent)
                                    .child("ready · idle"),
                            ),
                    ),
            );

        Some(sidebar.into_any_element())
    }

    fn top_bar(&self) -> Option<AnyElement> {
        let state = self.state.read().unwrap();
        // Si no hay pestañas múltiples ni sidebar de spaces activo, la barra superior no se dibuja
        if state.tabs.len() <= 1 && (!state.sidebar_open || state.spaces.is_empty()) {
            return None;
        }

        let sidebar_open = state.sidebar_open && !state.spaces.is_empty();
        let active_tab_idx = state.active_tab_index;
        let tabs = state.tabs.clone();
        let opacity = state.opacity;
        let accent = to_hsla(state.effective_accent());
        drop(state);

        let state_for_toggle = Arc::clone(&self.state);
        let toggle_btn = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .px(px(8.0))
            .py(px(4.0))
            .rounded(px(6.0))
            .bg(if sidebar_open {
                accent.opacity(0.20)
            } else {
                hsla_color(22, 22, 34, opacity)
            })
            .border_1()
            .border_color(if sidebar_open {
                accent.opacity(0.50)
            } else {
                accent.opacity(0.25)
            })
            .on_mouse_down(MouseButton::Left, move |_event, window: &mut Window, _cx| {
                let mut s = state_for_toggle.write().unwrap();
                s.sidebar_open = !s.sidebar_open;
                window.refresh();
            })
            .child(
                div()
                    .w(px(6.0))
                    .h(px(6.0))
                    .rounded(px(3.0))
                    .bg(if sidebar_open {
                        accent
                    } else {
                        rgb(0x6e7681).into()
                    }),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(if sidebar_open {
                        accent
                    } else {
                        rgb(0x8b949e).into()
                    })
                    .child("spaces"),
            );

        let mut tabs_row = div().flex().flex_row().items_center().gap(px(6.0));

        for (i, tab) in tabs.iter().enumerate() {
            let is_active = i == active_tab_idx;
            let (bg_col, border_col) = if is_active {
                (accent.opacity(0.22), accent.opacity(0.55))
            } else {
                (hsla_color(18, 18, 29, opacity), accent.opacity(0.20))
            };

            let state_for_click = Arc::clone(&self.state);
            let state_for_close = Arc::clone(&self.state);
            let tab_pill = div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .px(px(10.0))
                .py(px(4.0))
                .rounded(px(6.0))
                .bg(bg_col)
                .border_1()
                .border_color(border_col)
                .on_mouse_down(MouseButton::Left, move |_event, window: &mut Window, _cx| {
                    let mut s = state_for_click.write().unwrap();
                    s.active_tab_index = i;
                    window.refresh();
                })
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(if is_active {
                            accent
                        } else {
                            rgb(0x6e7681).into()
                        })
                        .child(">"),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .font_weight(if is_active {
                            FontWeight::BOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .text_color(if is_active {
                            rgb(0xf0f6fc)
                        } else {
                            rgb(0x8b949e)
                        })
                        .child(tab.title.clone()),
                )
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(rgb(0x6e7681))
                        .on_mouse_down(MouseButton::Left, move |_event, window: &mut Window, _cx| {
                            let mut s = state_for_close.write().unwrap();
                            if s.tabs.len() > 1 && i < s.tabs.len() {
                                let active_idx = s.active_tab_index;
                                s.tabs.remove(active_idx);
                                if s.active_tab_index >= s.tabs.len() {
                                    s.active_tab_index = s.tabs.len() - 1;
                                }
                                window.refresh();
                            }
                        })
                        .child("×"),
                );

            tabs_row = tabs_row.child(tab_pill);
        }

        let state_for_new = Arc::clone(&self.state);
        let add_tab_btn = div()
            .flex()
            .items_center()
            .justify_center()
            .w(px(24.0))
            .h(px(24.0))
            .rounded(px(4.0))
            .bg(accent.opacity(0.15))
            .border_1()
            .border_color(accent.opacity(0.35))
            .on_mouse_down(MouseButton::Left, move |_event, window: &mut Window, _cx| {
                let mut s = state_for_new.write().unwrap();
                let next_id = s.next_tab_id;
                s.next_tab_id += 1;
                s.tabs.push(HerdrTab {
                    id: next_id,
                    title: format!("term {}", next_id),
                });
                s.active_tab_index = s.tabs.len() - 1;
                window.refresh();
            })
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(accent)
                    .child("+"),
            );

        tabs_row = tabs_row.child(add_tab_btn);

        let top_bar = div()
            .h(px(38.0))
            .w_full()
            .bg(hsla_color(17, 17, 26, opacity))
            .border_b(px(1.0))
            .border_color(accent.opacity(0.30))
            .px(px(10.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(12.0))
            .child(toggle_btn)
            .child(tabs_row);

        Some(top_bar.into_any_element())
    }
}

impl InputHook for HerdrPlugin {
    fn on_key(&self, key: &Key) -> KeyAction {
        // Ctrl + Alt + T: crear un nuevo espacio y abrir de inmediato la barra lateral
        if key.ctrl && key.alt && !key.shift && key.key.to_lowercase() == "t" {
            self.create_space_and_open_sidebar();
            return KeyAction::Consume;
        }

        // Ctrl + Shift + S: alternar visibilidad de la barra lateral de espacios
        if key.ctrl && key.shift && !key.alt && key.key.to_lowercase() == "s" {
            self.toggle_sidebar();
            return KeyAction::Consume;
        }

        // Ctrl + T: nueva pestaña
        if key.ctrl && !key.alt && !key.shift && key.key.to_lowercase() == "t" {
            let next_id = {
                let s = self.state.read().unwrap();
                s.next_tab_id
            };
            self.new_tab(format!("term {next_id}"));
            return KeyAction::Consume;
        }

        // Ctrl + W: cerrar pestaña
        if key.ctrl && !key.alt && !key.shift && key.key.to_lowercase() == "w" {
            if self.close_active_tab() {
                return KeyAction::Consume;
            }
        }

        // Alt + 1..9: cambiar rápidamente de espacio
        if key.alt && !key.ctrl && !key.shift {
            if let Ok(num) = key.key.parse::<usize>() {
                if (1..=9).contains(&num) {
                    self.select_space(num - 1);
                    return KeyAction::Consume;
                }
            }
        }

        KeyAction::Pass
    }
}

impl Plugin for HerdrPlugin {
    fn id(&self) -> &'static str {
        "herdr"
    }

    fn name(&self) -> &'static str {
        "Herdr Customization Plugin"
    }

    fn version(&self) -> &'static str {
        "0.1.0"
    }

    fn appearance_hook(&self) -> Option<&dyn AppearanceHook> {
        Some(self)
    }

    fn layout_hook(&self) -> Option<&dyn LayoutHook> {
        Some(self)
    }

    fn input_hook(&self) -> Option<&dyn InputHook> {
        Some(self)
    }

    fn default_config(&self) -> Option<PluginConfig> {
        let mut cfg = PluginConfig::new();
        // Por defecto cerrado y sin espacios iniciales fijos
        cfg.set("sidebar_open", false);
        cfg.set("opacity", 0.85);
        cfg.set("accent", "auto");
        cfg.set("spaces", "");
        Some(cfg)
    }

    fn load_config(&self, config: &PluginConfig) {
        let mut s = self.state.write().unwrap();
        if let Some(open) = config.get_bool("sidebar_open") {
            s.sidebar_open = open;
        }
        if let Some(op) = config.get_f32("opacity") {
            s.opacity = op;
        }
        if let Some(acc) = config.get("accent") {
            s.accent_mode = acc.to_string();
        }
        if let Some(spaces_str) = config.get("spaces") {
            let mut parsed_spaces = Vec::new();
            for entry in spaces_str.split(',') {
                if let Some((name, branch)) = entry.trim().split_once(':') {
                    if !name.trim().is_empty() {
                        parsed_spaces.push(HerdrSpace {
                            name: name.trim().to_string(),
                            branch: branch.trim().to_string(),
                            custom_color: None,
                        });
                    }
                }
            }
            s.spaces = parsed_spaces;
        }
    }

    fn save_config(&self) -> Option<PluginConfig> {
        let s = self.state.read().unwrap();
        let mut cfg = PluginConfig::new();
        cfg.set("sidebar_open", s.sidebar_open);
        cfg.set("opacity", s.opacity);
        cfg.set("accent", s.accent_mode.clone());
        let spaces_str = s
            .spaces
            .iter()
            .map(|sp| format!("{}:{}", sp.name, sp.branch))
            .collect::<Vec<_>>()
            .join(",");
        cfg.set("spaces", spaces_str);
        Some(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_color_valid() {
        assert_eq!(
            parse_hex_color("#325573"),
            Some(Rgb::new(0x32, 0x55, 0x73))
        );
        assert_eq!(
            parse_hex_color("325573"),
            Some(Rgb::new(0x32, 0x55, 0x73))
        );
        assert_eq!(parse_hex_color("invalid"), None);
    }

    #[test]
    fn herdr_opens_clean_terminal_by_default() {
        let plugin = HerdrPlugin::default();
        assert_eq!(plugin.id(), "herdr");
        assert_eq!(plugin.name(), "Herdr Customization Plugin");
        // Al abrir la terminal por defecto es limpia: sin sidebar ni topbar
        assert_eq!(plugin.left_sidebar_width(), 0.0);
        assert_eq!(plugin.top_bar_height(), 0.0);
        assert!(plugin.left_sidebar().is_none());
        assert!(plugin.top_bar().is_none());
        assert_eq!(plugin.opacity(), Some(0.85));
    }

    #[test]
    fn ctrl_alt_t_creates_space_and_opens_sidebar() {
        let plugin = HerdrPlugin::default();
        assert_eq!(plugin.left_sidebar_width(), 0.0);

        // Pulsamos Ctrl+Alt+T
        let ctrl_alt_t = Key::new("t").ctrl().alt();
        assert_eq!(plugin.on_key(&ctrl_alt_t), KeyAction::Consume);

        // Ahora el sidebar está abierto y tiene ancho 236px
        assert_eq!(plugin.left_sidebar_width(), 236.0);
        assert!(plugin.left_sidebar().is_some());
        assert_eq!(plugin.top_bar_height(), 38.0);
        assert!(plugin.top_bar().is_some());

        // El primer espacio creado se llama space-1
        assert_eq!(plugin.state.read().unwrap().spaces.len(), 1);
        assert_eq!(plugin.state.read().unwrap().spaces[0].name, "space-1");
    }

    #[test]
    fn herdr_tabs_management() {
        let plugin = HerdrPlugin::default();
        let tab2 = plugin.new_tab("build");
        assert_eq!(tab2, 2);

        assert!(plugin.close_active_tab());
        assert!(!plugin.close_active_tab()); // Solo queda 1, no se cierra
    }

    #[test]
    fn herdr_accent_reads_system_color() {
        let plugin = HerdrPlugin::default();
        let accent = plugin.state.read().unwrap().effective_accent();
        assert_eq!(accent, system_accent_color());
    }
}
