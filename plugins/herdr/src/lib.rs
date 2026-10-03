//! Herdr Customization Plugin para PORT.
//!
//! Transforma PORT en el entorno visual y de flujo de trabajo de Herdr:
//! - Barra lateral izquierda con lista interactiva de Espacios (Spaces) y panel de Agentes.
//! - Barra superior de pestañas encapsuladas (Tabs) con selector y botón de nueva pestaña (+).
//! - Estética cromática idéntica a Herdr (#0e0e16 terminal, #13131e sidebar y acentos violeta #231536).
//! - Configurable y sincronizado en tiempo real a través de `~/.config/port/config.md`.

use std::sync::{Arc, RwLock};

use gpui::prelude::*;
use gpui::{
    div, px, rgb, AnyElement, FontWeight, IntoElement, MouseButton, ParentElement, Styled, Window,
};
use port_plugin_api::{
    AppearanceHook, InputHook, KeyAction, LayoutHook, Plugin, PluginConfig,
};
use port_term_core::frame::Rgb;
use port_term_core::input::Key;

/// Definición de un espacio de trabajo en Herdr.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HerdrSpace {
    pub name: String,
    pub branch: String,
    pub color: Rgb,
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
    next_tab_id: usize,
}

impl Default for HerdrState {
    fn default() -> Self {
        Self {
            sidebar_open: true,
            active_space_index: 0,
            spaces: vec![
                HerdrSpace {
                    name: "herdr".to_string(),
                    branch: "master".to_string(),
                    color: Rgb::new(0xa3, 0x71, 0xf7), // Púrpura
                },
                HerdrSpace {
                    name: "web-dashboard".to_string(),
                    branch: "feature/new-charts".to_string(),
                    color: Rgb::new(0x58, 0xa6, 0xff), // Azul
                },
                HerdrSpace {
                    name: "data-pipeline".to_string(),
                    branch: "fix/kafka-event-v2".to_string(),
                    color: Rgb::new(0x3f, 0xb9, 0x50), // Verde
                },
            ],
            active_tab_index: 0,
            tabs: vec![HerdrTab {
                id: 1,
                title: "terminal".to_string(),
            }],
            opacity: 0.90,
            next_tab_id: 2,
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

    /// Alterna la visibilidad de la barra lateral de espacios.
    pub fn toggle_sidebar(&self) -> bool {
        let mut s = self.state.write().unwrap();
        s.sidebar_open = !s.sidebar_open;
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
}

impl Default for HerdrPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl AppearanceHook for HerdrPlugin {
    /// Fondo oscuro violeta característico de Herdr (#0e0e16).
    fn background_tint(&self, _base: Rgb) -> Rgb {
        Rgb::new(14, 14, 22)
    }

    fn opacity(&self) -> Option<f32> {
        Some(self.state.read().unwrap().opacity)
    }
}

impl LayoutHook for HerdrPlugin {
    fn left_sidebar_width(&self) -> f32 {
        if self.state.read().unwrap().sidebar_open {
            236.0
        } else {
            0.0
        }
    }

    fn top_bar_height(&self) -> f32 {
        38.0
    }

    fn left_sidebar(&self) -> Option<AnyElement> {
        let state = self.state.read().unwrap();
        if !state.sidebar_open {
            return None;
        }

        let active_space_idx = state.active_space_index;
        let spaces = state.spaces.clone();
        drop(state);

        let mut spaces_list = div().flex().flex_col().gap(px(4.0));

        for (i, space) in spaces.iter().enumerate() {
            let is_active = i == active_space_idx;
            let dot_color = rgb(
                (space.color.r as u32) << 16
                    | (space.color.g as u32) << 8
                    | (space.color.b as u32),
            );

            let (card_bg, card_border) = if is_active {
                (rgb(0x1e1e2d), rgb(0x3a2254)) // Cápsula activa resaltada
            } else {
                (rgb(0x13131e), rgb(0x13131e))
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
                            // Punto de color distintivo del espacio
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
                                        .text_color(rgb(0x8b949e))
                                        .child(space.branch.clone()),
                                ),
                        ),
                );

            spaces_list = spaces_list.child(item);
        }

        let sidebar = div()
            .w(px(236.0))
            .h_full()
            .bg(rgb(0x13131e))
            .border_r(px(1.0))
            .border_color(rgb(0x231536))
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
                                    .bg(rgb(0x1e1e2d))
                                    .text_size(px(10.0))
                                    .text_color(rgb(0xa371f7))
                                    .child(format!("{}", spaces.len())),
                            ),
                    )
                    .child(spaces_list),
            )
            .child(
                // Sección inferior: AGENTS de Herdr
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .pt(px(12.0))
                    .border_t(px(1.0))
                    .border_color(rgb(0x231536))
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
                            .bg(rgb(0x181825))
                            .border_1()
                            .border_color(rgb(0x2a1a40))
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
                                            .bg(rgb(0x3fb950)),
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
                                    .text_color(rgb(0x8b949e))
                                    .child("ready · idle"),
                            ),
                    ),
            );

        Some(sidebar.into_any_element())
    }

    fn top_bar(&self) -> Option<AnyElement> {
        let state = self.state.read().unwrap();
        let sidebar_open = state.sidebar_open;
        let active_tab_idx = state.active_tab_index;
        let tabs = state.tabs.clone();
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
                rgb(0x1e1e2d)
            } else {
                rgb(0x161622)
            })
            .border_1()
            .border_color(if sidebar_open {
                rgb(0x3a2254)
            } else {
                rgb(0x25143a)
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
                        rgb(0xa371f7)
                    } else {
                        rgb(0x6e7681)
                    }),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(if sidebar_open {
                        rgb(0xd2a8ff)
                    } else {
                        rgb(0x8b949e)
                    })
                    .child("spaces"),
            );

        let mut tabs_row = div().flex().flex_row().items_center().gap(px(6.0));

        for (i, tab) in tabs.iter().enumerate() {
            let is_active = i == active_tab_idx;
            let (bg_col, border_col) = if is_active {
                (rgb(0x181825), rgb(0x3a2254))
            } else {
                (rgb(0x12121d), rgb(0x1c1c2b))
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
                            rgb(0xa371f7)
                        } else {
                            rgb(0x6e7681)
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
                                s.tabs.remove(i);
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
            .bg(rgb(0x181825))
            .border_1()
            .border_color(rgb(0x2a1a40))
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
                    .text_color(rgb(0x8b949e))
                    .child("+"),
            );

        tabs_row = tabs_row.child(add_tab_btn);

        let top_bar = div()
            .h(px(38.0))
            .w_full()
            .bg(rgb(0x13131e))
            .border_b(px(1.0))
            .border_color(rgb(0x231536))
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
        // Ctrl+Shift+S: alternar barra lateral de espacios
        if key.ctrl && key.shift && key.key.to_lowercase() == "s" {
            self.toggle_sidebar();
            return KeyAction::Consume;
        }

        // Ctrl+T: nueva pestaña
        if key.ctrl && !key.alt && !key.shift && key.key.to_lowercase() == "t" {
            let next_id = {
                let s = self.state.read().unwrap();
                s.next_tab_id
            };
            self.new_tab(format!("term {next_id}"));
            return KeyAction::Consume;
        }

        // Ctrl+W: cerrar pestaña
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
        cfg.set("sidebar_open", true);
        cfg.set("active_space", "herdr");
        cfg.set("opacity", 0.90);
        cfg.set(
            "spaces",
            "herdr:master,web-dashboard:feature/new-charts,data-pipeline:fix/kafka-event-v2",
        );
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
        if let Some(spaces_str) = config.get("spaces") {
            let mut parsed_spaces = Vec::new();
            let colors = [
                Rgb::new(0xa3, 0x71, 0xf7),
                Rgb::new(0x58, 0xa6, 0xff),
                Rgb::new(0x3f, 0xb9, 0x50),
                Rgb::new(0xd2, 0x99, 0x22),
            ];
            for (idx, entry) in spaces_str.split(',').enumerate() {
                if let Some((name, branch)) = entry.trim().split_once(':') {
                    parsed_spaces.push(HerdrSpace {
                        name: name.trim().to_string(),
                        branch: branch.trim().to_string(),
                        color: colors[idx % colors.len()],
                    });
                }
            }
            if !parsed_spaces.is_empty() {
                s.spaces = parsed_spaces;
            }
        }
    }

    fn save_config(&self) -> Option<PluginConfig> {
        let s = self.state.read().unwrap();
        let mut cfg = PluginConfig::new();
        cfg.set("sidebar_open", s.sidebar_open);
        cfg.set("opacity", s.opacity);
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
    fn herdr_defaults() {
        let plugin = HerdrPlugin::default();
        assert_eq!(plugin.id(), "herdr");
        assert_eq!(plugin.name(), "Herdr Customization Plugin");
        assert_eq!(plugin.left_sidebar_width(), 236.0);
        assert_eq!(plugin.top_bar_height(), 38.0);
        assert!(plugin.left_sidebar().is_some());
        assert!(plugin.top_bar().is_some());
    }

    #[test]
    fn herdr_toggle_sidebar() {
        let plugin = HerdrPlugin::default();
        assert!(plugin.toggle_sidebar() == false);
        assert_eq!(plugin.left_sidebar_width(), 0.0);
        assert!(plugin.left_sidebar().is_none());

        assert!(plugin.toggle_sidebar() == true);
        assert_eq!(plugin.left_sidebar_width(), 236.0);
        assert!(plugin.left_sidebar().is_some());
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
    fn herdr_input_hook_shortcuts() {
        let plugin = HerdrPlugin::default();

        // Ctrl+Shift+S alterna sidebar
        let toggle = Key::new("s").ctrl().shift();
        assert_eq!(plugin.on_key(&toggle), KeyAction::Consume);
        assert_eq!(plugin.left_sidebar_width(), 0.0);

        // Alt+2 cambia al espacio 2
        let alt_2 = Key::new("2").alt();
        assert_eq!(plugin.on_key(&alt_2), KeyAction::Consume);
        assert_eq!(plugin.state.read().unwrap().active_space_index, 1);
    }
}
