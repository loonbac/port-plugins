//! Puente entre el hilo de render y los clientes remotos.
//!
//! El anfitrión entrega los cuadros y consume la entrada **en el hilo de
//! render**; cada cliente vive en su propio hilo con su socket. Este módulo es
//! el único lugar donde esos dos mundos se cruzan, y por eso concentra todo el
//! estado compartido.
//!
//! De acá sale, además, la respuesta a la pregunta que le hace el anfitrión:
//! **qué sesiones hay que mirar**. El plugin le pide los cuadros de las sesiones
//! que algún cliente está mirando y de ninguna más, así que el costo depende de
//! cuántas terminales se estén viendo desde lejos y no de cuántas haya abiertas.
//!
//! Todo se muta bajo un mismo candado, y los envíos a los clientes ocurren
//! dentro de él: un cliente nuevo recibe su primera foto completa antes de que
//! llegue cualquier diferencia, así que nunca ve un cambio sin haber visto de
//! dónde venía.

use std::collections::BTreeMap;
use std::sync::mpsc::Sender;
use std::sync::Mutex;

use port_term_core::session::GridSize;

use crate::protocol::{changed_slice, ClientMessage, Layout, RowUpdate, ServerMessage};

/// Salida de un cliente: por dónde le llegan los mensajes.
pub type ClientSink = Sender<ServerMessage>;

/// Lo que el puente sabe de un cliente.
struct Client {
    /// Por dónde recibe sus mensajes.
    out: ClientSink,
    /// Sesión que está mirando.
    focus: usize,
    /// `true` cuando todavía hay que mandarle la foto completa de esa sesión.
    ///
    /// Pasa al entrar y cada vez que cambia de sesión: una diferencia sólo tiene
    /// sentido sobre algo que el cliente ya vio.
    needs_full: bool,
}

/// Estado compartido entre el hilo de render y los clientes.
#[derive(Default)]
struct Shared {
    /// Clientes conectados, por identificador.
    clients: BTreeMap<u64, Client>,
    /// Últimas filas publicadas de cada sesión que alguien mira.
    rows: BTreeMap<usize, Vec<RowUpdate>>,
    /// Ancho de la última rejilla publicada de cada sesión.
    columns: BTreeMap<usize, usize>,
    /// Entrada pendiente de escribir, en orden de llegada.
    input: Vec<(usize, Vec<u8>)>,
    /// Redimensiones pendientes, en orden de llegada.
    resize: Vec<(usize, GridSize)>,
    /// Identificador del próximo cliente.
    next_client: u64,
}

/// Puente entre la terminal y los clientes remotos.
#[derive(Default)]
pub struct Bridge {
    shared: Mutex<Shared>,
}

impl Bridge {
    /// Crea un puente sin clientes.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registra un cliente y le entrega el estado completo antes de cualquier
    /// diferencia.
    ///
    /// Devuelve su identificador, que hace falta para atenderlo después.
    pub fn join(&self, out: ClientSink, layout: Layout, focus: usize) -> u64 {
        let mut shared = self.lock();
        let id = shared.next_client;
        shared.next_client += 1;
        shared.clients.insert(
            id,
            Client {
                out: out.clone(),
                focus,
                needs_full: true,
            },
        );
        let _ = out.send(ServerMessage::Layout { layout });
        // Si ya hay filas de esa sesión, la foto completa sale ahora; si no, sale
        // con el primer cuadro que publique el anfitrión.
        let snapshot = shared.rows.get(&focus).cloned();
        let columns = shared.columns.get(&focus).copied().unwrap_or_default();
        if let Some(rows) = snapshot {
            let _ = out.send(frame_message(focus, columns, &rows, rows.clone()));
            if let Some(entry) = shared.clients.get_mut(&id) {
                entry.needs_full = false;
            }
        }
        id
    }

    /// Da de baja a un cliente.
    pub fn leave(&self, client: u64) {
        self.lock().clients.remove(&client);
    }

