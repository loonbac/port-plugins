//! Adaptador del plugin al host de PORT.
//!
//! Vive aparte de [`crate::plugin`] a propósito: aquí es donde el plugin toca
//! tipos de `port-plugin-api`, y son los únicos tipos que pueden cambiar cuando
//! el contrato de overlay evolucione. La lógica de la animación no depende de
//! nada de este archivo.
//!
//! El contrato llegó en `port-plugin-api` (commit 0570cb7 de PORT): los tipos
//! de aquí son los reales, no una copia del acuerdo escrito.

use port_plugin_api::hooks::{FrameView, Overlay, OverlayHook, OverlayQuad, Plugin};

use crate::anim::{CellRect, Shape};
use crate::plugin::{SmoothCursorPlugin, PLUGIN_ID, PLUGIN_NAME};

/// Traduce la forma del host al enum propio.
///
/// PORT solo pinta bloque hoy, así que su enum puede crecer; lo desconocido cae
/// a `Block`, que es la huella completa y siempre es correcta.
fn to_shape(shape: port_term_core::frame::CursorShape) -> Shape {
    use port_term_core::frame::CursorShape;
    match shape {
        CursorShape::Beam => Shape::Beam,
        CursorShape::Underline => Shape::Underline,
        _ => Shape::Block,
    }
}

/// `CellRect` del host a partir del propio.
///
/// Son dos tipos distintos a propósito: `anim` no depende de `port-plugin-api`,
/// así que su lógica se prueba sin construir tipos del host. La conversión vive
/// aquí, en la única frontera donde ambos se tocan.
fn to_host_rect(rect: CellRect) -> port_plugin_api::hooks::CellRect {
    port_plugin_api::hooks::CellRect {
        column: rect.column,
        row: rect.row,
        width: rect.width,
        height: rect.height,
    }
}

impl Plugin for SmoothCursorPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn name(&self) -> &'static str {
        PLUGIN_NAME
    }

    fn default_config(&self) -> Option<port_plugin_api::config::PluginConfig> {
        Some(self.default_plugin_config())
    }

    fn load_config(&self, config: &port_plugin_api::config::PluginConfig) {
        self.apply_config(config);
    }

    fn save_config(&self) -> Option<port_plugin_api::config::PluginConfig> {
        Some(self.current_plugin_config())
    }

    fn overlay_hook(&self) -> Option<&dyn OverlayHook> {
        Some(self)
    }
}

impl OverlayHook for SmoothCursorPlugin {
    fn overlay(&self, view: &FrameView) -> Option<Overlay> {
        // Invocación incondicional: `cursor: None` resetea en vez de dejar
        // estado viejo, así que siempre se registra el cuadro.
        let rect = view.cursor.map(|cursor| CellRect {
            column: cursor.rect.column,
            row: cursor.rect.row,
            width: cursor.rect.width,
            height: cursor.rect.height,
        });
        self.begin_frame(rect, view.now_ms);

        // Nada de `?` antes de `frame()`: este método consume el resultado
        // aunque no haya cursor que pintar, y es lo que actualiza la respuesta
        // de `needs_animation()`. Un retorno temprano aquí deja el flag pegado
        // en `true` cuando el cursor desaparece a mitad de una transición, y
        // entonces el host repinta para siempre.
        let shape = view.cursor.map(|c| to_shape(c.shape)).unwrap_or_default();
        let quads = self.frame(rect, shape, view.now_ms)?;

        // El color sale del host: es el relleno que produciría la inversión
        // nativa de esa celda, y respeta el tema sin que el plugin lo lea.
        let color = view.cursor?.color;

        Some(Overlay {
            quads: quads
                .into_iter()
                .map(|quad| OverlayQuad {
                    rect: to_host_rect(quad.rect),
                    color,
                    opacity: quad.opacity,
                    corner_radius_px: quad.corner_radius_px,
                })
                .collect(),
            // Solo se suprime el nativo mientras hay animación: en passthrough
            // el cursor tiene que volver a su sitio.
            suppress_native_cursor: true,
        })
    }

