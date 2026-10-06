//! Estado interno del plugin herdr.
//!
//! Única responsabilidad: describir los espacios, sus pestañas y el visor de un
//! subagente, junto con las operaciones que mantienen ese estado coherente.

use std::path::{Path, PathBuf};

use port_term_core::frame::Rgb;
use port_term_core::pty::RunningApp;

use crate::color::{parse_hex_color, system_accent_color};

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
    pub(crate) fn viewer_session_id(&self) -> Option<usize> {
        let previo = self.watching.as_ref()?;
        (self.session_id != previo.session_id).then_some(self.session_id)
    }
}

/// Identificador centinela para una pestaña que todavía no tiene sesión asignada.
/// Nunca colisiona con un id real de `SessionManager`, que empieza en 0.
pub const PENDING_SESSION: usize = usize::MAX;
/// Ancho por defecto de la barra lateral de espacios, en píxeles lógicos.
pub const SIDEBAR_DEFAULT_WIDTH: f32 = 236.0;
/// Ancho mínimo: por debajo el nombre de las ramas deja de caber.
pub const SIDEBAR_MIN_WIDTH: f32 = 170.0;
/// Ancho máximo: evita que el sidebar se coma toda la ventana.
pub const SIDEBAR_MAX_WIDTH: f32 = 520.0;
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
    /// Alfa del fondo de la sidebar y el topbar. Por defecto `0.0` (sin fondo
    /// propio): se ve el fondo de la ventana, igual que en la terminal. La
    /// transparencia global la decide el plugin de la tienda, no herdr; esta
    /// opacidad solo añade un tinte opcional encima (diseño de herdr).
    pub opacity: f32,
    pub accent_mode: String,
    pub new_session_requested: bool,
    pub close_session_requested: Option<usize>,
    /// Visor pedido por un clic y todavía no consumido por el núcleo.
    pub(crate) pending_watch: Option<PendingWatch>,
    /// Segunda preocupación del visor: el título que debe recibir la pestaña de
    /// la próxima sesión que cree el núcleo.
    ///
    /// `take_spawn_session_request` lo deja aquí al consumir `pending_watch`
    /// (la pestaña aún no existe cuando el núcleo pide la sesión) y
    /// `on_session_created` lo aplica y lo agota.
    pub(crate) pending_viewer_title: Option<String>,
    /// Ancho actual de la barra lateral, ajustable arrastrando el asidero.
    pub sidebar_width: f32,
    /// Posición X donde empezó el arrastre de ancho, si lo hay.
    pub(crate) resize_anchor_x: Option<f32>,
    /// Ancho que tenía la barra cuando empezó el arrastre.
    pub(crate) resize_start_width: f32,
    pub(crate) next_tab_id: usize,
    pub(crate) next_space_num: usize,
}

/// Espacio inicial: una sola terminal llamada `terminal` sobre la sesión 0.
pub(crate) fn default_space() -> HerdrSpace {
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
    pub(crate) fn open_watch_tab(
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
            // Sin fondo propio: la sidebar y el topbar comparten el fondo de
            // la ventana y su transparencia, como la terminal.
            opacity: 0.0,
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
