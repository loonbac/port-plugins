//! Estado interno del plugin herdr.
//!
//! Única responsabilidad: describir los espacios, sus pestañas y el visor de un
//! subagente, junto con las operaciones que mantienen ese estado coherente.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use port_plugin_api::AgentState;
use port_term_core::frame::Rgb;
use port_term_core::pty::RunningApp;

use crate::anim::Spring;
use crate::color::system_accent_color;
use crate::report::{ReportError, ReportRequest};

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
/// Tiempo de vida de la caché del color de acento del sistema en milisegundos.
pub const ACCENT_TTL_MS: u64 = 2_000;
/// Tiempo de vida de la caché de la rama git por directorio en milisegundos.
pub const GIT_BRANCH_TTL_MS: u64 = 3_000;
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

/// Umbral en píxeles para distinguir un clic de un arrastre al reordenar espacios.
///
/// Por debajo no pasa nada hasta el `mouse_up`: así un clic normal sigue
/// seleccionando al soltar, igual que antes.
pub(crate) const SPACE_DRAG_THRESHOLD: f32 = 6.0;
/// Paso vertical estimado entre tarjetas de espacio (altura + separación), en píxeles.
///
/// Es una estimación porque el plugin no mide el layout (GPUI no lo expone a
/// los plugins): basta porque el ancla se recentra en cada intercambio y el
/// error solo cambia ligeramente la sensibilidad del arrastre.
pub(crate) const SPACE_CARD_PITCH: f32 = 45.0;

/// Arrastre de reordenamiento de espacios en curso, si lo hay.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SpaceDrag {
    /// Posición actual de la tarjeta arrastrada dentro de `spaces`.
    pub index: usize,
    /// Coordenada Y donde empezó el tramo actual (se recentra en cada intercambio).
    pub anchor_y: f32,
    /// Última coordenada Y del puntero, para dibujar la tarjeta siguiéndolo.
    pub pointer_y: f32,
    /// `true` en cuanto el puntero superó el umbral: ya no es un clic.
    pub moved: bool,
}

/// Rigidez del resorte que sigue al puntero mientras se arrastra.
///
/// Alta a propósito: la tarjeta debe sentirse pegada al cursor, no flotando
/// detrás. El resorte solo aporta un retardo mínimo que suaviza los saltos
/// del puntero entre eventos.
const DRAG_STIFFNESS: f32 = 900.0;
/// Rigidez del resorte que desliza a las vecinas y asienta la tarjeta soltada.
///
/// Más blanda, para que el reacomodo se lea como movimiento y no como un salto.
const SHIFT_STIFFNESS: f32 = 320.0;
/// Duración del realce de levantado, en milisegundos.
const LIFT_MS: f32 = 140.0;

/// Animación de las tarjetas de espacio, en píxeles de desplazamiento.
///
/// Vive aparte del arrastre porque sobrevive a él: cuando se suelta la tarjeta
/// el arrastre ya terminó pero el resorte sigue acomodándola.
#[derive(Debug, Clone, Default)]
pub(crate) struct SpaceMotion {
    /// Un resorte por tarjeta, en el mismo orden que `spaces`.
    pub springs: Vec<Spring>,
    /// Realce de levantado de la tarjeta arrastrada, de 0 a 1.
    pub lift: f32,
    /// Momento del último cuadro simulado, para calcular el paso de tiempo.
    pub last_frame_ms: Option<u64>,
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

    /// Devuelve el índice de la pestaña que requiere atención en este espacio:
    /// primero la primera con `Blocked`, y si no hay, la primera con `Idle` sin ver.
    /// Ignora pestañas pendientes y pestañas ocupadas por el visor.
    pub(crate) fn attention_tab_index(
        &self,
        agents: &HashMap<usize, (AgentState, bool)>,
    ) -> Option<usize> {
        let candidate_tabs = self.tabs.iter().enumerate().filter(|(_, tab)| {
            tab.session_id != PENDING_SESSION && tab.viewer_session_id().is_none()
        });

        // 1. Primera pestaña con Blocked
        for (idx, tab) in candidate_tabs.clone() {
            if let Some((AgentState::Blocked, _)) = agents.get(&tab.session_id) {
                return Some(idx);
            }
        }

        // 2. Si no hay, primera pestaña con Idle sin ver
        for (idx, tab) in candidate_tabs {
            if let Some((AgentState::Idle, false)) = agents.get(&tab.session_id) {
                return Some(idx);
            }
        }

        None
    }

