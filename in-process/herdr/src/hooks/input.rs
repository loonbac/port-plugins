//! Hook de entrada del plugin herdr.
//!
//! Única responsabilidad: traducir las teclas que recibe PORT en acciones de
//! herdr (pestañas, espacios y salida del visor de un subagente).

use port_plugin_api::{InputHook, KeyAction};
use port_term_core::input::Key;

use crate::HerdrPlugin;

impl InputHook for HerdrPlugin {
    fn on_key(&self, key: &Key) -> KeyAction {
        // Escape sin modificadores: salir del visor de la pestaña activa. En una
        // pestaña normal la tecla sigue su curso hacia el PTY.
        if !key.ctrl && !key.alt && !key.shift && key.key == "Escape" && self.request_viewer_close()
        {
            return KeyAction::Consume;
        }

        // Ctrl + Shift + T: crear nueva pestaña de terminal en el espacio activo
        if key.ctrl && key.shift && !key.alt && key.key.to_lowercase() == "t" {
            self.create_tab_in_active_space();
            return KeyAction::Consume;
        }

        // Alt + Flecha Izquierda: cambiar a la pestaña anterior dentro del espacio actual
        if key.alt
            && !key.ctrl
            && !key.shift
            && (key.key == "Left" || key.key == "left")
            && self.select_previous_tab()
        {
            return KeyAction::Consume;
        }

        // Alt + Flecha Derecha: cambiar a la pestaña siguiente dentro del espacio actual
        if key.alt
            && !key.ctrl
            && !key.shift
            && (key.key == "Right" || key.key == "right")
            && self.select_next_tab()
        {
            return KeyAction::Consume;
        }

        // Ctrl + Alt + T: crear un nuevo espacio aparte del actual
        if key.ctrl && key.alt && !key.shift && key.key.to_lowercase() == "t" {
            self.create_space_and_open_sidebar();
            return KeyAction::Consume;
        }

        // Ctrl + W: cerrar el visor activo o, si no lo hay, la pestaña activa.
        if key.ctrl
            && !key.alt
            && !key.shift
            && key.key.to_lowercase() == "w"
            && (self.request_viewer_close() || self.close_active_tab())
        {
            return KeyAction::Consume;
        }

        // Alt + 1..9: cambiar rápidamente de espacio
        if key.alt && !key.ctrl && !key.shift {
            if let Ok(num) = key.key.parse::<usize>() {
                if (1..=9).contains(&num) {
                    self.select_space(num - 1);
                    return KeyAction::Consume;
                }
            }
        }

        KeyAction::Pass
    }
}
