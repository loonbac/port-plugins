//! Herdr Customization Plugin para PORT.
//!
//! Transforma PORT en el entorno visual y de flujo de trabajo de Herdr:
//! - Inicio limpio: al abrir la terminal es 100 % terminal sin barras invasivas.
//! - Con `Ctrl + Shift + T` se crea una nueva pestaña en el espacio activo, con su propio shell PTY.
//! - Con `Alt + Left` y `Alt + Right` se navega entre las pestañas del espacio activo.
//! - Con `Ctrl + Alt + T` se crea un nuevo Espacio (Space) aparte con su propia terminal y la barra lateral izquierda queda fija y visible.
//! - Barra lateral y superior completamente transparentes, compartiendo el color de la terminal.
//! - Color de acento sincronizado en tiempo real con el wallpaper de NixOS (`~/.config/mpvpaper/accent.txt`).
//! - Configurable y sincronizado a través de `~/.config/port/config.md`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use gpui::prelude::*;
use gpui::{
    div, px, rgb, AnyElement, FontWeight, IntoElement, MouseButton, ParentElement, Styled, Window,
};
use port_plugin_api::{
    AppearanceHook, ConfigFile, InputHook, KeyAction, LayoutHook, Plugin, PluginConfig, SpaceHook,
};
use port_term_core::frame::Rgb;
use port_term_core::input::Key;
use port_term_core::pty::{PtyConfig, RunningApp};

pub(crate) mod agents;
pub(crate) mod presence;
pub(crate) mod viewer;

#[cfg(test)]
mod agents_test;

#[cfg(test)]
mod presence_test;

#[cfg(test)]
mod viewer_test;

/// Obtiene el color de acento del wallpaper activo en NixOS (`~/.config/mpvpaper/accent.txt`).
pub fn system_accent_color() -> Rgb {
    if let Ok(home) = std::env::var("HOME") {
        let path = std::path::Path::new(&home).join(".config/mpvpaper/accent.txt");
        if let Ok(content) = std::fs::read_to_string(path) {
            if let Some(rgb) = parse_hex_color(content.trim()) {
                return rgb;
            }
        }
    }
    // Color de acento por defecto de NixOS (#325573)
    Rgb::new(0x32, 0x55, 0x73)
}

/// Parsea una cadena hexadecimal en formato `#RRGGBB` o `RRGGBB`.
pub fn parse_hex_color(hex: &str) -> Option<Rgb> {
    let clean = hex.strip_prefix('#').unwrap_or(hex).trim();
    if clean.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&clean[0..2], 16).ok()?;
    let g = u8::from_str_radix(&clean[2..4], 16).ok()?;
    let b = u8::from_str_radix(&clean[4..6], 16).ok()?;
    Some(Rgb::new(r, g, b))
}

/// Lector de subagentes de `pi`, compartido por todos los renders.
///
/// Se guarda en el plugin y no en el estado de la sidebar porque su trabajo es
/// exactamente el del watcher de configuración: mantener el dato fresco sin
/// que la UI tenga que pedirlo.
type SharedAgents = Arc<std::sync::Mutex<agents::AgentWatcher>>;

fn to_hsla(rgb_val: Rgb) -> gpui::Hsla {
    let packed = ((rgb_val.r as u32) << 16) | ((rgb_val.g as u32) << 8) | (rgb_val.b as u32);
    rgb(packed).into()
}

/// Estado original de una pestaña que el visor de un subagente está ocupando.
///
/// El clic no crea ninguna pestaña: retargetea la activa al visor y guarda aquí
/// lo que el usuario tenía a la vista, para devolvérselo tal cual cuando la
/// sesión del visor se cierre. La pestaña nunca se destruye, solo se restaura.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchedTab {
    /// Sesión original de la pestaña, la que hay que devolverle al salir.
    pub session_id: usize,
    /// Título original de la pestaña.
    pub title: String,
    /// Si el título original estaba fijado por el plugin.
    pub title_locked: bool,
}

/// Definición de una pestaña de terminal con su sesión asociada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HerdrTab {
    pub id: usize,
    pub title: String,
    pub session_id: usize,
    /// Programa que se está ejecutando en primer plano en esta pestaña.
    pub app: Option<RunningApp>,
    /// Título fijado por el plugin y no por el directorio de trabajo.
    ///
    /// Lo usa la pestaña del visor de un subagente: `update_session_cwd`
    /// renombra las pestañas normales con el nombre de la carpeta, pero pisar el
    /// título del visor borraría el nombre del subagente que se está mirando.
    pub title_locked: bool,
    /// Terminal original que el visor de un subagente está ocupando, si lo hay.
    ///
    /// Mientras está presente, la pestaña muestra la sesión del visor en lugar
    /// de la suya; [`WatchedTab`] conserva lo necesario para devolverla a su
    /// terminal original cuando el visor se cierre.
    pub watching: Option<WatchedTab>,
}

impl HerdrTab {
    /// Sesión del visor que ocupa esta pestaña, si el núcleo ya la creó.
    ///
    /// Antes de que exista, `session_id` sigue siendo el del terminal original
    /// guardado en `watching`: en esa ventana la pestaña todavía muestra el
    /// terminal del usuario y no hay visor que cerrar ni que ignorar.
    fn viewer_session_id(&self) -> Option<usize> {
        let previo = self.watching.as_ref()?;
        (self.session_id != previo.session_id).then_some(self.session_id)
    }
}

/// Identificador centinela para una pestaña que todavía no tiene sesión asignada.
/// Nunca colisiona con un id real de `SessionManager`, que empieza en 0.
pub const PENDING_SESSION: usize = usize::MAX;

/// Iconos Nerd Font (Symbols Nerd Font Mono) para los programas reconocidos.
const ICON_PI: char = '\u{f03ff}'; // md-pi
const ICON_GEMINI: char = '\u{f0a81}'; // md-zodiac_gemini
const ICON_ROBOT: char = '\u{f06a9}'; // md-robot
const ICON_CHIP: char = '\u{f061a}'; // md-chip
const ICON_HEXAGON: char = '\u{f02d8}'; // md-hexagon
const ICON_CPU: char = '\u{f0ee0}'; // md-cpu_64_bit
const ICON_MEMORY: char = '\u{f035b}'; // md-memory
const ICON_CONSOLE: char = '\u{f018d}'; // md-console
const ICON_CURSOR: char = '\u{f01bf}'; // md-cursor_default_outline

/// Traduce el binario de un programa al icono y nombre legible que muestra la pestaña.
fn app_identity(app: &RunningApp) -> (char, String) {
    let bin = app.bin.as_str();

    // Agentes de IA
    if bin == "pi" || bin.starts_with("pi-") || bin.starts_with("pi_") {
        return (ICON_PI, "Pi Agent".to_string());
    }
    if bin.contains("codex") {
        return (ICON_CHIP, "Codex".to_string());
    }
    if bin.contains("claude") {
        return (ICON_ROBOT, "Claude Code".to_string());
    }
    if bin.contains("antigravity") {
        return (ICON_HEXAGON, "Antigravity".to_string());
    }
    if bin.contains("opencode") {
        return (ICON_CPU, "OpenCode".to_string());
    }
    if bin.contains("gemini") {
        return (ICON_GEMINI, "Gemini CLI".to_string());
    }
    if bin.contains("aider") {
        return (ICON_MEMORY, "Aider".to_string());
    }
    if bin.contains("cursor") {
        return (ICON_CURSOR, "Cursor Agent".to_string());
    }

    // Utilidades de terminal conocidas
    match bin {
        "btop" | "htop" | "top" | "bpytop" => {
            return (ICON_CPU, "System Monitor".to_string());
        }
        "nvim" | "vim" | "vi" | "nvim-qt" => {
            return (ICON_MEMORY, "Editor".to_string());
        }
        "less" | "more" | "man" => {
            return (ICON_CONSOLE, "Pager".to_string());
        }
        "python" | "python3" | "node" | "deno" | "bun" => {
            return (ICON_CHIP, "Runtime".to_string());
        }
        _ => {}
    }

    // Cualquier otro agente o programa desconocido: robot + nombre capitalizado.
    (ICON_ROBOT, humanize(bin))
}

/// Identidad visible de una pestaña en la barra superior: `(icono, texto)`.
///
/// El título fijado por el plugin manda siempre sobre la app en primer plano:
/// la pestaña del visor corre `bash -c tail|jq`, y esa app no significa nada
/// para el usuario. Sin título fijado, manda la app (icono + nombre legible) y,
/// en reposo, manda el título de la pestaña.
fn tab_identity(tab: &HerdrTab) -> (Option<char>, String) {
    if tab.title_locked {
        return (None, tab.title.clone());
    }
    match tab.app.as_ref() {
        Some(app) => {
            let (icon, label) = app_identity(app);
            (Some(icon), label)
        }
        None => (None, tab.title.clone()),
    }
}

/// Ancho por defecto de la barra lateral de espacios, en píxeles lógicos.
pub const SIDEBAR_DEFAULT_WIDTH: f32 = 236.0;
/// Ancho mínimo: por debajo el nombre de las ramas deja de caber.
pub const SIDEBAR_MIN_WIDTH: f32 = 170.0;
/// Ancho máximo: evita que el sidebar se coma toda la ventana.
pub const SIDEBAR_MAX_WIDTH: f32 = 520.0;
/// Zona sensible del asidero, en píxeles. Centrada en el borde, se extiende
/// medio ancho a cada lado para que el arrastre no sea preciso al píxel.
const RESIZE_HANDLE_HIT: f32 = 7.0;

/// Convierte `opencode` en `Opencode` para mostrarlo como nombre legible.
fn humanize(bin: &str) -> String {
    let mut chars = bin.chars();
    match chars.next() {
        Some(first) => {
            let mut out: String = first.to_uppercase().collect();
            out.push_str(chars.as_str());
            out
        }
        None => bin.to_string(),
    }
}

/// Definición de un espacio de trabajo en Herdr.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HerdrSpace {
    pub name: String,
    pub branch: String,
    pub custom_color: Option<Rgb>,
    pub tabs: Vec<HerdrTab>,
    pub active_tab_index: usize,
}

