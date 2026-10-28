//! Estado interno del plugin herdr.
//!
//! Única responsabilidad: describir los espacios, sus pestañas y el visor de un
//! subagente, junto con las operaciones que mantienen ese estado coherente.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use port_plugin_api::AgentState;
use port_term_core::frame::Rgb;
use port_term_core::pty::RunningApp;

use crate::anim::{drop_slot, reordered, CardInput, CardMotion, SpaceFrame, CARD_PITCH};
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
    /// Identidad estable del espacio.
    ///
    /// Es la clave del movimiento de su tarjeta y del gesto de reordenamiento.
    /// Nunca se reutiliza: un espacio cerrado no vuelve a existir. Antes la
    /// identidad era el índice en la lista, y por eso cerrar un espacio
    /// intermedio desplazaba la tarjeta equivocada y arrastrar uno cambiaba de
    /// lugar a otro.
    pub id: usize,
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

/// Arrastre de reordenamiento de espacios en curso, si lo hay.
///
/// No toca la lista: guarda qué espacio se agarró, dónde estaba el puntero y a
/// qué ranura iría si se soltara. El orden nuevo se aplica al soltar, cuando los
/// resortes ya asentaron, así que nada más ve una lista a medio reordenar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SpaceDrag {
    /// Id del espacio que se arrastra.
    pub id: usize,
    /// Ranura visual que ocupaba su tarjeta al agarrarla.
    pub grab_slot: usize,
    /// Coordenada Y del puntero al agarrar, en coordenadas de ventana.
    pub grab_y: f32,
    /// Última coordenada Y del puntero, para dibujar la tarjeta siguiéndolo.
    pub pointer_y: f32,
    /// Ranura visual a la que iría si se soltara ahora.
    pub target: usize,
    /// `true` en cuanto el puntero superó el umbral: ya no es un clic.
    pub moved: bool,
}

/// Orden pendiente de aplicar, como ids de espacio en su orden final.
pub(crate) type PendingOrder = Vec<usize>;

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

