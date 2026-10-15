//! El bucle del socket: la terminal en vivo.
//!
//! Un cliente autenticado entra acá y se queda hasta que se va. El trabajo es
//! intercalar dos cosas: mandarle lo que el puente tenga pendiente para él y
//! escuchar lo que él mande.
//!
//! El intercalado se hace con un plazo de lectura corto, y un plazo vencido
//! **no es un fallo**: significa que el cliente todavía no dijo nada y es el
//! momento de mandar lo que haya. Tratar ese vencimiento como error cerraría la
//! conexión de un cliente sano en cuanto la terminal se quedara quieta.

use std::io::ErrorKind;
use std::net::TcpStream;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::Duration;

use tungstenite::{accept, Message, WebSocket};

use crate::bridge::Bridge;
use crate::protocol::{ClientMessage, Layout, ServerMessage};

/// Lo que un cliente necesita para empezar a mirar.
#[derive(Debug, Clone)]
pub struct InitialView {
    /// El modelo de espacios y pestañas al conectar.
    pub layout: Layout,
    /// La sesión que mira mientras no pida otra.
    pub session: usize,
}

/// De dónde sale lo que un cliente nuevo necesita saber.
///
/// Se pide en cada conexión y no se guarda: el layout cambia mientras la
/// terminal está abierta, y un cliente que entra tarde tiene que ver el de ahora.
pub type ViewProvider = Arc<dyn Fn() -> InitialView + Send + Sync>;

/// Cada cuánto se deja de escuchar al cliente para mandarle lo pendiente.
const LISTEN_TICK: Duration = Duration::from_millis(10);

/// Plazo para el apretón de manos.
///
/// Es una lectura más —la del cliente que se presenta— y merece un plazo
/// generoso: el plazo corto es para después, para intercalar el envío con la
/// escucha, y aplicarlo antes haría que el saludo se rindiera por impaciencia.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Atiende un socket ya autenticado hasta que el cliente se va.
pub fn serve_socket(
    stream: TcpStream,
    bridge: &Arc<Bridge>,
    view: InitialView,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT))?;
    let mut socket = accept(stream).map_err(|error| std::io::Error::other(error.to_string()))?;
    socket.get_ref().set_read_timeout(Some(LISTEN_TICK))?;
    let (out, incoming) = mpsc::channel();
    let client = bridge.join(out, view.layout, view.session);
    let outcome = run(&mut socket, bridge, client, &incoming);
    // Se da de baja pase lo que pase: un cliente que se fue no puede seguir
    // contando como alguien que mira una sesión.
    bridge.leave(client);
    outcome
}

