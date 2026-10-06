//! Subagentes de `pi` (gentle-pi) leídos en tiempo real desde la presencia viva.
//!
//! La extensión `gentle-agents` publica, por cada sesión que corre un subagente,
//! un par de archivos en `~/.pi/agent/gentle-agents/presence/` (ver
//! [`crate::presence`], dueño de ese contrato ajeno). Ahí está la verdad sobre
//! qué sesiones siguen vivas: mientras un proceso `pi` late, su header se
//! reescribe; cuando muere, el latido se agota y la sesión deja de contar.
//!
//! Los registros de `~/.pi/agent/gentle-agents/tasks/<id>.json` no participan:
//! gentle-pi los escribe una sola vez, al terminar la tarea, así que mientras un
//! subagente corre el registro no existe. La presencia viva es la única fuente:
//! de ella salen la identidad de la sesión, la etiqueta, el estado, la actividad
//! y el último paso. El hilo (`thread`) se proyecta en [`crate::presence`] y el
//! visor lo relee del propio archivo de actividad.
//!
//! # Visibilidad
//!
//! Una fila se pinta si su sesión late (`presence::read_live_sessions`) y,
//! además, la tarea está activa (`running`, `queued`, `waiting`) o terminó como
//! mucho hace [`FINISHED_GRACE_MS`]. Esa gracia es política de UI y vive aquí,
//! no en `presence.rs`.
//!
//! # Por qué hay caché y no se lee en cada render
//!
//! La ventana de PORT se repinta unas 60 veces por segundo porque el latido del
//! PTY pide un refresco constante, y `left_sidebar()` se construye en cada uno de
//! esos renders. Leer el contrato de presencia a 60 Hz no deja CPU para nada
//! más.
//!
//! La primera defensa es un `stat` sobre la CARPETA: el mtime cambia cuando se
//! crea o borra un archivo dentro, que es justo cuando hay algo nuevo que ver.
//! La segunda es el TTL: el mtime por sí solo se congela cuando la última sesión
//! de `pi` muere, así que una caché sin reloj mantendría su fila para siempre.
//! Ver [`REFRESH_TTL_MS`].

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Cuántos subagentes se pintan a la vez por defecto.
pub const DEFAULT_VISIBLE_LIMIT: usize = 6;

/// Ventana de gracia de una fila terminada: se pinta mientras
/// `now_ms - endedAt <= FINISHED_GRACE_MS`.
///
/// Existe para que un subagente que termina mientras el usuario está mirando la
/// sidebar no desaparezca a mitad de lectura. El valor replica la ventana del
/// latido del contrato ([`crate::presence::LIVE_WINDOW_MS`]) y es política de
/// UI: vive aquí, no en `presence.rs`, que solo es dueño del contrato ajeno.
pub const FINISHED_GRACE_MS: u64 = 15_000;

/// Cada cuánto se relee la presencia aunque el directorio no haya cambiado.
///
/// El `mtime` por sí solo no basta: una sesión de `pi` que muere deja de
/// reescribir su archivo de latido, así que el mtime del directorio se congela y
/// una caché basada solo en él mantendría esa sesión en pantalla para siempre.
/// Este TTL acota cuánto puede sobrevivir una fila obsoleta: como mucho se
/// refresca una vez por segundo.
pub const REFRESH_TTL_MS: u64 = 1_000;

/// El reloj del sistema en milisegundos desde el epoch.
///
/// Se expone para que quien consume el watcher pueda inyectar el mismo instante
/// en varios puntos de un render, y para que las pruebas controlen el tiempo.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

/// Estado de un subagente tal como lo reporta gentle-pi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentStatus {
    Running,
    Queued,
    /// En espera de un recurso o de otra tarea: sigue siendo trabajo vivo.
    Waiting,
    Completed,
    Failed,
    /// Un estado que no conocemos: se muestra, pero no se presume bueno.
    Unknown(String),
}

impl AgentStatus {
    /// Un subagente en curso, en cola o en espera es lo que el usuario necesita
    /// notar de un vistazo.
    pub fn is_live(&self) -> bool {
        matches!(self, Self::Running | Self::Queued | Self::Waiting)
    }

    /// Etiqueta corta en inglés, como el resto de la UI.
    pub fn label(&self) -> String {
        match self {
            Self::Running => "running".to_string(),
            Self::Queued => "queued".to_string(),
            Self::Waiting => "waiting".to_string(),
            Self::Completed => "done".to_string(),
            Self::Failed => "failed".to_string(),
            Self::Unknown(raw) => raw.clone(),
        }
    }

    /// Mapeo de los estados crudos: `running`; `queued`/`pending`; `waiting`;
    /// `completed`/`done`/`finished`/`success`; `failed`/`error`/`aborted`/
    /// `cancelled`/`canceled`; cualquier otro queda como `Unknown`.
    fn parse(raw: &str) -> Self {
        match raw.trim().to_lowercase().as_str() {
            "running" | "in_progress" | "active" => Self::Running,
            "queued" | "pending" => Self::Queued,
            "waiting" => Self::Waiting,
            "completed" | "done" | "finished" | "success" => Self::Completed,
            "failed" | "error" | "aborted" | "cancelled" | "canceled" => Self::Failed,
            other => Self::Unknown(other.to_string()),
        }
    }
}