    /// Estado agregado de agente para este espacio a partir del mapa de sesiones.
    pub(crate) fn aggregated_agent_state(
        &self,
        agents: &HashMap<usize, (AgentState, bool)>,
    ) -> Option<(AgentState, bool)> {
        let states = self
            .tabs
            .iter()
            .filter(|t| t.session_id != PENDING_SESSION && t.viewer_session_id().is_none())
            .filter_map(|t| agents.get(&t.session_id));
        aggregate_agent_state(states)
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

/// Tope de tokens vigentes por panel, copiado del contrato de herdr (C11).
pub(crate) const MAX_PANEL_TOKENS: usize = 32;

/// Token declarado por un panel, con su vencimiento.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PanelToken {
    value: String,
    /// Momento en que deja de valer; `None` si no caduca.
    deadline_ms: Option<u64>,
}

/// Libro de informes de panel.
///
/// Un panel es una sesión de PORT vista por la aplicación que corre dentro; su
/// identidad la acuña el plugin al lanzar el shell y el enlace con la sesión se
/// cierra cuando el núcleo la crea.
#[derive(Debug, Clone, Default)]
pub(crate) struct PanelReports {
    /// Último `seq` aceptado por par (panel, fuente).
    seq: BTreeMap<(String, String), u64>,
    /// Tokens vigentes por panel.
    tokens: BTreeMap<String, BTreeMap<String, PanelToken>>,
    /// Panel -> sesión, enlazado cuando el núcleo crea la sesión.
    panes: BTreeMap<String, usize>,
}

/// Qué pasó al aplicar un informe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PanelOutcome {
    /// El informe se aplicó.
    Applied,
    /// Llegó fuera de orden: se descarta en silencio, como hace herdr.
    Stale,
}

impl PanelReports {
    /// Enlaza un panel con la sesión que el núcleo acaba de crear.
    pub(crate) fn bind(&mut self, pane_id: &str, session_id: usize) {
        self.panes.insert(pane_id.to_string(), session_id);
    }

    /// Sesión de un panel, si está enlazado.
    pub(crate) fn session(&self, pane_id: &str) -> Option<usize> {
        self.panes.get(pane_id).copied()
    }

    /// Tokens vigentes de un panel.
    ///
    /// Sólo lo usan las pruebas: en producción los tokens se guardan para
    /// vigilar vencimientos y topes, y el estado del agente se lee del mapa de
    /// agentes, no de acá.
    #[cfg(test)]
    pub(crate) fn value(&self, pane_id: &str, key: &str) -> Option<&str> {
        self.tokens
            .get(pane_id)
            .and_then(|tokens| tokens.get(key))
            .map(|token| token.value.as_str())
    }

    /// Descarta los tokens vencidos del panel.
    fn purge_expired(&mut self, pane_id: &str, now_ms: u64) {
        let Some(tokens) = self.tokens.get_mut(pane_id) else {
            return;
        };
        tokens.retain(|_, token| match token.deadline_ms {
            Some(deadline) => deadline > now_ms,
            None => true,
        });
        if tokens.is_empty() {
            self.tokens.remove(pane_id);
        }
    }

    /// `true` si el `seq` es nuevo; lo registra cuando lo es.
    ///
    /// Sin `seq` el informe siempre entra, igual que en herdr: el orden sólo se
    /// vigila para quien lo declara.
    fn accept_seq(&mut self, pane_id: &str, source: &str, seq: Option<u64>) -> bool {
        let Some(seq) = seq else {
            return true;
        };
        let key = (pane_id.to_string(), source.to_string());
        if let Some(last) = self.seq.get(&key) {
            if seq <= *last {
                return false;
            }
        }
        self.seq.insert(key, seq);
        true
    }

    /// Aplica los tokens de una petición ya validada.
    fn apply_tokens(&mut self, request: &ReportRequest, now_ms: u64) -> Result<(), ReportError> {
        self.purge_expired(&request.pane_id, now_ms);
        let existing: Vec<String> = self
            .tokens
            .get(&request.pane_id)
            .map(|tokens| tokens.keys().cloned().collect())
            .unwrap_or_default();
        let adding = request
            .tokens
            .iter()
            .filter(|(key, value)| !value.is_empty() && !existing.contains(key))
            .count();
        if existing.len() + adding > MAX_PANEL_TOKENS {
            return Err(ReportError::TokenLimit);
        }

        let deadline_ms = request.ttl_ms.map(|ttl| now_ms.saturating_add(ttl));
        let tokens = self.tokens.entry(request.pane_id.clone()).or_default();
        for (key, value) in &request.tokens {
            if value.is_empty() {
                tokens.remove(key);
            } else {
                tokens.insert(
                    key.clone(),
                    PanelToken {
                        value: value.clone(),
                        deadline_ms,
                    },
                );
            }
        }
        if tokens.is_empty() {
            self.tokens.remove(&request.pane_id);
        }
        Ok(())
    }
}