/// Manda lo pendiente y escucha al cliente, hasta que alguno de los dos corta.
fn run(
    socket: &mut WebSocket<TcpStream>,
    bridge: &Arc<Bridge>,
    client: u64,
    incoming: &Receiver<ServerMessage>,
) -> std::io::Result<()> {
    loop {
        // 1. Lo que el puente tenga para este cliente.
        loop {
            match incoming.try_recv() {
                Ok(message) => {
                    let json = match serde_json::to_string(&message) {
                        Ok(json) => json,
                        // Un mensaje que no se puede serializar es un defecto
                        // nuestro, no del cliente: se descarta y se sigue.
                        Err(_) => continue,
                    };
                    if socket.send(Message::Text(json.into())).is_err() {
                        return Ok(());
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return Ok(()),
            }
        }

        // 2. Lo que mande el cliente.
        match socket.read() {
            Ok(Message::Text(text)) => {
                if let Ok(message) = serde_json::from_str::<ClientMessage>(&text) {
                    // El cambio de foco ya dejó su foto nueva en el canal, así
                    // que la próxima vuelta la manda.
                    let _ = bridge.handle(client, message);
                }
            }
            // Cerrar es la forma educada de irse.
            Ok(Message::Close(_)) => return Ok(()),
            // Binario, ping y pong los resuelve la biblioteca.
            Ok(_) => {}
            // El plazo vencido sólo significa que no hay nada nuevo que leer.
            Err(tungstenite::Error::Io(error))
                if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            // La conexión se fue: no hay nada que hacer salvo terminar.
            Err(tungstenite::Error::ConnectionClosed) | Err(tungstenite::Error::AlreadyClosed) => {
                return Ok(())
            }
            Err(_) => return Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{AgentView, RowUpdate, SpaceView, TabView, WireRun};
    use tungstenite::http::Request;

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

    fn layout() -> Layout {
        Layout {
            spaces: vec![SpaceView {
                name: "espacio".to_string(),
                branch: "main".to_string(),
                color: None,
                active_tab: 0,
                agent: AgentView::Blocked,
                tabs: vec![TabView {
                    title: "terminal".to_string(),
                    session: 7,
                    agent: AgentView::Blocked,
                    seen: false,
                }],
            }],
            active_space: 0,
        }
    }

    /// Petición de un cliente WebSocket, con la clave que exige el protocolo.
    ///
    /// La URI lleva esquema porque la biblioteca del lado cliente lo valida; lo
    /// que viaja en la línea de la petición es sólo la ruta.
    fn peticion() -> Request<()> {
        Request::builder()
            .uri("ws://localhost/ws")
            .header("Host", "localhost")
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .header("Sec-WebSocket-Version", "13")
            .header(
                "Sec-WebSocket-Key",
                tungstenite::handshake::client::generate_key(),
            )
            .body(())
            .expect("petición de cliente")
    }

    /// Levanta un cliente sobre un par de sockets en memoria.
    fn cliente_de_prueba(
        bridge: &Arc<Bridge>,
    ) -> (WebSocket<TcpStream>, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("escuchar");
        let address = listener.local_addr().expect("dirección");
        let puente = Arc::clone(bridge);
        let servidor = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("aceptar");
            let view = InitialView {
                layout: layout(),
                session: 7,
            };
            let _ = serve_socket(stream, &puente, view);
        });
        let stream = TcpStream::connect(address).expect("conectar");
        let (socket, _response) = tungstenite::client(peticion(), stream).expect("apretón");
        (socket, servidor)
    }

    /// Al entrar, el cliente recibe el layout antes que cualquier cuadro.
    #[test]
    fn al_entrar_llega_el_layout() {
        let puente = Arc::new(Bridge::new());
        let (mut socket, servidor) = cliente_de_prueba(&puente);

        match socket.read().expect("leer") {
            Message::Text(texto) => {
                let message: ServerMessage = serde_json::from_str(&texto).expect("protocolo");
                assert!(matches!(message, ServerMessage::Layout { .. }));
            }
            otro => panic!("esperaba el layout, llegó {otro:?}"),
        }

        let _ = socket.close(None);
        let _ = servidor.join();
    }

    /// Un cuadro publicado llega al cliente que mira esa sesión.
    #[test]
    fn un_cuadro_publicado_llega_al_cliente() {
        let puente = Arc::new(Bridge::new());
        let (mut socket, servidor) = cliente_de_prueba(&puente);
        let _ = socket.read().expect("layout");

        puente.publish_frame(7, 80, vec![fila(0, "hola")]);

        match socket.read().expect("leer") {
            Message::Text(texto) => {
                let message: ServerMessage = serde_json::from_str(&texto).expect("protocolo");
                match message {
                    ServerMessage::Frame {
                        session, changed, ..
                    } => {
                        assert_eq!(session, 7);
                        assert_eq!(changed.len(), 1);
                    }
                    otro => panic!("esperaba un cuadro, llegó {otro:?}"),
                }
            }
            otro => panic!("esperaba texto, llegó {otro:?}"),
        }

        let _ = socket.close(None);
        let _ = servidor.join();
    }

    /// Lo que el cliente escribe llega al puente para que el anfitrión lo drene.
    #[test]
    fn lo_que_escribe_el_cliente_llega_al_puente() {
        let puente = Arc::new(Bridge::new());
        let (mut socket, servidor) = cliente_de_prueba(&puente);
        let _ = socket.read().expect("layout");

        let mensaje = ClientMessage::Input {
            session: 7,
            data: "ls\n".to_string(),
        };
        socket
            .send(Message::Text(
                serde_json::to_string(&mensaje).expect("serializar").into(),
            ))
            .expect("enviar");

        // El puente lo recibe desde el hilo del servidor: se espera a que llegue
        // en vez de asumir que ya está.
        let mut recibido = Vec::new();
        for _ in 0..200 {
            recibido = puente.take_input();
            if !recibido.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(recibido, vec![(7, b"ls\n".to_vec())]);

        let _ = socket.close(None);
        let _ = servidor.join();
    }
}
