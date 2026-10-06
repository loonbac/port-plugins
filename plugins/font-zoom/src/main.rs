//! Arranque del plugin de zoom de fuente como proceso independiente.
//!
//! PORT lo instala con `port plugin add` y lo arranca; a partir de ahí todo el
//! trato pasa por el protocolo JSON Lines del SDK.

use port_plugin_font_zoom::FontZoomPlugin;
use port_plugin_sdk::runtime::serve;

fn main() {
    serve(FontZoomPlugin::default());
}