/// Identidad de panel que el plugin anuncia al lanzar una sesión.
///
/// PORT no copia el formato de herdr (`w1:p2`): el id sólo tiene que ser único
/// y estable mientras viva la sesión, y el de la pestaña ya lo es. Quien lo
/// recibe lo devuelve tal cual, así que la forma no importa.
pub(crate) fn panel_id(tab_id: usize) -> String {
    format!("p{tab_id}")
}

/// Marca de visto que corresponde a un estado reportado por un panel.
///
/// `Working` no le pide nada al usuario, así que no reclama atención; `Blocked`
/// e `Idle` sí, y entran sin ver para que la barra los destaque. `Unknown` no
/// pinta marcador, así que su marca da igual.
fn report_seen(state: AgentState) -> bool {
    matches!(state, AgentState::Working | AgentState::Unknown)
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
    /// Pestañas nuevas que nacieron con carpeta heredada: `(tab_id, cwd)`.
    ///
    /// Viajan por `take_spawn_session_request` (shell con `cwd`), no por la
    /// petición booleana, porque el núcleo solo acepta el cwd dentro del `PtyConfig`.
    pub(crate) pending_cwd_shell_tabs: VecDeque<(usize, PathBuf)>,
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
    /// Color de acento publicado por el hilo de fondo para lecturas de memoria puras sin E/S.
    pub published_accent: Rgb,
    /// Indica que el modo de acento cambió y debe republicarse pronto.
    pub(crate) accent_stale: bool,
    /// Momento en milisegundos en que el hilo de fondo leyó el acento por última vez.
    pub(crate) last_accent_fetch_ms: u64,
    /// Caché del directorio de trabajo de cada sesión para evitar I/O redundante.
    pub(crate) session_cwds: HashMap<usize, PathBuf>,
    /// Caché de la rama git por directorio de trabajo: `(rama, timestamp_ms)`.
    pub(crate) git_branch_cache: HashMap<PathBuf, (String, u64)>,
    /// Estado de agente por sesión con su marca de visto: `(AgentState, seen)`.
    pub(crate) agents: HashMap<usize, (AgentState, bool)>,
    /// Libro de informes de panel: la superficie de herdr que PORT intercepta.
    pub(crate) panel_reports: PanelReports,
    /// Panel acuñado para el lanzamiento en vuelo, a la espera de su sesión.
    pub(crate) pending_panel: Option<String>,
    /// Arrastre de reordenamiento en curso. Mientras está presente, la sidebar
    /// muestra una capa que sigue al puntero hasta el `mouse_up`.
    pub(crate) space_drag: Option<SpaceDrag>,
    /// Resortes y realce de las tarjetas de espacio: el movimiento que se ve
    /// mientras se arrastra y después de soltar.
    pub(crate) space_motion: SpaceMotion,
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
    /// Resuelve el color de acento actual como lectura pura en memoria.
    ///
    /// Invariante crítico: **cero llamadas a sistema y cero E/S de disco**.
    /// Nunca invoca `read_to_string`, `metadata` ni comandos externos. Devuelve
    /// directamente el valor publicado por el hilo en segundo plano, por lo que es
    /// completamente seguro llamarlo mientras se retiene un `RwLockReadGuard`.
    pub fn effective_accent(&self) -> Rgb {
        self.published_accent
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
        self.pending_cwd_shell_tabs.retain(|(id, _)| *id != tab_id);
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
        // La carpeta se hereda de la pestaña activa ANTES de añadir la nueva
        // (que pasa a ser la activa). Si la activa muestra el visor, se usa la
        // sesión original guardada, no la del visor. Sin cwd conocido, shell
        // por defecto como antes.
        let inherited_cwd = self
            .spaces
            .get(self.active_space_index)
            .and_then(|space| space.tabs.get(space.active_tab_index))
            .and_then(|tab| {
                let real = tab
                    .watching
                    .as_ref()
                    .and_then(|watched| watched.session_id)
                    .or((tab.session_id != PENDING_SESSION).then_some(tab.session_id))?;
                self.session_cwds.get(&real).cloned()
            });
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
        match inherited_cwd {
            Some(cwd) => self.pending_cwd_shell_tabs.push_back((tab_id, cwd)),
            None => self.pending_shell_tabs.push_back(tab_id),
        }
        Some(tab_id)
    }

    /// Selecciona el espacio indicado. Si el espacio tiene atención (alguna pestaña
    /// con `Blocked` o con `Idle` sin ver), selecciona además la primera pestaña que
    /// la requiere (primero `Blocked`, luego `Idle` sin ver). Si no hay atención,
    /// conserva la pestaña activa actual del espacio.
    pub(crate) fn select_space_with_attention(&mut self, space_index: usize) {
        let attention_tab_idx = self
            .spaces
            .get(space_index)
            .and_then(|space| space.attention_tab_index(&self.agents));
        self.active_space_index = space_index;
        if let Some(tab_idx) = attention_tab_idx {
            if let Some(space) = self.spaces.get_mut(space_index) {
                space.active_tab_index = tab_idx;
            }
        }
    }
}

