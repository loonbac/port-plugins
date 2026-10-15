//! Cliente móvil de PORT.
//!
//! Única responsabilidad: llevar la terminal y su modelo de espacios y pestañas
//! a un teléfono, sirviendo la aplicación web y hablando el protocolo remoto.
//!
//! El núcleo de PORT no sabe nada de esto: la capacidad de leer una rejilla y
//! escribir en una sesión es genérica, y el modelo de espacios lo publica como
//! servicio el plugin que ya lo tiene, en vez de duplicarlo acá.

pub mod bridge;
pub mod http;
pub mod plugin;
pub mod protocol;
pub mod server;
pub mod socket;
pub mod startup;

pub use crate::plugin::MobilePlugin;