    fn needs_animation(&self) -> bool {
        // El trait no recibe el reloj: la respuesta es la que dejó el último
        // `overlay()`. Por eso `overlay()` tiene que invocarse en cada cuadro,
        // incluso cuando devuelve `None`, o el canal no se apagaría.
        self.wants_frames_cached()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{KEY_ENABLED, KEY_TRAIL_STRENGTH};
    use port_plugin_api::config::PluginConfig;
    use port_plugin_api::hooks::{CellRect as HostRect, CursorState};
    use port_term_core::frame::{CursorShape, Rgb};

    fn host_rect(column: f32, row: f32) -> HostRect {
        HostRect {
            column,
            row,
            width: 1.0,
            height: 1.0,
        }
    }

    fn view(rect: Option<HostRect>, now_ms: u64) -> FrameView {
        FrameView {
            cursor: rect.map(|rect| CursorState {
                rect,
                shape: CursorShape::Block,
                color: Rgb::new(230, 230, 230),
            }),
            now_ms,
            focused: true,
        }
    }

    fn animated_plugin() -> SmoothCursorPlugin {
        let plugin = SmoothCursorPlugin::new();
        plugin.overlay(&view(Some(host_rect(0.0, 0.0)), 0));
        plugin.overlay(&view(Some(host_rect(0.0, 1.0)), 0));
        plugin
    }

    /// El caso central del contrato: mientras anima hay overlay y suprime el
    /// nativo; con el cursor quieto devuelve None y el nativo vuelve.
    #[test]
    fn overlay_suppresses_only_while_animating() {
        let plugin = animated_plugin();

        let overlay = plugin.overlay(&view(Some(host_rect(0.0, 1.0)), 10));
        let overlay = overlay.expect("animando");
        assert!(overlay.suppress_native_cursor);
        assert_eq!(overlay.quads.len(), 2, "estela más cursor");
        assert_eq!(overlay.quads[1].opacity, 1.0);
        assert!(overlay.quads[0].opacity < 1.0, "la estela es más tenue");

        // Terminada la transición y su gracia: passthrough puro.
        assert_eq!(
            plugin.overlay(&view(Some(host_rect(0.0, 1.0)), 10_000)),
            None
        );
    }

    /// `cursor: None` resetea y devuelve None, y además apaga el canal de
    /// cuadros en ese mismo cuadro.
    #[test]
    fn hidden_cursor_resets_and_stops_frames() {
        let plugin = animated_plugin();
        assert!(plugin.needs_animation(), "animando");

        assert_eq!(plugin.overlay(&view(None, 10)), None);
        assert!(
            !plugin.needs_animation(),
            "el cuadro sin cursor también consume el resultado"
        );
    }

    /// El color lo pone el host: el quad lo copia sin inventarlo.
    #[test]
    fn quads_use_the_host_color() {
        let plugin = animated_plugin();
        let overlay = plugin
            .overlay(&view(Some(host_rect(0.0, 1.0)), 10))
            .unwrap();

        for quad in &overlay.quads {
            assert_eq!(quad.color, Rgb::new(230, 230, 230));
        }
    }

    /// El radio llega en píxeles al host sin conversión.
    #[test]
    fn corner_radius_reaches_the_host_in_pixels() {
        let plugin = SmoothCursorPlugin::new();
        let mut raw = PluginConfig::new();
        raw.set(crate::config::KEY_CORNER_RADIUS, "2.0");
        plugin.load_config(&raw);

        plugin.overlay(&view(Some(host_rect(0.0, 0.0)), 0));
        plugin.overlay(&view(Some(host_rect(0.0, 1.0)), 0));

        let overlay = plugin
            .overlay(&view(Some(host_rect(0.0, 1.0)), 10))
            .unwrap();
        assert_eq!(overlay.quads[1].corner_radius_px, 2.0);
    }

    /// Deshabilitado por configuración: ni quads ni cuadros.
    #[test]
    fn disabled_plugin_is_passthrough_at_the_host_boundary() {
        let plugin = SmoothCursorPlugin::new();
        let mut raw = PluginConfig::new();
        raw.set(KEY_ENABLED, "false");
        raw.set(KEY_TRAIL_STRENGTH, "0.3");
        plugin.load_config(&raw);

        plugin.overlay(&view(Some(host_rect(0.0, 0.0)), 0));
        plugin.overlay(&view(Some(host_rect(0.0, 1.0)), 0));

        assert_eq!(plugin.overlay(&view(Some(host_rect(0.0, 1.0)), 10)), None);
        assert!(!plugin.needs_animation());
    }

    /// El plugin se anuncia con su id y expone su configuración por defecto.
    #[test]
    fn plugin_identity_and_config_are_wired() {
        let plugin = SmoothCursorPlugin::new();
        assert_eq!(plugin.id(), "smooth-cursor");
        assert_eq!(plugin.name(), "Smooth Cursor");
        assert!(plugin.overlay_hook().is_some());

        let config = plugin.default_config().expect("config por defecto");
        assert_eq!(config.get(crate::config::KEY_DURATION_MS), Some("120"));
    }
}
