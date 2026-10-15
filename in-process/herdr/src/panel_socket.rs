//! Socket de paneles: el transporte de la superficie de herdr que PORT intercepta.
//!
//! Única responsabilidad: escuchar en un socket de dominio Unix, leer una
//! petición por conexión, aplicarla y responder. El formato y su validación
//! viven en [`crate::report`]; el libro de lo declarado, en [`crate::state`].

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::report::{error_response, ok_response, parse_request};
use crate::state::HerdrState;

/// Tope de una petición (C8): más que esto no es una petición.
pub(crate) const MAX_REQUEST_BYTES: usize = 1024 * 1024;
/// Espera máxima por la petición de un cliente, en segundos.
///
/// Un cliente que conecta y no escribe no puede quedarse con el turno para
/// siempre; pasado este plazo se cierra su conexión.
const READ_TIMEOUT: Duration = Duration::from_secs(5);
/// Permisos del socket: sólo su dueño (C8).
const SOCKET_MODE: u32 = 0o600;

/// Escucha informes de panel en `path` y devuelve el hilo que los atiende.
pub(crate) fn spawn(
    state: Arc<RwLock<HerdrState>>,
    path: &Path,
) -> std::io::Result<JoinHandle<()>> {
    let listener = bind(path)?;
    std::thread::Builder::new()
        .name("herdr-panel-socket".to_string())
        .spawn(move || serve(&state, listener))
}

/// Crea el socket, reemplazando el archivo de una instancia que ya no corre.
fn bind(path: &Path) -> std::io::Result<UnixListener> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(SOCKET_MODE))?;
    Ok(listener)
}

/// Atiende conexiones una tras otra.
fn serve(state: &Arc<RwLock<HerdrState>>, listener: UnixListener) {
    for stream in listener.incoming() {
        let Ok(stream) = stream else {
            continue;
        };
        // El cliente es local y manda una sola línea: atenderlo en línea no
        // frena a nadie de forma apreciable y evita hilos sin tope.
        let _ = handle_connection(state, stream);
    }
}

/// Atiende una conexión: una petición, una respuesta.
pub(crate) fn handle_connection(
    state: &Arc<RwLock<HerdrState>>,
    mut stream: UnixStream,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    let mut line = String::new();
    {
        // Un byte de más permite detectar que la línea se pasó del tope.
        let reader = BufReader::new(&stream);
        Read::take(reader, (MAX_REQUEST_BYTES + 1) as u64).read_line(&mut line)?;
    }
    let response = respond(state, &line);
    stream.write_all(response.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()
}

/// Respuesta para una línea ya recibida.
///
/// Una petición ilegible no trae identificador, así que la respuesta va sin él;
/// el cliente de este contrato ignora la salida en cualquier caso.
fn respond(state: &Arc<RwLock<HerdrState>>, line: &str) -> String {
    let request = match parse_request(line) {
        Ok(request) => request,
        Err(error) => return error_response("", &error),
    };
    let outcome = state
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .apply_panel_report(&request, crate::agents::now_ms());
    match outcome {
        Ok(_) => ok_response(&request.id),
        Err(error) => error_response(&request.id, &error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use port_plugin_api::AgentState;
    use serde_json::{json, Value};

    /// Ruta de socket única por prueba, dentro del temporal del sistema.
    fn socket_path(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "herdr-panel-{tag}-{}-{:?}.sock",
            std::process::id(),
            std::thread::current().id()
        ))
    }

    /// Manda una línea al socket y devuelve la respuesta.
    fn ask(path: &Path, line: &str) -> String {
        let mut client = UnixStream::connect(path).expect("conectar al socket");
        client.write_all(line.as_bytes()).expect("escribir");
        client.write_all(b"\n").expect("cerrar la línea");
        let mut response = String::new();
        BufReader::new(&client)
            .read_line(&mut response)
            .expect("leer la respuesta");
        response
    }

    /// Un informe válido por el socket cambia el estado del agente y responde ok.
    #[test]
    fn un_informe_valido_cambia_el_estado() {
        let state = Arc::new(RwLock::new(HerdrState::default()));
        state.write().unwrap().panel_reports.bind("w1:p1", 7);
        let path = socket_path("informe");
        let _listener = spawn(Arc::clone(&state), &path).expect("escuchar");

        let request = json!({
            "id": "cli:request",
            "method": "pane.report_metadata",
            "params": {
                "pane_id": "w1:p1",
                "agent": "pi",
                "source": "gentle:activity",
                "tokens": { "summary": "leyendo", "port_state": "blocked" },
                "seq": 1,
                "ttl_ms": 30_000,
            },
        })
        .to_string();
        let response: Value = serde_json::from_str(&ask(&path, &request)).expect("json");

        assert_eq!(response["result"]["type"], "ok");
        let s = state.read().unwrap();
        assert_eq!(s.agents.get(&7), Some(&(AgentState::Blocked, false)));
        assert_eq!(s.panel_reports.value("w1:p1", "summary"), Some("leyendo"));
        drop(s);
        let _ = std::fs::remove_file(&path);
    }

    /// Un panel que el plugin no anunció es un error del contrato.
    #[test]
    fn panel_desconocido_responde_error() {
        let state = Arc::new(RwLock::new(HerdrState::default()));
        let path = socket_path("desconocido");
        let _listener = spawn(Arc::clone(&state), &path).expect("escuchar");

        let request = json!({
            "id": "cli:request",
            "method": "pane.report_metadata",
            "params": { "pane_id": "w9:p9", "source": "s" },
        })
        .to_string();
        let response: Value = serde_json::from_str(&ask(&path, &request)).expect("json");

        assert_eq!(response["id"], "cli:request");
        assert_eq!(response["error"]["code"], "pane_not_found");
        assert_eq!(response["error"]["message"], "pane w9:p9 not found");
        assert!(state.read().unwrap().agents.is_empty());
        let _ = std::fs::remove_file(&path);
    }

    /// Una secuencia vieja responde ok pero no toca nada, igual que herdr.
    #[test]
    fn secuencia_vieja_no_cambia_nada() {
        let state = Arc::new(RwLock::new(HerdrState::default()));
        state.write().unwrap().panel_reports.bind("w1:p1", 7);
        let path = socket_path("vieja");
        let _listener = spawn(Arc::clone(&state), &path).expect("escuchar");

        let enviar = |state: &str, seq: u64| {
            json!({
                "id": "cli:request",
                "method": "pane.report_metadata",
                "params": {
                    "pane_id": "w1:p1",
                    "source": "gentle:activity",
                    "tokens": { "port_state": state },
                    "seq": seq,
                },
            })
            .to_string()
        };

        // La primera entra y deja el panel bloqueado.
        let _ = ask(&path, &enviar("blocked", 5));
        assert_eq!(
            state.read().unwrap().agents.get(&7),
            Some(&(AgentState::Blocked, false))
        );

        // La vieja responde ok pero no cambia el estado.
        let response: Value =
            serde_json::from_str(&ask(&path, &enviar("working", 4))).expect("json");
        assert_eq!(response["result"]["type"], "ok");
        assert_eq!(
            state.read().unwrap().agents.get(&7),
            Some(&(AgentState::Blocked, false))
        );
        let _ = std::fs::remove_file(&path);
    }
}