/// Petición de visor de un subagente pendiente de que el núcleo cree su sesión.
///
/// La escribe el clic de una fila de AGENTS. Es la primera de las dos
/// preocupaciones del visor: aquí vive lo que el clic sabe (qué sesión seguir,
/// con qué identidad exacta, dónde está su presencia y a qué subagente
/// pertenece). Nunca es una ruta de registro.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingWatch {
    /// Hash de la sesión que se va a seguir.
    pub session_hash: String,
    /// Incarnación (UUID v4) de esa sesión.
    pub incarnation: String,
    /// Etiqueta del subagente dueño de la sesión, para titular su pestaña.
    pub agent_label: String,
    /// Directorio donde vive la actividad publicada de esa sesión.
    pub presence_dir: PathBuf,
}

/// Estado interno del plugin Herdr.
#[derive(Debug, Clone)]
pub struct HerdrState {
    pub active_space_index: usize,
    pub spaces: Vec<HerdrSpace>,
    pub opacity: f32,
    pub accent_mode: String,
    pub new_session_requested: bool,
    pub close_session_requested: Option<usize>,
    /// Visor pedido por un clic y todavía no consumido por el núcleo.
    pending_watch: Option<PendingWatch>,
    /// Segunda preocupación del visor: el título que debe recibir la pestaña de
    /// la próxima sesión que cree el núcleo.
    ///
    /// `take_spawn_session_request` lo deja aquí al consumir `pending_watch`
    /// (la pestaña aún no existe cuando el núcleo pide la sesión) y
    /// `on_session_created` lo aplica y lo agota.
    pending_viewer_title: Option<String>,
    /// Ancho actual de la barra lateral, ajustable arrastrando el asidero.
    pub sidebar_width: f32,
    /// Posición X donde empezó el arrastre de ancho, si lo hay.
    resize_anchor_x: Option<f32>,
    /// Ancho que tenía la barra cuando empezó el arrastre.
    resize_start_width: f32,
    next_tab_id: usize,
    next_space_num: usize,
}

/// Espacio inicial: una sola terminal llamada `terminal` sobre la sesión 0.
fn default_space() -> HerdrSpace {
    HerdrSpace {
        name: "space-1".to_string(),
        branch: "main".to_string(),
        custom_color: None,
        tabs: vec![HerdrTab {
            id: 1,
            title: "terminal".to_string(),
            session_id: 0,
            app: None,
            title_locked: false,
            watching: None,
        }],
        active_tab_index: 0,
    }
}

impl HerdrState {
    /// Resuelve el color de acento actual (leyendo el wallpaper si está en modo "auto").
    pub fn effective_accent(&self) -> Rgb {
        if self.accent_mode.to_lowercase() == "auto" {
            system_accent_color()
        } else if let Some(rgb) = parse_hex_color(&self.accent_mode) {
            rgb
        } else {
            system_accent_color()
        }
    }

    /// Retargetea la pestaña activa del espacio activo al visor de un subagente
    /// y deja pendiente su petición.
    ///
    /// Es el único camino del clic. No crea nada: guarda en la pestaña activa su
    /// terminal original (sesión, título y bloqueo) y marca en ella el visor. Al
    /// ocupar el área principal se siente como cambiar a un espacio limpio, pero
    /// sigue siendo la misma pestaña y el mismo espacio, así que al salir vuelve
    /// exactamente el terminal que el usuario tenía. No pide un shell:
    /// `new_session_requested` se queda en `false` a propósito para que el núcleo
    /// cree la sesión con `take_spawn_session_request` (el visor) y no con un
    /// shell por defecto.
    fn open_watch_tab(
        &mut self,
        presence_dir: &Path,
        session_hash: &str,
        incarnation: &str,
        agent_label: &str,
    ) {
        let space_idx = self.active_space_index;
        if let Some(space) = self.spaces.get_mut(space_idx) {
            let tab_idx = space.active_tab_index;
            if let Some(tab) = space.tabs.get_mut(tab_idx) {
                tab.watching = Some(WatchedTab {
                    session_id: tab.session_id,
                    title: tab.title.clone(),
                    title_locked: tab.title_locked,
                });
            }
        }
        self.pending_watch = Some(PendingWatch {
            session_hash: session_hash.to_string(),
            incarnation: incarnation.to_string(),
            agent_label: agent_label.to_string(),
            presence_dir: presence_dir.to_path_buf(),
        });
    }
}

impl Default for HerdrState {
    fn default() -> Self {
        Self {
            // Inicialmente 1 espacio limpio: la terminal abre pura a pantalla completa
            active_space_index: 0,
            spaces: vec![HerdrSpace {
                name: "space-1".to_string(),
                branch: "main".to_string(),
                custom_color: None,
                tabs: vec![HerdrTab {
                    id: 1,
                    title: "terminal".to_string(),
                    session_id: 0,
                    app: None,
                    title_locked: false,
                    watching: None,
                }],
                active_tab_index: 0,
            }],
            opacity: 0.85,
            accent_mode: "auto".to_string(),
            new_session_requested: false,
            close_session_requested: None,
            pending_watch: None,
            pending_viewer_title: None,
            sidebar_width: SIDEBAR_DEFAULT_WIDTH,
            resize_anchor_x: None,
            resize_start_width: SIDEBAR_DEFAULT_WIDTH,
            next_tab_id: 2,
            next_space_num: 2,
        }
    }
}

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
fn request_watch(
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
const AGENT_STEP_MAX_CHARS: usize = 80;

/// Recorta el último paso a una sola línea y a [`AGENT_STEP_MAX_CHARS`]
/// caracteres, con `…` cuando se corta.
fn clip_last_step(raw: &str) -> String {
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= AGENT_STEP_MAX_CHARS {
        return collapsed;
    }
    let mut clipped: String = collapsed.chars().take(AGENT_STEP_MAX_CHARS).collect();
    clipped.push('…');
    clipped
}

/// Plugin de personalización visual y estructural Herdr.
pub struct HerdrPlugin {
    state: Arc<RwLock<HerdrState>>,
    agents: SharedAgents,
}

impl HerdrPlugin {
    /// Crea una nueva instancia del plugin Herdr con valores por defecto.
    pub fn new() -> Self {
        Self::with_presence_dir(presence::default_presence_dir())
    }

    /// Igual que `new`, pero apuntando a otro directorio de presencia.
    ///
    /// Existe para que las pruebas no lean el disco real: con la ruta por
    /// defecto, un subagente recién terminado en la máquina del desarrollador
    /// cambiaba el resultado del test de "terminal limpio" sin que nada en el
    /// código hubiera cambiado.
    pub fn with_presence_dir(presence_dir: impl Into<PathBuf>) -> Self {
        Self {
            state: Arc::new(RwLock::new(HerdrState::default())),
            agents: Arc::new(std::sync::Mutex::new(agents::AgentWatcher::new(
                presence_dir,
            ))),
        }
    }

    /// Subagentes de `pi` visibles ahora mismo, vivos primero.
    ///
    /// Se lee de la misma cache que usa la sidebar, asi que una llamada
    /// tras otra no vuelve a tocar el disco si no ha cambiado nada.
    pub fn visible_agents(&self, limit: usize) -> Vec<agents::AgentEntry> {
        let mut guard = self
            .agents
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.visible(agents::now_ms(), limit)
    }

    /// Pide el visor para la fila de AGENTS pulsada.
    ///
    /// Devuelve `false` sin pedir nada si la fila no trae identidad de sesión. La
    /// prueba la usa para ejercitar el mismo camino que el clic, que GPUI solo
    /// puede despachar con una ventana real.
    pub fn watch_entry(&self, entry: &agents::AgentEntry) -> bool {
        let presence_dir = self
            .agents
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .presence_dir()
            .to_path_buf();
        request_watch(&self.state, &presence_dir, entry)
    }

    /// ¿La barra lateral tiene que estar en pantalla?
    ///
    /// Un solo criterio decide el ancho que se reserva en el layout y el dibujo
    /// de la barra. Importa que no diverjan: si el ancho reservado fuera 0
    /// mientras la barra se pinta (un solo espacio con subagentes vivos), PORT
    /// la dibujaría fuera de la caja de su contenedor y GPUI, que solo entrega
    /// el ratón dentro de la caja del padre, dejaría de dar el clic a sus filas.
    /// La barra se seguiría viendo y ninguna fila respondería.
    ///
    /// Cuenta tanto los subagentes vivos como los terminados dentro de su gracia
    /// a propósito: los subagentes acaban casi siempre tan rápidos que, sin esa
    /// gracia, la barra se borraba a los pocos segundos de mostrarse.
    fn sidebar_shown(&self, state: &HerdrState, now_ms: u64) -> bool {
        if state.spaces.len() > 1 {
            return true;
        }
        let mut guard = self
            .agents
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        !guard.entries(now_ms).is_empty()
    }

    /// Crea un nuevo espacio aparte del actual, solicita una nueva sesión PTY y lo activa.
    pub fn create_space_and_open_sidebar(&self) {
        let mut s = self.state.write().unwrap();
        let num = s.next_space_num;
        s.next_space_num += 1;

        let next_tab = s.next_tab_id;
        s.next_tab_id += 1;

        s.spaces.push(HerdrSpace {
            name: format!("space-{num}"),
            branch: "main".to_string(),
            custom_color: None,
            tabs: vec![HerdrTab {
                id: next_tab,
                title: "terminal".to_string(),
                session_id: PENDING_SESSION, // on_session_created lo reemplaza
                app: None,
                title_locked: false,
                watching: None,
            }],
            active_tab_index: 0,
        });

        s.active_space_index = s.spaces.len() - 1;
        s.new_session_requested = true;
    }

    /// Crea una nueva pestaña dentro del espacio actualmente activo.
    pub fn create_tab_in_active_space(&self) {
        let mut s = self.state.write().unwrap();
        let tab_id = s.next_tab_id;
        s.next_tab_id += 1;

        let space_idx = s.active_space_index;
        if let Some(space) = s.spaces.get_mut(space_idx) {
            space.tabs.push(HerdrTab {
                id: tab_id,
                title: format!("term {}", space.tabs.len() + 1),
                session_id: PENDING_SESSION, // on_session_created lo reemplaza
                app: None,
                title_locked: false,
                watching: None,
            });
            space.active_tab_index = space.tabs.len() - 1;
            s.new_session_requested = true;
        }
    }

    /// Cambia a la pestaña anterior dentro del espacio actual.
    pub fn select_previous_tab(&self) -> bool {
        let mut s = self.state.write().unwrap();
        let space_idx = s.active_space_index;
        if let Some(space) = s.spaces.get_mut(space_idx) {
            if space.tabs.len() > 1 {
                if space.active_tab_index == 0 {
                    space.active_tab_index = space.tabs.len() - 1;
                } else {
                    space.active_tab_index -= 1;
                }
                return true;
            }
        }
        false
    }

    /// Cambia a la pestaña siguiente dentro del espacio actual.
    pub fn select_next_tab(&self) -> bool {
        let mut s = self.state.write().unwrap();
        let space_idx = s.active_space_index;
        if let Some(space) = s.spaces.get_mut(space_idx) {
            if space.tabs.len() > 1 {
                space.active_tab_index = (space.active_tab_index + 1) % space.tabs.len();
                return true;
            }
        }
        false
    }

    /// Selecciona un espacio por índice.
    /// Ancho actual de la barra lateral.
    pub fn sidebar_width(&self) -> f32 {
        self.state.read().unwrap().sidebar_width
    }

    /// Fija el ancho de la barra lateral, limitado a un rango usable.
    pub fn set_sidebar_width(&self, width: f32) {
        self.state.write().unwrap().sidebar_width =
            width.clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH);
    }

    /// Indica si hay un arrastre de ancho en curso.
    pub fn is_resizing(&self) -> bool {
        self.state.read().unwrap().resize_anchor_x.is_some()
    }

    /// Comienza el arrastre de ancho en la coordenada X indicada.
    pub fn begin_resize(&self, x: f32) {
        let mut s = self.state.write().unwrap();
        s.resize_anchor_x = Some(x);
        s.resize_start_width = s.sidebar_width;
    }

    /// Continúa el arrastre aplicando el desplazamiento del cursor.
    pub fn update_resize(&self, x: f32) -> bool {
        let mut s = self.state.write().unwrap();
        let Some(anchor) = s.resize_anchor_x else {
            return false;
        };
        let next = s.resize_start_width + (x - anchor);
        s.sidebar_width = next.clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH);
        true
    }

    /// Termina el arrastre de ancho.
    pub fn end_resize(&self) {
        self.state.write().unwrap().resize_anchor_x = None;
    }

    /// Selecciona un espacio por índice.
    pub fn select_space(&self, index: usize) {
        let mut s = self.state.write().unwrap();
        if index < s.spaces.len() {
            s.active_space_index = index;
        }
    }

    /// Selecciona una pestaña por índice dentro del espacio activo.
    pub fn select_tab(&self, index: usize) {
        let mut s = self.state.write().unwrap();
        let space_idx = s.active_space_index;
        if let Some(space) = s.spaces.get_mut(space_idx) {
            if index < space.tabs.len() {
                space.active_tab_index = index;
            }
        }
    }

    /// Pide al núcleo cerrar la sesión del visor que ocupa la pestaña activa.
    ///
    /// Devuelve `false` en una pestaña normal, para que su atajo siga su curso.
    /// Cerrar la sesión del visor es lo único que hace salir del visor: el
    /// núcleo avisa con `on_session_closed` y ahí la pestaña se restaura.
    pub fn request_viewer_close(&self) -> bool {
        let mut s = self.state.write().unwrap();
        let viewer_session_id = s
            .spaces
            .get(s.active_space_index)
            .and_then(|space| space.tabs.get(space.active_tab_index))
            .and_then(HerdrTab::viewer_session_id);
        match viewer_session_id {
            Some(session_id) => {
                s.close_session_requested = Some(session_id);
                true
            }
            None => false,
        }
    }

    /// Cierra la pestaña activa en el espacio actual si hay más de una.
    ///
    /// Una pestaña ocupada por el visor no se borra nunca: se le pide al núcleo
    /// cerrar la sesión del visor y `on_session_closed` la devuelve a su
    /// terminal original.
    pub fn close_active_tab(&self) -> bool {
        if self.request_viewer_close() {
            return true;
        }
        let mut s = self.state.write().unwrap();
        let space_idx = s.active_space_index;
        let mut closed_session = None;
        if let Some(space) = s.spaces.get_mut(space_idx) {
            if space.tabs.len() > 1 {
                let tab_idx = space.active_tab_index;
                let removed_tab = space.tabs.remove(tab_idx);
                closed_session = Some(removed_tab.session_id);
                if space.active_tab_index >= space.tabs.len() {
                    space.active_tab_index = space.tabs.len() - 1;
                }
            }
        }
        if let Some(sess_id) = closed_session {
            s.close_session_requested = Some(sess_id);
            true
        } else {
            false
        }
    }

    /// Guarda la configuración actual en la ruta predeterminada (`~/.config/port/config.md`).
    pub fn save_to_default_file(&self) -> std::io::Result<()> {
        if let Some(config) = self.save_config() {
            ConfigFile::save_plugin(&ConfigFile::default_path(), self.id(), &config)?;
        }
        Ok(())
    }
}

