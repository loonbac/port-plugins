//! Presencia en vivo de los subagentes de `gentle-agents` (gentle-pi).
//!
//! La extensión `gentle-agents` de gentle-pi publica, por cada sesión que corre
//! un subagente, un par de archivos en
//! `~/.pi/agent/gentle-agents/presence/`:
//!
//! ```text
//! <sessionHash>.<incarnation>.header.json    identidad, latido y contadores
//! <sessionHash>.<incarnation>.activity.json  tareas y su hilo (thread)
//! ```
//!
//! Este módulo es un **adaptador de un contrato ajeno**: solo lee y valida. No
//! escribe, no borra y no decide nada por sí mismo. El reloj también se inyecta
//! (`now_ms`), así que las funciones son puras respecto del tiempo y las pruebas
//! no dependen del reloj de la máquina.
//!
//! # Vivencia y prefiltro
//!
//! El contrato no borra los archivos viejos: el directorio acumula historia para
//! siempre. Por eso, antes de leer un header se mira su `mtime`: si es más viejo
//! que [`LIVE_WINDOW_MS`] + [`STALE_FILE_MARGIN_MS`] respecto de `now_ms`, se
//! omite sin abrirlo. Ese prefiltro es solo un ahorro de E/S; la autoridad sobre
//! la vivencia sigue siendo el campo `heartbeat`:
//! `now_ms >= heartbeat && now_ms - heartbeat <= LIVE_WINDOW_MS`. Un latido en el
//! futuro no está vivo.
//!
//! # El hilo (`thread`) se parsea, no se pinta aquí
//!
//! Cada tarea trae un `thread` con los mensajes y llamadas de herramienta del
//! subagente. Este módulo lo **valida y proyecta** a [`AgentThreadItem`] para que
//! la fila de AGENTS y el visor consuman la misma proyección; quién lo pinta (y
//! cómo) es decisión de `lib.rs` y `viewer.rs`. Un `thread` ausente o mal formado
//! no es un error: la tarea queda con hilo vacío.
//!
//! # Todo falla cerrado, nada entra en pánico
//!
//! Directorio ilegible o inexistente: vector vacío. Header o actividad rotos,
//! identidad que no casa con el nombre, tarea con un campo requerido mal tipado:
//! esa pieza se descarta y el resto sobrevive. Nunca se entra en pánico por un
//! JSON mal formado ni por bytes que no sean UTF-8 válido.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

/// Ventana de vida del latido del contrato: `now - heartbeat <= LIVE_WINDOW_MS`.
pub const LIVE_WINDOW_MS: u64 = 15_000;

/// Margen del prefiltro por `mtime`: un archivo que no se ha tocado ni siquiera
/// puede describir una sesión viva, así que se omite sin leerlo.
pub const STALE_FILE_MARGIN_MS: u64 = 60_000;

/// Estados que reconoce el contrato. Cualquier otro valor invalida la tarea.
const STATUSES: [&str; 7] = [
    "running",
    "queued",
    "waiting",
    "completed",
    "failed",
    "cancelled",
    "timed_out",
];

/// Un item del hilo publicado por el contrato.
///
/// Las formas reales son `{"kind":"tool","callId","name","output","running",
/// "isError"}` y `{"kind":...,"text":...}`. Cualquier campo opcional puede
/// faltar o venir `null`: se cae al valor neutro y el item **nunca** se descarta
/// por eso. Un item textual sin `text` legible no aporta fila y se omite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentThreadItem {
    Tool {
        call_id: String,
        name: String,
        output: String,
        running: bool,
        is_error: bool,
    },
    Text {
        /// `text`, `thinking`, `note` (o cualquier tipo textual futuro).
        kind: String,
        text: String,
    },
}

/// Una tarea de subagente tal y como la publica el contrato.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresenceTask {
    pub id: String,
    pub agent: String,
    pub label: String,
    /// Estado crudo del contrato; el mapeo a `AgentStatus` vive en `agents.rs`.
    pub status: String,
    pub ended_at: Option<u64>,
    pub last_activity_at: u64,
    /// Último paso crudo (`summary.lastStep`); `""` si falta o no es string.
    pub last_step: String,
    /// Hilo del subagente, en orden. Vacío si falta o no se puede leer.
    pub thread: Vec<AgentThreadItem>,
}

/// Una sesión de gentle-pi con el latido vivo en este instante.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveSession {
    pub session_hash: String,
    pub incarnation: String,
    pub label: String,
    pub heartbeat: u64,
    /// `false` si el contrato declaró la actividad indisponible o si su archivo
    /// no se pudo leer o validar. La sesión se devuelve igual.
    pub thread_available: bool,
    pub tasks: Vec<PresenceTask>,
}

/// Directorio de presencia dentro de la raíz de `gentle-agents`.
pub fn presence_dir(agents_root: &Path) -> PathBuf {
    agents_root.join("presence")
}

