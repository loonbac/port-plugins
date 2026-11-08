//! Plugin `smooth-cursor`: anima el cursor de la terminal.
//!
//! El cursor de Ghostty se anima con un shader sobre la terminal ya renderizada.
//! PORT no tiene esa textura, así que la mecánica es otra: PORT resuelve la
//! geometría y expone un overlay por cuadro; el plugin interpola y devuelve
//! quads. Ver `odd/tasks/smooth-cursor.md` para el contrato completo.

pub mod anim;
pub mod config;
/// Adaptador de los traits del host. Requiere la API de overlay en
/// `port-plugin-api`; ver la nota del flag en `Cargo.toml`.
#[cfg(feature = "overlay-hook")]
pub mod host;
pub mod plugin;

pub use plugin::{Quad, SmoothCursorPlugin, PLUGIN_ID, PLUGIN_NAME};
