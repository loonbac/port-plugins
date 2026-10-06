//! Plugin de zoom de fuente para PORT.
//!
//! Es dueño del tamaño de fuente vigente y de sus propios atajos: informa el
//! tamaño por la apariencia y responde a `zoom_in`, `zoom_out`, `reset` y
//! `size`. Las combinaciones que disparan esas acciones (`Ctrl + =`,
//! `Ctrl + -`, `Ctrl + 0`) se declaran en el saludo y son configurables desde
//! el bloque de configuración del plugin.
//!
//! Corre como proceso independiente: PORT lo arranca, lee su saludo y le envía
//! la configuración por el protocolo JSON Lines del SDK.

use std::collections::BTreeMap;
use std::sync::RwLock;

use port_plugin_sdk::protocol::{Appearance, Binding, Capability};
use port_plugin_sdk::runtime::Plugin;

/// Identificador con el que PORT registra el plugin.
///
/// Debe coincidir con `[package.metadata.port] id` del `Cargo.toml`; la prueba
/// de contrato lo comprueba leyendo el manifiesto.
pub const PLUGIN_ID: &str = "font-zoom";

/// Tamaño de fuente por defecto, en puntos.
pub const DEFAULT_SIZE: f32 = 14.0;

/// Paso de incremento/decremento por defecto, en puntos.
pub const DEFAULT_STEP: f32 = 1.0;

/// Límites por defecto del tamaño de fuente.
pub const DEFAULT_MIN_SIZE: f32 = 6.0;
pub const DEFAULT_MAX_SIZE: f32 = 72.0;

/// Plantillas de atajos por defecto, en el formato `modificador+tecla`.
pub const DEFAULT_ZOOM_IN_KEY: &str = "ctrl+=";
pub const DEFAULT_ZOOM_OUT_KEY: &str = "ctrl+-";
pub const DEFAULT_RESET_KEY: &str = "ctrl+0";

/// Convierte una plantilla `ctrl+shift+=` en el [`Binding`] del protocolo.
///
/// La última parte es la tecla; las anteriores son modificadores. Devuelve
/// `None` si la plantilla está vacía o trae un modificador desconocido, para
/// que una configuración inválida no borre un atajo ya válido.
fn parse_binding(pattern: &str, action: &str) -> Option<Binding> {
    let parts: Vec<&str> = pattern.split('+').map(str::trim).collect();
    let (key, modifiers) = parts.split_last()?;
    if key.is_empty() {
        return None;
    }

    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;
    for modifier in modifiers {
        match modifier.to_lowercase().as_str() {
            "ctrl" | "control" => ctrl = true,
            "alt" => alt = true,
            "shift" => shift = true,
            _ => return None,
        }
    }

    Some(Binding {
        key: key.to_string(),
        ctrl,
        alt,
        shift,
        action: action.to_string(),
    })
}

/// Plugin para zoom interactivo de tipografía.
pub struct FontZoomPlugin {
    base_size: RwLock<f32>,
    current_size: RwLock<f32>,
    step: RwLock<f32>,
    min_size: RwLock<f32>,
    max_size: RwLock<f32>,
    key_zoom_in: String,
    key_zoom_out: String,
    key_reset: String,
}

impl FontZoomPlugin {
    /// Crea una nueva instancia con el tamaño base deseado (ej. 14.0 pt).
    pub fn new(base_size: f32) -> Self {
        Self {
            base_size: RwLock::new(base_size),
            current_size: RwLock::new(base_size),
            step: RwLock::new(DEFAULT_STEP),
            min_size: RwLock::new(DEFAULT_MIN_SIZE),
            max_size: RwLock::new(DEFAULT_MAX_SIZE),
            key_zoom_in: DEFAULT_ZOOM_IN_KEY.to_string(),
            key_zoom_out: DEFAULT_ZOOM_OUT_KEY.to_string(),
            key_reset: DEFAULT_RESET_KEY.to_string(),
        }
    }

    /// Personaliza el paso de incremento/decremento en puntos.
    pub fn with_step(self, step: f32) -> Self {
        *self.step.write().unwrap() = step;
        self
    }

    /// Personaliza los límites mínimo y máximo.
    pub fn with_limits(self, min: f32, max: f32) -> Self {
        *self.min_size.write().unwrap() = min;
        *self.max_size.write().unwrap() = max;
        self
    }

    /// Obtiene el tamaño actual de fuente.
    pub fn size(&self) -> f32 {
        *self.current_size.read().unwrap()
    }

