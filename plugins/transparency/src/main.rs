//! Arranque del plugin de transparencia como proceso independiente.
//!
//! PORT lo instala con `port plugin add` y lo arranca; a partir de ahí todo el
//! trato pasa por el protocolo JSON Lines del SDK.

use port_plugin_sdk::runtime::serve;
use port_plugin_transparency::TransparencyPlugin;

fn main() {
    serve(TransparencyPlugin::default());
}
