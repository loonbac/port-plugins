//! Hook de espacios del plugin herdr.
//!
//! Única responsabilidad: mantener sincronizado el estado de espacios y
//! pestañas con las sesiones que crea, actualiza y cierra el núcleo de PORT.

use std::path::Path;

use port_plugin_api::SpaceHook;
use port_term_core::pty::{PtyConfig, RunningApp};

use crate::identity::detect_git_branch;
use crate::state::{default_space, PENDING_SESSION};
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
