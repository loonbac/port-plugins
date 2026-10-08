//! Estado interno del plugin herdr.
//!
//! Única responsabilidad: describir los espacios, sus pestañas y el visor de un
//! subagente, junto con las operaciones que mantienen ese estado coherente.

use std::collections::{HashSet, VecDeque};
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
    ///
    /// `None` cuando la pestaña aún no tenía terminal (nació pendiente): no hay
    /// ninguna sesión real a la que volver, así que al salir el visor la pestaña
    /// pide un shell fresco en lugar de arrastrar el centinela.
    pub session_id: Option<usize>,
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
    /// terminal del usuario y no hay visor que cerrar ni que ignorar. Una
    /// pestaña pendiente nunca reporta un visor mientras siga en `PENDING_SESSION`.
    pub(crate) fn viewer_session_id(&self) -> Option<usize> {
        let previo = self.watching.as_ref()?;
        match previo.session_id {
            Some(previo_id) => (self.session_id != previo_id).then_some(self.session_id),
            None => (self.session_id != PENDING_SESSION).then_some(self.session_id),
        }
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

impl HerdrSpace {
    /// Corrige `active_tab_index` después de quitar pestañas del espacio.
    pub(crate) fn clamp_active_tab(&mut self) {
        if self.tabs.is_empty() {
            self.active_tab_index = 0;
        } else if self.active_tab_index >= self.tabs.len() {
            self.active_tab_index = self.tabs.len() - 1;
        }
    }
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

/// La sesión que el núcleo está creando ahora mismo, si la hay.
///
/// Distingue el tipo de petición en vuelo para que cada camino sepa si el host
/// está reintentando la suya o si debe esperar a que la otra resuelva.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InFlight {
    /// Shell por defecto pedido por una pestaña pendiente.
    Shell(usize),
    /// Sesión del visor de un subagente.
    Viewer(usize),
}

impl InFlight {
    /// Pestaña a la que pertenece esta sesión en vuelo.
    pub(crate) fn tab_id(self) -> usize {
        match self {
            InFlight::Shell(id) | InFlight::Viewer(id) => id,
        }
    }
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
    /// Ids de pestañas que pidieron un shell por defecto, en orden de petición.
    ///
    /// Es una cola y no un booleano: el núcleo crea una sesión por render, así
    /// que dos `Ctrl+Shift+T` en el mismo frame no pueden perder la segunda.
    pub(crate) pending_shell_tabs: VecDeque<usize>,
    pub close_session_requested: Option<usize>,
    /// Visor pedido por un clic y todavía no consumido por el núcleo.
    pub(crate) pending_watch: Option<PendingWatch>,
    /// Pestaña que el clic del visor retargeteó, pendiente de que el núcleo cree
    /// su sesión. Se conserva hasta que `on_session_created` la consume.
    pub(crate) pending_viewer_tab: Option<usize>,
    /// Sesión que el núcleo está creando ahora mismo, si la hay.
    ///
    /// Es una ranura única y no una cola: el núcleo crea una sesión por render y
    /// solo hay un id en vuelo. Si el spawn falla, la ranura conserva el id para
    /// que el host lo reintente en vez de emparejarlo con una sesión posterior.
    pub(crate) in_flight: Option<InFlight>,
    /// Ids de sesiones que el núcleo ya cerró. Nunca se reutilizan, así que el
    /// visor no debe devolver una pestaña a una de ellas.
    pub(crate) closed_sessions: HashSet<usize>,
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
    /// exactamente el terminal que el usuario tenía. Tampoco pide un shell: la
    /// sesión la crea el núcleo con `take_spawn_session_request` (el visor).
    ///
    /// Un segundo clic sobre una pestaña que YA muestra un visor conserva el
    /// [`WatchedTab`] existente (el terminal original) y pide cerrar la sesión
    /// del visor anterior; sin esto, el camino de vuelta quedaría apuntando al
    /// primer visor huérfano.
    pub(crate) fn open_watch_tab(
        &mut self,
        presence_dir: &Path,
        session_hash: &str,
        incarnation: &str,
        agent_label: &str,
    ) {
        let space_idx = self.active_space_index;
        // Mientras un visor está en vuelo, cualquier clic se ignora (misma
        // pestaña u otra): pisaría la petición que el núcleo está por crear.
        // Una vez el visor está arriba, `in_flight` es `None` y el camino de la
        // ronda 4 vuelve a honrar los clics.
        if matches!(self.in_flight, Some(InFlight::Viewer(_))) {
            return;
        }
        let mut viewer_tab = None;
        let mut previous_viewer = None;
        if let Some(space) = self.spaces.get_mut(space_idx) {
            let tab_idx = space.active_tab_index;
            if let Some(tab) = space.tabs.get_mut(tab_idx) {
                viewer_tab = Some(tab.id);
                if let Some(viewer_id) = tab.viewer_session_id() {
                    // Ya muestra un visor: se conserva el original guardado y se
                    // pide cerrar el visor anterior.
                    previous_viewer = Some(viewer_id);
                } else {
                    tab.watching = Some(WatchedTab {
                        session_id: (tab.session_id != PENDING_SESSION).then_some(tab.session_id),
                        title: tab.title.clone(),
                        title_locked: tab.title_locked,
                    });
                }
            }
        }
        if let Some(viewer_id) = previous_viewer {
            self.close_session_requested = Some(viewer_id);
        }
        self.pending_viewer_tab = viewer_tab;
        self.pending_watch = Some(PendingWatch {
            session_hash: session_hash.to_string(),
            incarnation: incarnation.to_string(),
            agent_label: agent_label.to_string(),
            presence_dir: presence_dir.to_path_buf(),
        });
    }

    /// Encuentra una pestaña por su `id` en cualquier espacio.
    ///
    /// El id es la única identidad estable: `active_space_index` y
    /// `active_tab_index` se mueven al cerrar espacios o pestañas, así que
    /// enlazar por índice puede apuntar a otra pestaña.
    pub(crate) fn tab_by_id_mut(&mut self, tab_id: usize) -> Option<&mut HerdrTab> {
        self.spaces
            .iter_mut()
            .flat_map(|space| space.tabs.iter_mut())
            .find(|tab| tab.id == tab_id)
    }

    /// Olvida todo rastro de una pestaña que se está cerrando.
    ///
    /// Las colas de peticiones pueden seguir nombrando un id que ya no existe
    /// cuando la pestaña se cierra antes de que el núcleo consuma su petición.
    /// Sin limpiarlo, el siguiente `on_session_created` emparejaría esa sesión
    /// con una pestaña viva y le pisaría el terminal.
    pub(crate) fn forget_tab(&mut self, tab_id: usize) {
        self.pending_shell_tabs.retain(|id| *id != tab_id);
        if self.in_flight.is_some_and(|f| f.tab_id() == tab_id) {
            self.in_flight = None;
        }
        if self.pending_viewer_tab == Some(tab_id) {
            self.pending_viewer_tab = None;
        }
    }

    /// Restaura la pestaña que ocupaba un visor a su terminal original.
    ///
    /// Devuelve `true` si la pestaña todavía apuntaba a un visor. Recupera la
    /// sesión guardada cuando sigue viva; si no, deja la pestaña pendiente y le
    /// encola un shell fresco. Siempre devuelve el título y el bloqueo
    /// originales y limpia `watching`.
    pub(crate) fn restore_watched_tab(&mut self, tab_id: usize) -> bool {
        let previo = self
            .tab_by_id_mut(tab_id)
            .and_then(|tab| tab.watching.take());
        let Some(previo) = previo else {
            return false;
        };
        let restored = previo
            .session_id
            .filter(|id| !self.closed_sessions.contains(id));
        let needs_shell = restored.is_none();
        if let Some(tab) = self.tab_by_id_mut(tab_id) {
            tab.session_id = restored.unwrap_or(PENDING_SESSION);
            tab.title = previo.title;
            tab.title_locked = previo.title_locked;
        }
        if needs_shell {
            self.pending_shell_tabs.push_back(tab_id);
        }
        true
    }

    /// Cierra la pestaña `(space_index, tab_index)` con la semántica del visor.
    ///
    /// Es el único camino de cierre (botón `+`/`Ctrl+W` y la `×` del topbar).
    /// Una pestaña que muestra el visor no se borra: se pide cerrar la sesión
    /// del visor y la pestaña se restaura EN EL ACTO, porque el host no avisa
    /// con `on_session_closed` cuando cierra por petición. Una pestaña normal se
    /// quita, se olvida su id y se pide cerrar su sesión solo si es real (nunca
    /// `PENDING_SESSION`).
    pub(crate) fn close_tab_at(&mut self, space_index: usize, tab_index: usize) -> bool {
        let viewer_target = self
            .spaces
            .get(space_index)
            .and_then(|space| space.tabs.get(tab_index))
            .map(|tab| (tab.id, tab.viewer_session_id()));
        if let Some((tab_id, Some(viewer_id))) = viewer_target {
            self.close_session_requested = Some(viewer_id);
            self.restore_watched_tab(tab_id);
            return true;
        }

        let mut removed_session = None;
        let mut removed_id = None;
        let mut closed = false;
        if let Some(space) = self.spaces.get_mut(space_index) {
            if space.tabs.len() > 1 && tab_index < space.tabs.len() {
                let removed = space.tabs.remove(tab_index);
                if removed.session_id != PENDING_SESSION {
                    removed_session = Some(removed.session_id);
                }
                removed_id = Some(removed.id);
                space.clamp_active_tab();
                closed = true;
            }
        }
        if let Some(tab_id) = removed_id {
            self.forget_tab(tab_id);
        }
        if let Some(session_id) = removed_session {
            self.close_session_requested = Some(session_id);
        }
        closed
    }

    /// Crea una pestaña pendiente en el espacio activo y registra su id para
    /// que el núcleo le asigne la próxima sesión que cree.
    ///
    /// Es la acción del `+` del topbar y de `Ctrl+Shift+T`: la pestaña nace con
    /// `PENDING_SESSION`, nunca con la sesión de otra pestaña.
    pub(crate) fn create_pending_tab_in_active_space(&mut self) -> Option<usize> {
        let tab_id = self.next_tab_id;
        self.next_tab_id += 1;
        let space = self.spaces.get_mut(self.active_space_index)?;
        let title = format!("term {}", space.tabs.len() + 1);
        space.tabs.push(HerdrTab {
            id: tab_id,
            title,
            session_id: PENDING_SESSION,
            app: None,
            title_locked: false,
            watching: None,
        });
        space.active_tab_index = space.tabs.len() - 1;
        self.pending_shell_tabs.push_back(tab_id);
        Some(tab_id)
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
            pending_shell_tabs: VecDeque::new(),
            close_session_requested: None,
            pending_watch: None,
            pending_viewer_tab: None,
            in_flight: None,
            closed_sessions: HashSet::new(),
            pending_viewer_title: None,
            sidebar_width: SIDEBAR_DEFAULT_WIDTH,
            resize_anchor_x: None,
            resize_start_width: SIDEBAR_DEFAULT_WIDTH,
            next_tab_id: 2,
            next_space_num: 2,
        }
    }
}
