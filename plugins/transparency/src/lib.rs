//! Plugin de transparencia para PORT.
//!
//! Controla la opacidad del fondo de la ventana para que compositores
//! como Niri, Hyprland o Sway muestren el fondo o blur del escritorio.

use std::path::Path;
use std::sync::RwLock;

use port_plugin_api::{AppearanceHook, ConfigFile, Plugin, PluginConfig};

/// Plugin que define la opacidad del fondo de la terminal.
pub struct TransparencyPlugin {
    opacity: RwLock<f32>,
}

impl TransparencyPlugin {
    /// Crea una nueva instancia con un nivel de opacidad (0.0 ..= 1.0).
    pub fn new(opacity: f32) -> Self {
        Self {
            opacity: RwLock::new(opacity.clamp(0.0, 1.0)),
        }
    }

    /// Opacidad recomendada por defecto (85 %).
    pub fn default_opacity() -> f32 {
        0.85
    }

    /// Asigna una nueva opacidad.
    pub fn set_opacity(&self, opacity: f32) {
        *self.opacity.write().unwrap() = opacity.clamp(0.0, 1.0);
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

impl Default for TransparencyPlugin {
    fn default() -> Self {
        Self::new(Self::default_opacity())
    }
}

impl AppearanceHook for TransparencyPlugin {
    fn opacity(&self) -> Option<f32> {
        Some(*self.opacity.read().unwrap())
    }
}

impl Plugin for TransparencyPlugin {
    fn id(&self) -> &'static str {
        "transparency"
    }

    fn name(&self) -> &'static str {
        "Transparency"
    }

    fn version(&self) -> &'static str {
        "0.1.0"
    }

    fn appearance_hook(&self) -> Option<&dyn AppearanceHook> {
        Some(self)
    }

    fn default_config(&self) -> Option<PluginConfig> {
        let mut cfg = PluginConfig::new();
        cfg.set("opacity", Self::default_opacity());
        Some(cfg)
    }

    fn load_config(&self, config: &PluginConfig) {
        if let Some(op) = config.get_f32("opacity") {
            self.set_opacity(op);
        }
    }

    fn save_config(&self) -> Option<PluginConfig> {
        let mut cfg = PluginConfig::new();
        cfg.set("opacity", *self.opacity.read().unwrap());
        Some(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transparency_plugin_clamps_opacity() {
        let over = TransparencyPlugin::new(1.5);
        assert_eq!(over.opacity(), Some(1.0));

        let under = TransparencyPlugin::new(-0.2);
        assert_eq!(under.opacity(), Some(0.0));

        let normal = TransparencyPlugin::new(0.85);
        assert_eq!(normal.opacity(), Some(0.85));
    }

    #[test]
    fn transparency_plugin_metadata() {
        let plugin = TransparencyPlugin::default();
        assert_eq!(plugin.id(), "transparency");
        assert_eq!(plugin.name(), "Transparency");
        assert!(plugin.appearance_hook().is_some());
    }

    #[test]
    fn transparency_config_load_and_save() {
        let plugin = TransparencyPlugin::default();
        let mut cfg = PluginConfig::new();
        cfg.set("opacity", 0.70);
        plugin.load_config(&cfg);
        assert_eq!(plugin.opacity(), Some(0.70));
    }
}
