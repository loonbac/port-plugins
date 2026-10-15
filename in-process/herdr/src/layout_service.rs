//! Servicio que publica el modelo de espacios para otros plugins.
//!
//! El cliente móvil no duplica el modelo: lo pide acá. Es la vía que el registro
//! de servicios ya ofrece para que un plugin use lo que publica otro sin conocer
//! su tipo, y la que evita que dos plugins mantengan dos copias de lo mismo.
//!
//! Viaja como JSON porque es lo único que el registro sabe llevar entre plugins
//! (`Ret::Text`). Su forma es la que espera el cliente, y las claves están fijadas
//! por una prueba para que un cambio de forma no pase inadvertido: el que la
//! consume vive en otro plugin y no lo vería hasta fallar en el teléfono.

use std::sync::{Arc, RwLock};

use port_plugin_api::{AgentState, Arg, Ret, Service, ServiceError};
use port_term_core::frame::Rgb;
use serde_json::{json, Value};

use crate::state::HerdrState;

/// Identificador con el que se busca este servicio.
pub(crate) const LAYOUT_SERVICE_ID: &str = "herdr";
/// Acción que devuelve el modelo.
pub(crate) const LAYOUT_ACTION: &str = "layout";

/// Publica el modelo de espacios y pestañas.
pub(crate) struct LayoutService {
    state: Arc<RwLock<HerdrState>>,
}

impl LayoutService {
    /// Publica el modelo del estado compartido de este plugin.
    pub(crate) fn new(state: Arc<RwLock<HerdrState>>) -> Self {
        Self { state }
    }
}

impl Service for LayoutService {
    fn id(&self) -> &str {
        LAYOUT_SERVICE_ID
    }

    fn name(&self) -> &str {
        "Espacios y pestañas"
    }

    fn actions(&self) -> Vec<&'static str> {
        vec![LAYOUT_ACTION]
    }

    fn invoke(&self, action: &str, _args: &[Arg]) -> Option<Result<Ret, ServiceError>> {
        if action != LAYOUT_ACTION {
            return None;
        }
        let state = self
            .state
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Some(Ok(Ret::Text(layout_json(&state).to_string())))
    }
}

/// El modelo, en la forma que espera el cliente.
pub(crate) fn layout_json(state: &HerdrState) -> Value {
    let spaces: Vec<Value> = state
        .spaces
        .iter()
        .map(|space| {
            let aggregated = space.aggregated_agent_state(&state.agents);
            let tabs: Vec<Value> = space
                .tabs
                .iter()
                .map(|tab| {
                    let entry = state.agents.get(&tab.session_id);
                    json!({
                        "title": tab.title,
                        "session": tab.session_id,
                        "agent": agent_name(entry.map(|(agent, _)| *agent)),
                        // Un estado que nadie vio reclama atención; uno sin dato
                        // no reclama nada.
                        "seen": entry.map(|(_, seen)| *seen).unwrap_or(true),
                    })
                })
                .collect();
            json!({
                "name": space.name,
                "branch": space.branch,
                "color": space.custom_color.map(hex),
                "active_tab": space.active_tab_index,
                "agent": agent_name(aggregated.map(|(agent, _)| agent)),
                "tabs": tabs,
            })
        })
        .collect();
    json!({ "spaces": spaces, "active_space": state.active_space_index })
}

/// Nombre del estado tal como lo espera el cliente.
fn agent_name(state: Option<AgentState>) -> &'static str {
    match state {
        Some(AgentState::Working) => "working",
        Some(AgentState::Blocked) => "blocked",
        Some(AgentState::Idle) => "idle",
        _ => "unknown",
    }
}

/// Color en la forma que viaja por el cable.
fn hex(color: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", color.r, color.g, color.b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{default_space, HerdrTab, PENDING_SESSION};

    /// Estado con un espacio y una pestaña, para mirar la forma del JSON.
    fn estado_de_prueba() -> HerdrState {
        let mut state = HerdrState {
            spaces: vec![default_space()],
            ..HerdrState::default()
        };
        state.agents.insert(0, (AgentState::Blocked, false));
        state
    }

    /// El modelo lleva las claves que el cliente espera, ni más ni menos.
    #[test]
    fn el_modelo_lleva_las_claves_del_cliente() {
        let state = estado_de_prueba();
        let value = layout_json(&state);

        assert!(value.get("spaces").is_some(), "{value}");
        assert!(value.get("active_space").is_some(), "{value}");

        let space = &value["spaces"][0];
        for clave in ["name", "branch", "color", "active_tab", "agent", "tabs"] {
            assert!(space.get(clave).is_some(), "falta {clave} en {space}");
        }

        let tab = &space["tabs"][0];
        for clave in ["title", "session", "agent", "seen"] {
            assert!(tab.get(clave).is_some(), "falta {clave} en {tab}");
        }
    }

    /// El estado del agente viaja con el nombre que el cliente entiende.
    #[test]
    fn el_estado_del_agente_viaja_por_nombre() {
        let state = estado_de_prueba();
        let value = layout_json(&state);

        assert_eq!(value["spaces"][0]["agent"], "blocked");
        assert_eq!(value["spaces"][0]["tabs"][0]["agent"], "blocked");
        assert_eq!(value["spaces"][0]["tabs"][0]["seen"], false);

        // Sin dato, el estado es desconocido y no reclama atención.
        assert_eq!(agent_name(None), "unknown");
        assert_eq!(agent_name(Some(AgentState::Working)), "working");
        assert_eq!(agent_name(Some(AgentState::Idle)), "idle");
    }

    /// El servicio responde sólo a su acción, y con el modelo serializado.
    #[test]
    fn el_servicio_responde_su_accion() {
        let state = Arc::new(RwLock::new(estado_de_prueba()));
        let service = LayoutService::new(Arc::clone(&state));

        assert_eq!(service.id(), LAYOUT_SERVICE_ID);
        assert_eq!(service.actions(), vec![LAYOUT_ACTION]);
        assert!(service.invoke("otra", &[]).is_none());

        match service.invoke(LAYOUT_ACTION, &[]) {
            Some(Ok(Ret::Text(texto))) => {
                let value: Value = serde_json::from_str(&texto).expect("JSON válido");
                assert!(value["spaces"].is_array(), "{texto}");
            }
            otro => panic!("esperaba el modelo, llegó {otro:?}"),
        }
    }

    /// Una pestaña pendiente no debería aparecer con estado inventado.
    #[test]
    fn una_pestana_pendiente_no_reclama_atencion() {
        let mut state = HerdrState {
            spaces: vec![default_space()],
            ..HerdrState::default()
        };
        state.spaces[0].tabs.push(HerdrTab {
            id: 99,
            title: "pendiente".to_string(),
            session_id: PENDING_SESSION,
            app: None,
            title_locked: false,
            watching: None,
        });

        let value = layout_json(&state);
        let pendiente = &value["spaces"][0]["tabs"][1];
        assert_eq!(pendiente["agent"], "unknown");
        assert_eq!(pendiente["seen"], true, "sin dato no se reclama atención");
    }
}
