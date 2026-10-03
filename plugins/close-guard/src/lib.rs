//! Plugin de protección de cierre para PORT.
//!
//! Al pedir cerrar la ventana, comprueba si hay programas ejecutándose en
//! primer plano dentro de las sesiones. Si los hay, cancela el cierre y muestra
//! un diálogo de confirmación con dos opciones: **Sí, cerrar** o **No, seguir**.
//!
//! Si no hay ningún programa corriendo, la terminal se cierra sin preguntar.

use std::sync::RwLock;

use port_plugin_api::{CloseDecision, LifecycleHook, Plugin, PluginConfig};

/// Nombres de los programas que se consideran parte de la terminal y no deben
/// bloquear el cierre: son el prompt o una utilidad trivial.
const IGNORED: &[&str] = &["true", "echo", "printf", "sleep"];

/// Plugin que protege frente a cierres accidentales con trabajo en curso.
#[derive(Default)]
pub struct CloseGuardPlugin {
    /// Si el diálogo de confirmación está visible.
    dialog_open: RwLock<bool>,
    /// Programas detectados en el momento de pedir el cierre.
    pending: RwLock<Vec<String>>,
    /// Recordatorio de cuántas veces se pidió confirmación, útil para pruebas.
    ask_count: RwLock<u32>,
}

impl CloseGuardPlugin {
    pub fn new() -> Self {
        Self::default()
    }

    /// `true` si el diálogo debe mostrarse.
    pub fn is_dialog_open(&self) -> bool {
        *self.dialog_open.read().unwrap()
    }

    /// Programas que se perderían al cerrar.
    pub fn pending_programs(&self) -> Vec<String> {
        self.pending.read().unwrap().clone()
    }

    /// `true` si el binario debe impedir cerrar la terminal.
    ///
    /// Cuenta como bloqueante todo menos los auxiliares inocuos de la lista.
    pub fn should_block(bin: &str) -> bool {
        !IGNORED.contains(&bin)
    }

    /// Registra una petición de cierre con los programas detectados.
    /// Devuelve `true` si hay que preguntar al usuario.
    pub fn request_close(&self, bins: Vec<String>) -> bool {
        let blocking: Vec<String> = bins
            .into_iter()
            .filter(|bin| Self::should_block(bin))
            .collect();

        if blocking.is_empty() {
            *self.dialog_open.write().unwrap() = false;
            *self.pending.write().unwrap() = Vec::new();
            return false;
        }

        *self.pending.write().unwrap() = blocking;
        *self.dialog_open.write().unwrap() = true;
        *self.ask_count.write().unwrap() += 1;
        true
    }

    /// Responde "No, quiero seguir": el diálogo se oculta y nada se pierde.
    pub fn cancel(&self) {
        *self.dialog_open.write().unwrap() = false;
        *self.pending.write().unwrap() = Vec::new();
    }

    /// Responde "Sí, cerrar": limpia el diálogo; el core cierra la ventana.
    pub fn confirm(&self) {
        *self.dialog_open.write().unwrap() = false;
        *self.pending.write().unwrap() = Vec::new();
    }

    /// Cuántas veces se pidió confirmación, útil para pruebas.
    pub fn ask_count(&self) -> u32 {
        *self.ask_count.read().unwrap()
    }

    /// Texto del diálogo con la lista de programas que se van a perder.
    pub fn summary(bins: &[String]) -> String {
        match bins.len() {
            0 => "No hay programas en ejecución.".to_string(),
            1 => format!("Hay 1 programa en ejecución: {}", bins[0]),
            n => format!("Hay {n} programas en ejecución: {}", bins.join(", ")),
        }
    }
}

impl LifecycleHook for CloseGuardPlugin {
    /// El core ya consultó los programas y dejó la decisión anotada con
    /// `request_close`; aquí solo se refleja en el diálogo.
    fn on_close_request(&self) -> CloseDecision {
        if self.is_dialog_open() {
            CloseDecision::Confirm
        } else {
            CloseDecision::Allow
        }
    }

    fn on_close_confirmed(&self) {
        self.confirm();
    }
}

impl Plugin for CloseGuardPlugin {
    fn id(&self) -> &'static str {
        "close-guard"
    }

    fn name(&self) -> &'static str {
        "Close Guard"
    }

    fn version(&self) -> &'static str {
        "0.1.0"
    }

    fn lifecycle_hook(&self) -> Option<&dyn LifecycleHook> {
        Some(self)
    }

    fn default_config(&self) -> Option<PluginConfig> {
        let mut cfg = PluginConfig::new();
        cfg.set("enabled", true);
        Some(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trivial_commands_do_not_block_closing() {
        for bin in ["true", "echo", "printf", "sleep"] {
            assert!(!CloseGuardPlugin::should_block(bin), "{bin} no debe bloquear");
        }
    }

    #[test]
    fn real_programs_block_closing() {
        for bin in ["pi", "codex", "claude", "btop", "vim", "cargo"] {
            assert!(CloseGuardPlugin::should_block(bin), "{bin} debe bloquear");
        }
    }

    #[test]
    fn close_without_programs_is_allowed_immediately() {
        let plugin = CloseGuardPlugin::new();
        assert!(!plugin.request_close(Vec::new()));
        assert!(!plugin.is_dialog_open());
        assert_eq!(plugin.ask_count(), 0);
    }

    #[test]
    fn close_with_trivial_command_does_not_ask() {
        let plugin = CloseGuardPlugin::new();
        assert!(!plugin.request_close(vec!["true".to_string()]));
        assert!(!plugin.is_dialog_open());
    }

    #[test]
    fn close_with_running_program_asks_confirmation() {
        let plugin = CloseGuardPlugin::new();
        assert!(plugin.request_close(vec!["pi".to_string(), "btop".to_string()]));
        assert!(plugin.is_dialog_open());
        assert_eq!(plugin.pending_programs(), vec!["pi", "btop"]);
        assert_eq!(plugin.ask_count(), 1);
        assert_eq!(
            plugin.lifecycle_hook().unwrap().on_close_request(),
            CloseDecision::Confirm,
            "con diálogo abierto el cierre debe quedar bloqueado"
        );
    }

    #[test]
    fn answering_no_hides_the_dialog_and_keeps_the_terminal() {
        let plugin = CloseGuardPlugin::new();
        plugin.request_close(vec!["cargo".to_string()]);
        plugin.cancel();
        assert!(!plugin.is_dialog_open());
        assert!(plugin.pending_programs().is_empty());
        assert_eq!(
            plugin.lifecycle_hook().unwrap().on_close_request(),
            CloseDecision::Allow
        );
    }

    #[test]
    fn answering_yes_clears_state_for_the_next_time() {
        let plugin = CloseGuardPlugin::new();
        plugin.request_close(vec!["cargo".to_string()]);
        plugin.confirm();
        assert!(!plugin.is_dialog_open());
        assert_eq!(
            plugin.lifecycle_hook().unwrap().on_close_request(),
            CloseDecision::Allow
        );
    }

    #[test]
    fn summary_counts_running_programs() {
        assert_eq!(
            CloseGuardPlugin::summary(&[]),
            "No hay programas en ejecución."
        );
        assert_eq!(
            CloseGuardPlugin::summary(&["pi".to_string()]),
            "Hay 1 programa en ejecución: pi"
        );
        assert_eq!(
            CloseGuardPlugin::summary(&["pi".to_string(), "btop".to_string()]),
            "Hay 2 programas en ejecución: pi, btop"
        );
    }
}
