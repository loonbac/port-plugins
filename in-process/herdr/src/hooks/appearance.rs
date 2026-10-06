//! Hook de apariencia del plugin herdr.
//!
//! Única responsabilidad: declarar el tinte de fondo y la opacidad global que
//! herdr aporta a la terminal, que en este plugin no son ninguno.

use port_plugin_api::AppearanceHook;
use port_term_core::frame::Rgb;

use crate::HerdrPlugin;

impl AppearanceHook for HerdrPlugin {
    fn background_tint(&self, base: Rgb) -> Rgb {
        base
    }

    /// herdr **no** declara opacidad global. El dueño de la transparencia de
    /// la terminal es el plugin de la tienda que la mete (composición `min`
    /// en el núcleo): si herdr también la declarara, con ambos en 0.85 el
    /// toggle de la tienda no cambiaría ni un píxel. El `opacity` de la
    /// config de herdr sigue pintando solo el fondo interno de su sidebar
    /// (diseño propio, ver `:1171` y `:1517`).
    fn opacity(&self) -> Option<f32> {
        None
    }
}
