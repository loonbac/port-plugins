//! Plugin de atajos de teclado personalizados para PORT.
//!
//! Permite registrar combinaciones de teclas (modificadores + tecla) y
//! asociarlas a acciones personalizadas (callbacks) de forma desacoplada.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use port_plugin_api::{InputHook, KeyAction, Plugin, Services};
use port_term_core::input::Key;

/// Definición de una combinación de teclas.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Shortcut {
    /// Nombre de la tecla en minúsculas (ej. "t", "enter", "f1").
    pub key: String,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Shortcut {
    /// Crea un atajo con la tecla y modificadores especificados.
    pub fn new(key: impl Into<String>, ctrl: bool, alt: bool, shift: bool) -> Self {
        Self {
            key: key.into().to_lowercase(),
            ctrl,
            alt,
            shift,
        }
    }

    /// Atajo con modificador Control.
    pub fn ctrl(key: impl Into<String>) -> Self {
        Self::new(key, true, false, false)
    }

    /// Atajo con modificador Alt.
    pub fn alt(key: impl Into<String>) -> Self {
        Self::new(key, false, true, false)
    }

    /// Atajo con modificadores Control y Shift.
    pub fn ctrl_shift(key: impl Into<String>) -> Self {
        Self::new(key, true, false, true)
    }

    /// Parsea una cadena de texto en formato "ctrl+shift+t" o "alt+enter".
    pub fn parse(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.split('+').map(str::trim).collect();
        if parts.is_empty() {
            return None;
        }

        let mut ctrl = false;
        let mut alt = false;
        let mut shift = false;
        let mut key = String::new();

        for (i, part) in parts.iter().enumerate() {
            let lower = part.to_lowercase();
            if i == parts.len() - 1 {
                // Última parte es la tecla principal
                key = lower;
            } else {
                match lower.as_str() {
                    "ctrl" | "control" => ctrl = true,
                    "alt" => alt = true,
                    "shift" => shift = true,
                    _ => return None,
                }
            }
        }

        if key.is_empty() {
            None
        } else {
            Some(Self {
                key,
                ctrl,
                alt,
                shift,
            })
        }
    }

    /// Comprueba si este atajo coincide con una tecla recibida de la terminal.
    pub fn matches(&self, key: &Key) -> bool {
        if self.ctrl != key.ctrl || self.alt != key.alt || self.shift != key.shift {
            return false;
        }
        let incoming_key = key.key.to_lowercase();
        if self.key == incoming_key {
            return true;
        }
        if let Some(text) = &key.text {
            if self.key == text.to_lowercase() {
                return true;
            }
        }
        false
    }
}

type ShortcutCallback = Arc<dyn Fn() + Send + Sync + 'static>;

/// Plugin para gestionar atajos de teclado personalizados.
///
/// Además de callbacks propios, puede ligar una tecla a una acción publicada por
/// otro plugin a través del directorio de servicios: así `shortcuts` es el único
/// dueño de los atajos de PORT.
#[derive(Default)]
pub struct ShortcutsPlugin {
    bindings: RwLock<HashMap<Shortcut, ShortcutCallback>>,
    services: Option<Arc<Services>>,
}

impl ShortcutsPlugin {
    /// Crea un nuevo gestor de atajos vacío.
    pub fn new() -> Self {
        Self {
            bindings: RwLock::new(HashMap::new()),
            services: None,
        }
    }

    /// Proporciona el directorio de servicios para poder llamar a otros plugins.
    pub fn with_services(mut self, services: Arc<Services>) -> Self {
        self.services = Some(services);
        self
    }

    /// Liga una tecla a una acción publicada por otro plugin.
    ///
    /// El servicio se resuelve en el momento de pulsar la tecla, así que da igual
    /// el orden de registro: `bind_service("ctrl+=", "font-zoom", "zoom_in")`
    /// funciona aunque el destino se publique después.
    pub fn bind_service(&self, pattern: &str, service_id: &str, action: &str) -> bool {
        let Some(shortcut) = Shortcut::parse(pattern) else {
            return false;
        };
        let services = self.services.clone();
        let service_id = service_id.to_string();
        let action = action.to_string();
        self.bind(shortcut, move || {
            if let Some(directories) = services.as_ref() {
                let _ = directories.call(&service_id, &action, &[]);
            }
        });
        true
    }

    /// Registra un atajo y su acción correspondiente.
    pub fn bind<F>(&self, shortcut: Shortcut, action: F)
    where
        F: Fn() + Send + Sync + 'static,
    {
        let mut bindings = self.bindings.write().unwrap();
        bindings.insert(shortcut, Arc::new(action));
    }