impl Default for HerdrPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl AppearanceHook for HerdrPlugin {
    fn background_tint(&self, base: Rgb) -> Rgb {
        base
    }

    fn opacity(&self) -> Option<f32> {
        Some(self.state.read().unwrap().opacity)
    }
}

impl SpaceHook for HerdrPlugin {
    fn active_session(&self) -> usize {
        let s = self.state.read().unwrap();
        if let Some(space) = s.spaces.get(s.active_space_index) {
            if let Some(tab) = space.tabs.get(space.active_tab_index) {
                return tab.session_id;
            }
        }
        0
    }

    fn active_space(&self) -> usize {
        self.state.read().unwrap().active_space_index
    }

    fn take_new_session_request(&self) -> bool {
        let mut s = self.state.write().unwrap();
        let req = s.new_session_requested;
        s.new_session_requested = false;
        req
    }

    /// Petición de una sesión que corre el visor de un subagente.
    ///
    /// Es de un solo uso: la primera llamada se lleva el visor pendiente y
    /// guarda su etiqueta para la pestaña; las siguientes devuelven `None`.
    fn take_spawn_session_request(&self) -> Option<PtyConfig> {
        let mut s = self.state.write().unwrap();
        let watch = s.pending_watch.take()?;
        // La etiqueta sobrevive a la petición porque la pestaña que la va a
        // recibir no existe hasta que el núcleo cree la sesión.
        s.pending_viewer_title = Some(watch.agent_label.clone());
        Some(viewer::watch_command(
            &watch.presence_dir,
            &watch.session_hash,
            &watch.incarnation,
            &watch.agent_label,
        ))
    }

    fn on_session_created(&self, session_id: usize) {
        let mut s = self.state.write().unwrap();
        // El título del visor se aplica una sola vez: la pestaña activa —la que
        // el clic dejó apuntando al visor, o la que el núcleo acaba de crear sin
        // sesión— lo recibe, y se agota para que la siguiente no lo herede.
        let target_is_viewer = s
            .spaces
            .get(s.active_space_index)
            .and_then(|space| space.tabs.get(space.active_tab_index))
            .is_some_and(|tab| tab.watching.is_some() || tab.session_id == PENDING_SESSION);
        let viewer_title = if target_is_viewer {
            s.pending_viewer_title.take()
        } else {
            None
        };

        let space_idx = s.active_space_index;
        if let Some(space) = s.spaces.get_mut(space_idx) {
            let tab_idx = space.active_tab_index;
            if let Some(tab) = space.tabs.get_mut(tab_idx) {
                tab.session_id = session_id;
                if let Some(title) = viewer_title {
                    tab.title = title;
                    tab.title_locked = true;
                }
            }
        }
    }

    fn update_session_app(&self, session_id: usize, app: Option<&RunningApp>) {
        let mut s = self.state.write().unwrap();
        for space in s.spaces.iter_mut() {
            for tab in space.tabs.iter_mut() {
                // Una pestaña sin sesión asignada no puede recibir datos: si se
                // aceptara, heredaría la app de otra pestaña por colisión de id.
                // La pestaña que el visor ocupa tampoco: su app es `bash -c
                // tail|jq` y no significa nada para el usuario.
                if tab.session_id == session_id
                    && tab.session_id != PENDING_SESSION
                    && tab.viewer_session_id() != Some(session_id)
                {
                    tab.app = app.cloned();
                }
            }
        }
    }

    fn take_close_session_request(&self) -> Option<usize> {
        let mut s = self.state.write().unwrap();
        s.close_session_requested.take()
    }

    fn on_session_closed(&self, session_id: usize) {
        let mut s = self.state.write().unwrap();

        // La sesión del visor no se lleva su pestaña: la pestaña vuelve a su
        // terminal original con la sesión, el título y el bloqueo que tenía.
        for space in s.spaces.iter_mut() {
            for tab in space.tabs.iter_mut() {
                if tab.viewer_session_id() == Some(session_id) {
                    if let Some(previo) = tab.watching.take() {
                        tab.session_id = previo.session_id;
                        tab.title = previo.title;
                        tab.title_locked = previo.title_locked;
                    }
                }
            }
        }

        // Quita la pestaña que pertenecía a esa sesión.
        for space in s.spaces.iter_mut() {
            space.tabs.retain(|t| t.session_id != session_id);
        }

        // Un espacio sin pestañas deja de existir: si la terminal entero se
        // cierra, no debe quedar un espacio fantasma en la barra lateral.
        s.spaces.retain(|space| !space.tabs.is_empty());

        if s.spaces.is_empty() {
            // La ventana está a punto de cerrarse, pero dejamos el estado
            // coherente por si alguien vuelve a mirar el registro.
            s.spaces.push(default_space());
            s.active_space_index = 0;
        } else if s.active_space_index >= s.spaces.len() {
            s.active_space_index = s.spaces.len() - 1;
        }
    }