    /// Atiende un mensaje de un cliente.
    ///
    /// Devuelve `true` si el pedido cambia qué hay que mostrar, para que el
    /// plugin vuelva a anunciar lo que mira.
    pub fn handle(&self, client: u64, message: ClientMessage) -> bool {
        let mut shared = self.lock();
        match message {
            ClientMessage::Input { session, data } => {
                shared.input.push((session, data.into_bytes()));
                false
            }
            ClientMessage::Resize {
                session,
                columns,
                rows,
            } => {
                shared.resize.push((session, GridSize::new(columns, rows)));
                false
            }
            ClientMessage::Focus { session } => {
                // Se lee primero y se muta después: tomar el cliente prestado
                // mientras se miran las filas entrelaza dos préstamos del mismo
                // estado sin necesidad.
                let cambia = match shared.clients.get(&client) {
                    Some(entry) => entry.focus != session,
                    None => return false,
                };
                if !cambia {
                    return false;
                }
                let snapshot = shared.rows.get(&session).cloned();
                let columns = shared.columns.get(&session).copied().unwrap_or_default();
                if let Some(entry) = shared.clients.get_mut(&client) {
                    entry.focus = session;
                    // Al cambiar de sesión hace falta una foto nueva: lo que se
                    // mandó de la anterior no dice nada de ésta.
                    entry.needs_full = true;
                }
                if let Some(rows) = snapshot {
                    if let Some(entry) = shared.clients.get_mut(&client) {
                        let _ =
                            entry
                                .out
                                .send(frame_message(session, columns, &rows, rows.clone()));
                        entry.needs_full = false;
                    }
                }
                true
            }
        }
    }

    /// Sesiones que algún cliente está mirando, sin repetir.
    ///
    /// Es lo que el plugin le responde al anfitrión cuando le pregunta qué
    /// quiere ver.
    pub fn watched(&self) -> Vec<usize> {
        let mut watched: Vec<usize> = Vec::new();
        for client in self.lock().clients.values() {
            if !watched.contains(&client.focus) {
                watched.push(client.focus);
            }
        }
        watched
    }

    /// Publica un cuadro nuevo de una sesión.
    ///
    /// A cada cliente que la mira se le manda **sólo lo que cambió**, salvo que
    /// acabe de entrar o de cambiar de sesión, que entonces recibe la foto
    /// completa.
    pub fn publish_frame(&self, session: usize, columns: usize, rows: Vec<RowUpdate>) {
        let mut shared = self.lock();
        let delta = match shared.rows.get(&session) {
            Some(previous) => changed_slice(previous, &rows),
            None => rows.clone(),
        };
        shared.rows.insert(session, rows.clone());
        shared.columns.insert(session, columns);
        for client in shared.clients.values_mut() {
            if client.focus != session {
                continue;
            }
            let payload = if client.needs_full {
                rows.clone()
            } else {
                delta.clone()
            };
            client.needs_full = false;
            let _ = client
                .out
                .send(frame_message(session, columns, &rows, payload));
        }
    }

    /// Avisa a los clientes que una sesión dejó de existir.
    pub fn publish_closed(&self, session: usize) {
        let mut shared = self.lock();
        shared.rows.remove(&session);
        shared.columns.remove(&session);
        for client in shared.clients.values_mut() {
            // El que la miraba se queda sin nada que mostrar: su pestaña se cerró
            // y el layout que recibirá a continuación ya no la trae.
            if client.focus == session {
                let _ = client.out.send(ServerMessage::Closed { session });
            }
        }
    }

    /// Entrada que los clientes quieren escribir, y que se drena una sola vez.
    pub fn take_input(&self) -> Vec<(usize, Vec<u8>)> {
        let mut shared = self.lock();
        std::mem::take(&mut shared.input)
    }

    /// Redimensiones que los clientes piden, y que se drenan una sola vez.
    pub fn take_resize(&self) -> Vec<(usize, GridSize)> {
        let mut shared = self.lock();
        std::mem::take(&mut shared.resize)
    }

    /// Cuántos clientes hay conectados.
    pub fn clients(&self) -> usize {
        self.lock().clients.len()
    }