    /// Registra un atajo a partir de una cadena de texto (ej. "ctrl+shift+c").
    pub fn bind_str<F>(&self, pattern: &str, action: F) -> bool
    where
        F: Fn() + Send + Sync + 'static,
    {
        if let Some(shortcut) = Shortcut::parse(pattern) {
            self.bind(shortcut, action);
            true
        } else {
            false
        }
    }

    /// Devuelve el número de atajos registrados.
    pub fn len(&self) -> usize {
        self.bindings.read().unwrap().len()
    }

    /// Devuelve `true` si no hay atajos registrados.
    pub fn is_empty(&self) -> bool {
        self.bindings.read().unwrap().is_empty()
    }
}

impl InputHook for ShortcutsPlugin {
    fn on_key(&self, key: &Key) -> KeyAction {
        let bindings = self.bindings.read().unwrap();
        for (shortcut, action) in bindings.iter() {
            if shortcut.matches(key) {
                action();
                return KeyAction::Consume;
            }
        }
        KeyAction::Pass
    }
}

impl Plugin for ShortcutsPlugin {
    fn id(&self) -> &'static str {
        "shortcuts"
    }

    fn name(&self) -> &'static str {
        "Custom Shortcuts"
    }

    fn version(&self) -> &'static str {
        "0.1.0"
    }

    fn input_hook(&self) -> Option<&dyn InputHook> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn parse_shortcut_strings() {
        assert_eq!(
            Shortcut::parse("ctrl+shift+t"),
            Some(Shortcut::new("t", true, false, true))
        );
        assert_eq!(
            Shortcut::parse("alt+enter"),
            Some(Shortcut::new("enter", false, true, false))
        );
        assert_eq!(Shortcut::parse("ctrl+q"), Some(Shortcut::ctrl("q")));
        assert_eq!(Shortcut::parse(""), None);
    }

    #[test]
    fn shortcut_matching() {
        let sc = Shortcut::ctrl_shift("t");
        let matching_key = Key::new("t").ctrl().shift();
        assert!(sc.matches(&matching_key));

        let non_matching_key = Key::new("t").ctrl(); // sin shift
        assert!(!sc.matches(&non_matching_key));

        let other_key = Key::new("w").ctrl().shift();
        assert!(!sc.matches(&other_key));
    }

    #[test]
    fn bindings_can_invoke_a_service_published_by_another_plugin() {
        use port_plugin_api::{Arg, Ret, Service, ServiceError};

        struct Fake;
        impl Service for Fake {
            fn id(&self) -> &str {
                "fake"
            }
            fn name(&self) -> &str {
                "Fake"
            }
            fn actions(&self) -> Vec<&'static str> {
                vec!["ping"]
            }
            fn invoke(&self, action: &str, _args: &[Arg]) -> Option<Result<Ret, ServiceError>> {
                match action {
                    "ping" => Some(Ok(Ret::Num(42.0))),
                    _ => None,
                }
            }
        }

        let services = Arc::new(Services::new());
        services.publish(Arc::new(Fake));

        let plugin = ShortcutsPlugin::new().with_services(Arc::clone(&services));
        assert!(plugin.bind_service("ctrl+shift+p", "fake", "ping"));

        let ctrl_shift_p = Key::new("p").ctrl().shift();
        assert_eq!(plugin.on_key(&ctrl_shift_p), KeyAction::Consume);
    }

    #[test]
    fn a_service_binding_survives_the_service_not_being_there() {
        // Resolver en el momento de pulsar hace que da igual el orden de
        // registro: si el servicio no existe, el atajo simplemente no hace nada.
        let plugin = ShortcutsPlugin::new();
        assert!(plugin.bind_service("ctrl+shift+q", "nope", "ping"));

        let ctrl_shift_q = Key::new("q").ctrl().shift();
        assert_eq!(plugin.on_key(&ctrl_shift_q), KeyAction::Consume);
    }

    #[test]
    fn shortcuts_plugin_triggers_action_and_consumes() {
        let plugin = ShortcutsPlugin::new();
        let counter = Arc::new(AtomicUsize::new(0));

        let c1 = Arc::clone(&counter);
        plugin.bind_str("ctrl+shift+k", move || {
            c1.fetch_add(1, Ordering::SeqCst);
        });

        assert_eq!(plugin.len(), 1);

        // Disparo de tecla coincidente
        let matched = Key::new("k").ctrl().shift();
        assert_eq!(plugin.on_key(&matched), KeyAction::Consume);
        assert_eq!(counter.load(Ordering::SeqCst), 1);

        // Tecla no coincidente (pass)
        let unmatched = Key::new("k").ctrl();
        assert_eq!(plugin.on_key(&unmatched), KeyAction::Pass);
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }
}