/// Directorio de presencia por defecto.
///
/// `$PORT_PI_PRESENCE_DIR` manda si existe (es lo que usan las pruebas y las
/// instalaciones no estándar); si no, `~/.pi/agent/gentle-agents/presence`.
pub fn default_presence_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("PORT_PI_PRESENCE_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let agents_root = PathBuf::from(home)
        .join(".pi")
        .join("agent")
        .join("gentle-agents");
    presence_dir(&agents_root)
}

/// Lee las sesiones cuyo latido está vivo ahora mismo, con `now` inyectado.
///
/// El orden de salida es el del directorio, sin ordenar: quien consuma esto
/// decide cómo presentarlo.
pub fn read_live_sessions(dir: &Path, now_ms: u64) -> Vec<LiveSession> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut sessions = Vec::new();
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        // Los temporales del editor atómico empiezan por punto.
        if file_name.starts_with('.') {
            continue;
        }
        // Solo interesan los headers, y el nombre debe declarar la identidad.
        let Some(stem) = file_name.strip_suffix(".header.json") else {
            continue;
        };
        let Some((name_hash, name_incarnation)) = stem.split_once('.') else {
            continue;
        };

        // Prefiltro por mtime: no se abre historia que ya no puede estar viva.
        let Some(mtime_ms) = entry
            .metadata()
            .ok()
            .and_then(|meta| meta.modified().ok())
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|elapsed| elapsed.as_millis() as u64)
        else {
            continue;
        };
        if mtime_ms < now_ms.saturating_sub(LIVE_WINDOW_MS + STALE_FILE_MARGIN_MS) {
            continue;
        }

        // `read_to_string` rechaza UTF-8 inválido: se descarta, no se adivina.
        let Ok(source) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        let Some(mut session) = parse_header(&source) else {
            continue;
        };
        // Integridad del contrato: el nombre y el contenido deben coincidir.
        if session.session_hash != name_hash || session.incarnation != name_incarnation {
            continue;
        }
        if !is_live(session.heartbeat, now_ms) {
            continue;
        }

        if session.thread_available {
            let activity_path =
                dir.join(format!("{}.{}.activity.json", name_hash, name_incarnation));
            match std::fs::read_to_string(&activity_path)
                .ok()
                .and_then(|text| parse_activity(&text, &session))
            {
                Some(tasks) => session.tasks = tasks,
                // Archivo ausente, JSON roto o identidad distinta: la sesión se
                // queda, pero sin hilo. Nunca se descarta la sesión por esto.
                None => session.thread_available = false,
            }
        }

        sessions.push(session);
    }

    sessions
}

/// Valida el header de una sesión. Devuelve la sesión sin tareas.
///
/// Es la validación pura que también usa [`read_live_sessions`]; no comprueba el
/// nombre del archivo (eso es responsabilidad del lector, que sí lo conoce).
pub fn parse_header(source: &str) -> Option<LiveSession> {
    let value: Value = serde_json::from_str(source).ok()?;
    let object = value.as_object()?;

    if object.get("schema")?.as_u64()? != 1 {
        return None;
    }
    let session_hash = object.get("sessionHash")?.as_str()?;
    if !is_session_hash(session_hash) {
        return None;
    }
    let incarnation = object.get("incarnation")?.as_str()?;
    if !is_uuid_v4(incarnation) {
        return None;
    }
    let label = object.get("label")?.as_str()?;
    let heartbeat = object.get("heartbeat")?.as_u64()?;
    let generation = object.get("generation")?.as_u64()?;
    if generation == 0 {
        return None;
    }
    // Cualquier `unavailable` distinto de null (o ausente) deja el hilo fuera,
    // incluso si el motivo es uno que este lector todavía no conoce.
    let thread_available = matches!(object.get("unavailable"), None | Some(Value::Null));

    Some(LiveSession {
        session_hash: session_hash.to_string(),
        incarnation: incarnation.to_string(),
        label: normalize_label(label),
        heartbeat,
        thread_available,
        tasks: Vec::new(),
    })
}

/// Valida el archivo de actividad y extrae sus tareas.
///
/// `None` significa que el archivo entero es inutilizable (JSON roto, `schema`
/// distinto, identidad que no casa con el header o forma inesperada). Una tarea
/// individual mal formada no invalida el archivo: se omite y las demás siguen.
pub fn parse_activity(source: &str, session: &LiveSession) -> Option<Vec<PresenceTask>> {
    let value: Value = serde_json::from_str(source).ok()?;
    let object = value.as_object()?;

    if object.get("schema")?.as_u64()? != 1 {
        return None;
    }
    if object.get("sessionHash")?.as_str()? != session.session_hash {
        return None;
    }
    if object.get("incarnation")?.as_str()? != session.incarnation {
        return None;
    }
    let tasks = object.get("activity")?.get("tasks")?.as_array()?;

    Some(tasks.iter().filter_map(parse_task).collect())
}

