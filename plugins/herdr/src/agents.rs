//! Subagentes de `pi` (gentle-pi) leídos en tiempo real.
//!
//! gentle-pi deja un archivo JSON por subagente en
//! `~/.pi/agent/gentle-agents/tasks/`. Aquí solo se leen: ni se escriben ni se
//! launching. Es exactamente el mismo patrón que el watcher de configuración del
//! núcleo, pero acotado a lo que el plugin necesita pintar.
//!
//! # Por qué hay caché y no se lee en cada render
//!
//! La ventana de PORT se repinta unas 60 veces por segundo porque el latido del
//! PTY pide un refresco constante, y `left_sidebar()` se construye en cada uno de
//! esos renders. Leer 1259 archivos JSON a 60 Hz no deja CPU para nada más.
//!
//! La solución es una sola llamada barata por render: `stat` sobre la CARPETA.
//! El mtime de un directorio cambia cuando se crea o borra un archivo dentro,
//! que es justo cuando hay algo nuevo que ver. Si el mtime no cambió, la lista
//! cacheada se devuelve sin tocar ni un archivo.
//!
//! Consecuencia: un subagente nuevo aparece en el siguiente frame (~16 ms), y en
//! reposo el coste es un `stat`.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Cuántos subagentes se pintan a la vez por defecto.
pub const DEFAULT_VISIBLE_LIMIT: usize = 6;

/// Dónde guarda gentle-pi la descripción de cada subagente.
pub fn default_tasks_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("PORT_PI_TASKS_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".pi")
        .join("agent")
        .join("gentle-agents")
        .join("tasks")
}

/// Estado de un subagente tal como lo reporta gentle-pi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentStatus {
    Running,
    Queued,
    Completed,
    Failed,
    /// Un estado que no conocemos: se muestra, pero no se presume bueno.
    Unknown(String),
}

impl AgentStatus {
    /// Un subagente en curso o en cola es lo que el usuario necesita notar.
    pub fn is_live(&self) -> bool {
        matches!(self, Self::Running | Self::Queued)
    }

    /// Etiqueta corta en inglés, como el resto de la UI.
    pub fn label(&self) -> String {
        match self {
            Self::Running => "running".to_string(),
            Self::Queued => "queued".to_string(),
            Self::Completed => "done".to_string(),
            Self::Failed => "failed".to_string(),
            Self::Unknown(raw) => raw.clone(),
        }
    }

    fn parse(raw: &str) -> Self {
        match raw.trim().to_lowercase().as_str() {
            "running" | "in_progress" | "active" => Self::Running,
            "queued" | "pending" | "waiting" => Self::Queued,
            "completed" | "done" | "finished" | "success" => Self::Completed,
            "failed" | "error" | "aborted" | "cancelled" | "canceled" => Self::Failed,
            other => Self::Unknown(other.to_string()),
        }
    }
}

/// Un subagente tal y como se pinta en la sidebar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentEntry {
    pub id: String,
    pub agent: String,
    pub label: String,
    pub status: AgentStatus,
    /// Epoch en milisegundos. `0` si el campo no venía.
    pub last_activity_at: u64,
    pub turns: u32,
    pub tool_calls: u32,
    /// UUID de la sesión, extraído del nombre del archivo `.jsonl`.
    pub session_uuid: Option<String>,
}

/// Comando que reabre un subagente en una sesión de pi.
///
/// Un plugin no puede lanzar procesos, así que esto se expone y se documenta
/// para que el usuario lo ate a una tecla; no se ejecuta desde aquí.
pub fn resume_command(session_dir: &Path, uuid: &str) -> String {
    format!(
        "pi --session-dir {} --session {}",
        session_dir.display(),
        uuid
    )
}

/// Directorio de sesiones, que es el hermano de `tasks/`.
pub fn sessions_dir(tasks_dir: &Path) -> PathBuf {
    tasks_dir
        .parent()
        .map(|parent| parent.join("sessions"))
        .unwrap_or_else(|| PathBuf::from("sessions"))
}

/// Saca el UUID del nombre de un `.jsonl` de sesión.
///
/// Los nombres tienen la forma `2026-10-05T12-41-58-266Z_<uuid>.jsonl`, pero
/// también se acepta un nombre que sea solo el UUID: si no hay `_`, el nombre
/// sin extensión ES el identificador.
pub fn uuid_from_session_path(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let stem = name.strip_suffix(".jsonl").unwrap_or(name);
    let candidate = match stem.rsplit_once('_') {
        Some((_, tail)) => tail,
        None => stem,
    };
    let candidate = candidate.trim();
    if candidate.is_empty() {
        None
    } else {
        Some(candidate.to_string())
    }
}

// ── Lectura de JSON ────────────────────────────────────────────────────────
//
// A propósito sin `serde_json`: el workspace no lo trae y añadir una
// dependencia por leer cinco campos sería más ruido que el propio lector. Solo
// se承认 lo que se usa y se descarta el resto.

