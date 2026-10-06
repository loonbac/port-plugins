//! Plugin de configuración de fuente para PORT.
//!
//! Permite personalizar la familia de fuente, el tamaño y las fuentes
//! de respaldo preferidas (ej. Nerd Fonts para iconos).
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
pub const PLUGIN_ID: &str = "font";

/// Familia tipográfica por defecto, la misma que asume PORT sin plugins.
pub const DEFAULT_FAMILY: &str = "FiraCode Nerd Font Mono";

/// Plugin para configurar la tipografía de la terminal.
pub struct FontPlugin {
    family: String,
    size: Option<f32>,
    fallbacks: Option<Vec<String>>,
}

impl FontPlugin {
    /// Crea un nuevo plugin con la familia de fuente indicada (ej. "FiraCode Nerd Font Mono").
    pub fn new(family: impl Into<String>) -> Self {
        Self {
            family: family.into(),
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

    /// Asigna la familia tipográfica activa.
    pub fn set_family(&mut self, family: impl Into<String>) {
        self.family = family.into();
    }

    /// Asigna el tamaño de fuente activo.
    pub fn set_size(&mut self, size: f32) {
        self.size = Some(size);
    }

    /// Familia tipográfica actual, con la forma que espera la apariencia.
    pub fn font_family(&self) -> Option<String> {
        Some(self.family.clone())
    }

    /// Tamaño de fuente actual, si se declaró.
    pub fn font_size(&self) -> Option<f32> {
        self.size
    }

    /// Fuentes de respaldo declaradas, si las hay.
    ///
    /// El protocolo externo todavía no tiene un campo para las fuentes de
    /// respaldo dentro de [`Appearance`]; el valor se conserva aquí para no
    /// perderlo y la prueba de contrato lo cubre.
    pub fn font_fallbacks(&self) -> Option<Vec<String>> {
        self.fallbacks.clone()
    }

    /// Identificador del plugin, para las pruebas de contrato.
    pub fn id(&self) -> &'static str {
        PLUGIN_ID
    }
}

impl Default for FontPlugin {
    fn default() -> Self {
        Self::new(DEFAULT_FAMILY)
    }
}

impl Plugin for FontPlugin {
    fn name(&self) -> &'static str {
        "Font Configuration"
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn capabilities(&self) -> Vec<Capability> {
        vec![Capability::Appearance]
    }

    fn appearance(&self) -> Appearance {
        Appearance {
            font_family: Some(self.family.clone()),
            font_size: self.size,
            ..Default::default()
        }
    }

    /// El núcleo todavía no empuja la configuración a los plugins externos;
    /// cuando lo haga, este es el punto de entrada. Las claves son las mismas
    /// que usaba `PluginConfig::get("family")` y `get_f32("size")`.
    fn configure(&mut self, values: &BTreeMap<String, String>) {
        if let Some(family) = values.get("family") {
            self.family = family.to_string();
        }
        if let Some(size) = values
            .get("size")
            .and_then(|v| v.trim().parse::<f32>().ok())
        {
            self.size = Some(size);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn los_valores_por_defecto_son_la_familia_de_port() {
        let plugin = FontPlugin::default();
        assert_eq!(plugin.font_family(), Some(DEFAULT_FAMILY.to_string()));
        assert_eq!(plugin.font_size(), None);
        assert_eq!(plugin.font_fallbacks(), None);
    }

    #[test]
    fn las_opciones_construyen_el_plugin() {
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
    fn la_configuracion_aplica_familia_y_tamano() {
        let mut plugin = FontPlugin::default();
        let mut values = BTreeMap::new();
        values.insert("family".to_string(), "DejaVu Sans Mono".to_string());
        values.insert("size".to_string(), "13.5".to_string());
        plugin.configure(&values);
        assert_eq!(plugin.font_family(), Some("DejaVu Sans Mono".to_string()));
        assert_eq!(plugin.font_size(), Some(13.5));
    }

    #[test]
    fn un_tamano_invalido_no_cambia_el_tamano() {
        let mut plugin = FontPlugin::default().with_size(14.0);
        let mut values = BTreeMap::new();
        values.insert("size".to_string(), "grande".to_string());
        plugin.configure(&values);
        assert_eq!(plugin.font_size(), Some(14.0));
    }

    #[test]
    fn el_plugin_declara_apariencia_e_identidad() {
        let plugin = FontPlugin::default().with_size(14.0);
        assert_eq!(plugin.id(), PLUGIN_ID);
        assert_eq!(plugin.name(), "Font Configuration");
        assert_eq!(plugin.capabilities(), vec![Capability::Appearance]);

        let appearance = plugin.appearance();
        assert_eq!(appearance.font_family, Some(DEFAULT_FAMILY.to_string()));
        assert_eq!(appearance.font_size, Some(14.0));
    }
}
