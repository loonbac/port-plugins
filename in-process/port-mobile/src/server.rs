//! Servidor del cliente móvil.
//!
//! Escucha **sólo en localhost** y sirve la aplicación web. Publicarla en la red
//! es trabajo de `tailscale serve`, que además pone el TLS que iOS exige para
//! instalar la app y usar un service worker: si esto escuchara en la VPN, el
//! teléfono no podría instalarla aunque llegara.
//!
//! Cada conexión se atiende en su propio hilo. Son pocas y locales, y así una
//! lectura lenta de un cliente no frena a los demás.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::bridge::Bridge;
use crate::http::{self, Route, MAX_REQUEST_BYTES};
use crate::socket::ViewProvider;

/// Dónde escucha el servidor y qué sirve.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Dirección de escucha. Por defecto, localhost.
    pub bind: String,
    /// Token que guarda el socket.
    ///
    /// Si está vacío el servidor no arranca: sin token, cualquiera que alcance
    /// el puerto podría escribir en la terminal.
    pub token: String,
    /// Directorio con los archivos de la PWA.
    pub assets: PathBuf,
}

impl ServerConfig {
    /// Configuración que escucha donde corresponde y sirve ese directorio.
    pub fn new(token: String, assets: PathBuf) -> Self {
        Self {
            bind: "127.0.0.1:0".to_string(),
            token,
            assets,
        }
    }
}

/// Espera máxima por la petición de un cliente, en segundos.
///
/// Un cliente que conecta y no escribe no puede quedarse con el turno para
/// siempre, y éste es su propio hilo: pasado el plazo se cierra su conexión.
const READ_TIMEOUT: Duration = Duration::from_secs(10);

/// Enlaza el puerto y devuelve el escucha con su dirección real.
///
/// Se separa de [`serve`] para que las pruebas puedan pedir un puerto efímero y
/// saber cuál les tocó.
pub fn bind(config: &ServerConfig) -> std::io::Result<(TcpListener, SocketAddr)> {
    let listener = TcpListener::bind(&config.bind)?;
    let address = listener.local_addr()?;
    Ok((listener, address))
}

/// Atiende conexiones hasta que el escucha se cierre.
pub fn serve(
    listener: TcpListener,
    config: ServerConfig,
    bridge: Arc<Bridge>,
    view: ViewProvider,
) -> std::io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("port-mobile".to_string())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else {
                    continue;
                };
                let config = config.clone();
                let bridge = Arc::clone(&bridge);
                let view = Arc::clone(&view);
                // Un hilo por cliente: son pocos y locales, y así una lectura
                // lenta no frena al resto.
                let _ = thread::Builder::new()
                    .name("port-mobile-client".to_string())
                    .spawn(move || {
                        let _ = handle_connection(stream, &config, &bridge, &view);
                    });
            }
        })
}

/// Enlaza y atiende en un solo paso.
pub fn spawn(
    config: ServerConfig,
    bridge: Arc<Bridge>,
    view: ViewProvider,
) -> std::io::Result<(SocketAddr, JoinHandle<()>)> {
    let (listener, address) = bind(&config)?;
    let handle = serve(listener, config, bridge, view)?;
    Ok((address, handle))
}

/// Atiende una conexión: una petición, una respuesta.
pub fn handle_connection(
    mut stream: TcpStream,
    config: &ServerConfig,
    bridge: &Arc<Bridge>,
    view: &ViewProvider,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    let mut raw = Vec::new();
    let mut buffer = [0u8; 2048];
    // Se lee hasta la línea en blanco que cierra la cabecera, o hasta el tope.
    loop {
        let read = stream.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        raw.extend_from_slice(&buffer[..read]);
        if raw.windows(4).any(|window| window == b"\r\n\r\n") || raw.len() >= MAX_REQUEST_BYTES {
            break;
        }
    }

    let body = match http::parse_request(&raw) {
        Some(request) => match http::route(&request, &config.token) {
            // El apretón de manos del socket lo escribe la propia biblioteca: acá
            // no se responde nada, se le entrega la conexión al bucle.
            Route::Upgrade => return crate::socket::serve_socket(stream, bridge, view()),
            Route::Asset(path) => serve_asset(&config.assets, &path),
            Route::Unauthorized => http::response(401, "text/plain", b"token requerido"),
            Route::NotFound => http::response(404, "text/plain", b"no encontrado"),
        },
        // Una petición ilegible no se responde con un error del protocolo: no se
        // atiende.
        None => http::response(404, "text/plain", b"no encontrado"),
    };
    stream.write_all(&body)?;
    stream.flush()
}