/// Vida que se le supone al estado de un informe que no declara `ttl_ms`.
///
/// Basta para que un informe sin vida explícita siga mandando un rato en vez de
/// caducar en el mismo instante en que llegó.
pub(crate) const REPORT_GRACE_MS: u64 = 10_000;

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
    /// Sesiones cuyo estado vino de un informe, con su vencimiento.
    ///
    /// Mientras siga vivo, el informe manda: es una señal exacta de la
    /// aplicación y no una deducción, así que el sondeo del núcleo no puede
    /// pisarla. Sin esto la barra volvería a decir «working» justo cuando el
    /// agente está esperando respuesta, que es el síntoma que se está corrigiendo.
    reported: BTreeMap<usize, u64>,
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

    /// Marca que el estado de una sesión vino de un informe, hasta `deadline_ms`.
    fn mark_reported(&mut self, session_id: usize, deadline_ms: u64) {
        self.reported.insert(session_id, deadline_ms);
    }

    /// `true` si el estado de la sesión viene de un informe todavía vigente.
    ///
    /// Descarta de paso los vencidos, así que el mapa no crece con sesiones que
    /// ya nadie reporta.
    pub(crate) fn panel_state_live(&mut self, session_id: usize, now_ms: u64) -> bool {
        self.reported.retain(|_, deadline| *deadline > now_ms);
        self.reported.contains_key(&session_id)
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

/// Dónde publica el plugin su superficie de intercepción.
///
/// Se resuelve una sola vez, al construir el plugin, para que anunciar el
/// entorno de un panel no dependa de mirar el proceso ni el sistema de archivos
/// en cada lanzamiento.
#[derive(Debug, Clone, Default)]
pub(crate) struct PanelEnv {
    /// Socket propio del plugin, si se declaró dónde escuchar.
    pub socket: Option<PathBuf>,
    /// Ejecutable compatible que se anuncia a los paneles.
    pub bin: PathBuf,
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
///
/// No es `Clone`: el movimiento de las tarjetas es estado retenido (trayectorias
/// con velocidad), y duplicarlo daría dos animaciones del mismo objeto.
#[derive(Debug)]
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
    /// Próximo id de espacio. No se reutiliza nunca (ver `HerdrSpace::id`).
    pub(crate) next_space_id: usize,
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
    /// Dónde se publica la superficie de intercepción.
    pub(crate) panel_env: PanelEnv,
    /// Arrastre de reordenamiento en curso. Mientras está presente, la sidebar
    /// muestra una capa que sigue al puntero hasta el `mouse_up`.
    pub(crate) space_drag: Option<SpaceDrag>,
    /// Orden nuevo acordado al soltar, a la espera de que los resortes asienten.
    pub(crate) pending_order: Option<PendingOrder>,
    /// Movimiento de las tarjetas de espacio: el desplazamiento que se ve
    /// mientras se arrastra, después de soltar y al entrar una tarjeta nueva.
    pub(crate) space_motion: CardMotion,
    /// Último fotograma calculado, que es lo que pinta el render.
    pub(crate) card_frame: SpaceFrame,
}

/// Espacio inicial: una sola terminal llamada `terminal` sobre la sesión 0.
pub(crate) fn default_space() -> HerdrSpace {
    HerdrSpace {
        id: 1,
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
    /// Acuña un id nuevo de espacio.
    ///
    /// Los ids no se reutilizan: un espacio cerrado no vuelve a existir, ni
    /// siquiera mientras su tarjeta termina de salir de la lista.
    pub(crate) fn allocate_space_id(&mut self) -> usize {
        let id = self.next_space_id;
        self.next_space_id += 1;
        id
    }

    /// Índice actual del espacio con ese id dentro de `spaces`.
    pub(crate) fn space_index_of(&self, id: usize) -> Option<usize> {
        self.spaces.iter().position(|space| space.id == id)
    }

    /// Ids de los espacios, en el orden de la lista.
    fn space_ids(&self) -> Vec<usize> {
        self.spaces.iter().map(|space| space.id).collect()
    }

    /// Orden que se está viendo: el pendiente de aplicar si lo hay.
    ///
    /// Al soltar un arrastre el orden nuevo se ve en el acto (los resortes ya lo
    /// dibujan) pero la lista se mueve recién cuando el movimiento asienta. Todo
    /// lo que razona sobre ranuras —el gesto y el motor de movimiento— trabaja
    /// con este orden, no con el de la lista.
    fn effective_order(&self) -> Vec<usize> {
        self.pending_order
            .clone()
            .unwrap_or_else(|| self.space_ids())
    }

    /// Inicia el posible arrastre de la tarjeta `id` en la coordenada Y dada.
    ///
    /// Todavía no selecciona ni mueve nada: si el puntero no supera el umbral,
    /// `end_space_drag` lo tratará como un clic normal. El espacio se resuelve
    /// por id, así que un índice viejo no puede agarrar otra tarjeta.
    pub(crate) fn begin_space_drag(&mut self, id: usize, y: f32) {
        let Some(grab_slot) = self.effective_order().iter().position(|other| *other == id) else {
            return;
        };
        self.space_drag = Some(SpaceDrag {
            id,
            grab_slot,
            grab_y: y,
            pointer_y: y,
            target: grab_slot,
            moved: false,
        });
    }

    /// Avanza el arrastre a la coordenada Y indicada y calcula la ranura destino.
    ///
    /// Devuelve `true` si hay que repintar: mientras la tarjeta se arrastra,
    /// cada evento del puntero cambia su posición aunque la ranura destino sea la
    /// misma.
    ///
    /// La ranura se calcula desde el puntero y el punto de agarre, sin acumular
    /// nada entre llamadas: repetir la misma posición da la misma ranura y un
    /// evento perdido no descoloca el gesto.
    pub(crate) fn move_space_drag(&mut self, y: f32) -> bool {
        let Some(mut drag) = self.space_drag else {
            return false;
        };
        // La ranura de reposo se recalcula por id: si la lista cambió por debajo
        // (se cerró un espacio anterior), el ancla se corre con la tarjeta para
        // que el desplazamiento siga midiendo lo mismo.
        let Some(slot) = self
            .effective_order()
            .iter()
            .position(|other| *other == drag.id)
        else {
            // El espacio desapareció: el gesto ya no tiene dueño.
            self.space_drag = None;
            return true;
        };
        if slot != drag.grab_slot {
            drag.grab_y += (slot as f32 - drag.grab_slot as f32) * CARD_PITCH;
            drag.grab_slot = slot;
        }
        drag.pointer_y = y;
        if !drag.moved && (y - drag.grab_y).abs() < SPACE_DRAG_THRESHOLD {
            self.space_drag = Some(drag);
            return false;
        }
        drag.moved = true;
        drag.target = drop_slot(drag.grab_slot, y - drag.grab_y, self.spaces.len());
        self.space_drag = Some(drag);
        true
    }

    /// Termina el arrastre: un clic selecciona, un arrastre deja el orden nuevo
    /// pendiente y selecciona la tarjeta soltada.
    ///
    /// La lista no se toca acá. El orden nuevo se ve en el acto porque los
    /// resortes lo dibujan, y se aplica cuando el movimiento asienta (ver
    /// `step_space_motion`): mover la lista a mitad del gesto es lo que hacía que
    /// el espacio activo y los resortes cambiaran de dueño mientras se
    /// arrastraba.
    pub(crate) fn end_space_drag(&mut self) {
        let Some(drag) = self.space_drag.take() else {
            return;
        };
        let Some(index) = self.space_index_of(drag.id) else {
            return;
        };
        if drag.moved {
            let effective = self.effective_order();
            if let Some(from) = effective.iter().position(|id| *id == drag.id) {
                if drag.target != from {
                    let order = reordered(&effective, from, drag.target);
                    if order != self.space_ids() {
                        self.pending_order = Some(order);
                    }
                }
            }
        }
        self.select_space_with_attention(index);
    }

    /// Aplica el orden pendiente y realinea los resortes con la lista nueva.
    ///
    /// Solo se llama cuando el movimiento ya asentó: en ese momento cada tarjeta
    /// está visualmente en su ranura nueva, así que descartar los resortes es
    /// exacto y no hay salto. El espacio activo se resuelve por id, de modo que
    /// reordenar no cambia cuál está en pantalla.
    fn apply_pending_order(&mut self) {
        let Some(order) = self.pending_order.take() else {
            return;
        };
        let active_id = self
            .spaces
            .get(self.active_space_index)
            .map(|space| space.id);
        let mut rest: Vec<Option<HerdrSpace>> = self.spaces.drain(..).map(Some).collect();
        let mut arranged = Vec::with_capacity(rest.len());
        for id in order {
            let slot = rest
                .iter()
                .position(|space| space.as_ref().is_some_and(|space| space.id == id));
            if let Some(space) = slot.and_then(|slot| rest[slot].take()) {
                arranged.push(space);
            }
        }
        // Defensivo: un espacio que el orden no nombraba (nació durante el gesto)
        // se queda al final en vez de desaparecer.
        arranged.extend(rest.into_iter().flatten());
        self.spaces = arranged;
        self.active_space_index = active_id
            .and_then(|id| self.space_index_of(id))
            .unwrap_or(0);
        self.space_motion.rebase();
    }

    /// Lo que el motor de movimiento necesita saber de la lista actual.
    ///
    /// Las ranuras se miden contra el orden efectivo (el pendiente si lo hay):
    /// así el desplazamiento de cada tarjeta incluye tanto el reordenamiento ya
    /// acordado como el arrastre en curso.
    fn card_inputs(&self) -> Vec<CardInput> {
        let effective = self.effective_order();
        let slot_of = |id: usize, order: &[usize]| order.iter().position(|other| *other == id);
        let drag = self.space_drag.filter(|drag| drag.moved);
        let settled_order = match drag {
            Some(drag) => {
                let from = slot_of(drag.id, &effective).unwrap_or(drag.target);
                reordered(&effective, from, drag.target)
            }
            None => effective.clone(),
        };
        self.spaces
            .iter()
            .enumerate()
            .map(|(index, space)| {
                let start = slot_of(space.id, &effective).unwrap_or(index);
                let end = slot_of(space.id, &settled_order).unwrap_or(index);
                let drag_offset = drag.filter(|drag| drag.id == space.id).map(|drag| {
                    // El puntero se suma a la ranura que la tarjeta ya ocupaba:
                    // si venía de un orden pendiente, arrancar de cero la
                    // teletransportaría a su índice de lista.
                    (start as f32 - index as f32) * CARD_PITCH + (drag.pointer_y - drag.grab_y)
                });
                CardInput {
                    id: space.id,
                    active: index == self.active_space_index,
                    drag_offset,
                    slot_delta: end as i32 - index as i32,
                }
            })
            .collect()
    }

    /// Avanza un cuadro de la barra lateral y responde si queda algo en
    /// movimiento (es lo que mantiene los cuadros fluyendo).
    pub(crate) fn step_space_motion(&mut self, now_ms: u64) -> bool {
        let lifting = self.space_drag.is_some_and(|drag| drag.moved);
        let inputs = self.card_inputs();
        let mut frame = self.space_motion.sample(now_ms, &inputs, lifting, false);
        // Con los resortes quietos, el orden acordado ya no tiene nada que
        // dibujar: se aplica a la lista y los resortes se realinean.
        if !frame.animating && self.space_drag.is_none() && self.pending_order.is_some() {
            self.apply_pending_order();
            let inputs = self.card_inputs();
            frame = self.space_motion.sample(now_ms, &inputs, lifting, false);
        }
        self.card_frame = frame;
        self.card_frame.animating
    }

    /// `true` mientras haya movimiento que dibujar o un orden por aplicar.
    ///
    /// El orden pendiente cuenta aunque los resortes ya estén quietos: hay que
    /// pedir un cuadro para que `step_space_motion` lo aplique a la lista.
    pub(crate) fn space_motion_active(&self) -> bool {
        self.card_frame.animating || self.pending_order.is_some()
    }

    /// Entorno que se anuncia al panel que está por lanzarse.
    ///
    /// Vacío cuando el plugin no escucha en ningún socket: la superficie se
    /// publica sólo si existe, para no anunciar a la aplicación un socket que
    /// nadie atiende.
    pub(crate) fn pending_panel_env(&self) -> Vec<(String, String)> {
        let (Some(socket), Some(pane)) =
            (self.panel_env.socket.as_ref(), self.pending_panel.as_ref())
        else {
            return Vec::new();
        };
        vec![
            ("HERDR_ENV".to_string(), "1".to_string()),
            (
                "HERDR_SOCKET_PATH".to_string(),
                socket.to_string_lossy().into_owned(),
            ),
            (
                "HERDR_BIN_PATH".to_string(),
                self.panel_env.bin.to_string_lossy().into_owned(),
            ),
            ("HERDR_PANE_ID".to_string(), pane.clone()),
        ]
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
            let deadline = request
                .ttl_ms
                .map(|ttl| now_ms.saturating_add(ttl))
                .unwrap_or_else(|| now_ms.saturating_add(REPORT_GRACE_MS));
            self.panel_reports.mark_reported(session, deadline);
        }

        Ok(PanelOutcome::Applied)
    }
}

impl Default for HerdrState {
    fn default() -> Self {
        Self {
            // Inicialmente 1 espacio limpio: la terminal abre pura a pantalla completa
            active_space_index: 0,
            spaces: vec![default_space()],
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
            next_space_id: 2,
            published_accent: system_accent_color(),
            accent_stale: false,
            last_accent_fetch_ms: crate::agents::now_ms(),
            session_cwds: HashMap::new(),
            git_branch_cache: HashMap::new(),
            agents: HashMap::new(),
            space_drag: None,
            pending_order: None,
            space_motion: CardMotion::default(),
            card_frame: SpaceFrame::default(),
            panel_reports: PanelReports::default(),
            pending_panel: None,
            panel_env: PanelEnv::default(),
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