/// Valida una fila de tarea. El `thread` se proyecta a [`AgentThreadItem`].
fn parse_task(value: &Value) -> Option<PresenceTask> {
    let summary = value.get("summary")?.as_object()?;

    let id = summary.get("id")?.as_str()?;
    if id.is_empty() {
        return None;
    }
    let agent = summary.get("agent")?.as_str()?;
    let label = summary.get("label")?.as_str()?;
    let status = summary.get("status")?.as_str()?;
    if !STATUSES.contains(&status) {
        return None;
    }
    // `createdAt` y `lastActivityAt` son obligatorios y enteros no negativos.
    summary.get("createdAt")?.as_u64()?;
    let last_activity_at = summary.get("lastActivityAt")?.as_u64()?;
    // `startedAt` y `endedAt` son obligatorios, pero admiten `null`.
    let started_at = summary.get("startedAt")?;
    if !started_at.is_null() {
        started_at.as_u64()?;
    }
    let ended_at = match summary.get("endedAt")? {
        Value::Null => None,
        other => Some(other.as_u64()?),
    };
    // `lastStep` es opcional: crudo y con `""` de reserva, nunca invalida la fila.
    let last_step = summary
        .get("lastStep")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    Some(PresenceTask {
        id: id.to_string(),
        agent: agent.to_string(),
        label: normalize_label(label),
        status: status.to_string(),
        ended_at,
        last_activity_at,
        last_step,
        thread: parse_thread(value),
    })
}

/// Proyecta el hilo de una fila de tarea.
///
/// `thread` ausente, `items` que no sea un array o un item que no sea un objeto
/// dan hilo vacío o se omiten, sin invalidar la tarea ni el archivo.
fn parse_thread(value: &Value) -> Vec<AgentThreadItem> {
    let Some(items) = value
        .get("thread")
        .and_then(|thread| thread.get("items"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    items.iter().filter_map(parse_thread_item).collect()
}

/// Valida un item del hilo. Nunca se descarta una herramienta por un campo
/// opcional ausente; un item textual sin `text` legible no aporta fila y se omite.
fn parse_thread_item(item: &Value) -> Option<AgentThreadItem> {
    let object = item.as_object()?;
    let kind = object.get("kind").and_then(Value::as_str).unwrap_or("");
    if kind == "tool" {
        return Some(AgentThreadItem::Tool {
            call_id: string_or_empty(object, "callId"),
            name: string_or_empty(object, "name"),
            output: string_or_empty(object, "output"),
            running: object
                .get("running")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            is_error: object
                .get("isError")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        });
    }
    // `text`, `thinking`, `note` (y cualquier tipo textual que publique el
    // contrato en el futuro). Sin texto no hay nada que pintar.
    let text = object.get("text").and_then(Value::as_str)?;
    Some(AgentThreadItem::Text {
        kind: kind.to_string(),
        text: text.to_string(),
    })
}

/// Campo de texto opcional: ausente o `null` cae a `""`.
fn string_or_empty(object: &Map<String, Value>, key: &str) -> String {
    object
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// Vivencia exacta del contrato: ni futuro ni más viejo que la ventana.
fn is_live(heartbeat: u64, now_ms: u64) -> bool {
    now_ms >= heartbeat && now_ms - heartbeat <= LIVE_WINDOW_MS
}

/// `sessionHash`: 64 dígitos hexadecimales en minúscula.
fn is_session_hash(raw: &str) -> bool {
    raw.len() == 64 && raw.bytes().all(is_lower_hex)
}

/// `incarnation`: UUID v4 en minúsculas, con la forma `8-4-4-4-12`.
fn is_uuid_v4(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    bytes.iter().enumerate().all(|(index, byte)| match index {
        8 | 13 | 18 | 23 => *byte == b'-',
        14 => *byte == b'4',
        19 => matches!(*byte, b'8' | b'9' | b'a' | b'b'),
        _ => is_lower_hex(*byte),
    })
}

fn is_lower_hex(byte: u8) -> bool {
    byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
}

/// Normaliza una etiqueta como el contrato: control a espacio, espacios
/// colapsados, sin extremos y cortada a 120 caracteres.
fn normalize_label(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut pending_space = false;
    for ch in raw.chars() {
        if ch.is_control() || ch.is_whitespace() {
            // Solo cuenta como separador si ya hay contenido: así se recorta el
            // inicio sin una pasada aparte.
            pending_space = !out.is_empty();
            continue;
        }
        if pending_space {
            out.push(' ');
            pending_space = false;
        }
        out.push(ch);
    }
    let truncated: String = out.chars().take(120).collect();
    truncated.trim_end().to_string()
}
