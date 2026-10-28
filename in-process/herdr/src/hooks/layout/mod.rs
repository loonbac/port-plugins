//! Hook de layout del plugin herdr.
//!
//! Única responsabilidad: decidir qué espacio reservan la barra lateral y la
//! barra superior, y dibujar su contenido (espacios, subagentes y pestañas).

use gpui::AnyElement;

use port_plugin_api::{AgentState, LayoutHook};

use crate::HerdrPlugin;

mod sidebar;
mod topbar;

impl LayoutHook for HerdrPlugin {
    fn left_sidebar_width(&self) -> f32 {
        let s = self.state.read().unwrap();
        if self.sidebar_shown(&s) {
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
        sidebar::left_sidebar(self)
    }

    fn top_bar(&self) -> Option<AnyElement> {
        topbar::top_bar(self)
    }

    /// Pide cuadros continuos solo mientras haya algo que mover a la vista.
    ///
    /// Cuenta el movimiento de las tarjetas (el arrastre, el reacomodo al
    /// soltar y la entrada de una tarjeta nueva) y los marcadores animados
    /// `Working` (spinner) o `Blocked` (parpadeo). Sin nada de eso, la ventana
    /// vuelve a repintar solo a demanda: pedir cuadros por tener un gesto abierto
    /// dejaba la ventana a 60 fps mientras el usuario no movía nada.
    fn needs_animation(&self) -> bool {
        let s = self.state.read().unwrap();
        if s.space_motion_active() {
            return true;
        }
        if !self.sidebar_shown(&s) {
            return false;
        }
        s.spaces.iter().any(|space| {
            matches!(
                space.aggregated_agent_state(&s.agents),
                Some((AgentState::Working, _)) | Some((AgentState::Blocked, _))
            )
        })
    }
}
