//! Plugin de configuración de fuente para PORT.
//!
//! Permite personalizar la familia de fuente, el tamaño y las fuentes
//! de respaldo preferidas (ej. Nerd Fonts para iconos).

use port_plugin_api::{AppearanceHook, Plugin};

/// Plugin para configurar la tipografía de la terminal.
#[derive(Debug, Clone)]
pub struct FontPlugin {
    family: &'static str,
    size: Option<f32>,
    fallbacks: Option<Vec<String>>,
}

impl FontPlugin {
    /// Crea un nuevo plugin con la familia de fuente indicada (ej. "FiraCode Nerd Font Mono").
    pub fn new(family: &'static str) -> Self {
        Self {
            family,
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
}

impl AppearanceHook for FontPlugin {
    fn font_family(&self) -> Option<&'static str> {
        Some(self.family)
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_plugin_defaults() {
        let plugin = FontPlugin::new("FiraCode Nerd Font Mono");
        assert_eq!(plugin.font_family(), Some("FiraCode Nerd Font Mono"));
        assert_eq!(plugin.font_size(), None);
        assert_eq!(plugin.font_fallbacks(), None);
    }

    #[test]
    fn font_plugin_with_options() {
        let plugin = FontPlugin::new("JetBrainsMono Nerd Font Mono")
            .with_size(15.0)
            .with_fallbacks(vec!["Symbols Nerd Font Mono".to_string()]);

        assert_eq!(plugin.font_family(), Some("JetBrainsMono Nerd Font Mono"));
        assert_eq!(plugin.font_size(), Some(15.0));
        assert_eq!(
            plugin.font_fallbacks(),
            Some(vec!["Symbols Nerd Font Mono".to_string()])
        );
    }
}
