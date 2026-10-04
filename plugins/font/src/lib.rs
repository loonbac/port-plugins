//! Plugin de configuración de fuente para PORT.
//!
//! Permite personalizar la familia de fuente, el tamaño y las fuentes
//! de respaldo preferidas (ej. Nerd Fonts para iconos).

use std::path::Path;
use std::sync::RwLock;

use port_plugin_api::{AppearanceHook, ConfigFile, Plugin, PluginConfig};

/// Plugin para configurar la tipografía de la terminal.
pub struct FontPlugin {
    family: RwLock<String>,
    size: Option<f32>,
    fallbacks: Option<Vec<String>>,
}

impl FontPlugin {
    /// Crea un nuevo plugin con la familia de fuente indicada (ej. "FiraCode Nerd Font Mono").
    pub fn new(family: impl Into<String>) -> Self {
        Self {
            family: RwLock::new(family.into()),
            size: None,
            fallbacks: None,
        }
    }

    /// Define el tamaño de fuente en puntos/píxeles lógicos.
    pub fn with_size(mut self, size: f32) -> Self {
        self.size = Some(size);
        self
    }

    /// Define la lista de fuentes de respaldo en orden de prioridad.
    pub fn with_fallbacks(mut self, fallbacks: Vec<String>) -> Self {
        self.fallbacks = Some(fallbacks);
        self
    }

    /// Guarda la configuración actual en una ruta concreta.
    pub fn save_to_file(&self, path: &Path) -> std::io::Result<()> {
        if let Some(config) = self.save_config() {
            ConfigFile::save_plugin(path, self.id(), &config)?;
        }
        Ok(())
    }

    /// Guarda la configuración actual en la ruta predeterminada (`~/.config/port/config.md`).
    pub fn save_to_default_file(&self) -> std::io::Result<()> {
        self.save_to_file(&ConfigFile::default_path())
    }
}

impl AppearanceHook for FontPlugin {
    fn font_family(&self) -> Option<String> {
        Some(self.family.read().unwrap().clone())
    }

    fn font_size(&self) -> Option<f32> {
        self.size
    }

    fn font_fallbacks(&self) -> Option<Vec<String>> {
        self.fallbacks.clone()
    }
}

impl Plugin for FontPlugin {
    fn id(&self) -> &'static str {
        "font"
    }

    fn name(&self) -> &'static str {
        "Font Configuration"
    }

    fn version(&self) -> &'static str {
        "0.1.0"
    }

    fn appearance_hook(&self) -> Option<&dyn AppearanceHook> {
        Some(self)
    }

    fn default_config(&self) -> Option<PluginConfig> {
        let mut cfg = PluginConfig::new();
        cfg.set("family", self.family.read().unwrap().clone());
        Some(cfg)
    }

    fn load_config(&self, config: &PluginConfig) {
        if let Some(family) = config.get("family") {
            *self.family.write().unwrap() = family.to_string();
        }
    }

    fn save_config(&self) -> Option<PluginConfig> {
        let mut cfg = PluginConfig::new();
        cfg.set("family", self.family.read().unwrap().clone());
        Some(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_plugin_defaults() {
        let plugin = FontPlugin::new("FiraCode Nerd Font Mono");
        assert_eq!(
            plugin.font_family(),
            Some("FiraCode Nerd Font Mono".to_string())
        );
        assert_eq!(plugin.font_size(), None);
        assert_eq!(plugin.font_fallbacks(), None);
    }

    #[test]
    fn font_plugin_with_options() {
        let plugin = FontPlugin::new("JetBrainsMono Nerd Font Mono")
            .with_size(15.0)
            .with_fallbacks(vec!["Symbols Nerd Font Mono".to_string()]);

        assert_eq!(
            plugin.font_family(),
            Some("JetBrainsMono Nerd Font Mono".to_string())
        );
        assert_eq!(plugin.font_size(), Some(15.0));
        assert_eq!(
            plugin.font_fallbacks(),
            Some(vec!["Symbols Nerd Font Mono".to_string()])
        );
    }

    #[test]
    fn font_config_load_and_save() {
        let plugin = FontPlugin::new("FiraCode Nerd Font Mono");
        let mut cfg = PluginConfig::new();
        cfg.set("family", "DejaVu Sans Mono");
        plugin.load_config(&cfg);
        assert_eq!(plugin.font_family(), Some("DejaVu Sans Mono".to_string()));
    }
}