impl HerdrState {
    /// Inicia el posible arrastre de la tarjeta `index` en la coordenada Y dada.
    ///
    /// Todavía no selecciona ni mueve nada: si el puntero no supera el umbral,
    /// `end_space_drag` lo tratará como un clic normal.
    pub(crate) fn begin_space_drag(&mut self, index: usize, y: f32) {
        if index < self.spaces.len() {
            self.sync_space_springs();
            self.space_drag = Some(SpaceDrag {
                index,
                anchor_y: y,
                pointer_y: y,
                moved: false,
            });
        }
    }

    /// Deja un resorte por tarjeta, en el mismo orden que `spaces`.
    ///
    /// Se llama antes de animar: crear o cerrar espacios cambia la lista y los
    /// resortes tienen que seguirla para no desplazar a la tarjeta equivocada.
    fn sync_space_springs(&mut self) {
        self.space_motion
            .springs
            .resize(self.spaces.len(), Spring::new());
    }

    /// Avanza el arrastre a la coordenada Y indicada, intercambiando la tarjeta
    /// con sus vecinas al cruzar medio paso y siguiendo al espacio activo.
    ///
    /// Devuelve `true` si algo cambió (inicio del arrastre o algún intercambio)
    /// y hay que repintar.
    pub(crate) fn move_space_drag(&mut self, y: f32) -> bool {
        let Some(mut drag) = self.space_drag else {
            return false;
        };
        drag.pointer_y = y;
        if !drag.moved && (y - drag.anchor_y).abs() < SPACE_DRAG_THRESHOLD {
            self.space_drag = Some(drag);
            return false;
        }
        drag.moved = true;
        self.sync_space_springs();
        while y - drag.anchor_y > SPACE_CARD_PITCH / 2.0 && drag.index + 1 < self.spaces.len() {
            self.swap_space_slots(drag.index, drag.index + 1);
            drag.index += 1;
            drag.anchor_y += SPACE_CARD_PITCH;
        }
        while y - drag.anchor_y < -SPACE_CARD_PITCH / 2.0 && drag.index > 0 {
            self.swap_space_slots(drag.index, drag.index - 1);
            drag.index -= 1;
            drag.anchor_y -= SPACE_CARD_PITCH;
        }
        self.space_drag = Some(drag);
        true
    }

    /// Intercambia dos tarjetas y prepara el movimiento de la que no se arrastra.
    ///
    /// La tarjeta que llega a la ranura nueva arranca visualmente donde estaba
    /// (un paso de distancia) y el resorte la lleva a su lugar, así el reacomodo
    /// se lee como deslizamiento y no como salto. La arrastrada se excluye: esa
    /// sigue al puntero, no a su ranura.
    fn swap_space_slots(&mut self, a: usize, b: usize) {
        let dragged = self.space_drag.map(|drag| drag.index);
        self.spaces.swap(a, b);
        self.space_motion.springs.swap(a, b);

        // La tarjeta que sube (va a un índice menor) entra desde abajo y la que
        // baja entra desde arriba; el signo del arranque conserva esa lectura.
        let pitch = SPACE_CARD_PITCH;
        let shift = |from: usize, to: usize| if to > from { -pitch } else { pitch };
        // El arrastre trabaja con los índices de antes del intercambio; hay que
        // saber dónde queda su resorte para no desplazarlo a él.
        let dragged_after = match dragged {
            Some(d) if d == a => Some(b),
            Some(d) if d == b => Some(a),
            other => other,
        };
        if dragged_after != Some(a) {
            self.space_motion.springs[a].place(shift(b, a));
        }
        if dragged_after != Some(b) {
            self.space_motion.springs[b].place(shift(a, b));
        }

        if self.active_space_index == a {
            self.active_space_index = b;
        } else if self.active_space_index == b {
            self.active_space_index = a;
        }
    }