/// Un subagente tal y como se pinta en la sidebar.
///
/// Lleva todo lo que el clic necesita para abrir su visor sin volver a tocar el
/// disco: la identidad exacta de su sesión (hash e incarnación), la etiqueta y
/// el último paso crudo que pinta la fila.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentEntry {
    pub id: String,
    pub agent: String,
    pub label: String,
    pub status: AgentStatus,
    /// Epoch en milisegundos. `0` si el campo no venía.
    pub last_activity_at: u64,
    /// Último paso crudo (`summary.lastStep`); `""` si falta.
    pub last_step: String,
    /// Hash de la sesión dueña de la tarea.
    pub session_hash: String,
    /// Incarnación (UUID v4) de la sesión dueña de la tarea.
    pub incarnation: String,
}

/// Ordena: vivos primero, y dentro de cada grupo por actividad reciente.
fn order(entries: &mut [AgentEntry]) {
    entries.sort_by(|a, b| {
        b.status
            .is_live()
            .cmp(&a.status.is_live())
            .then(b.last_activity_at.cmp(&a.last_activity_at))
    });
}

/// Decide si una tarea de la presencia se pinta.
///
/// Las activas siempre; las terminadas solo mientras su `endedAt` cae dentro de
/// la gracia. Una tarea terminada sin `endedAt` no se pinta.
fn is_visible(status: &AgentStatus, ended_at: Option<u64>, now_ms: u64) -> bool {
    if status.is_live() {
        return true;
    }
    match ended_at {
        Some(ended) => now_ms >= ended && now_ms - ended <= FINISHED_GRACE_MS,
        None => false,
    }
}

/// Caché de la presencia viva.
///
/// Es lo que hace que leer sea "en tiempo real" sin coste en reposo: un `stat`
/// por render, y relectura completa solo cuando hay algo nuevo o cuando vence el
/// TTL.
#[derive(Debug, Default)]
pub struct AgentWatcher {
    presence_dir: PathBuf,
    stamp: Option<SystemTime>,
    last_refresh_ms: u64,
    cached: Vec<AgentEntry>,
    /// Cuántas veces se ha reescrito la caché. Sirve para demostrar en los tests
    /// que una lectura sin cambios no vuelve a tocar el disco.
    pub refreshes: u32,
}

impl AgentWatcher {
    /// `presence_dir` es la única fuente: no hay registro de tarea que leer.
    pub fn new(presence_dir: impl Into<PathBuf>) -> Self {
        Self {
            presence_dir: presence_dir.into(),
            stamp: None,
            last_refresh_ms: 0,
            cached: Vec::new(),
            refreshes: 0,
        }
    }

    /// Directorio de presencia que vigila. El clic lo necesita para construir el
    /// comando del visor sin depender del registro de tarea.
    pub fn presence_dir(&self) -> &Path {
        &self.presence_dir
    }

    /// Filas visibles ahora mismo, con el reloj inyectado.
    ///
    /// Se relee si el mtime de la carpeta de presencia cambió o si se cumplió
    /// [`REFRESH_TTL_MS`]. El TTL es una condición de correctitud, no una
    /// optimización: sin él, la última fila de una sesión muerta quedaría
    /// congelada en pantalla porque el directorio deja de cambiar.
    pub fn entries(&mut self, now_ms: u64) -> &[AgentEntry] {
        let stamp = std::fs::metadata(&self.presence_dir)
            .and_then(|meta| meta.modified())
            .ok();
        let ttl_expired = now_ms.saturating_sub(self.last_refresh_ms) >= REFRESH_TTL_MS;
        if stamp != self.stamp || ttl_expired {
            self.stamp = stamp;
            self.last_refresh_ms = now_ms;
            self.cached = self.refresh(now_ms);
            self.refreshes += 1;
        }
        &self.cached
    }

    /// Las que se pintan, vivos primero, con tope.
    pub fn visible(&mut self, now_ms: u64, limit: usize) -> Vec<AgentEntry> {
        self.entries(now_ms).iter().take(limit).cloned().collect()
    }

    /// Reconstruye la lista desde la presencia viva.
    fn refresh(&self, now_ms: u64) -> Vec<AgentEntry> {
        let mut out: Vec<AgentEntry> = Vec::new();
        for session in crate::presence::read_live_sessions(&self.presence_dir, now_ms) {
            for task in session.tasks {
                let status = AgentStatus::parse(&task.status);
                if !is_visible(&status, task.ended_at, now_ms) {
                    continue;
                }
                out.push(AgentEntry {
                    id: task.id,
                    agent: task.agent,
                    label: task.label,
                    status,
                    last_activity_at: task.last_activity_at,
                    last_step: task.last_step,
                    session_hash: session.session_hash.clone(),
                    incarnation: session.incarnation.clone(),
                });
            }
        }
        order(&mut out);
        out
    }
}
