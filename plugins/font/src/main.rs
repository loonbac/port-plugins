//! Arranque del plugin de fuente como proceso independiente.
//!
//! PORT lo instala con `port plugin add` y lo arranca; a partir de ahí todo el
//! trato pasa por el protocolo JSON Lines del SDK.

use port_plugin_font::FontPlugin;
use port_plugin_sdk::runtime::serve;

fn main() {
    serve(FontPlugin::default());
}