    fn update_session_cwd(&self, session_id: usize, cwd: &Path, folder_name: &str) {
        let mut s = self.state.write().unwrap();
        for space in s.spaces.iter_mut() {
            // Solo el espacio propietario de la sesión se renombra. Una pestaña
            // con título bloqueado (el visor de un subagente) no cuenta como
            // dueña: su sesión vive dentro del espacio del usuario, y dejar que
            // su cwd renombrara el espacio pisaría el contexto que el usuario ya
            // tenía. El nombre del subagente vive en la pestaña, no en el
            // espacio.
            let contains_session = space.tabs.iter().any(|t| {
                t.session_id == session_id
                    && t.session_id != PENDING_SESSION
                    && !t.title_locked
                    && t.viewer_session_id() != Some(session_id)
            });
            if !contains_session {
                continue;
            }

            // El espacio propietario de la sesión se renombra con la carpeta.
            if !folder_name.is_empty() {
                space.name = folder_name.to_string();
            }

            // Si la pestaña tiene nombre por defecto, se actualiza con la carpeta.
            // La pestaña del visor queda fuera: su título es el nombre del
            // subagente que se está mirando, no la carpeta del shell, y
            // pisarlo borraría justo el dato que el usuario abrió a ver.
            for tab in space.tabs.iter_mut() {
                if tab.session_id == session_id
                    && tab.session_id != PENDING_SESSION
                    && !tab.title_locked
                    && tab.viewer_session_id() != Some(session_id)
                    && (tab.title == "terminal" || tab.title.starts_with("term "))
                {
                    tab.title = folder_name.to_string();
                }
            }

            // Detecta automáticamente la rama git si la carpeta está en un repositorio.
            space.branch = detect_git_branch(cwd).unwrap_or_else(|| "local".to_string());
            break;
        }
    }
}

/// Detecta la rama activa de git leyendo directamente `.git/HEAD` sin invocar subprocesos.
fn detect_git_branch(cwd: &Path) -> Option<String> {
    let mut current = Some(cwd);
    while let Some(dir) = current {
        let git_head = dir.join(".git").join("HEAD");
        if git_head.exists() {
            if let Ok(content) = std::fs::read_to_string(&git_head) {
                let trimmed = content.trim();
                if let Some(branch) = trimmed.strip_prefix("ref: refs/heads/") {
                    return Some(branch.to_string());
                } else if trimmed.len() >= 7 {
                    return Some(trimmed[..7].to_string());
                }
            }
            break;
        }
        current = dir.parent();
    }
    None
}

impl LayoutHook for HerdrPlugin {
    fn left_sidebar_width(&self) -> f32 {
        let s = self.state.read().unwrap();
        if self.sidebar_shown(&s, agents::now_ms()) {
            s.sidebar_width
        } else {
            0.0
        }
    }

    fn top_bar_height(&self) -> f32 {
        let s = self.state.read().unwrap();
        let space_tabs_len = s
            .spaces
            .get(s.active_space_index)
            .map(|sp| sp.tabs.len())
            .unwrap_or(0);

        if space_tabs_len > 1 || s.spaces.len() > 1 {
            38.0
        } else {
            0.0
        }
    }