    /// Define directamente el tamaño actual.
    pub fn set_size(&self, size: f32) {
        let min = *self.min_size.read().unwrap();
        let max = *self.max_size.read().unwrap();
        let mut cur = self.current_size.write().unwrap();
        *cur = size.clamp(min, max);
    }

    /// Aumenta el tamaño en un paso.
    pub fn zoom_in(&self) -> f32 {
        let step = *self.step.read().unwrap();
        let max = *self.max_size.read().unwrap();
        let mut cur = self.current_size.write().unwrap();
        *cur = (*cur + step).min(max);
        *cur
    }

    /// Disminuye el tamaño en un paso.
    pub fn zoom_out(&self) -> f32 {
        let step = *self.step.read().unwrap();
        let min = *self.min_size.read().unwrap();
        let mut cur = self.current_size.write().unwrap();
        *cur = (*cur - step).max(min);
        *cur
    }

    /// Restablece el tamaño al valor base original.
    pub fn reset_zoom(&self) -> f32 {
        let base = *self.base_size.read().unwrap();
        let mut cur = self.current_size.write().unwrap();
        *cur = base;
        *cur
    }

    /// Identificador del plugin, para las pruebas de contrato.
    pub fn id(&self) -> &'static str {
        PLUGIN_ID
    }
}

impl Default for FontZoomPlugin {
    fn default() -> Self {
        Self::new(DEFAULT_SIZE)
    }
}

impl Plugin for FontZoomPlugin {
    fn name(&self) -> &'static str {
        "Font Zoom"
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn capabilities(&self) -> Vec<Capability> {
        vec![Capability::Appearance, Capability::Input]
    }

    /// El plugin resuelve sus propios atajos: no pasa por el puente de
    /// servicios del proceso de PORT, que no cruza el límite del protocolo.
    fn bindings(&self) -> Vec<Binding> {
        [
            (self.key_zoom_in.as_str(), "zoom_in"),
            (self.key_zoom_out.as_str(), "zoom_out"),
            (self.key_reset.as_str(), "reset"),
        ]
        .into_iter()
        .filter_map(|(pattern, action)| parse_binding(pattern, action))
        .collect()
    }

    fn appearance(&self) -> Appearance {
        Appearance {
            font_size: Some(self.size()),
            ..Default::default()
        }
    }

    /// Ejecuta las acciones que ya no se publican como servicio: el zoom vive
    /// dentro de este proceso, así que el núcleo le pregunta directamente.
    fn invoke(&self, action: &str, _params: &serde_json::Value) -> Option<serde_json::Value> {
        match action {
            "zoom_in" => Some(serde_json::json!(self.zoom_in())),
            "zoom_out" => Some(serde_json::json!(self.zoom_out())),
            "reset" => Some(serde_json::json!(self.reset_zoom())),
            "size" => Some(serde_json::json!(self.size())),
            _ => None,
        }
    }

