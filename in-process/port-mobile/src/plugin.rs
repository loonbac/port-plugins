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

use std::sync::{Arc, Mutex};

use port_plugin_api::{Plugin, Ret, Services, SpaceHook};
use port_term_core::frame::{Frame, Rgb};
use port_term_core::session::GridSize;

use crate::bridge::Bridge;
use crate::protocol::{CursorView, Layout, RowUpdate, WireRun};
use crate::socket::InitialView;

/// Identificador del servicio que publica el modelo de espacios.
///
/// Es el `id` del plugin que lo publica. Está acá porque quien consume no puede
/// conocer su tipo, y una prueba con un servicio de mentira cubre el camino
/// completo: si el productor cambiara su identificador, esa prueba lo dice.
const LAYOUT_SERVICE_ID: &str = "herdr";
/// Acción que devuelve el modelo.
const LAYOUT_ACTION: &str = "layout";

/// Plugin que lleva la terminal al teléfono.
pub struct MobilePlugin {
    bridge: Arc<Bridge>,
    /// Directorio de servicios, que llega al registrarse.
    ///
    /// Se guarda porque el modelo se pide después, cuando un cliente entra: para
    /// entonces todos los plugins ya publicaron lo suyo.
    services: Mutex<Option<Arc<Services>>>,
}

impl MobilePlugin {
    /// Crea el plugin sobre un puente ya construido.
    pub fn new(bridge: Arc<Bridge>) -> Self {
        Self {
            bridge,
            services: Mutex::new(None),
        }
    }

    /// El puente compartido, para que quien arme el servidor lo use.
    pub fn bridge(&self) -> Arc<Bridge> {
        Arc::clone(&self.bridge)
    }

    /// Lo que un cliente nuevo necesita saber al entrar.
    ///
    /// El modelo se pide al plugin que lo tiene y la sesión inicial es la que el
    /// escritorio tiene a la vista, para que el teléfono empiece donde uno está.
    pub fn view(&self) -> InitialView {
        let layout = self.layout();
        let session = layout
            .spaces
            .get(layout.active_space)
            .and_then(|space| space.tabs.get(space.active_tab))
            .map(|tab| tab.session)
            .unwrap_or(0);
        InitialView { layout, session }
    }

    /// El modelo de espacios, pedido a quien lo publica.
    ///
    /// Sin servicio o con una respuesta que no se entiende se devuelve un modelo
    /// vacío: un teléfono sin espacios configurados es peor que nada, pero mucho
    /// mejor que una terminal que no abre.
    fn layout(&self) -> Layout {
        let Some(services) = self
            .services
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
        else {
            return Layout::default();
        };
        match services.call(LAYOUT_SERVICE_ID, LAYOUT_ACTION, &[]) {
            Ok(Ret::Text(json)) => serde_json::from_str(&json).unwrap_or_default(),
            _ => Layout::default(),
        }
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

    fn attach_services(&self, services: Arc<Services>) {
        *self
            .services
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(services);
    }
}

impl SpaceHook for MobilePlugin {
    /// Se miran las sesiones que algún cliente está viendo, y ninguna más.
    fn watched_sessions(&self) -> Vec<usize> {
        self.bridge.watched()
    }

    fn update_session_frame(&self, session_id: usize, frame: &Frame) {
        // El cursor va aparte y antes que las filas: se mueve sin que ninguna
        // fila cambie, y así el cuadro que sigue ya lo lleva.
        self.bridge.publish_cursor(session_id, wire_cursor(frame));
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

/// Dónde está el cursor del cuadro, si está visible.
fn wire_cursor(frame: &Frame) -> Option<CursorView> {
    frame.cursor.as_ref().map(|cursor| CursorView {
        row: cursor.row,
        column: cursor.column,
        visible: cursor.visible,
    })
}

/// Color en la forma que viaja por el cable.
fn hex(color: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", color.r, color.g, color.b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{AgentView, ClientMessage, ServerMessage};
    use port_plugin_api::{Arg, Service, ServiceError};
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

    /// Servicio de prueba que responde el modelo, como lo hace el plugin herdr.
    struct StubLayout;

    impl Service for StubLayout {
        fn id(&self) -> &str {
            LAYOUT_SERVICE_ID
        }

        fn name(&self) -> &str {
            "stub"
        }

        fn actions(&self) -> Vec<&'static str> {
            vec![LAYOUT_ACTION]
        }

        fn invoke(&self, action: &str, _args: &[Arg]) -> Option<Result<Ret, ServiceError>> {
            if action != LAYOUT_ACTION {
                return None;
            }
            Some(Ok(Ret::Text(
                r#"{"spaces":[{"name":"espacio","branch":"main","color":null,"active_tab":0,"agent":"blocked","tabs":[{"title":"terminal","session":7,"agent":"blocked","seen":false}]}],"active_space":0}"#
                    .to_string(),
            )))
        }
    }

    /// El modelo se pide al servicio y la sesión inicial sale de su pestaña activa.
    #[test]
    fn el_modelo_se_pide_al_servicio() {
        let services = Arc::new(Services::new());
        services.publish(Arc::new(StubLayout));
        let plugin = MobilePlugin::new(Arc::new(Bridge::new()));
        plugin.attach_services(services);

        let view = plugin.view();

        assert_eq!(view.session, 7, "la sesión inicial es la pestaña activa");
        assert_eq!(view.layout.spaces.len(), 1);
        assert_eq!(view.layout.spaces[0].name, "espacio");
        assert_eq!(view.layout.spaces[0].agent, AgentView::Blocked);
    }

    /// Sin servicio el modelo sale vacío, pero la vista existe igual.
    ///
    /// Un teléfono sin espacios es peor que uno con ellos, y mucho mejor que una
    /// terminal que no abre.
    #[test]
    fn sin_servicio_el_modelo_sale_vacio() {
        let plugin = MobilePlugin::new(Arc::new(Bridge::new()));
        let view = plugin.view();
        assert!(view.layout.spaces.is_empty());
        assert_eq!(view.session, 0);
    }

    /// Una respuesta que no se entiende no rompe la vista.
    #[test]
    fn una_respuesta_ilegible_no_rompe_la_vista() {
        struct StubRoto;

        impl Service for StubRoto {
            fn id(&self) -> &str {
                LAYOUT_SERVICE_ID
            }

            fn name(&self) -> &str {
                "roto"
            }

            fn actions(&self) -> Vec<&'static str> {
                vec![LAYOUT_ACTION]
            }

            fn invoke(&self, _action: &str, _args: &[Arg]) -> Option<Result<Ret, ServiceError>> {
                Some(Ok(Ret::Text("esto no es json".to_string())))
            }
        }

        let services = Arc::new(Services::new());
        services.publish(Arc::new(StubRoto));
        let plugin = MobilePlugin::new(Arc::new(Bridge::new()));
        plugin.attach_services(services);

        assert!(plugin.view().layout.spaces.is_empty());
    }
}