    fn left_sidebar(&self) -> Option<AnyElement> {
        let state = self.state.read().unwrap();
        // Subagentes de pi, leidos en vivo. Se consultan ANTES del gate: si
        // hay alguno visible, la sidebar tiene que aparecer aunque solo haya un
        // espacio, que es el estado normal de una terminal recien abierta.
        // El reloj se toma una sola vez por render y se reutiliza en la lista.
        let now = agents::now_ms();

        // El mismo criterio que reserva el ancho de la barra decide si se pinta
        // (ver `sidebar_shown`). Si sólo hay un espacio y no hay subagentes
        // visibles, la terminal es limpia.
        if !self.sidebar_shown(&state, now) {
            return None;
        }

        let active_space_idx = state.active_space_index;
        let spaces = state.spaces.clone();
        let opacity = state.opacity;
        let accent = to_hsla(state.effective_accent());
        let state_sidebar_width = state.sidebar_width;
        let state_is_resizing = state.resize_anchor_x.is_some();
        drop(state);

        let term_bg = to_hsla(Rgb::DEFAULT_BG);
        let mut spaces_list = div().flex().flex_col().gap(px(4.0));

        for (i, space) in spaces.iter().enumerate() {
            let is_active = i == active_space_idx;
            let dot_color = space.custom_color.map(to_hsla).unwrap_or(accent);

            // Fondo y borde del espacio: transparente con tinte del acento si está activo
            let (card_bg, card_border) = if is_active {
                (accent.opacity(0.20), accent.opacity(0.55))
            } else {
                (term_bg.opacity(0.0), term_bg.opacity(0.0))
            };

            let state_for_click = Arc::clone(&self.state);
            let item = div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .px(px(10.0))
                .py(px(6.0))
                .rounded(px(6.0))
                .bg(card_bg)
                .border_1()
                .border_color(card_border)
                .on_mouse_down(
                    MouseButton::Left,
                    move |_event, window: &mut Window, _cx| {
                        let mut s = state_for_click.write().unwrap();
                        s.active_space_index = i;
                        window.refresh();
                    },
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(10.0))
                        .child(div().w(px(8.0)).h(px(8.0)).rounded(px(4.0)).bg(dot_color))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .text_size(px(13.0))
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(if is_active {
                                            rgb(0xf0f6fc)
                                        } else {
                                            rgb(0xc9d1d9)
                                        })
                                        .child(space.name.clone()),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.0))
                                        .text_color(if is_active {
                                            accent
                                        } else {
                                            rgb(0x8b949e).into()
                                        })
                                        .child(space.branch.clone()),
                                ),
                        ),
                );

            spaces_list = spaces_list.child(item);
        }

        // ── Sección AGENTS ──────────────────────────────────────────────────
        // Lista de subagentes de pi, en vivo. Los vivos van arriba y llevan el
        // punto encendido; los terminados, en apagado, para que la diferencia
        // se lea de un vistazo sin leer una sola palabra.
        //
        // Cada fila es pulsable: abre el visor de su sesión. La copia del
        // estado se toma fuera del cierre y se escribe bajo el candado dentro,
        // igual que el resto de filas pulsables de este archivo.
        let state_for_agent_rows = Arc::clone(&self.state);
        let agent_rows = {
            let mut guard = self
                .agents
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            // El clic necesita el directorio de presencia para construir el
            // comando del visor; se copia antes del préstamo mutable.
            let presence_dir = guard.presence_dir().to_path_buf();
            guard
                .visible(now, agents::DEFAULT_VISIBLE_LIMIT)
                .into_iter()
                .map(|entry| {
                    let dot = if entry.status.is_live() {
                        accent
                    } else {
                        rgb(0x6e7681).into()
                    };
                    // El cierre se puede invocar más de una vez, así que la
                    // fila se captura por valor en lugar de prestarse.
                    let row_entry = entry.clone();
                    let state_for_click = Arc::clone(&state_for_agent_rows);
                    let presence_dir_for_click = presence_dir.clone();
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .px(px(8.0))
                        .py(px(6.0))
                        .rounded(px(6.0))
                        .bg(accent.opacity(0.10))
                        .border_1()
                        .border_color(accent.opacity(0.28))
                        .cursor_pointer()
                        .on_mouse_down(
                            MouseButton::Left,
                            move |_event, window: &mut Window, _cx| {
                                // Sin identidad de sesión no hay nada que seguir.
                                if request_watch(
                                    &state_for_click,
                                    &presence_dir_for_click,
                                    &row_entry,
                                ) {
                                    window.refresh();
                                }
                            },
                        )
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .items_center()
                                        .gap(px(6.0))
                                        .child(div().w(px(6.0)).h(px(6.0)).rounded(px(3.0)).bg(dot))
                                        .child(
                                            div()
                                                .text_size(px(12.0))
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(if entry.status.is_live() {
                                                    rgb(0xf0f6fc)
                                                } else {
                                                    rgb(0x8b949e)
                                                })
                                                .child(entry.agent.clone()),
                                        ),
                                )
                                .child(
                                    div()
                                        .text_size(px(10.0))
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(if entry.status.is_live() {
                                            accent
                                        } else {
                                            rgb(0x6e7681).into()
                                        })
                                        .child(entry.status.label()),
                                ),
                        )
                        .child(div().text_size(px(11.0)).text_color(rgb(0x8b949e)).child(
                            if entry.label.is_empty() {
                                entry.id.clone()
                            } else {
                                entry.label.clone()
                            },
                        ))
                        .child(
                            div()
                                .text_size(px(10.0))
                                .text_color(rgb(0x6e7681))
                                .child(clip_last_step(&entry.last_step)),
                        )
                })
                .collect::<Vec<_>>()
        };

        // Sidebar con fondo idéntico al de la terminal y compartiendo su opacidad
        let width = state_sidebar_width;
        let resizing = state_is_resizing;
        let sidebar = div()
            .w(px(width))
            .h_full()
            .bg(term_bg.opacity(opacity))
            .border_r(px(1.0))
            .border_color(accent.opacity(0.30))
            .p(px(12.0))
            .flex()
            .flex_col()
            .justify_between()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .px(px(6.0))
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(rgb(0x8b949e))
                                    .child("SPACES"),
                            )
                            .child(
                                div()
                                    .px(px(6.0))
                                    .py(px(1.0))
                                    .rounded(px(4.0))
                                    .bg(accent.opacity(0.20))
                                    .border_1()
                                    .border_color(accent.opacity(0.40))
                                    .text_size(px(10.0))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(accent)
                                    .child(format!("{}", spaces.len())),
                            ),
                    )
                    .child(spaces_list)
                    // Subagentes de pi. Solo ocupa sitio si hay alguno, y solo
                    // muestra los primeros: hay cientos en el historico y todos
                    // a la vez no aportan nada.
                    .when(!agent_rows.is_empty(), |this| {
                        this.child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .px(px(6.0))
                                .pt(px(10.0))
                                .mt(px(4.0))
                                .border_t(px(1.0))
                                .border_color(accent.opacity(0.25))
                                .child(
                                    div()
                                        .text_size(px(11.0))
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(rgb(0x8b949e))
                                        .child("AGENTS"),
                                )
                                .child(
                                    div()
                                        .px(px(6.0))
                                        .py(px(1.0))
                                        .rounded(px(4.0))
                                        .bg(accent.opacity(0.20))
                                        .border_1()
                                        .border_color(accent.opacity(0.40))
                                        .text_size(px(10.0))
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(accent)
                                        .child(format!("{}", agent_rows.len())),
                                ),
                        )
                    })
                    .when(!agent_rows.is_empty(), |this| {
                        this.child(div().flex().flex_col().gap(px(4.0)).children(agent_rows))
                    }),
            )
            .child(
                div()
                    .px(px(6.0))
                    .py(px(6.0))
                    .text_size(px(11.0))
                    .text_color(rgb(0x6e7681))
                    .child("Ctrl+Alt+T Nuevo espacio"),
            );

        // Contenedor relativo: el asidero y la capa de arrastre se posicionan
        // contra su borde derecho.
        let mut shell = div().relative().w(px(width)).h_full().child(sidebar);

        // Asidero de ancho: zona sensible centrada en el borde derecho.
        let state_for_begin = Arc::clone(&self.state);
        let handle = div()
            .absolute()
            .left(px(width - RESIZE_HANDLE_HIT))
            .top(px(0.0))
            .h_full()
            .w(px(RESIZE_HANDLE_HIT * 2.0))
            .cursor_col_resize()
            .child(
                // Pista visual de que el borde es agarrable.
                div()
                    .absolute()
                    .right(px(0.0))
                    .top(px(0.0))
                    .h_full()
                    .w(px(2.0))
                    .bg(accent.opacity(0.0)),
            )
            .on_mouse_down(MouseButton::Left, move |event, window, _cx| {
                let x: f32 = event.position.x.into();
                {
                    let mut s = state_for_begin.write().unwrap();
                    s.resize_anchor_x = Some(x);
                    s.resize_start_width = s.sidebar_width;
                }
                window.refresh();
            });

        shell = shell.child(handle);

        // Mientras se arrastra, una capa invisible cubre toda la ventana para
        // seguir recibiendo movimiento. Debe abarcar también hacia la IZQUIERDA
        // del asidero: estrechar implica mover el cursor hacia atrás, y si la
        // capa empezara en el borde el rastreo se perdería al primer paso.
        if resizing {
            let state_for_move = Arc::clone(&self.state);
            let state_for_end = Arc::clone(&self.state);
            let overlay = div()
                .absolute()
                .left(px(0.0))
                .top(px(0.0))
                .h_full()
                .w(px(6000.0))
                .cursor_col_resize()
                .on_mouse_move(move |event, window, _cx| {
                    let x: f32 = event.position.x.into();
                    let mut s = state_for_move.write().unwrap();
                    // `resize_anchor_x` es None cuando el puntero no esta
                    // arrastrando la barra. Se comprueba antes de escribir
                    // para no ensuciar el estado con un movimiento suelto.
                    let anchor = match s.resize_anchor_x {
                        Some(anchor) => anchor,
                        None => return,
                    };
                    s.sidebar_width = (s.resize_start_width + (x - anchor))
                        .clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH);
                    drop(s);
                    window.refresh();
                })
                .on_mouse_up(MouseButton::Left, move |_event, window, _cx| {
                    state_for_end.write().unwrap().resize_anchor_x = None;
                    window.refresh();
                });
            shell = shell.child(overlay);
        }

        Some(shell.into_any_element())
    }

    fn top_bar(&self) -> Option<AnyElement> {
        let state = self.state.read().unwrap();
        let space_tabs = match state.spaces.get(state.active_space_index) {
            Some(sp) => sp.tabs.clone(),
            None => Vec::new(),
        };
        let active_tab_idx = state
            .spaces
            .get(state.active_space_index)
            .map(|sp| sp.active_tab_index)
            .unwrap_or(0);

        // Si solo hay una pestaña y un solo espacio, no dibujamos barra superior (terminal limpia)
        if space_tabs.len() <= 1 && state.spaces.len() <= 1 {
            return None;
        }

        let opacity = state.opacity;
        let accent = to_hsla(state.effective_accent());
        let term_bg = to_hsla(Rgb::DEFAULT_BG);
        drop(state);

        let mut tabs_row = div().flex().flex_row().items_center().gap(px(6.0));

        for (i, tab) in space_tabs.iter().enumerate() {
            let is_active = i == active_tab_idx;
            let (bg_col, border_col) = if is_active {
                (accent.opacity(0.22), accent.opacity(0.55))
            } else {
                (term_bg.opacity(opacity), accent.opacity(0.18))
            };

            let state_for_click = Arc::clone(&self.state);
            let state_for_close = Arc::clone(&self.state);
            let tab_pill = div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .px(px(10.0))
                .py(px(4.0))
                .rounded(px(6.0))
                .bg(bg_col)
                .border_1()
                .border_color(border_col)
                .on_mouse_down(
                    MouseButton::Left,
                    move |_event, window: &mut Window, _cx| {
                        let mut s = state_for_click.write().unwrap();
                        let space_idx = s.active_space_index;
                        if let Some(sp) = s.spaces.get_mut(space_idx) {
                            sp.active_tab_index = i;
                        }
                        window.refresh();
                    },
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(if is_active {
                            accent
                        } else {
                            rgb(0x6e7681).into()
                        })
                        .child(">"),
                )
                .child(match tab_identity(tab) {
                    // Hay un programa en primer plano: icono Nerd Font + nombre legible
                    (Some(icon), label) => div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .child(
                            div()
                                .text_size(px(12.0))
                                .text_color(accent)
                                .child(icon.to_string()),
                        )
                        .child(
                            div()
                                .text_size(px(12.0))
                                .font_weight(if is_active {
                                    FontWeight::BOLD
                                } else {
                                    FontWeight::NORMAL
                                })
                                .text_color(if is_active {
                                    rgb(0xf0f6fc)
                                } else {
                                    rgb(0x8b949e)
                                })
                                .child(label),
                        ),
                    // Terminal en reposo o título fijado por el plugin: el texto
                    // de la pestaña (nombre del subagente en el visor).
                    (None, label) => div()
                        .text_size(px(12.0))
                        .font_weight(if is_active {
                            FontWeight::BOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .text_color(if is_active {
                            rgb(0xf0f6fc)
                        } else {
                            rgb(0x8b949e)
                        })
                        .child(label),
                })
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(rgb(0x6e7681))
                        .on_mouse_down(
                            MouseButton::Left,
                            move |_event, window: &mut Window, _cx| {
                                let mut s = state_for_close.write().unwrap();
                                let space_idx = s.active_space_index;
                                let mut closed_session = None;
                                if let Some(sp) = s.spaces.get_mut(space_idx) {
                                    if sp.tabs.len() > 1 && i < sp.tabs.len() {
                                        let removed = sp.tabs.remove(i);
                                        closed_session = Some(removed.session_id);
                                        if sp.active_tab_index >= sp.tabs.len() {
                                            sp.active_tab_index = sp.tabs.len() - 1;
                                        }
                                    }
                                }
                                if let Some(sess_id) = closed_session {
                                    s.close_session_requested = Some(sess_id);
                                    window.refresh();
                                }
                            },
                        )
                        .child("×"),
                );

            tabs_row = tabs_row.child(tab_pill);
        }

        let state_for_new = Arc::clone(&self.state);
        let add_tab_btn = div()
            .flex()
            .items_center()
            .justify_center()
            .w(px(24.0))
            .h(px(24.0))
            .rounded(px(4.0))
            .bg(accent.opacity(0.15))
            .border_1()
            .border_color(accent.opacity(0.35))
            .on_mouse_down(
                MouseButton::Left,
                move |_event, window: &mut Window, _cx| {
                    let mut s = state_for_new.write().unwrap();
                    let next_id = s.next_tab_id;
                    s.next_tab_id += 1;
                    let space_idx = s.active_space_index;
                    if let Some(sp) = s.spaces.get_mut(space_idx) {
                        sp.tabs.push(HerdrTab {
                            id: next_id,
                            title: format!("term {}", sp.tabs.len() + 1),
                            session_id: 0,
                            app: None,
                            title_locked: false,
                            watching: None,
                        });
                        sp.active_tab_index = sp.tabs.len() - 1;
                        s.new_session_requested = true;
                    }
                    window.refresh();
                },
            )
            .child(div().text_size(px(13.0)).text_color(accent).child("+"));

        tabs_row = tabs_row.child(add_tab_btn);

        let top_bar = div()
            .h(px(38.0))
            .w_full()
            .bg(term_bg.opacity(opacity))
            .border_b(px(1.0))
            .border_color(accent.opacity(0.30))
            .px(px(10.0))
            .flex()
            .flex_row()
            .items_center()
            .child(tabs_row);

        Some(top_bar.into_any_element())
    }
}

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