/// Respuesta con el contenido de un archivo de la PWA.
fn serve_asset(root: &std::path::Path, path: &str) -> Vec<u8> {
    let Some(file) = http::resolve_asset(root, path) else {
        return http::response(404, "text/plain", b"no encontrado");
    };
    match std::fs::read(&file) {
        Ok(bytes) => http::response(200, http::content_type_for(path), &bytes),
        Err(_) => http::response(404, "text/plain", b"no encontrado"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Layout;
    use crate::socket::InitialView;
    use std::io::BufRead;

    /// Directorio temporal con una PWA mínima.
    fn pwa_de_prueba(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "port-mobile-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("crear directorio");
        std::fs::write(dir.join("index.html"), b"<h1>PORT</h1>").expect("escribir índice");
        dir
    }

    /// Manda una petición cruda y devuelve la respuesta cruda.
    fn pedir(address: SocketAddr, peticion: &str) -> String {
        let mut stream = TcpStream::connect(address).expect("conectar");
        stream
            .write_all(peticion.as_bytes())
            .expect("escribir petición");
        stream.flush().expect("enviar");
        let mut respuesta = String::new();
        let _ = std::io::BufReader::new(&stream).read_line(&mut respuesta);
        respuesta
    }

    /// Vista inicial de las pruebas: sin espacios, mirando la sesión 7.
    fn vista() -> ViewProvider {
        Arc::new(|| InitialView {
            layout: Layout {
                spaces: Vec::new(),
                active_space: 0,
            },
            session: 7,
        })
    }

    /// Pone el servidor a escuchar en un puerto efímero.
    fn servidor(tag: &str, token: &str) -> (SocketAddr, PathBuf) {
        let assets = pwa_de_prueba(tag);
        let mut config = ServerConfig::new(token.to_string(), assets.clone());
        config.bind = "127.0.0.1:0".to_string();
        let (address, _handle) = spawn(config, Arc::new(Bridge::new()), vista()).expect("servidor");
        (address, assets)
    }

    /// La PWA se sirve sin pedir token, para que el navegador pueda instalarla.
    #[test]
    fn la_pwa_se_sirve_sin_token() {
        let (address, assets) = servidor("pwa", "secreto");
        let respuesta = pedir(address, "GET / HTTP/1.1\r\nHost: x\r\n\r\n");

        assert!(respuesta.starts_with("HTTP/1.1 200 OK"), "{respuesta}");
        // La cabecera sola: el cuerpo va después y lo pide el navegador.
        let mut stream = TcpStream::connect(address).expect("conectar");
        stream
            .write_all(b"GET /index.html HTTP/1.1\r\nHost: x\r\n\r\n")
            .expect("escribir");
        let mut completa = String::new();
        let _ = stream.read_to_string(&mut completa);
        assert!(completa.contains("<h1>PORT</h1>"), "{completa}");
        let _ = std::fs::remove_dir_all(&assets);
    }

    /// El socket no se entrega sin el token.
    #[test]
    fn el_socket_exige_token() {
        let (address, assets) = servidor("token", "secreto");
        assert!(
            pedir(address, "GET /ws HTTP/1.1\r\nHost: x\r\n\r\n").starts_with("HTTP/1.1 401"),
            "sin token debía negarse"
        );
        assert!(
            pedir(address, "GET /ws?token=nope HTTP/1.1\r\nHost: x\r\n\r\n")
                .starts_with("HTTP/1.1 401"),
            "con un token equivocado debía negarse"
        );
        // Con el token correcto la autenticación pasa y la conexión se entrega
        // al bucle del socket. Un `GET` sin las cabeceras del protocolo no puede
        // completar el apretón de manos, así que no hay respuesta: lo que se
        // comprueba es que ya no es un rechazo por token.
        let con_token = pedir(address, "GET /ws?token=secreto HTTP/1.1\r\nHost: x\r\n\r\n");
        assert!(!con_token.starts_with("HTTP/1.1 401"), "{con_token}");
        let _ = std::fs::remove_dir_all(&assets);
    }

    /// No se puede salir del directorio de la PWA.
    #[test]
    fn no_se_sirve_fuera_de_la_raiz() {
        let (address, assets) = servidor("travesia", "secreto");
        let respuesta = pedir(address, "GET /../etc/passwd HTTP/1.1\r\nHost: x\r\n\r\n");
        assert!(respuesta.starts_with("HTTP/1.1 404"), "{respuesta}");
        let _ = std::fs::remove_dir_all(&assets);
    }
}
