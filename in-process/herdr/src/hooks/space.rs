//! Hook de espacios del plugin herdr.
//!
//! Única responsabilidad: mantener sincronizado el estado de espacios y
//! pestañas con las sesiones que crea, actualiza y cierra el núcleo de PORT.

use std::path::Path;

use port_plugin_api::SpaceHook;
use port_term_core::pty::{PtyConfig, RunningApp};

use crate::identity::detect_git_branch;
use crate::state::{default_space, InFlight, WatchedTab, PENDING_SESSION};
use crate::viewer;
use crate::HerdrPlugin;

impl SpaceHook for HerdrPlugin {
    fn active_session(&self) -> usize {
        let s = self.state.read().unwrap();
        if let Some(space) = s.spaces.get(s.active_space_index) {
            if let Some(tab) = space.tabs.get(space.active_tab_index) {
                return tab.session_id;
            }
        }
        // Sin pestaña activa no se cae en la sesión 0, que es real: el núcleo
        // dibujaría un terminal ajeno. El centinela hace fallar su `select` y
        // conserva lo que ya estaba en pantalla.
        PENDING_SESSION
    }

    fn active_space(&self) -> usize {
        self.state.read().unwrap().active_space_index
    }

    fn take_new_session_request(&self) -> bool {
        let mut s = self.state.write().unwrap();
        match s.in_flight {
            // El host reintenta la MISMA pestaña: no se encola una petición nueva.
            Some(InFlight::Shell(_)) => true,
            // Un visor está en vuelo; el shell espera a que resuelva.
            Some(InFlight::Viewer(_)) => false,
            None => {
                // Se descartan los ids de pestañas que ya no existen: enlazar
                // por un id muerto terminaría pisando el terminal de una viva.
                while let Some(tab_id) = s.pending_shell_tabs.pop_front() {
                    if s.tab_by_id_mut(tab_id).is_some() {
                        s.in_flight = Some(InFlight::Shell(tab_id));
                        return true;
                    }
                }
                false
            }
        }
    }

    /// Petición de una sesión que corre el visor de un subagente.
    ///
    /// Mientras el shell del espacio está en vuelo devuelve `None` y deja la
    /// petición intacta; si el host reintenta la sesión del visor, reconstruye
    /// el mismo comando a partir de la petición que sigue viva.
    fn take_spawn_session_request(&self) -> Option<PtyConfig> {
        let mut s = self.state.write().unwrap();
        match s.in_flight {
            // Un shell está en vuelo: el visor se pospone a un render posterior
            // sin tocar `pending_watch` ni `pending_viewer_tab`.
            Some(InFlight::Shell(_)) => None,
            // El host reintenta la sesión del visor: se revalida la pestaña,
            // que pudo desaparecer entre reintentos.
            Some(InFlight::Viewer(tab_id)) => {
                if s.tab_by_id_mut(tab_id).is_none() {
                    // Sin pestaña no hay a dónde enlazar: se descarta el visor
                    // entero en vez de redirigirlo a otra pestaña.
                    s.in_flight = None;
                    s.pending_watch = None;
                    s.pending_viewer_tab = None;
                    return None;
                }
                let watch = s.pending_watch.as_ref()?;
                Some(viewer::watch_command(
                    &watch.presence_dir,
                    &watch.session_hash,
                    &watch.incarnation,
                    &watch.agent_label,
                ))
            }
            None => {
                // Sin pestaña vigente no hay visor que crear: la fila apuntaba
                // a una pestaña que ya se cerró, así que la petición se
                // descarta entera. `pending_viewer_tab` se conserva hasta que
                // `on_session_created` confirme la sesión.
                let Some(tab_id) = s.pending_viewer_tab else {
                    s.pending_watch = None;
                    return None;
                };
                if s.tab_by_id_mut(tab_id).is_none() {
                    s.pending_watch = None;
                    s.pending_viewer_tab = None;
                    return None;
                }
                // La petición se conserva hasta que el núcleo cree la sesión;
                // su etiqueta sobrevive porque la pestaña aún no existe cuando
                // el núcleo la pide.
                let watch = s.pending_watch.clone()?;
                s.pending_viewer_title = Some(watch.agent_label.clone());
                s.in_flight = Some(InFlight::Viewer(tab_id));
                Some(viewer::watch_command(
                    &watch.presence_dir,
                    &watch.session_hash,
                    &watch.incarnation,
                    &watch.agent_label,
                ))
            }
        }
    }