    /// El núcleo todavía no empuja la configuración a los plugins externos;
    /// cuando lo haga, este es el punto de entrada. Las claves de tamaño son
    /// las mismas que usaba `PluginConfig` (`default_size`, `step`, `min_size`,
    /// `max_size`) y las de los atajos son `key_zoom_in`, `key_zoom_out` y
    /// `key_reset`.
    fn configure(&mut self, values: &BTreeMap<String, String>) {
        if let Some(size) = values
            .get("default_size")
            .and_then(|v| v.trim().parse::<f32>().ok())
        {
            *self.base_size.write().unwrap() = size;
            self.set_size(size);
        }
        if let Some(step) = values
            .get("step")
            .and_then(|v| v.trim().parse::<f32>().ok())
        {
            *self.step.write().unwrap() = step;
        }
        if let Some(min) = values
            .get("min_size")
            .and_then(|v| v.trim().parse::<f32>().ok())
        {
            *self.min_size.write().unwrap() = min;
        }
        if let Some(max) = values
            .get("max_size")
            .and_then(|v| v.trim().parse::<f32>().ok())
        {
            *self.max_size.write().unwrap() = max;
        }

        // Cada atajo solo se reemplaza si la plantilla es válida: un valor mal
        // escrito deja el anterior en pie en vez de dejar el zoom sin tecla.
        if let Some(pattern) = values.get("key_zoom_in") {
            if parse_binding(pattern, "zoom_in").is_some() {
                self.key_zoom_in = pattern.clone();
            }
        }
        if let Some(pattern) = values.get("key_zoom_out") {
            if parse_binding(pattern, "zoom_out").is_some() {
                self.key_zoom_out = pattern.clone();
            }
        }
        if let Some(pattern) = values.get("key_reset") {
            if parse_binding(pattern, "reset").is_some() {
                self.key_reset = pattern.clone();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_in_and_out_modifies_size() {
        let plugin = FontZoomPlugin::new(14.0);
        assert_eq!(plugin.size(), 14.0);

        assert_eq!(plugin.zoom_in(), 15.0);
        assert_eq!(plugin.zoom_in(), 16.0);
        assert_eq!(plugin.zoom_out(), 15.0);
        assert_eq!(plugin.reset_zoom(), 14.0);
    }

    #[test]
    fn zoom_clamps_to_limits() {
        let plugin = FontZoomPlugin::new(10.0).with_limits(8.0, 12.0);
        plugin.zoom_in(); // 11
        plugin.zoom_in(); // 12
        plugin.zoom_in(); // 12 (tope)
        assert_eq!(plugin.size(), 12.0);

        plugin.zoom_out(); // 11
        plugin.zoom_out(); // 10
        plugin.zoom_out(); // 9
        plugin.zoom_out(); // 8
        plugin.zoom_out(); // 8 (tope)
        assert_eq!(plugin.size(), 8.0);
    }

    #[test]
    fn el_plugin_declara_apariencia_y_atajos_por_defecto() {
        let plugin = FontZoomPlugin::new(14.0);
        assert_eq!(plugin.id(), PLUGIN_ID);
        assert_eq!(plugin.appearance().font_size, Some(14.0));
        assert_eq!(
            plugin.capabilities(),
            vec![Capability::Appearance, Capability::Input]
        );

        let bindings = plugin.bindings();
        assert_eq!(bindings.len(), 3, "zoom_in, zoom_out y reset");
        assert!(bindings[0].matches("=", true, false, false));
        assert_eq!(bindings[0].action, "zoom_in");
        assert!(bindings[1].matches("-", true, false, false));
        assert_eq!(bindings[1].action, "zoom_out");
        assert!(bindings[2].matches("0", true, false, false));
        assert_eq!(bindings[2].action, "reset");
    }

    #[test]
    fn la_configuracion_aplica_tamano_limites_y_atajos() {
        let mut plugin = FontZoomPlugin::new(14.0);
        let mut values = BTreeMap::new();
        values.insert("default_size".to_string(), "18".to_string());
        values.insert("step".to_string(), "2.0".to_string());
        values.insert("min_size".to_string(), "8".to_string());
        values.insert("max_size".to_string(), "30".to_string());
        values.insert("key_zoom_in".to_string(), "ctrl+shift+=".to_string());
        values.insert("key_reset".to_string(), "alt+r".to_string());
        plugin.configure(&values);

        assert_eq!(plugin.size(), 18.0);
        assert_eq!(plugin.reset_zoom(), 18.0);
        assert_eq!(plugin.zoom_in(), 20.0);
        assert_eq!(plugin.zoom_out(), 18.0);

        let bindings = plugin.bindings();
        assert!(bindings[0].matches("=", true, false, true));
        assert_eq!(bindings[0].action, "zoom_in");
        assert!(bindings[2].matches("r", false, true, false));
        assert_eq!(bindings[2].action, "reset");
        assert!(
            bindings[1].matches("-", true, false, false),
            "key_zoom_out conserva su valor por defecto"
        );
    }

    #[test]
    fn un_atajo_invalido_no_cambia_el_binding() {
        let mut plugin = FontZoomPlugin::new(14.0);
        let mut values = BTreeMap::new();
        values.insert("key_zoom_in".to_string(), "ctrl+nope+=".to_string());
        plugin.configure(&values);
        assert!(
            plugin.bindings()[0].matches("=", true, false, false),
            "una plantilla inválida deja el atajo por defecto"
        );
    }

    #[test]
    fn invoke_responde_las_cuatro_acciones() {
        let plugin = FontZoomPlugin::new(14.0);
        let params = serde_json::json!({});
        assert_eq!(
            plugin.invoke("zoom_in", &params),
            Some(serde_json::json!(15.0))
        );
        assert_eq!(
            plugin.invoke("zoom_out", &params),
            Some(serde_json::json!(14.0))
        );
        assert_eq!(
            plugin.invoke("size", &params),
            Some(serde_json::json!(14.0))
        );
        assert_eq!(
            plugin.invoke("reset", &params),
            Some(serde_json::json!(14.0))
        );
        assert_eq!(plugin.invoke("otra", &params), None);
    }
}