    /// Termina el arrastre: selecciona la tarjeta arrastrada (haya sido clic o
    /// soltado tras mover) y limpia el estado. Conserva el salto a la pestaña
    /// con atención del clic normal.
    pub(crate) fn end_space_drag(&mut self) {
        if let Some(drag) = self.space_drag.take() {
            self.select_space_with_attention(drag.index);
        }
    }

    /// Avanza la animación de las tarjetas hasta `now_ms` y responde si algo se
    /// sigue moviendo (para pedir otro cuadro).
    pub(crate) fn step_space_motion(&mut self, now_ms: u64) -> bool {
        self.sync_space_springs();
        let dt_s = match self.space_motion.last_frame_ms {
            Some(prev) => now_ms.saturating_sub(prev) as f32 / 1000.0,
            None => 1.0 / 60.0,
        };
        self.space_motion.last_frame_ms = Some(now_ms);

        // El realce de levantado sube de golpe al empezar el arrastre y baja al
        // soltar; no persigue una posición, así que basta una curva.
        let lift_target = if self.space_drag.is_some_and(|drag| drag.moved) {
            1.0
        } else {
            0.0
        };
        let lift_step = (dt_s * 1000.0 / LIFT_MS).min(1.0);
        let lift = self.space_motion.lift;
        self.space_motion.lift = if lift_target > lift {
            (lift + lift_step).min(1.0)
        } else {
            (lift - lift_step).max(0.0)
        };

        // Objetivo de cada tarjeta: la arrastrada persigue al puntero, el resto
        // vuelve a su ranura.
        let dragged = self.space_drag.filter(|drag| drag.moved);
        let mut moving = (self.space_motion.lift - lift_target).abs() > 0.001;
        for (i, spring) in self.space_motion.springs.iter_mut().enumerate() {
            let (target, stiffness) = match dragged {
                Some(drag) if drag.index == i => (drag.pointer_y - drag.anchor_y, DRAG_STIFFNESS),
                _ => (0.0, SHIFT_STIFFNESS),
            };
            moving |= spring.step(target, dt_s, stiffness);
        }
        moving
    }

    /// `true` mientras la animación de tarjetas no está en reposo.
    pub(crate) fn space_motion_active(&self) -> bool {
        self.space_motion.lift > 0.001
            || self
                .space_motion
                .springs
                .iter()
                .any(|spring| spring.value() != 0.0)
    }

    /// Aplica un informe de panel ya validado.
    ///
    /// Un panel desconocido es un error del contrato, no un detalle: quien
    /// escribe se identifica con un panel que el plugin anunció al lanzarlo.
    pub(crate) fn apply_panel_report(
        &mut self,
        request: &ReportRequest,
        now_ms: u64,
    ) -> Result<PanelOutcome, ReportError> {
        let session = self
            .panel_reports
            .session(&request.pane_id)
            .ok_or_else(|| ReportError::PaneNotFound(request.pane_id.clone()))?;

        // Fuera de orden: se descarta sin tocar los tokens ni el estado.
        if !self
            .panel_reports
            .accept_seq(&request.pane_id, &request.source, request.seq)
        {
            return Ok(PanelOutcome::Stale);
        }

        self.panel_reports.apply_tokens(request, now_ms)?;

        if let Some(state) = request.agent_state() {
            self.agents.insert(session, (state, report_seen(state)));
        }

        Ok(PanelOutcome::Applied)
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
            pending_cwd_shell_tabs: VecDeque::new(),
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
            published_accent: system_accent_color(),
            accent_stale: false,
            last_accent_fetch_ms: crate::agents::now_ms(),
            session_cwds: HashMap::new(),
            git_branch_cache: HashMap::new(),
            agents: HashMap::new(),
            space_drag: None,
            space_motion: SpaceMotion::default(),
            panel_reports: PanelReports::default(),
            pending_panel: None,
        }
    }
}

