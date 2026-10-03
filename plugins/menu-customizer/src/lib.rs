//! Plugin de personalización para el menú de gestión de plugins del core de PORT.
//!
//! Demuestra cómo cualquier funcionalidad central de la terminal puede ser parcheada
//! o reemplazada por completo desde un plugin desacoplado implementando [`PluginManagerHook`].

use std::sync::RwLock;

use gpui::{
    anchored, deferred, div, point, px, rgb, AnyElement, FontWeight, IntoElement, ParentElement,
    Styled,
};
use port_plugin_api::{Plugin, PluginConfig, PluginInfo, PluginManagerHook};

/// Plugin para personalizar el atajo, título y aspecto visual del gestor de plugins.
pub struct MenuCustomizerPlugin {
    title: RwLock<String>,
    shortcut: RwLock<String>,
    custom_theme: RwLock<bool>,
}

impl MenuCustomizerPlugin {
    /// Crea una nueva instancia con título y atajo por defecto.
    pub fn new() -> Self {
        Self {
            title: RwLock::new("Centro de Control de Extensiones".to_string()),
            shortcut: RwLock::new("ctrl+shift+l".to_string()),
            custom_theme: RwLock::new(false),
        }
    }

    /// Personaliza el título que se mostrará en el menú.
    pub fn with_title(self, title: impl Into<String>) -> Self {
        *self.title.write().unwrap() = title.into();
        self
    }

    /// Personaliza el atajo de apertura/cierre (ej. "ctrl+shift+p").
    pub fn with_shortcut(self, shortcut: impl Into<String>) -> Self {
        *self.shortcut.write().unwrap() = shortcut.into();
        self
    }

    /// Activa o desactiva el tema visual personalizado del plugin.
    pub fn with_custom_theme(self, enabled: bool) -> Self {
        *self.custom_theme.write().unwrap() = enabled;
        self
    }
}

impl Default for MenuCustomizerPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginManagerHook for MenuCustomizerPlugin {
    fn toggle_shortcut(&self) -> Option<&'static str> {
        let s = self.shortcut.read().unwrap();
        Some(Box::leak(s.clone().into_boxed_str()))
    }

    fn menu_title(&self) -> Option<&'static str> {
        let t = self.title.read().unwrap();
        Some(Box::leak(t.clone().into_boxed_str()))
    }

    fn render_menu(&self, plugins: &[PluginInfo], selected_index: usize) -> Option<AnyElement> {
        if !*self.custom_theme.read().unwrap() {
            return None;
        }

        // Tema personalizado con acentos púrpuras y tarjeta flotante estilizada
        let title = self.title.read().unwrap().clone();
        let modal_w = 560.0f32;

        let mut list = div().flex().flex_col().gap_1();
        for (i, p) in plugins.iter().enumerate() {
            let is_selected = i == selected_index;
            let (status_badge, status_color) = if p.enabled {
                ("⚡ ON", rgb(0x2ea043))
            } else {
                ("✕ OFF", rgb(0xda3633))
            };

            let row_bg = if is_selected {
                rgb(0x261c36) // Morado oscuro
            } else {
                rgb(0x13111c)
            };

            let row_border = if is_selected {
                rgb(0xa371f7) // Púrpura brillante
            } else {
                rgb(0x2d2640)
            };

            let item = div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .p_2()
                .rounded_md()
                .bg(row_bg)
                .border_1()
                .border_color(row_border)
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_size(px(12.0))
                                .font_weight(FontWeight::BOLD)
                                .text_color(status_color)
                                .child(status_badge),
                        )
                        .child(
                            div()
                                .text_size(px(13.0))
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(0xf0f6fc))
                                .child(p.name.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(12.0))
                                .text_color(rgb(0x8b949e))
                                .child(format!("({})", p.id)),
                        ),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(rgb(0xa371f7))
                        .child(format!("v{}", p.version)),
                );

            list = list.child(item);
        }

        let modal = div()
            .w(px(modal_w))
            .p_4()
            .rounded_lg()
            .bg(rgb(0x0e0d14))
            .border_1()
            .border_color(rgb(0x8957e5))
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(15.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(0xd2a8ff))
                            .child(format!("✨ {}", title)),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(0xa371f7))
                            .child("CUSTOM PATCHED"),
                    ),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(rgb(0x8b949e))
                    .child("[↑/↓] Navegar   [Espacio/Enter] Alternar   [Esc] Cerrar"),
            )
            .child(list);

        Some(
            deferred(
                anchored()
                    .position(point(px(100.0), px(80.0)))
                    .child(modal),
            )
            .priority(100)
            .into_any_element(),
        )
    }
}

impl Plugin for MenuCustomizerPlugin {
    fn id(&self) -> &'static str {
        "menu-customizer"
    }

    fn name(&self) -> &'static str {
        "Menu Customizer Patch"
    }

    fn version(&self) -> &'static str {
        "0.1.0"
    }

    fn plugin_manager_hook(&self) -> Option<&dyn PluginManagerHook> {
        Some(self)
    }

    fn default_config(&self) -> Option<PluginConfig> {
        let mut cfg = PluginConfig::new();
        cfg.set("title", self.title.read().unwrap().clone());
        cfg.set("shortcut", self.shortcut.read().unwrap().clone());
        cfg.set("custom_theme", *self.custom_theme.read().unwrap());
        Some(cfg)
    }

    fn load_config(&self, config: &PluginConfig) {
        if let Some(t) = config.get("title") {
            *self.title.write().unwrap() = t.to_string();
        }
        if let Some(s) = config.get("shortcut") {
            *self.shortcut.write().unwrap() = s.to_string();
        }
        if let Some(ct) = config.get_bool("custom_theme") {
            *self.custom_theme.write().unwrap() = ct;
        }
    }

    fn save_config(&self) -> Option<PluginConfig> {
        let mut cfg = PluginConfig::new();
        cfg.set("title", self.title.read().unwrap().clone());
        cfg.set("shortcut", self.shortcut.read().unwrap().clone());
        cfg.set("custom_theme", *self.custom_theme.read().unwrap());
        Some(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_customizer_hook_defaults() {
        let plugin = MenuCustomizerPlugin::default();
        assert_eq!(plugin.toggle_shortcut(), Some("ctrl+shift+l"));
        assert_eq!(
            plugin.menu_title(),
            Some("Centro de Control de Extensiones")
        );
        assert!(plugin.render_menu(&[], 0).is_none());
    }

    #[test]
    fn menu_customizer_config_load_and_save() {
        let plugin = MenuCustomizerPlugin::default();
        let mut cfg = PluginConfig::new();
        cfg.set("title", "Mi Gestor Personalizado");
        cfg.set("shortcut", "ctrl+shift+p");
        cfg.set("custom_theme", true);
        plugin.load_config(&cfg);

        assert_eq!(plugin.toggle_shortcut(), Some("ctrl+shift+p"));
        assert_eq!(plugin.menu_title(), Some("Mi Gestor Personalizado"));
        assert!(plugin.render_menu(&[], 0).is_some());
    }
}
