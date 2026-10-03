//! Plugin de zoom de fuente para PORT.
//!
//! Solo aporta la **capacidad**: cual es el tamano de fuente vigente, y como
//! aumentarlo, reducirlo o restablecerlo. No captura ninguna tecla.
//!
//! Las combinaciones que disparan esas acciones son responsabilidad del plugin
//! `shortcuts`, que registra los bindings que se quiera. Asi el zoom se puede
//! enlazar a otras teclas, o dejarlo sin atajo, sin tocar este plugin.

use std::path::Path;
use std::sync::{Arc, RwLock};

use port_plugin_api::{
    AppearanceHook, Arg, ConfigFile, Plugin, PluginConfig, Ret, Service, ServiceError,
};

/// Plugin para zoom interactivo de tipografía.
///
/// Es clonable: la app registra una copia en el registro de plugins y conserva
/// otra para enlazarla desde los atajos de `shortcuts`.
#[derive(Clone)]
pub struct FontZoomPlugin {
    base_size: Arc<RwLock<f32>>,
    current_size: Arc<RwLock<f32>>,
    step: Arc<RwLock<f32>>,
    min_size: Arc<RwLock<f32>>,
    max_size: Arc<RwLock<f32>>,
}

impl FontZoomPlugin {
    /// Crea una nueva instancia con el tamaño base deseado (ej. 14.0 pt).
    pub fn new(base_size: f32) -> Self {
        Self {
            base_size: Arc::new(RwLock::new(base_size)),
            current_size: Arc::new(RwLock::new(base_size)),
            step: Arc::new(RwLock::new(1.0)),
            min_size: Arc::new(RwLock::new(6.0)),
            max_size: Arc::new(RwLock::new(72.0)),
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

    /// Guarda la configuración actual en una ruta concreta de archivo de configuración.
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

impl Default for FontZoomPlugin {
    fn default() -> Self {
        Self::new(14.0)
    }
}

impl AppearanceHook for FontZoomPlugin {
    fn font_size(&self) -> Option<f32> {
        Some(self.size())
    }
}

/// Servicio que publica el zoom para que otros plugins lo invoquen.
struct FontZoomService(FontZoomPlugin);

impl Service for FontZoomService {
    fn id(&self) -> &str {
        "font-zoom"
    }

    fn name(&self) -> &str {
        "Font Zoom"
    }

    fn actions(&self) -> Vec<&'static str> {
        vec!["zoom_in", "zoom_out", "reset", "size"]
    }

    fn invoke(&self, action: &str, _args: &[Arg]) -> Option<Result<Ret, ServiceError>> {
        match action {
            "zoom_in" => Some(Ok(Ret::Num(self.0.zoom_in()))),
            "zoom_out" => Some(Ok(Ret::Num(self.0.zoom_out()))),
            "reset" => Some(Ok(Ret::Num(self.0.reset_zoom()))),
            "size" => Some(Ok(Ret::Num(self.0.size()))),
            _ => None,
        }
    }
}

impl Plugin for FontZoomPlugin {
    fn id(&self) -> &'static str {
        "font-zoom"
    }

    fn services(&self) -> Vec<Arc<dyn Service>> {
        vec![Arc::new(FontZoomService(self.clone()))]
    }

    fn name(&self) -> &'static str {
        "Font Zoom Shortcuts"
    }

    fn version(&self) -> &'static str {
        "0.1.0"
    }

    fn appearance_hook(&self) -> Option<&dyn AppearanceHook> {
        Some(self)
    }

    fn default_config(&self) -> Option<PluginConfig> {
        let mut config = PluginConfig::new();
        config.set("default_size", *self.base_size.read().unwrap());
        config.set("step", *self.step.read().unwrap());
        config.set("min_size", *self.min_size.read().unwrap());
        config.set("max_size", *self.max_size.read().unwrap());
        Some(config)
    }

    fn load_config(&self, config: &PluginConfig) {
        if let Some(size) = config.get_f32("default_size") {
            *self.base_size.write().unwrap() = size;
            self.set_size(size);
        }
        if let Some(step) = config.get_f32("step") {
            *self.step.write().unwrap() = step;
        }
        if let Some(min) = config.get_f32("min_size") {
            *self.min_size.write().unwrap() = min;
        }
        if let Some(max) = config.get_f32("max_size") {
            *self.max_size.write().unwrap() = max;
        }
    }

    fn save_config(&self) -> Option<PluginConfig> {
        let mut config = PluginConfig::new();
        config.set("default_size", self.size());
        config.set("step", *self.step.read().unwrap());
        config.set("min_size", *self.min_size.read().unwrap());
        config.set("max_size", *self.max_size.read().unwrap());
        Some(config)
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
        plugin.zoom_in(); // 12 (clamped)
        assert_eq!(plugin.size(), 12.0);

        plugin.zoom_out(); // 11
        plugin.zoom_out(); // 10
        plugin.zoom_out(); // 9
        plugin.zoom_out(); // 8
        plugin.zoom_out(); // 8 (clamped)
        assert_eq!(plugin.size(), 8.0);
    }

    #[test]
    fn zoom_operations_are_pure_state_changes() {
        // El plugin solo cambia su propio estado; la captura de teclas es
        // trabajo del plugin de atajos, asi que aqui no debe haber ningun hook
        // de entrada registrado.
        let plugin = FontZoomPlugin::new(14.0);
        assert!(
            plugin.input_hook().is_none(),
            "font-zoom no debe capturar teclas: eso es del plugin shortcuts"
        );

        plugin.zoom_in();
        assert_eq!(plugin.size(), 15.0);
        plugin.zoom_out();
        assert_eq!(plugin.size(), 14.0);
    }

    #[test]
    fn config_support_load_and_save() {
        let plugin = FontZoomPlugin::new(14.0);
        let default_cfg = plugin.default_config().unwrap();
        assert_eq!(default_cfg.get_f32("default_size"), Some(14.0));

        let mut custom_cfg = PluginConfig::new();
        custom_cfg.set("default_size", 18.0);
        plugin.load_config(&custom_cfg);
        assert_eq!(plugin.size(), 18.0);
        assert_eq!(plugin.reset_zoom(), 18.0);

        let saved = plugin.save_config().unwrap();
        assert_eq!(saved.get_f32("default_size"), Some(18.0));
    }
}
