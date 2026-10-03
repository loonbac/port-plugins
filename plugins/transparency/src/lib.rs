//! Plugin de transparencia para PORT.
//!
//! Controla la opacidad del fondo de la ventana para que compositores
//! como Niri, Hyprland o Sway muestren el fondo o blur del escritorio.

use port_plugin_api::{AppearanceHook, Plugin};

/// Plugin que define la opacidad del fondo de la terminal.
#[derive(Debug, Clone, Copy)]
pub struct TransparencyPlugin {
    opacity: f32,
}

impl TransparencyPlugin {
    /// Crea una nueva instancia con un nivel de opacidad (0.0 ..= 1.0).
    pub fn new(opacity: f32) -> Self {
        Self {
            opacity: opacity.clamp(0.0, 1.0),
        }
    }

    /// Opacidad recomendada por defecto (85 %).
    pub fn default_opacity() -> f32 {
        0.85
    }
}

impl Default for TransparencyPlugin {
    fn default() -> Self {
        Self::new(Self::default_opacity())
    }
}

impl AppearanceHook for TransparencyPlugin {
    fn opacity(&self) -> Option<f32> {
        Some(self.opacity)
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
}