    /// Toma el candado tratando un veneno como un estado válido.
    ///
    /// Un hilo que muere con el candado tomado no puede dejar la terminal sin
    /// atender a los clientes: el estado sigue siendo coherente, así que se usa
    /// igual en vez de propagar el pánico.
    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Arma el mensaje de cuadro con las filas que corresponda mandar.
fn frame_message(
    session: usize,
    columns: usize,
    rows: &[RowUpdate],
    changed: Vec<RowUpdate>,
) -> ServerMessage {
    ServerMessage::Frame {
        session,
        rows: rows.len(),
        columns,
        changed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{AgentView, ServerMessage, SpaceView, TabView, WireRun};
    use std::sync::mpsc::{channel, Receiver};

    /// Fila de una sola pieza, para comparar cuadros en las pruebas.
    fn fila(index: usize, texto: &str) -> RowUpdate {
        RowUpdate {
            index,
            runs: vec![WireRun {
                text: texto.to_string(),
                fg: "#ffffff".to_string(),
                bg: "#000000".to_string(),
            }],
        }
    }

    fn cuadro() -> Vec<RowUpdate> {
        vec![fila(0, "uno"), fila(1, "dos")]
    }

    fn layout() -> Layout {
        Layout {
            spaces: vec![SpaceView {
                name: "espacio".to_string(),
                branch: "main".to_string(),
                color: None,
                active_tab: 0,
                agent: AgentView::Unknown,
                tabs: vec![TabView {
                    title: "terminal".to_string(),
                    session: 7,
                    agent: AgentView::Unknown,
                    seen: true,
                }],
            }],
            active_space: 0,
        }
    }

    /// Un cliente nuevo recibe el layout antes que cualquier cuadro.
    #[test]
    fn el_cliente_nuevo_recibe_el_layout_primero() {
        let puente = Bridge::new();
        let (tx, rx): (ClientSink, Receiver<ServerMessage>) = channel();
        puente.join(tx, layout(), 7);

        assert!(matches!(rx.try_recv(), Ok(ServerMessage::Layout { .. })));
        assert!(rx.try_recv().is_err(), "no debía llegar nada más aún");
    }

    /// Al entrar con una sesión ya publicada, la primera foto es completa.
    #[test]
    fn la_primera_foto_de_una_sesion_ya_publicada_es_completa() {
        let puente = Bridge::new();
        puente.publish_frame(7, 80, cuadro());

        let (tx, rx): (ClientSink, Receiver<ServerMessage>) = channel();
        puente.join(tx, layout(), 7);

        let _ = rx.try_recv().expect("layout");
        match rx.try_recv().expect("cuadro inicial") {
            ServerMessage::Frame { changed, rows, .. } => {
                assert_eq!(rows, 2);
                assert_eq!(changed.len(), 2, "la primera foto es completa");
            }
            otro => panic!("esperaba un cuadro, llegó {otro:?}"),
        }
    }

    /// Después de la primera foto, sólo viaja lo que cambió.
    #[test]
    fn despues_solo_viaja_lo_que_cambio() {
        let puente = Bridge::new();
        let (tx, rx): (ClientSink, Receiver<ServerMessage>) = channel();
        puente.join(tx, layout(), 7);
        let _ = rx.try_recv().expect("layout");

        puente.publish_frame(7, 80, cuadro());
        let _ = rx.try_recv().expect("primera foto");

        puente.publish_frame(7, 80, vec![fila(0, "uno"), fila(1, "DOS")]);
        match rx.try_recv().expect("diferencia") {
            ServerMessage::Frame { changed, .. } => {
                assert_eq!(changed.len(), 1, "sólo cambió una fila");
                assert_eq!(changed[0].index, 1);
            }
            otro => panic!("esperaba un cuadro, llegó {otro:?}"),
        }
    }

    /// Sólo se miran las sesiones que algún cliente está viendo.
    #[test]
    fn solo_se_miran_las_sesiones_que_alguien_ve() {
        let puente = Bridge::new();
        let (tx, _rx): (ClientSink, Receiver<ServerMessage>) = channel();
        puente.join(tx, layout(), 7);
        assert_eq!(puente.watched(), vec![7]);

        let (tx2, _rx2): (ClientSink, Receiver<ServerMessage>) = channel();
        puente.join(tx2, layout(), 7);
        assert_eq!(
            puente.watched(),
            vec![7],
            "dos clientes no repiten la sesión"
        );

        let (tx3, _rx3): (ClientSink, Receiver<ServerMessage>) = channel();
        puente.join(tx3, layout(), 9);
        assert_eq!(puente.watched(), vec![7, 9]);
    }

    /// Al cambiar de sesión, el cliente recibe la foto de la nueva.
    #[test]
    fn cambiar_de_sesion_manda_la_foto_de_la_nueva() {
        let puente = Bridge::new();
        puente.publish_frame(9, 100, vec![fila(0, "nueva")]);
        let (tx, rx): (ClientSink, Receiver<ServerMessage>) = channel();
        let cliente = puente.join(tx, layout(), 7);
        let _ = rx.try_recv().expect("layout");

        assert!(puente.handle(cliente, ClientMessage::Focus { session: 9 }));
        match rx.try_recv().expect("foto de la sesión nueva") {
            ServerMessage::Frame {
                session, changed, ..
            } => {
                assert_eq!(session, 9);
                assert_eq!(changed.len(), 1);
            }
            otro => panic!("esperaba un cuadro, llegó {otro:?}"),
        }
        assert_eq!(puente.watched(), vec![9]);
    }

    /// Donde mira uno, el otro no recibe nada.
    #[test]
    fn cada_cliente_recibe_solo_lo_suyo() {
        let puente = Bridge::new();
        let (tx_a, rx_a): (ClientSink, Receiver<ServerMessage>) = channel();
        let (tx_b, rx_b): (ClientSink, Receiver<ServerMessage>) = channel();
        puente.join(tx_a, layout(), 7);
        puente.join(tx_b, layout(), 9);
        let _ = rx_a.try_recv();
        let _ = rx_b.try_recv();

        puente.publish_frame(9, 80, cuadro());

        assert!(rx_a.try_recv().is_err(), "el de la 7 no debía recibir la 9");
        assert!(rx_b.try_recv().is_ok(), "el de la 9 sí");
    }

    /// La entrada de los clientes se drena una sola vez.
    #[test]
    fn la_entrada_se_drena_una_vez() {
        let puente = Bridge::new();
        let (tx, _rx): (ClientSink, Receiver<ServerMessage>) = channel();
        let cliente = puente.join(tx, layout(), 7);

        puente.handle(
            cliente,
            ClientMessage::Input {
                session: 7,
                data: "ls\n".to_string(),
            },
        );
        puente.handle(
            cliente,
            ClientMessage::Resize {
                session: 7,
                columns: 100,
                rows: 30,
            },
        );

        assert_eq!(puente.take_input(), vec![(7, b"ls\n".to_vec())]);
        assert!(
            puente.take_input().is_empty(),
            "la entrada se entregó dos veces"
        );
        assert_eq!(puente.take_resize(), vec![(7, GridSize::new(100, 30))]);
        assert!(puente.take_resize().is_empty());
    }

    /// Un cliente que se va deja de recibir cuadros.
    #[test]
    fn el_cliente_que_se_va_deja_de_recibir() {
        let puente = Bridge::new();
        let (tx, rx): (ClientSink, Receiver<ServerMessage>) = channel();
        let cliente = puente.join(tx, layout(), 7);
        let _ = rx.try_recv();

        puente.leave(cliente);
        puente.publish_frame(7, 80, cuadro());

        assert!(rx.try_recv().is_err());
        assert!(puente.watched().is_empty());
    }

    /// Una sesión cerrada se avisa y deja de mirarse.
    #[test]
    fn una_sesion_cerrada_se_avisa() {
        let puente = Bridge::new();
        let (tx, rx): (ClientSink, Receiver<ServerMessage>) = channel();
        puente.join(tx, layout(), 7);
        let _ = rx.try_recv();

        puente.publish_closed(7);

        assert!(matches!(
            rx.try_recv(),
            Ok(ServerMessage::Closed { session: 7 })
        ));
    }
}
