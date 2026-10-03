//! Plugin de zoom y escalado interactivo de fuente para PORT.
//!
//! Permite aumentar, reducir y restablecer dinámicamente el tamaño de la
//! fuente mediante combinaciones estándar de teclado (`Ctrl++`, `Ctrl+-`, `Ctrl+0`).

use std::sync::RwLock;

use port_plugin_api::{AppearanceHook, InputHook, KeyAction, Plugin};
use port_term_core::input::Key;

/// Plugin para zoom interactivo de tipografía.
pub struct FontZoomPlugin {
    base_size: f32,
    current_size: RwLock<f32>,
    step: f32,
    min_size: f32,
    max_size: f32,
}

impl FontZoomPlugin {
    /// Crea una nueva instancia con el tamaño base deseado (ej. 14.0 pt).
    pub fn new(base_size: f32) -> Self {
        Self {
            base_size,
            current_size: RwLock::new(base_size),
            step: 1.0,
            min_size: 6.0,
            max_size: 72.0,
        }
    }

    /// Personaliza el paso de incremento/decremento en puntos.
    pub fn with_step(mut self, step: f32) -> Self {
        self.step = step;
        self
    }

    /// Personaliza los límites mínimo y máximo.
    pub fn with_limits(mut self, min: f32, max: f32) -> Self {
        self.min_size = min;
        self.max_size = max;
        self
    }

    /// Obtiene el tamaño actual de fuente.
    pub fn size(&self) -> f32 {
        *self.current_size.read().unwrap()
    }

    /// Aumenta el tamaño en un paso.
    pub fn zoom_in(&self) -> f32 {
        let mut cur = self.current_size.write().unwrap();
        *cur = (*cur + self.step).min(self.max_size);
        *cur
    }

    /// Disminuye el tamaño en un paso.
    pub fn zoom_out(&self) -> f32 {
        let mut cur = self.current_size.write().unwrap();
        *cur = (*cur - self.step).max(self.min_size);
        *cur
    }

    /// Restablece el tamaño al valor base original.
    pub fn reset_zoom(&self) -> f32 {
        let mut cur = self.current_size.write().unwrap();
        *cur = self.base_size;
        *cur
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

impl InputHook for FontZoomPlugin {
    fn on_key(&self, key: &Key) -> KeyAction {
        if !key.ctrl {
            return KeyAction::Pass;
        }

        // Ctrl + '+' o '=' (teclado numérico o fila de números)
        let is_plus = key.key == "+"
            || key.key == "="
            || key.text.as_deref() == Some("+")
            || key.text.as_deref() == Some("=");

        // Ctrl + '-' (menos o guion)
        let is_minus = key.key == "-" || key.text.as_deref() == Some("-");

        // Ctrl + '0' (restablecer)
        let is_zero = key.key == "0" || key.text.as_deref() == Some("0");

        if is_plus {
            self.zoom_in();
            KeyAction::Consume
        } else if is_minus {
            self.zoom_out();
            KeyAction::Consume
        } else if is_zero {
            self.reset_zoom();
            KeyAction::Consume
        } else {
            KeyAction::Pass
        }
    }
}

impl Plugin for FontZoomPlugin {
    fn id(&self) -> &'static str {
        "font-zoom"
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

    fn input_hook(&self) -> Option<&dyn InputHook> {
        Some(self)
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
    fn input_hook_consumes_zoom_keystrokes() {
        let plugin = FontZoomPlugin::new(14.0);

        // Ctrl + '+'
        let ctrl_plus = Key::new("+").ctrl();
        assert_eq!(plugin.on_key(&ctrl_plus), KeyAction::Consume);
        assert_eq!(plugin.size(), 15.0);

        // Ctrl + '='
        let ctrl_equal = Key::new("=").ctrl();
        assert_eq!(plugin.on_key(&ctrl_equal), KeyAction::Consume);
        assert_eq!(plugin.size(), 16.0);

        // Ctrl + '-'
        let ctrl_minus = Key::new("-").ctrl();
        assert_eq!(plugin.on_key(&ctrl_minus), KeyAction::Consume);
        assert_eq!(plugin.size(), 15.0);

        // Ctrl + '0'
        let ctrl_zero = Key::new("0").ctrl();
        assert_eq!(plugin.on_key(&ctrl_zero), KeyAction::Consume);
        assert_eq!(plugin.size(), 14.0);

        // Regular key (pass)
        let regular_a = Key::new("a").ctrl();
        assert_eq!(plugin.on_key(&regular_a), KeyAction::Pass);
        assert_eq!(plugin.size(), 14.0);
    }
}
