//! Plugin de transparencia para PORT.
//!
//! Controla la opacidad del fondo de la ventana para que compositores
//! como Niri, Hyprland o Sway muestren el fondo o blur del escritorio.
//!
//! Corre como proceso independiente: PORT lo arranca, lee su saludo y le
//! envía la configuración por el protocolo JSON Lines del SDK.

use std::collections::BTreeMap;

use port_plugin_sdk::protocol::{Appearance, Capability};
use port_plugin_sdk::runtime::Plugin;

/// Identificador con el que PORT registra el plugin.
///
/// Debe coincidir con `[package.metadata.port] id` del `Cargo.toml`; la prueba
/// de contrato lo comprueba leyendo el manifiesto.
pub const PLUGIN_ID: &str = "transparency";

/// Plugin que define la opacidad del fondo de la terminal.
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

    /// Asigna una nueva opacidad.
    pub fn set_opacity(&mut self, opacity: f32) {
        self.opacity = opacity.clamp(0.0, 1.0);
    }

    /// Opacidad actual, con la forma que espera la apariencia del protocolo.
    pub fn opacity(&self) -> Option<f32> {
        Some(self.opacity)
    }

    /// Identificador del plugin, para las pruebas de contrato.
    pub fn id(&self) -> &'static str {
        PLUGIN_ID
    }
}

impl Default for TransparencyPlugin {
    fn default() -> Self {
        Self::new(Self::default_opacity())
    }
}

impl Plugin for TransparencyPlugin {
    fn name(&self) -> &'static str {
        "Transparency"
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn capabilities(&self) -> Vec<Capability> {
        vec![Capability::Appearance]
    }

    fn appearance(&self) -> Appearance {
        Appearance {
            opacity: Some(self.opacity),
            ..Default::default()
        }
    }

    /// El núcleo todavía no empuja la configuración a los plugins externos;
    /// cuando lo haga, este es el punto de entrada. Las claves son las mismas
    /// que usaba `PluginConfig::get_f32("opacity")`.
    fn configure(&mut self, values: &BTreeMap<String, String>) {
        if let Some(opacity) = values
            .get("opacity")
            .and_then(|v| v.trim().parse::<f32>().ok())
        {
            self.set_opacity(opacity);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_opacidad_se_limita_al_rango_valido() {
        let over = TransparencyPlugin::new(1.5);
        assert_eq!(over.opacity(), Some(1.0));

        let under = TransparencyPlugin::new(-0.2);
        assert_eq!(under.opacity(), Some(0.0));

        let normal = TransparencyPlugin::new(0.85);
        assert_eq!(normal.opacity(), Some(0.85));
    }

    #[test]
    fn la_opacidad_por_defecto_es_85_por_ciento() {
        let plugin = TransparencyPlugin::default();
        assert_eq!(plugin.opacity(), Some(0.85));
        assert_eq!(TransparencyPlugin::default_opacity(), 0.85);
    }

    #[test]
    fn la_configuracion_aplica_la_opacidad() {
        let mut plugin = TransparencyPlugin::default();
        let mut values = BTreeMap::new();
        values.insert("opacity".to_string(), "0.70".to_string());
        plugin.configure(&values);
        assert_eq!(plugin.opacity(), Some(0.70));
    }

    #[test]
    fn una_opacidad_invalida_no_cambia_el_valor() {
        let mut plugin = TransparencyPlugin::default();
        let mut values = BTreeMap::new();
        values.insert("opacity".to_string(), "no-es-un-numero".to_string());
        plugin.configure(&values);
        assert_eq!(plugin.opacity(), Some(0.85));
    }

    #[test]
    fn el_plugin_declara_apariencia_e_identidad() {
        let plugin = TransparencyPlugin::default();
        assert_eq!(plugin.id(), PLUGIN_ID);
        assert_eq!(plugin.name(), "Transparency");
        assert_eq!(plugin.capabilities(), vec![Capability::Appearance]);
        assert_eq!(plugin.appearance().opacity, Some(0.85));
    }
}