    fn on_session_created(&self, session_id: usize) {
        let mut s = self.state.write().unwrap();
        // Solo se enlaza lo que herdr pidió. Sin una sesión en vuelo propia, la
        // sesión es de otro plugin: escribirla en la pestaña activa le pisaría
        // el terminal y quemaría el título del visor.
        let Some(target) = s.in_flight.take() else {
            return;
        };
        let tab_id = target.tab_id();
        // La pestaña pudo desaparecer mientras la sesión estaba en vuelo.
        if s.tab_by_id_mut(tab_id).is_none() {
            return;
        }

        // Si era el visor y su pestaña sigue viva, la petición queda satisfecha.
        if let InFlight::Viewer(_) = target {
            // Un binding de visor siempre necesita un camino de vuelta. Si la
            // pestaña perdió su `watching` en el camino (el cierre del visor
            // anterior se procesó antes de este spawn), se sintetiza desde su
            // estado actual; nunca se pisa uno existente.
            if let Some(tab) = s.tab_by_id_mut(tab_id) {
                if tab.watching.is_none() {
                    tab.watching = Some(WatchedTab {
                        session_id: (tab.session_id != PENDING_SESSION).then_some(tab.session_id),
                        title: tab.title.clone(),
                        title_locked: tab.title_locked,
                    });
                }
            }
            s.pending_watch = None;
            s.pending_viewer_tab = None;
        }

        // El título del visor se aplica solo a la pestaña que el clic tomó, y
        // se agota entonces para que la siguiente no lo herede.
        let is_viewer = s
            .tab_by_id_mut(tab_id)
            .is_some_and(|tab| tab.watching.is_some());
        let viewer_title = if is_viewer {
            s.pending_viewer_title.take()
        } else {
            None
        };

        let Some(tab) = s.tab_by_id_mut(tab_id) else {
            return;
        };
        tab.session_id = session_id;
        if let Some(title) = viewer_title {
            tab.title = title;
            tab.title_locked = true;
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
        // La sesión ya no existe: el visor nunca debe devolver una pestaña a ella.
        s.closed_sessions.insert(session_id);

        // La sesión del visor no se lleva su pestaña: vuelve a su terminal
        // original con la sesión, el título y el bloqueo que tenía. Si no tenía
        // ninguno, o su terminal ya murió, pide un shell fresco: nunca se le
        // deja el centinela pegado.
        let watched_ids: Vec<usize> = s
            .spaces
            .iter()
            .flat_map(|space| space.tabs.iter())
            .filter(|tab| tab.viewer_session_id() == Some(session_id))
            .map(|tab| tab.id)
            .collect();
        for tab_id in watched_ids {
            s.restore_watched_tab(tab_id);
        }

        // Quita la pestaña que pertenecía a esa sesión y reajusta su índice.
        // Cada id removido se olvida: `retain` también borra sin pasar por un
        // cierre de usuario, y su id no debe seguir en ninguna cola.
        let mut removed_tabs = Vec::new();
        for space in s.spaces.iter_mut() {
            space.tabs.retain(|t| {
                let keep = t.session_id != session_id;
                if !keep {
                    removed_tabs.push(t.id);
                }
                keep
            });
            space.clamp_active_tab();
        }
        for tab_id in removed_tabs {
            s.forget_tab(tab_id);
        }

        // Un espacio sin pestañas deja de existir. El activo se conserva por
        // aritmética de índices, no por nombre: varias carpetas pueden llamarse
        // igual, así que resolverlo por nombre elegiría otro espacio. Solo se
        // retrocede si el espacio activo desapareció.
        let previous_active = s.active_space_index;
        let active_survives = s
            .spaces
            .get(previous_active)
            .is_some_and(|space| !space.tabs.is_empty());
        let kept_before = s
            .spaces
            .iter()
            .take(previous_active.min(s.spaces.len()))
            .filter(|space| !space.tabs.is_empty())
            .count();
        s.spaces.retain(|space| !space.tabs.is_empty());

        if s.spaces.is_empty() {
            // Todas las sesiones murieron. `default_space` arranca en la sesión
            // 0, que puede ser justo la que se cerró: se reconstruye pendiente
            // para no dejar una sesión muerta pegada a la pestaña.
            let mut space = default_space();
            if let Some(tab) = space.tabs.first_mut() {
                tab.session_id = PENDING_SESSION;
                s.pending_shell_tabs.push_back(tab.id);
            }
            s.spaces.push(space);
            s.active_space_index = 0;
        } else if active_survives {
            s.active_space_index = kept_before;
        } else {
            s.active_space_index = kept_before.min(s.spaces.len() - 1);
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