impl Plugin for HerdrPlugin {
    fn id(&self) -> &'static str {
        "herdr"
    }

    fn name(&self) -> &'static str {
        "Herdr Customization Plugin"
    }

    fn version(&self) -> &'static str {
        "0.1.0"
    }

    fn appearance_hook(&self) -> Option<&dyn AppearanceHook> {
        Some(self)
    }

    fn space_hook(&self) -> Option<&dyn SpaceHook> {
        Some(self)
    }

    fn layout_hook(&self) -> Option<&dyn LayoutHook> {
        Some(self)
    }

    fn input_hook(&self) -> Option<&dyn InputHook> {
        Some(self)
    }

    fn default_config(&self) -> Option<PluginConfig> {
        let mut cfg = PluginConfig::new();
        cfg.set("opacity", 0.85);
        cfg.set("accent", "auto");
        cfg.set("sidebar_width", SIDEBAR_DEFAULT_WIDTH);
        cfg.set("spaces", "");
        Some(cfg)
    }

    fn load_config(&self, config: &PluginConfig) {
        let mut s = self.state.write().unwrap();
        if let Some(op) = config.get_f32("opacity") {
            s.opacity = op;
        }
        if let Some(acc) = config.get("accent") {
            s.accent_mode = acc.to_string();
        }
        if let Some(width) = config.get_f32("sidebar_width") {
            s.sidebar_width = width.clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH);
            s.resize_start_width = s.sidebar_width;
        }
        if let Some(spaces_str) = config.get("spaces") {
            let mut parsed_spaces = Vec::new();
            for (idx, entry) in spaces_str.split(',').enumerate() {
                if let Some((name, branch)) = entry.trim().split_once(':') {
                    if !name.trim().is_empty() {
                        parsed_spaces.push(HerdrSpace {
                            name: name.trim().to_string(),
                            branch: branch.trim().to_string(),
                            custom_color: None,
                            tabs: vec![HerdrTab {
                                id: idx + 1,
                                title: "terminal".to_string(),
                                session_id: idx,
                                app: None,
                                title_locked: false,
                                watching: None,
                            }],
                            active_tab_index: 0,
                        });
                    }
                }
            }
            if !parsed_spaces.is_empty() {
                s.spaces = parsed_spaces;
            }
        }
    }

    fn save_config(&self) -> Option<PluginConfig> {
        let s = self.state.read().unwrap();
        let mut cfg = PluginConfig::new();
        cfg.set("opacity", s.opacity);
        cfg.set("accent", s.accent_mode.clone());
        cfg.set("sidebar_width", s.sidebar_width);
        let spaces_str = s
            .spaces
            .iter()
            .map(|sp| format!("{}:{}", sp.name, sp.branch))
            .collect::<Vec<_>>()
            .join(",");
        cfg.set("spaces", spaces_str);
        Some(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Presencia vacía aislada del disco real.
    ///
    /// Con la ruta real, un subagente vivo en esta máquina abriría la barra y
    /// los tests que esperan una terminal limpia fallarían sin que nada en el
    /// código hubiera cambiado.
    fn empty_presence(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "herdr-vacio-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("crear presencia vacia");
        dir
    }

    #[test]
    fn parse_hex_color_valid() {
        assert_eq!(parse_hex_color("#325573"), Some(Rgb::new(0x32, 0x55, 0x73)));
        assert_eq!(parse_hex_color("325573"), Some(Rgb::new(0x32, 0x55, 0x73)));
        assert_eq!(parse_hex_color("invalid"), None);
    }

    #[test]
    fn herdr_opens_clean_terminal_by_default() {
        // Presencia vacía: sin subagentes que fuercen la barra.
        let presence = empty_presence("limpio");
        let plugin = HerdrPlugin::with_presence_dir(&presence);
        assert_eq!(plugin.id(), "herdr");
        assert_eq!(plugin.name(), "Herdr Customization Plugin");
        // Al abrir la terminal por defecto es limpia: sin sidebar ni topbar
        assert_eq!(plugin.left_sidebar_width(), 0.0);
        assert_eq!(plugin.top_bar_height(), 0.0);
        assert!(plugin.left_sidebar().is_none());
        assert!(plugin.top_bar().is_none());
        assert_eq!(plugin.opacity(), Some(0.85));
    }

    #[test]
    fn ctrl_shift_t_creates_tab_in_active_space() {
        let plugin = HerdrPlugin::default();
        // Inicialmente 1 tab, sin top bar
        assert_eq!(plugin.top_bar_height(), 0.0);

        // Pulsamos Ctrl+Shift+T
        let ctrl_shift_t = Key::new("t").ctrl().shift();
        assert_eq!(plugin.on_key(&ctrl_shift_t), KeyAction::Consume);

        // Ahora hay 2 tabs en el espacio activo y el top bar aparece
        assert_eq!(plugin.top_bar_height(), 38.0);
        assert!(plugin.top_bar().is_some());
        assert!(plugin.take_new_session_request());

        // Simulamos que el core asigna session_id = 1
        plugin.on_session_created(1);
        assert_eq!(plugin.active_session(), 1);

        // Navegación con Alt+Left y Alt+Right
        let alt_left = Key::new("Left").alt();
        assert_eq!(plugin.on_key(&alt_left), KeyAction::Consume);
        assert_eq!(plugin.active_session(), 0);

        let alt_right = Key::new("Right").alt();
        assert_eq!(plugin.on_key(&alt_right), KeyAction::Consume);
        assert_eq!(plugin.active_session(), 1);

        // Cerrar pestaña activa
        assert!(plugin.close_active_tab());
        assert_eq!(plugin.active_session(), 0);
    }

    #[test]
    fn ctrl_alt_t_creates_space_and_opens_sidebar() {
        // Presencia aislada: el test espera una terminal limpia al empezar, y
        // con la presencia real un subagente vivo ya habría abierto la barra.
        let presence = empty_presence("ctrl-alt-t");
        let plugin = HerdrPlugin::with_presence_dir(&presence);
        assert_eq!(plugin.left_sidebar_width(), 0.0);

        // Pulsamos Ctrl+Alt+T
        let ctrl_alt_t = Key::new("t").ctrl().alt();
        assert_eq!(plugin.on_key(&ctrl_alt_t), KeyAction::Consume);

        // Ahora el sidebar está abierto permanente y tiene ancho 236px
        assert_eq!(plugin.left_sidebar_width(), 236.0);
        assert!(plugin.left_sidebar().is_some());
        assert_eq!(plugin.top_bar_height(), 38.0);
        assert!(plugin.top_bar().is_some());

        // Debe haber 2 espacios: space-1 (el previo) y space-2 (el nuevo)
        let spaces = plugin.state.read().unwrap().spaces.clone();
        assert_eq!(spaces.len(), 2);
        assert_eq!(spaces[0].name, "space-1");
        assert_eq!(spaces[1].name, "space-2");
        assert_eq!(plugin.state.read().unwrap().active_space_index, 1);
        assert!(plugin.take_new_session_request());
    }

    #[test]
    fn herdr_accent_reads_system_color() {
        let plugin = HerdrPlugin::default();
        let accent = plugin.state.read().unwrap().effective_accent();
        assert_eq!(accent, system_accent_color());
    }

    #[test]
    fn herdr_space_updates_name_and_branch_from_cwd() {
        let plugin = HerdrPlugin::default();
        let path = std::path::Path::new("/home/loonbac/Proyectos/port");
        plugin.update_session_cwd(0, path, "port");

        let s = plugin.state.read().unwrap();
        assert_eq!(s.spaces[0].name, "port");
        assert_eq!(s.spaces[0].branch, "master");
    }

    #[test]
    fn ai_agents_get_their_nerd_font_icon_and_readable_name() {
        let cases = [
            ("pi", ICON_PI, "Pi Agent"),
            ("codex", ICON_CHIP, "Codex"),
            ("claude", ICON_ROBOT, "Claude Code"),
            ("antigravity", ICON_HEXAGON, "Antigravity"),
            ("opencode", ICON_CPU, "OpenCode"),
            ("gemini", ICON_GEMINI, "Gemini CLI"),
        ];
        for (bin, icon, label) in cases {
            let app = RunningApp {
                pid: 1,
                bin: bin.to_string(),
            };
            let (got_icon, got_label) = app_identity(&app);
            assert_eq!(got_icon, icon, "icono incorrecto para {bin}");
            assert_eq!(got_label, label, "nombre incorrecto para {bin}");
        }
    }

    #[test]
    fn a_new_tab_never_inherits_the_app_of_another_session() {
        let plugin = HerdrPlugin::default();

        // La primera pestaña corresponde a la sesión 0 real.
        let pi = RunningApp {
            pid: 1,
            bin: "pi".to_string(),
        };
        plugin.update_session_app(0, Some(&pi));

        // Se abre una pestaña nueva: nace sin sesión asignada.
        let ctrl_shift_t = Key::new("t").ctrl().shift();
        plugin.on_key(&ctrl_shift_t);
        plugin.on_session_created(1);

        // La sesión 0 sigue siendo pi, pero la pestaña nueva no puede heredarlo.
        plugin.update_session_app(1, None);

        let s = plugin.state.read().unwrap();
        assert_eq!(s.spaces[0].tabs[0].app, Some(pi));
        assert_eq!(
            s.spaces[0].tabs[1].app, None,
            "una pestaña nueva no debe mostrar la app de otra sesión"
        );
        assert_eq!(s.spaces[0].tabs[1].session_id, 1);
    }

    #[test]
    fn sidebar_width_is_clamped_to_usable_bounds() {
        let plugin = HerdrPlugin::default();
        assert_eq!(plugin.sidebar_width(), SIDEBAR_DEFAULT_WIDTH);

        plugin.set_sidebar_width(9999.0);
        assert_eq!(plugin.sidebar_width(), SIDEBAR_MAX_WIDTH);

        plugin.set_sidebar_width(10.0);
        assert_eq!(plugin.sidebar_width(), SIDEBAR_MIN_WIDTH);

        plugin.set_sidebar_width(320.0);
        assert_eq!(plugin.sidebar_width(), 320.0);
    }

    #[test]
    fn sidebar_width_is_persisted_in_config() {
        let plugin = HerdrPlugin::default();
        plugin.set_sidebar_width(300.0);
        let saved = plugin.save_config().unwrap();
        assert_eq!(saved.get_f32("sidebar_width"), Some(300.0));

        // Una sesión nueva debe recuperar el ancho guardado.
        let other = HerdrPlugin::default();
        other.load_config(&saved);
        assert_eq!(other.sidebar_width(), 300.0);
    }

    #[test]
    fn dragging_the_handle_changes_the_width_from_the_cursor_delta() {
        let plugin = HerdrPlugin::default();
        plugin.set_sidebar_width(236.0);
        plugin.begin_resize(236.0);
        assert!(plugin.is_resizing());

        // Arrastrar 60 px a la derecha ensancha la barra 60 px.
        assert!(plugin.update_resize(296.0));
        assert_eq!(plugin.sidebar_width(), 296.0);

        // Arrastrar de vuelta la devuelve al ancho original.
        assert!(plugin.update_resize(236.0));
        assert_eq!(plugin.sidebar_width(), 236.0);

        plugin.end_resize();
        assert!(!plugin.is_resizing());
        assert!(
            !plugin.update_resize(500.0),
            "sin arrastre activo no debe cambiar"
        );
        assert_eq!(plugin.sidebar_width(), 236.0);
    }

    #[test]
    fn dragging_left_narrows_the_sidebar_down_to_the_limit() {
        let plugin = HerdrPlugin::default();
        plugin.set_sidebar_width(300.0);
        plugin.begin_resize(300.0);

        // Estrechar: el cursor avanza en X negativas respecto al asidero.
        assert!(plugin.update_resize(260.0));
        assert_eq!(plugin.sidebar_width(), 260.0);

        assert!(plugin.update_resize(200.0));
        assert_eq!(plugin.sidebar_width(), 200.0);

        // Más allá del mínimo queda fijado en el límite inferior.
        assert!(plugin.update_resize(40.0));
        assert_eq!(plugin.sidebar_width(), SIDEBAR_MIN_WIDTH);
        assert!(plugin.update_resize(0.0));
        assert_eq!(plugin.sidebar_width(), SIDEBAR_MIN_WIDTH);
    }

    #[test]
    fn closing_a_tab_removes_it_and_deletes_the_space_when_empty() {
        let plugin = HerdrPlugin::default();
        assert_eq!(plugin.state.read().unwrap().spaces.len(), 1);

        // Se abre una segunda pestaña con su propia sesión.
        let ctrl_shift_t = Key::new("t").ctrl().shift();
        plugin.on_key(&ctrl_shift_t);
        plugin.on_session_created(1);
        assert_eq!(
            plugin.state.read().unwrap().spaces[0].tabs.len(),
            2,
            "deben existir dos pestañas en el espacio"
        );

        // Se cierra la segunda pestaña: el espacio conserva su otra pestaña.
        plugin.on_session_closed(1);
        let s = plugin.state.read().unwrap();
        assert_eq!(s.spaces.len(), 1, "el espacio sigue vivo con una pestaña");
        assert_eq!(s.spaces[0].tabs.len(), 1);

        // Se cierra la última: el espacio deja de existir y se vuelve al inicial.
        drop(s);
        plugin.on_session_closed(0);
        let s = plugin.state.read().unwrap();
        assert_eq!(
            s.spaces[0].tabs.len(),
            1,
            "un espacio sin pestañas no debe quedar en la barra lateral"
        );
        assert_eq!(s.active_space_index, 0);
    }

    #[test]
    fn unknown_agents_fall_back_to_robot_and_humanized_name() {
        let app = RunningApp {
            pid: 1,
            bin: "miagente".to_string(),
        };
        let (icon, label) = app_identity(&app);
        assert_eq!(icon, ICON_ROBOT);
        assert_eq!(label, "Miagente");
    }

    #[test]
    fn running_app_updates_the_matching_tab() {
        let plugin = HerdrPlugin::default();
        // Se crean dos pestañas en el espacio 1
        let ctrl_shift_t = Key::new("t").ctrl().shift();
        plugin.on_key(&ctrl_shift_t);
        plugin.on_session_created(1);

        let app = RunningApp {
            pid: 42,
            bin: "pi".to_string(),
        };
        plugin.update_session_app(1, Some(&app));

        let s = plugin.state.read().unwrap();
        assert_eq!(s.spaces[0].tabs[1].app, Some(app));
        assert_eq!(s.spaces[0].tabs[0].app, None);
    }

    // ── Visor por clic ─────────────────────────────────────────────────────

    const WATCH_HASH: &str = "5ad9a04398e3b840b64c906d0f794d3d17a140ed3ada1a0bbcded38b16de3ccc";
    const WATCH_INCARNATION: &str = "81a8a249-55b7-4b19-bddd-f1db0fb2dd69";

    /// Presencia viva con una tarea, en un directorio temporal.
    ///
    /// No hay ningún `tasks/<id>.json`: la identidad y el paso salen de la
    /// presencia, que es la única fuente que existe mientras el subagente corre.
    /// Devuelve la ruta de presencia que espera el plugin.
    fn seed_watch_agent(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "herdr-viewer-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let presence = root.join("gentle-agents").join("presence");
        std::fs::create_dir_all(&presence).expect("crear presencia");

        let now = agents::now_ms();
        let header = format!(
            "{{\"schema\":1,\"sessionHash\":\"{WATCH_HASH}\",\"incarnation\":\"{WATCH_INCARNATION}\",\"label\":\"sesion de prueba\",\"heartbeat\":{now},\"generation\":1,\"unavailable\":null}}"
        );
        std::fs::write(
            presence.join(format!("{WATCH_HASH}.{WATCH_INCARNATION}.header.json")),
            header,
        )
        .expect("escribir header");
        let activity = format!(
            "{{\"schema\":1,\"sessionHash\":\"{WATCH_HASH}\",\"incarnation\":\"{WATCH_INCARNATION}\",\"generation\":1,\"activity\":{{\"tasks\":[{{\"summary\":{{\"id\":\"t_abc123\",\"agent\":\"gentle-ai-worker\",\"label\":\"herdr viewer\",\"status\":\"running\",\"lastStep\":\"reading the presence\",\"createdAt\":1,\"startedAt\":2,\"endedAt\":null,\"lastActivityAt\":{now}}}}}]}}}}"
        );
        std::fs::write(
            presence.join(format!("{WATCH_HASH}.{WATCH_INCARNATION}.activity.json")),
            activity,
        )
        .expect("escribir actividad");

        presence
    }

    #[test]
    fn el_ultimo_paso_se_recorta_a_ochenta_caracteres() {
        assert_eq!(
            clip_last_step("  reading   the  presence "),
            "reading the presence",
            "una sola línea, sin extremos"
        );
        let largo = "a".repeat(120);
        let corto = clip_last_step(&largo);
        assert_eq!(corto.chars().count(), AGENT_STEP_MAX_CHARS + 1);
        assert!(corto.ends_with('…'), "se corta con elipsis");
        assert!(!clip_last_step("queued").ends_with('…'));
    }

    #[test]
    fn con_un_subagente_vivo_la_sidebar_reserva_su_ancho() {
        let presence = seed_watch_agent("ancho");
        let plugin = HerdrPlugin::with_presence_dir(&presence);

        assert_eq!(
            plugin.state.read().unwrap().spaces.len(),
            1,
            "el caso que fallaba es el de un solo espacio"
        );
        // La barra se pinta, así que tiene que reservar ancho. Sin este ancho
        // PORT la dibuja fuera de la caja del layout: se ve, pero ninguna de sus
        // filas recibe el clic (el ancho y el dibujo tienen que coincidir).
        assert!(
            plugin.left_sidebar_width() > 0.0,
            "una sidebar visible tiene que reservar su ancho"
        );
        assert!(plugin.left_sidebar().is_some());
    }

    #[test]
    fn hacer_clic_en_un_subagente_ocupa_la_pestana_activa_sin_crear_nada() {
        let presence = seed_watch_agent("clic");
        let plugin = HerdrPlugin::with_presence_dir(&presence);

        // La fila visible es la que el clic sigue.
        let entry = plugin
            .visible_agents(1)
            .into_iter()
            .next()
            .expect("un subagente visible");
        assert_eq!(entry.session_hash, WATCH_HASH);
        assert_eq!(entry.incarnation, WATCH_INCARNATION);
        assert_eq!(entry.last_step, "reading the presence");
        let expected_label = entry.label.clone();
        assert_eq!(expected_label, "herdr viewer");

        let (espacios_antes, pestanas_antes, id_antes, titulo_antes) = {
            let s = plugin.state.read().unwrap();
            let space = &s.spaces[0];
            (
                s.spaces.len(),
                space.tabs.len(),
                space.tabs[0].id,
                space.tabs[0].title.clone(),
            )
        };
        assert!(plugin.watch_entry(&entry));

        // El clic no crea nada: mismo espacio, misma pestaña, mismo id.
        {
            let s = plugin.state.read().unwrap();
            assert_eq!(
                s.spaces.len(),
                espacios_antes,
                "el visor no crea un espacio"
            );
            assert_eq!(s.active_space_index, 0, "seguimos en el espacio activo");
            let space = &s.spaces[s.active_space_index];
            assert_eq!(
                space.tabs.len(),
                pestanas_antes,
                "el visor no agrega pestañas"
            );
            assert_eq!(
                space.active_tab_index, 0,
                "la pestaña activa es la de siempre"
            );
            let tab = &space.tabs[0];
            assert_eq!(tab.id, id_antes, "la pestaña conserva su identidad");
            // Aún no existe la sesión del visor: la pestaña sigue mostrando el
            // terminal original, pero ya guarda lo necesario para volver a él.
            assert_eq!(tab.session_id, 0, "el terminal original sigue a la vista");
            let previo = tab.watching.as_ref().expect("la pestaña apunta al visor");
            assert_eq!(previo.session_id, 0, "se guardó la sesión original");
            assert_eq!(previo.title, titulo_antes, "se guardó el título original");
            assert!(!previo.title_locked, "se guardó el bloqueo original");
            assert_eq!(
                s.pending_watch
                    .as_ref()
                    .expect("la petición del visor")
                    .agent_label,
                expected_label,
                "la petición lleva la etiqueta del subagente"
            );
            // El resto del espacio conserva su terminal de siempre.
            assert_eq!(space.name, "space-1");
        }
        assert!(
            !plugin.take_new_session_request(),
            "el visor nunca pide un shell por defecto"
        );

        // El núcleo consume la petición una sola vez: el comando lleva la
        // identidad de la sesión y el directorio de presencia, no una ruta de
        // registro.
        let config = plugin
            .take_spawn_session_request()
            .expect("el clic debe pedir una sesión de visor");
        assert_eq!(config.command, "bash");
        assert_eq!(config.args[2], "herdr-view");
        assert_eq!(config.args[3], presence.display().to_string());
        assert_eq!(config.args[4], WATCH_HASH);
        assert_eq!(config.args[5], WATCH_INCARNATION);
        assert_eq!(config.args[6], expected_label);
        assert!(
            plugin.take_spawn_session_request().is_none(),
            "una petición de visor se consume una sola vez"
        );
    }

    #[test]
    fn crear_la_sesion_del_visor_enlaza_la_pestana_activa_y_bloquea_su_titulo() {
        let presence = seed_watch_agent("enlace");
        let plugin = HerdrPlugin::with_presence_dir(&presence);
        let entry = plugin
            .visible_agents(1)
            .into_iter()
            .next()
            .expect("un subagente visible");
        let expected_label = entry.label.clone();
        assert!(plugin.watch_entry(&entry));
        assert!(plugin.take_spawn_session_request().is_some());

        // El núcleo crea la sesión del visor: la MISMA pestaña activa toma su id
        // y la etiqueta del subagente, y queda con el título bloqueado.
        plugin.on_session_created(7);
        {
            let s = plugin.state.read().unwrap();
            let space = &s.spaces[0];
            assert_eq!(space.tabs.len(), 1, "no aparece ninguna pestaña nueva");
            let tab = &space.tabs[0];
            assert_eq!(
                tab.session_id, 7,
                "la pestaña activa pasa a mostrar el visor"
            );
            assert_eq!(tab.title, expected_label, "el título ES el subagente");
            assert!(tab.title_locked, "la pestaña del visor no se renombra");
            assert_eq!(
                tab.watching
                    .as_ref()
                    .expect("sigue apuntando al original")
                    .session_id,
                0,
                "el terminal original sigue guardado para volver a él"
            );
            assert!(s.pending_viewer_title.is_none(), "la etiqueta se agota");
        }
        // El área principal dibuja solo el visor desde que su sesión existe.
        assert_eq!(
            plugin.active_session(),
            7,
            "el núcleo dibuja la sesión del visor, no la del terminal original"
        );
    }

    #[test]
    fn cerrar_la_sesion_del_visor_restaura_el_terminal_original() {
        let presence = seed_watch_agent("restaura");
        let plugin = HerdrPlugin::with_presence_dir(&presence);
        // Título original no trivial: la restauración tiene que devolverlo tal cual.
        plugin.state.write().unwrap().spaces[0].tabs[0].title = "mi terminal".to_string();

        let entry = plugin
            .visible_agents(1)
            .into_iter()
            .next()
            .expect("un subagente visible");
        assert!(plugin.watch_entry(&entry));
        assert!(plugin.take_spawn_session_request().is_some());
        plugin.on_session_created(7);

        plugin.on_session_closed(7);

        let s = plugin.state.read().unwrap();
        assert_eq!(
            s.spaces.len(),
            1,
            "el espacio no se borra al salir del visor"
        );
        let space = &s.spaces[0];
        assert_eq!(space.tabs.len(), 1, "la pestaña se restaura, no se cierra");
        let tab = &space.tabs[0];
        assert_eq!(tab.session_id, 0, "vuelve el terminal original");
        assert_eq!(tab.title, "mi terminal", "vuelve el título original");
        assert!(!tab.title_locked, "vuelve el bloqueo original");
        assert!(
            tab.watching.is_none(),
            "la pestaña deja de apuntar al visor"
        );
    }

    #[test]
    fn escape_y_ctrl_w_en_el_visor_piden_cerrar_su_sesion() {
        let presence = seed_watch_agent("salida");
        let plugin = HerdrPlugin::with_presence_dir(&presence);
        let entry = plugin
            .visible_agents(1)
            .into_iter()
            .next()
            .expect("un subagente visible");
        assert!(plugin.watch_entry(&entry));
        assert!(plugin.take_spawn_session_request().is_some());
        plugin.on_session_created(7);

        // Escape sin modificadores pide cerrar la sesión del visor y consume.
        assert_eq!(plugin.on_key(&Key::new("Escape")), KeyAction::Consume);
        assert_eq!(plugin.take_close_session_request(), Some(7));
        assert!(
            plugin.state.read().unwrap().spaces[0].tabs[0]
                .watching
                .is_some(),
            "la pestaña no se borra: solo se pide cerrar la sesión del visor"
        );

        // Ctrl+W hace exactamente lo mismo, nunca borra la pestaña del usuario.
        assert_eq!(plugin.on_key(&Key::new("w").ctrl()), KeyAction::Consume);
        assert_eq!(plugin.take_close_session_request(), Some(7));
        assert_eq!(plugin.state.read().unwrap().spaces[0].tabs.len(), 1);

        // close_active_tab tampoco borra una pestaña ocupada por el visor.
        assert!(plugin.close_active_tab());
        assert_eq!(plugin.take_close_session_request(), Some(7));
        assert_eq!(plugin.state.read().unwrap().spaces[0].tabs.len(), 1);

        // Escape con modificadores no es la salida del visor.
        assert_eq!(plugin.on_key(&Key::new("Escape").ctrl()), KeyAction::Pass);
        assert!(plugin.take_close_session_request().is_none());
    }

    #[test]
    fn los_atajos_normales_de_pestana_no_cambian() {
        let plugin = HerdrPlugin::default();
        // Escape en una pestaña normal sigue su curso hacia el PTY.
        assert_eq!(plugin.on_key(&Key::new("Escape")), KeyAction::Pass);
        // Ctrl+W sigue cerrando la pestaña activa cuando hay más de una.
        plugin.create_tab_in_active_space();
        plugin.on_session_created(1);
        assert_eq!(plugin.on_key(&Key::new("w").ctrl()), KeyAction::Consume);
        assert_eq!(plugin.take_close_session_request(), Some(1));
        let s = plugin.state.read().unwrap();
        assert_eq!(s.spaces[0].tabs.len(), 1, "la pestaña normal sí se cierra");
        assert_eq!(s.spaces[0].tabs[0].session_id, 0);
    }

    #[test]
    fn una_fila_sin_identidad_no_pide_visor() {
        let presence = seed_watch_agent("sin-identidad");
        let plugin = HerdrPlugin::with_presence_dir(&presence);

        // La presencia siempre trae identidad; se fuerza una fila sin ella para
        // ejercitar la guarda del clic.
        let entry = agents::AgentEntry {
            id: "t_huerfana".to_string(),
            agent: "gentle-ai-worker".to_string(),
            label: "huerfana".to_string(),
            status: agents::AgentStatus::Running,
            last_activity_at: 1,
            last_step: String::new(),
            session_hash: String::new(),
            incarnation: String::new(),
        };
        assert!(
            !plugin.watch_entry(&entry),
            "sin identidad no hay nada que ver"
        );
        assert!(
            plugin.take_spawn_session_request().is_none(),
            "sin identidad no se pide sesión"
        );
        assert_eq!(
            plugin.state.read().unwrap().spaces[0].tabs.len(),
            1,
            "tampoco se crea una pestaña vacía"
        );
    }

    #[test]
    fn el_directorio_y_la_app_del_visor_no_tocan_la_pestana_ocupada() {
        let presence = seed_watch_agent("titulo");
        let plugin = HerdrPlugin::with_presence_dir(&presence);
        let entry = plugin
            .visible_agents(1)
            .into_iter()
            .next()
            .expect("un subagente visible");
        let expected_label = entry.label.clone();
        assert!(plugin.watch_entry(&entry));
        assert!(plugin.take_spawn_session_request().is_some());
        plugin.on_session_created(7);

        // El shell del visor informa su carpeta y su app: ni el espacio del
        // usuario se renombra ni la pestaña ocupada pierde el nombre del subagente.
        plugin.update_session_cwd(7, Path::new("/home/loonbac/Proyectos/port"), "port");
        plugin.update_session_app(
            7,
            Some(&RunningApp {
                pid: 1,
                bin: "bash".to_string(),
            }),
        );

        let s = plugin.state.read().unwrap();
        let space = &s.spaces[0];
        assert_eq!(
            space.name, "space-1",
            "el cwd de la sesión visora no renombra el espacio del usuario"
        );
        let tab = &space.tabs[0];
        assert_eq!(
            tab.title, expected_label,
            "el título bloqueado sigue siendo el subagente"
        );
        assert_eq!(tab.app, None, "la app del visor no sustituye al subagente");
        drop(s);

        // Un terminal normal (sin visor) sí sigue renombrando su espacio.
        let normal = HerdrPlugin::default();
        normal.update_session_cwd(0, Path::new("/home/loonbac/Proyectos/port"), "port");
        assert_eq!(normal.state.read().unwrap().spaces[0].name, "port");
    }

    #[test]
    fn la_pestana_con_titulo_bloqueado_ignora_la_app_en_primer_plano() {
        // Regresión de "Sleep": el visor corre `bash -c tail|jq`, así que su
        // app en primer plano no debe sustituir el nombre del subagente que
        // pinta la píldora de la barra superior.
        let bloqueada = HerdrTab {
            id: 1,
            title: "Sleep".to_string(),
            session_id: 7,
            app: Some(RunningApp {
                pid: 1,
                bin: "bash".to_string(),
            }),
            title_locked: true,
            watching: None,
        };
        assert_eq!(
            tab_identity(&bloqueada),
            (None, "Sleep".to_string()),
            "el título fijado manda sobre la app en primer plano"
        );

        // Una pestaña normal conserva icono y nombre derivados de su app.
        let normal = HerdrTab {
            title_locked: false,
            ..bloqueada.clone()
        };
        assert_eq!(
            tab_identity(&normal),
            (Some(ICON_ROBOT), "Bash".to_string())
        );
    }
}