/// Busca el valor de una clave de primer nivel dentro de un objeto JSON.
fn json_field<'a>(source: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("\"{key}\"");
    let mut from = 0usize;
    while let Some(at) = source[from..].find(&needle) {
        let start = from + at + needle.len();
        let bytes = source.as_bytes();
        // Saltar espacios y el `:` antes del valor.
        let mut i = start;
        while i < bytes.len() && (bytes[i] as char).is_whitespace() {
            i += 1;
        }
        if i < bytes.len() && bytes[i] == b':' {
            i += 1;
            while i < bytes.len() && (bytes[i] as char).is_whitespace() {
                i += 1;
            }
            if i < bytes.len() {
                return Some(&source[i..]);
            }
        }
        from = start;
    }
    None
}

/// Valor de una clave como string, sin comillas ni escapes.
fn json_string(source: &str, key: &str) -> Option<String> {
    let raw = json_field(source, key)?;
    let raw = raw.trim_start();
    if !raw.starts_with('"') {
        // Puede ser null, un número o un objeto: no es un string.
        return None;
    }
    let bytes = raw.as_bytes();
    let mut out = String::new();
    let mut i = 1usize;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => return Some(out),
            b'\\' if i + 1 < bytes.len() => {
                i += 1;
                out.push(bytes[i] as char);
            }
            other => out.push(other as char),
        }
        i += 1;
    }
    None
}

/// Valor de una clave como entero, aceptando también `12.0`.
fn json_u64(source: &str, key: &str) -> Option<u64> {
    let raw = json_field(source, key)?;
    let raw = raw.trim_start();
    if raw.starts_with('"') {
        return json_string(source, key)?.parse().ok();
    }
    let end = raw
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == 'e' || c == '+'))
        .unwrap_or(raw.len());
    let number: f64 = raw[..end].parse().ok()?;
    if number < 0.0 {
        None
    } else {
        Some(number as u64)
    }
}

/// Parsea un archivo de task de gentle-pi.
///
/// Devuelve `None` si el archivo no es un task válido. Un JSON roto no debe
/// romper la sidebar, así que se descarta en silencio.
pub fn parse_task(source: &str) -> Option<AgentEntry> {
    let id = json_string(source, "id")?;
    let agent = json_string(source, "agent").unwrap_or_else(|| "agent".to_string());
    let label = json_string(source, "label").unwrap_or_default();
    let status = json_string(source, "status")
        .map(|raw| AgentStatus::parse(&raw))
        .unwrap_or(AgentStatus::Unknown("unknown".to_string()));
    let session_uuid =
        json_string(source, "sessionPath").and_then(|raw| uuid_from_session_path(Path::new(&raw)));

    Some(AgentEntry {
        id,
        agent,
        label,
        status,
        last_activity_at: json_u64(source, "lastActivityAt").unwrap_or(0),
        turns: json_u64(source, "turns").unwrap_or(0) as u32,
        tool_calls: json_u64(source, "toolCalls").unwrap_or(0) as u32,
        session_uuid,
    })
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

/// Lee los tasks del disco, sin caché. Pensado para tests y para el refresco.
pub fn read_tasks(dir: &Path) -> Vec<AgentEntry> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<AgentEntry> = entries
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .filter_map(|text| parse_task(&text))
        .collect();
    order(&mut out);
    out
}

/// Caché con detección de cambios por `mtime` de la carpeta.
///
/// Es lo que hace que leer sea "en tiempo real" sin coste en reposo: un `stat`
/// por render, y relectura completa solo cuando hay algo nuevo.
#[derive(Debug, Default)]
pub struct AgentWatcher {
    dir: PathBuf,
    stamp: Option<SystemTime>,
    cached: Vec<AgentEntry>,
    /// Cuántas veces se ha reescrito la caché. Sirve para demostrar en los tests
    /// que una lectura sin cambios no vuelve a tocar el disco.
    pub refreshes: u32,
}

impl AgentWatcher {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            stamp: None,
            cached: Vec::new(),
            refreshes: 0,
        }
    }

    /// Lista de subagentes, releyendo solo si la carpeta cambió.
    pub fn entries(&mut self) -> &[AgentEntry] {
        let stamp = std::fs::metadata(&self.dir).and_then(|m| m.modified()).ok();
        if stamp != self.stamp {
            self.stamp = stamp;
            self.cached = read_tasks(&self.dir);
            self.refreshes += 1;
        }
        &self.cached
    }

    /// Subagentes vivos: los que el usuario necesita notar de un vistazo.
    pub fn live_count(&mut self) -> usize {
        self.entries().iter().filter(|e| e.status.is_live()).count()
    }

    /// Los que se pintan: vivos primero, con tope.
    pub fn visible(&mut self, limit: usize) -> Vec<AgentEntry> {
        self.entries().iter().take(limit).cloned().collect()
    }
}
