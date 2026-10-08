//! Hook de layout del plugin herdr.
//!
//! Única responsabilidad: decidir qué espacio reservan la barra lateral y la
//! barra superior, y dibujar su contenido (espacios, subagentes y pestañas).

use gpui::AnyElement;

use port_plugin_api::LayoutHook;

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
}
