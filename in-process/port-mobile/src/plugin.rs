//! El plugin: lo que el anfitrión ve del cliente móvil.
//!
//! Única responsabilidad: traducir entre el anfitrión y el puente. Cuando el
//! anfitrión pregunta qué sesiones hay que mirar, la respuesta sale de lo que los
//! clientes están mirando; cuando entrega un cuadro, se convierte a las filas del
//! cable y se publica; cuando pide la entrada que hay que escribir, se la lleva
//! del puente ya drenada.
//!
//! Acá no hay red: el servidor y el socket viven aparte y sólo comparten este
//! puente. Por eso el plugin se prueba sin abrir un puerto.

use std::sync::Arc;

use port_plugin_api::{Plugin, SpaceHook};
use port_term_core::frame::{Frame, Rgb};
use port_term_core::session::GridSize;

use crate::bridge::Bridge;
use crate::protocol::{RowUpdate, WireRun};

/// Plugin que lleva la terminal al teléfono.
pub struct MobilePlugin {
    bridge: Arc<Bridge>,
}

impl MobilePlugin {
    /// Crea el plugin sobre un puente ya construido.
    pub fn new(bridge: Arc<Bridge>) -> Self {
        Self { bridge }
    }

    /// El puente compartido, para que quien arme el servidor lo use.
    pub fn bridge(&self) -> Arc<Bridge> {
        Arc::clone(&self.bridge)
    }
}

impl Plugin for MobilePlugin {
    fn id(&self) -> &'static str {
        "mobile"
    }

    fn name(&self) -> &'static str {
        "Cliente móvil"
    }

    fn space_hook(&self) -> Option<&dyn SpaceHook> {
        Some(self)
    }
}

impl SpaceHook for MobilePlugin {
    /// Se miran las sesiones que algún cliente está viendo, y ninguna más.
    fn watched_sessions(&self) -> Vec<usize> {
        self.bridge.watched()
    }

    fn update_session_frame(&self, session_id: usize, frame: &Frame) {
        self.bridge
            .publish_frame(session_id, frame.columns, wire_rows(frame));
    }

    fn take_session_input(&self) -> Vec<(usize, Vec<u8>)> {
        self.bridge.take_input()
    }

    fn take_session_resize(&self) -> Vec<(usize, GridSize)> {
        self.bridge.take_resize()
    }
}

/// Filas del cable a partir de un cuadro del núcleo.
///
/// El texto y el color son lo que el teléfono necesita para dibujar: la negrita
/// y el subrayado no viajan porque el cliente los deduce del color con el que
/// cada terminal los representa, y el cursor viaja en su propio mensaje.
pub fn wire_rows(frame: &Frame) -> Vec<RowUpdate> {
    frame
        .rows
        .iter()
        .enumerate()
        .map(|(index, row)| RowUpdate {
            index,
            runs: row
                .runs
                .iter()
                .map(|run| WireRun {
                    text: run.text.clone(),
                    fg: hex(run.style.fg),
                    bg: hex(run.style.bg),
                })
                .collect(),
        })
        .collect()
}

/// Color en la forma que viaja por el cable.
fn hex(color: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", color.r, color.g, color.b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{ClientMessage, ServerMessage};
    use port_term_core::frame::{Row, Run, Style};
    use std::sync::mpsc::{channel, Receiver};

    /// Un cuadro de una fila con un tramo, para probar la conversión sin PTY.
    fn cuadro(texto: &str) -> Frame {
        Frame {
            rows: vec![Row {
                runs: vec![Run {
                    text: texto.to_string(),
                    style: Style::plain(),
                    columns: texto.chars().count(),
                }],
                selection: None,
            }],
            columns: 80,
            cursor: None,
            cursor_run: None,
            display_offset: 0,
        }
    }

    /// El texto y el color del núcleo llegan al cable tal como son.
    #[test]
    fn la_conversion_lleva_texto_y_color() {
        let mut frame = cuadro("hola");
        frame.rows[0].runs[0].style.fg = Rgb::new(0xe5, 0x53, 0x4b);
        frame.rows[0].runs[0].style.bg = Rgb::new(0x0a, 0x0b, 0x0c);

        let filas = wire_rows(&frame);

        assert_eq!(filas.len(), 1);
        assert_eq!(filas[0].index, 0);
        assert_eq!(filas[0].runs[0].text, "hola");
        assert_eq!(filas[0].runs[0].fg, "#e5534b");
        assert_eq!(filas[0].runs[0].bg, "#0a0b0c");
    }

    /// Cada fila del cuadro viaja con su índice, en orden.
    #[test]
    fn cada_fila_viaja_con_su_indice() {
        let mut frame = cuadro("uno");
        frame.rows.push(Row {
            runs: vec![Run {
                text: "dos".to_string(),
                style: Style::plain(),
                columns: 3,
            }],
            selection: None,
        });

        let filas = wire_rows(&frame);
        assert_eq!(filas.len(), 2);
        assert_eq!(filas[0].index, 0);
        assert_eq!(filas[1].index, 1);
        assert_eq!(filas[1].runs[0].text, "dos");
    }

    /// El plugin mira lo que miran los clientes.
    #[test]
    fn el_plugin_mira_lo_que_miran_los_clientes() {
        let puente = Arc::new(Bridge::new());
        assert!(MobilePlugin::new(puente.clone())
            .watched_sessions()
            .is_empty());

        let (tx, _rx): (crate::bridge::ClientSink, Receiver<ServerMessage>) = channel();
        puente.join(
            tx,
            crate::protocol::Layout {
                spaces: Vec::new(),
                active_space: 0,
            },
            7,
        );

        assert_eq!(MobilePlugin::new(puente).watched_sessions(), vec![7]);
    }

    /// El cuadro que entrega el anfitrión llega al cliente que mira esa sesión.
    #[test]
    fn el_cuadro_del_anfitrion_llega_al_cliente() {
        let puente = Arc::new(Bridge::new());
        let plugin = MobilePlugin::new(puente.clone());
        let (tx, rx): (crate::bridge::ClientSink, Receiver<ServerMessage>) = channel();
        puente.join(
            tx,
            crate::protocol::Layout {
                spaces: Vec::new(),
                active_space: 0,
            },
            7,
        );

        plugin.update_session_frame(7, &cuadro("hola"));

        let _ = rx.try_recv().expect("layout");
        match rx.try_recv().expect("cuadro") {
            ServerMessage::Frame {
                session,
                changed,
                columns,
                ..
            } => {
                assert_eq!(session, 7);
                assert_eq!(columns, 80);
                assert_eq!(changed[0].runs[0].text, "hola");
            }
            otro => panic!("esperaba un cuadro, llegó {otro:?}"),
        }
    }

    /// La entrada que dejó un cliente sale por el hook, ya drenada.
    #[test]
    fn la_entrada_del_cliente_sale_por_el_hook() {
        let puente = Arc::new(Bridge::new());
        let plugin = MobilePlugin::new(puente.clone());
        let (tx, _rx): (crate::bridge::ClientSink, Receiver<ServerMessage>) = channel();
        let cliente = puente.join(
            tx,
            crate::protocol::Layout {
                spaces: Vec::new(),
                active_space: 0,
            },
            7,
        );

        puente.handle(
            cliente,
            ClientMessage::Input {
                session: 7,
                data: "ls\n".to_string(),
            },
        );

        assert_eq!(plugin.take_session_input(), vec![(7, b"ls\n".to_vec())]);
        assert!(
            plugin.take_session_input().is_empty(),
            "el anfitrión no puede recibirla dos veces"
        );
    }
}