fn agent_state_priority(state: AgentState, seen: bool) -> (u8, bool) {
    let p = match (state, seen) {
        (AgentState::Blocked, _) => 4,
        (AgentState::Idle, false) => 3,
        (AgentState::Working, _) => 2,
        (AgentState::Idle, true) => 1,
        (AgentState::Unknown, _) => 0,
    };
    (p, !seen)
}

/// Estado agregado de un espacio según la prioridad de herdr:
/// Blocked (4) > Idle sin ver (3) > Working (2) > Idle visto (1) > Unknown (0).
pub(crate) fn aggregate_agent_state<'a>(
    states: impl Iterator<Item = &'a (AgentState, bool)>,
) -> Option<(AgentState, bool)> {
    states
        .max_by_key(|(state, seen)| agent_state_priority(*state, *seen))
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::STATE_TOKEN;

    /// Petición ya validada, como la entregaría el protocolo.
    fn request(
        pane: &str,
        tokens: &[(&str, &str)],
        seq: Option<u64>,
        ttl_ms: Option<u64>,
    ) -> ReportRequest {
        ReportRequest {
            id: "cli:request".to_string(),
            pane_id: pane.to_string(),
            source: "gentle:activity".to_string(),
            tokens: tokens
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect(),
            seq,
            ttl_ms,
        }
    }

    /// Un estado reportado llega al mapa de agentes con la marca correcta.
    ///
    /// `Working` no le pide nada al usuario; `Blocked` e `Idle` sí, y sin ver
    /// para que la barra los destaque.
    #[test]
    fn el_estado_reportado_llega_con_su_marca_de_visto() {
        let mut state = HerdrState::default();
        state.panel_reports.bind("w1:p1", 7);

        for (reported, expected) in [
            ("working", (AgentState::Working, true)),
            ("blocked", (AgentState::Blocked, false)),
            ("idle", (AgentState::Idle, false)),
        ] {
            let request = request("w1:p1", &[(STATE_TOKEN, reported)], None, None);
            assert_eq!(
                state.apply_panel_report(&request, 1_000),
                Ok(PanelOutcome::Applied)
            );
            assert_eq!(state.agents.get(&7), Some(&expected), "estado {reported}");
        }
    }

    /// Un token con vida deja de valer cuando vence.
    #[test]
    fn los_tokens_vencen() {
        let mut state = HerdrState::default();
        state.panel_reports.bind("w1:p1", 1);

        let con_vida = request("w1:p1", &[("summary", "leyendo")], None, Some(100));
        assert_eq!(
            state.apply_panel_report(&con_vida, 1_000),
            Ok(PanelOutcome::Applied)
        );
        assert_eq!(
            state.panel_reports.value("w1:p1", "summary"),
            Some("leyendo")
        );

        // Antes del vencimiento sigue vigente.
        let otro = request("w1:p1", &[("otro", "x")], None, None);
        assert!(state.apply_panel_report(&otro, 1_099).is_ok());
        assert_eq!(
            state.panel_reports.value("w1:p1", "summary"),
            Some("leyendo")
        );

        // Pasado el vencimiento se descarta.
        assert!(state.apply_panel_report(&otro, 1_101).is_ok());
        assert_eq!(state.panel_reports.value("w1:p1", "summary"), None);
        assert_eq!(state.panel_reports.value("w1:p1", "otro"), Some("x"));
    }

    /// No se acumulan más tokens de los que el contrato permite por panel.
    #[test]
    fn el_tope_de_tokens_por_panel_se_respeta() {
        let mut state = HerdrState::default();
        state.panel_reports.bind("w1:p1", 1);

        // Dos peticiones llenan el panel hasta el tope.
        for start in [0, 16] {
            let claves: Vec<String> = (start..start + 16).map(|n| format!("k{n}")).collect();
            let tokens: Vec<(&str, &str)> = claves.iter().map(|key| (key.as_str(), "v")).collect();
            let request = request("w1:p1", &tokens, None, None);
            assert!(state.apply_panel_report(&request, 1_000).is_ok());
        }

        // El token número 33 no entra.
        let extra = request("w1:p1", &[("k99", "v")], None, None);
        assert_eq!(
            state.apply_panel_report(&extra, 1_000),
            Err(ReportError::TokenLimit)
        );

        // Reescribir uno que ya existe no cuenta como nuevo.
        let repetido = request("w1:p1", &[("k0", "otro")], None, None);
        assert!(state.apply_panel_report(&repetido, 1_000).is_ok());
        assert_eq!(state.panel_reports.value("w1:p1", "k0"), Some("otro"));
    }
}
