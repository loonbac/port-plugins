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

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use port_plugin_api::{
    AppearanceHook, ConfigFile, InputHook, LayoutHook, Plugin, PluginConfig, SpaceHook,
};

use crate::watch::request_watch;

pub(crate) mod agents;
mod color;
mod hooks;
mod identity;
pub(crate) mod presence;
mod state;
pub(crate) mod viewer;
mod watch;

#[cfg(test)]
mod agents_test;

#[cfg(test)]
mod presence_test;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod viewer_test;

pub use crate::color::{parse_hex_color, system_accent_color};
pub use crate::state::{
    HerdrSpace, HerdrState, HerdrTab, PendingWatch, WatchedTab, PENDING_SESSION,
    SIDEBAR_DEFAULT_WIDTH, SIDEBAR_MAX_WIDTH, SIDEBAR_MIN_WIDTH,
};

/// Lector de subagentes de `pi`, compartido por todos los renders.
///
/// Se guarda en el plugin y no en el estado de la sidebar porque su trabajo es
/// exactamente el del watcher de configuración: mantener el dato fresco sin
/// que la UI tenga que pedirlo.
type SharedAgents = Arc<std::sync::Mutex<agents::AgentWatcher>>;

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
        let mut watcher = agents::AgentWatcher::new(presence_dir);
        let now = agents::now_ms();
        watcher.entries(now);
        let agents = Arc::new(std::sync::Mutex::new(watcher));
        let mut initial_state = HerdrState::default();
        initial_state.last_accent_fetch_ms = now;
        let state = Arc::new(RwLock::new(initial_state));
        agents::spawn_background_refresher(&agents, &state);
        Self {
            state,
            agents,
        }
    }

    /// Subagentes de `pi` visibles en la instantánea actual, vivos primero,
    /// sin realizar operaciones de E/S de disco en el hilo de UI.
    pub fn visible_agents_snapshot(&self, limit: usize) -> Vec<agents::AgentEntry> {
        let guard = self
            .agents
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.snapshot_visible(limit)
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
    fn sidebar_shown(&self, state: &HerdrState) -> bool {
        if state.spaces.len() > 1 {
            return true;
        }
        let guard = self
            .agents
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        !guard.snapshot().is_empty()
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
        s.pending_shell_tabs.push_back(next_tab);
    }

    /// Crea una nueva pestaña dentro del espacio actualmente activo.
    pub fn create_tab_in_active_space(&self) {
        let mut s = self.state.write().unwrap();
        s.create_pending_tab_in_active_space();
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
    /// La pestaña se restaura EN EL ACTO, no cuando llegue `on_session_closed`:
    /// el host cierra la sesión por petición sin notificar a los plugins, así
    /// que esperar al callback dejaría la pestaña con un id muerto.
    pub fn request_viewer_close(&self) -> bool {
        let mut s = self.state.write().unwrap();
        let target = s
            .spaces
            .get(s.active_space_index)
            .and_then(|space| space.tabs.get(space.active_tab_index))
            .map(|tab| (tab.id, tab.viewer_session_id()));
        match target {
            Some((tab_id, Some(session_id))) => {
                s.close_session_requested = Some(session_id);
                s.restore_watched_tab(tab_id);
                true
            }
            _ => false,
        }
    }

    /// Cierra la pestaña activa en el espacio actual si hay más de una.
    ///
    /// Una pestaña ocupada por el visor no se borra nunca: se le pide al núcleo
    /// cerrar la sesión del visor y `on_session_closed` la devuelve a su
    /// terminal original.
    pub fn close_active_tab(&self) -> bool {
        let mut s = self.state.write().unwrap();
        let space_idx = s.active_space_index;
        let tab_idx = s
            .spaces
            .get(space_idx)
            .map(|space| space.active_tab_index)
            .unwrap_or(0);
        s.close_tab_at(space_idx, tab_idx)
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
            s.accent_stale = true;
            if let Some(rgb) = parse_hex_color(&s.accent_mode) {
                s.published_accent = rgb;
            }
        }
        if let Some(width) = config.get_f32("sidebar_width") {
            s.sidebar_width = width.clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH);
            s.resize_start_width = s.sidebar_width;
        }
        if let Some(spaces_str) = config.get("spaces") {
            let mut parsed_spaces = Vec::new();
            let mut pending_shells = Vec::new();
            for (idx, entry) in spaces_str.split(',').enumerate() {
                if let Some((name, branch)) = entry.trim().split_once(':') {
                    if !name.trim().is_empty() {
                        // La sesión 0 solo se reutiliza si sigue viva: un
                        // recargado después de que muriera ataría una pestaña
                        // nueva a un id muerto. Los espacios siguientes siempre
                        // nacen pendientes.
                        let is_first = parsed_spaces.is_empty();
                        let use_session_zero = is_first && !s.closed_sessions.contains(&0);
                        let session_id = if use_session_zero { 0 } else { PENDING_SESSION };
                        let tab_id = idx + 1;
                        if !use_session_zero {
                            pending_shells.push(tab_id);
                        }
                        parsed_spaces.push(HerdrSpace {
                            name: name.trim().to_string(),
                            branch: branch.trim().to_string(),
                            custom_color: None,
                            tabs: vec![HerdrTab {
                                id: tab_id,
                                title: "terminal".to_string(),
                                session_id,
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
                // Resincroniza los contadores: con `next_tab_id` viejo, el `+` o
                // `Ctrl+Shift+T` crearían una pestaña con un id ya declarado y
                // `tab_by_id_mut` enlazaría al primero que coincida.
                let max_tab = parsed_spaces
                    .iter()
                    .flat_map(|space| space.tabs.iter().map(|tab| tab.id))
                    .max()
                    .unwrap_or(0);
                let max_space_num = parsed_spaces
                    .iter()
                    .filter_map(|space| space.name.strip_prefix("space-"))
                    .filter_map(|num| num.parse::<usize>().ok())
                    .max()
                    .unwrap_or(0);
                s.next_tab_id = max_tab + 1;
                s.next_space_num = parsed_spaces.len().max(max_space_num) + 1;
                s.spaces = parsed_spaces;
                // El índice activo no puede quedar fuera del vector nuevo.
                s.active_space_index = if s.spaces.is_empty() {
                    0
                } else {
                    s.active_space_index.min(s.spaces.len() - 1)
                };
                // Un recargado no puede dejar peticiones apuntando a pestañas
                // que ya no existen.
                s.pending_shell_tabs = pending_shells.into_iter().collect();
                s.in_flight = None;
                s.pending_viewer_tab = None;
                s.pending_watch = None;
                s.pending_viewer_title = None;
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
