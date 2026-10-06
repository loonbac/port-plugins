//! Puente de vigilancia del plugin herdr.
//!
//! Única responsabilidad: convertir una fila de AGENTS en la petición del visor
//! de un subagente y recortar el último paso que pinta la sidebar.

use std::path::Path;
use std::sync::RwLock;

use crate::agents;
use crate::state::HerdrState;
#[allow(unused_imports)] // solo lo usa el enlace de la documentación de `request_watch`
use crate::HerdrPlugin;

/// Extrae de una fila de AGENTS lo que el visor necesita: la identidad exacta
/// de su sesión (hash e incarnación) y la etiqueta con la que titular la
/// pestaña.
///
/// `None` solo si la fila no trae identidad de sesión, algo que la presencia ya
/// valida al publicarla. La etiqueta cae al `id` cuando viene vacía, igual que la
/// propia fila pintada.
fn watch_target(entry: &agents::AgentEntry) -> Option<(&str, &str, &str)> {
    if entry.session_hash.is_empty() || entry.incarnation.is_empty() {
        return None;
    }
    let label = if entry.label.is_empty() {
        entry.id.as_str()
    } else {
        entry.label.as_str()
    };
    Some((
        entry.session_hash.as_str(),
        entry.incarnation.as_str(),
        label,
    ))
}

/// Camino único del clic: pide el visor de una fila de AGENTS.
///
/// `false` sin pedir nada cuando la fila no trae identidad de sesión. El clic de
/// la sidebar y [`HerdrPlugin::watch_entry`] entran por aquí, así que la prueba
/// ejercita exactamente el mismo código que el clic.
pub(crate) fn request_watch(
    state: &RwLock<HerdrState>,
    presence_dir: &Path,
    entry: &agents::AgentEntry,
) -> bool {
    let Some((session_hash, incarnation, label)) = watch_target(entry) else {
        return false;
    };
    state
        .write()
        .unwrap()
        .open_watch_tab(presence_dir, session_hash, incarnation, label);
    true
}

/// Máximo de caracteres del último paso que pinta la fila de AGENTS.
pub(crate) const AGENT_STEP_MAX_CHARS: usize = 80;

/// Recorta el último paso a una sola línea y a [`AGENT_STEP_MAX_CHARS`]
/// caracteres, con `…` cuando se corta.
pub(crate) fn clip_last_step(raw: &str) -> String {
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= AGENT_STEP_MAX_CHARS {
        return collapsed;
    }
    let mut clipped: String = collapsed.chars().take(AGENT_STEP_MAX_CHARS).collect();
    clipped.push('…');
    clipped
}
