//! Biblioteca de soporte para el ejecutable `port-herdr`.
//!
//! Intercepta las llamadas que herramientas como `pi` realizan a la CLI de herdr
//! (`herdr pane report-metadata ...`), construyendo la trama JSON esperada por
//! el socket de PORT y comunicándose mediante un stream Unix bloqueante con tope
//! de tiempo para evitar demoras en el anfitrión.

use std::collections::BTreeMap;
use std::fmt;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

/// Código de salida ante ejecución exitosa (el socket respondió con `result`).
pub const EXIT_SUCCESS: i32 = 0;
/// Código de salida ante falla en tiempo de ejecución (socket no disponible, error en respuesta, timeout).
pub const EXIT_RUNTIME_ERROR: i32 = 1;
/// Código de salida ante error de uso (subcomando desconocido, banderas inválidas o faltantes).
pub const EXIT_USAGE_ERROR: i32 = 2;

/// Tiempo límite de lectura y escritura para la conexión con el socket.
///
/// La especificación exige un tope cercano a 1500 ms para que el proceso llamador
/// no quede bloqueado indefinidamente si el socket no responde o se traba.
const SOCKET_TIMEOUT: Duration = Duration::from_millis(1500);

/// Argumentos analizados del comando `pane report-metadata`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportMetadataArgs {
    /// Identificador del panel destino.
    pub pane_id: String,
    /// Origen opcional de la metadata (ej. `gentle:activity`).
    pub source: Option<String>,
    /// Identificador opcional del agente (ej. `pi`).
    pub agent: Option<String>,
    /// Secuencia numérica monotónica opcional.
    pub seq: Option<u64>,
    /// Tiempo de vida opcional de los tokens en milisegundos.
    pub ttl_ms: Option<u64>,
    /// Mapa de tokens acumulados. Un valor `None` representa una clave a limpiar (`null`).
    pub tokens: BTreeMap<String, Option<String>>,
}

/// Errores clasificados de la CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliError {
    /// Error en la invocación o argumentos provistos.
    Usage(String),
    /// Error al interactuar con el socket de dominio Unix.
    Socket(String),
    /// Error en la respuesta devuelta por el servidor.
    Protocol(String),
}

impl CliError {
    /// Mapea el error al código de salida que la especificación exige.
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Usage(_) => EXIT_USAGE_ERROR,
            Self::Socket(_) | Self::Protocol(_) => EXIT_RUNTIME_ERROR,
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(msg) => write!(f, "error de uso: {msg}"),
            Self::Socket(msg) => write!(f, "error de socket: {msg}"),
            Self::Protocol(msg) => write!(f, "error de protocolo: {msg}"),
        }
    }
}

impl std::error::Error for CliError {}

/// Analiza los argumentos provistos al comando `pane report-metadata`.
///
/// La especificación exige exactamente dos argumentos de subcomando (`pane` y `report-metadata`),
/// seguidos del identificador posicional del panel y un conjunto de banderas opcionales en cualquier orden.
/// La ausencia de tokens para fijar o limpiar es considerada un error de uso (código 2).
pub fn parse_args<I, S>(args: I) -> Result<ReportMetadataArgs, CliError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args: Vec<String> = args.into_iter().map(|s| s.as_ref().to_string()).collect();

    if args.len() < 3 {
        return Err(CliError::Usage(
            "se requieren al menos los argumentos: pane report-metadata <pane>".to_string(),
        ));
    }

    if args[0] != "pane" || args[1] != "report-metadata" {
        return Err(CliError::Usage(
            "subcomando desconocido; se esperaba 'pane report-metadata'".to_string(),
        ));
    }

    let pane_id = &args[2];
    if pane_id.starts_with("--") || pane_id.is_empty() {
        return Err(CliError::Usage(
            "se requiere un identificador de panel posicional válido".to_string(),
        ));
    }

    let mut source: Option<String> = None;
    let mut agent: Option<String> = None;
    let mut seq: Option<u64> = None;
    let mut ttl_ms: Option<u64> = None;
    let mut tokens: BTreeMap<String, Option<String>> = BTreeMap::new();

    let mut i = 3;
    while i < args.len() {
        match args[i].as_str() {
            "--source" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliError::Usage(
                        "la bandera --source requiere un valor".to_string(),
                    ));
                }
                source = Some(args[i].clone());
            }
            "--agent" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliError::Usage(
                        "la bandera --agent requiere un valor".to_string(),
                    ));
                }
                agent = Some(args[i].clone());
            }
            "--seq" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliError::Usage(
                        "la bandera --seq requiere un número entero".to_string(),
                    ));
                }
                let val = args[i].parse::<u64>().map_err(|_| {
                    CliError::Usage(format!(
                        "el valor '{}' para --seq no es un entero u64 válido",
                        args[i]
                    ))
                })?;
                seq = Some(val);
            }
            "--ttl-ms" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliError::Usage(
                        "la bandera --ttl-ms requiere un número entero".to_string(),
                    ));
                }
                let val = args[i].parse::<u64>().map_err(|_| {
                    CliError::Usage(format!(
                        "el valor '{}' para --ttl-ms no es un entero u64 válido",
                        args[i]
                    ))
                })?;
                ttl_ms = Some(val);
            }
            "--token" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliError::Usage(
                        "la bandera --token requiere un par CLAVE=VALOR".to_string(),
                    ));
                }
                let raw = &args[i];
                let Some((key, val)) = raw.split_once('=') else {
                    return Err(CliError::Usage(format!(
                        "formato inválido para --token '{raw}'; se esperaba CLAVE=VALOR"
                    )));
                };
                if key.is_empty() {
                    return Err(CliError::Usage(
                        "la clave del token no puede estar vacía".to_string(),
                    ));
                }
                if val.is_empty() {
                    tokens.insert(key.to_string(), None);
                } else {
                    tokens.insert(key.to_string(), Some(val.to_string()));
                }
            }
            "--clear-token" => {
                i += 1;
                if i >= args.len() {
                    return Err(CliError::Usage(
                        "la bandera --clear-token requiere el nombre de una clave".to_string(),
                    ));
                }
                let key = &args[i];
                if key.is_empty() {
                    return Err(CliError::Usage(
                        "la clave a limpiar en --clear-token no puede estar vacía".to_string(),
                    ));
                }
                tokens.insert(key.to_string(), None);
            }
            other => {
                return Err(CliError::Usage(format!(
                    "argumento o bandera no reconocida: '{other}'"
                )));
            }
        }
        i += 1;
    }

    if tokens.is_empty() {
        return Err(CliError::Usage(
            "debe especificarse al menos un token a fijar o limpiar".to_string(),
        ));
    }

    Ok(ReportMetadataArgs {
        pane_id: pane_id.clone(),
        source,
        agent,
        seq,
        ttl_ms,
        tokens,
    })
}

/// Construye la trama JSON correspondiente a los argumentos analizados.
///
/// La serialización asegura el escape adecuado de caracteres especiales (comillas,
/// saltos de línea y bytes no ASCII) garantizando una única línea delimitada en cable.
pub fn build_frame(args: &ReportMetadataArgs) -> Result<String, CliError> {
    let mut params = serde_json::Map::new();
    params.insert(
        "pane_id".to_string(),
        serde_json::Value::String(args.pane_id.clone()),
    );

    if let Some(ref source) = args.source {
        params.insert(
            "source".to_string(),
            serde_json::Value::String(source.clone()),
        );
    }
    if let Some(ref agent) = args.agent {
        params.insert(
            "agent".to_string(),
            serde_json::Value::String(agent.clone()),
        );
    }
    if let Some(seq) = args.seq {
        params.insert("seq".to_string(), serde_json::Value::Number(seq.into()));
    }
    if let Some(ttl_ms) = args.ttl_ms {
        params.insert(
            "ttl_ms".to_string(),
            serde_json::Value::Number(ttl_ms.into()),
        );
    }

    let mut tokens_map = serde_json::Map::new();
    for (key, val) in &args.tokens {
        match val {
            Some(v) => tokens_map.insert(key.clone(), serde_json::Value::String(v.clone())),
            None => tokens_map.insert(key.clone(), serde_json::Value::Null),
        };
    }
    params.insert("tokens".to_string(), serde_json::Value::Object(tokens_map));

    let request = serde_json::json!({
        "id": "cli:request",
        "method": "pane.report_metadata",
        "params": serde_json::Value::Object(params),
    });

    serde_json::to_string(&request)
        .map_err(|err| CliError::Protocol(format!("error al serializar la trama JSON: {err}")))
}

/// Evalúa la respuesta recibida desde el socket Unix.
///
/// Siguiendo el contrato de herdr, una respuesta exitosa contiene el campo `result`,
/// mientras que una respuesta con fallo contiene el campo `error`.
pub fn evaluate_response(response: &str) -> Result<(), CliError> {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(response.trim()) {
        if value.get("error").is_some() {
            return Err(CliError::Protocol(format!(
                "respuesta de error del servidor: {response}"
            )));
        }
        if value.get("result").is_some() {
            return Ok(());
        }
    }

    // Comprobación de respaldo si la respuesta contiene texto sin estructura de objeto
    if response.contains("\"error\"") {
        return Err(CliError::Protocol(format!(
            "respuesta contiene error: {response}"
        )));
    }
    if response.contains("\"result\"") {
        return Ok(());
    }

    Err(CliError::Protocol(format!(
        "respuesta inesperada del socket: '{response}'"
    )))
}

/// Envía una trama al socket Unix indicado y procesa la respuesta.
///
/// Establece un tiempo límite de lectura y escritura de 1500 ms para asegurar
/// que el proceso cliente no quede colgado si el servidor no atiende la petición.
pub fn send_frame(socket_path: &Path, frame: &str) -> Result<(), CliError> {
    let mut stream = UnixStream::connect(socket_path).map_err(|err| {
        CliError::Socket(format!(
            "no se pudo conectar al socket en '{}': {err}",
            socket_path.display()
        ))
    })?;

    stream
        .set_write_timeout(Some(SOCKET_TIMEOUT))
        .map_err(|err| {
            CliError::Socket(format!(
                "no se pudo configurar el tiempo límite de escritura: {err}"
            ))
        })?;

    stream
        .set_read_timeout(Some(SOCKET_TIMEOUT))
        .map_err(|err| {
            CliError::Socket(format!(
                "no se pudo configurar el tiempo límite de lectura: {err}"
            ))
        })?;

    let wire_bytes = format!("{frame}\n");
    stream
        .write_all(wire_bytes.as_bytes())
        .map_err(|err| CliError::Socket(format!("error al enviar la petición al socket: {err}")))?;
    stream
        .flush()
        .map_err(|err| CliError::Socket(format!("error al volcar el búfer al socket: {err}")))?;

    let mut reader = BufReader::new(stream);
    let mut response_line = String::new();
    reader
        .read_line(&mut response_line)
        .map_err(|err| CliError::Socket(format!("error al leer la respuesta del socket: {err}")))?;

    if response_line.is_empty() {
        return Err(CliError::Socket(
            "el socket cerró la conexión sin responder".to_string(),
        ));
    }

    evaluate_response(&response_line)
}

/// Ejecuta el flujo principal del ejecutable y retorna el código de salida.
///
/// Gestiona la interacción con las variables de entorno, la llamada al socket y
/// la emisión de mensajes breves a stderr en caso de fallos.
pub fn run<I, S>(args: I) -> i32
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let parsed = match parse_args(args) {
        Ok(parsed) => parsed,
        Err(err) => {
            eprintln!("{err}");
            return err.exit_code();
        }
    };

    let socket_path_str = match std::env::var("HERDR_SOCKET_PATH") {
        Ok(path) if !path.trim().is_empty() => path,
        _ => {
            eprintln!(
                "error: la variable de entorno HERDR_SOCKET_PATH no está definida o está vacía"
            );
            return EXIT_RUNTIME_ERROR;
        }
    };

    let frame = match build_frame(&parsed) {
        Ok(frame) => frame,
        Err(err) => {
            eprintln!("{err}");
            return err.exit_code();
        }
    };

    if let Err(err) = send_frame(Path::new(&socket_path_str), &frame) {
        eprintln!("{err}");
        return err.exit_code();
    }

    EXIT_SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn full_set_of_flags() {
        let args = [
            "pane",
            "report-metadata",
            "w1:p1",
            "--source",
            "gentle:activity",
            "--agent",
            "pi",
            "--seq",
            "42",
            "--ttl-ms",
            "30000",
            "--token",
            "foo=bar",
            "--clear-token",
            "cleared",
        ];
        let parsed = parse_args(args).expect("debe analizar argumentos completos");
        assert_eq!(parsed.pane_id, "w1:p1");
        assert_eq!(parsed.source.as_deref(), Some("gentle:activity"));
        assert_eq!(parsed.agent.as_deref(), Some("pi"));
        assert_eq!(parsed.seq, Some(42));
        assert_eq!(parsed.ttl_ms, Some(30000));
        assert_eq!(parsed.tokens.get("foo"), Some(&Some("bar".to_string())));
        assert_eq!(parsed.tokens.get("cleared"), Some(&None));

        let frame = build_frame(&parsed).expect("debe construir la trama JSON");
        assert!(
            !frame.contains('\n'),
            "la trama debe ser una sola línea sin saltos"
        );
        let v: Value = serde_json::from_str(&frame).expect("trama JSON válida");
        assert_eq!(v["id"], "cli:request");
        assert_eq!(v["method"], "pane.report_metadata");
        assert_eq!(v["params"]["pane_id"], "w1:p1");
        assert_eq!(v["params"]["source"], "gentle:activity");
        assert_eq!(v["params"]["agent"], "pi");
        assert_eq!(v["params"]["seq"], 42);
        assert_eq!(v["params"]["ttl_ms"], 30000);
        assert_eq!(v["params"]["tokens"]["foo"], "bar");
        assert_eq!(v["params"]["tokens"]["cleared"], Value::Null);
    }

    #[test]
    fn clear_token_produces_null() {
        let args = [
            "pane",
            "report-metadata",
            "w1:p2",
            "--clear-token",
            "summary",
        ];
        let parsed = parse_args(args).expect("debe analizar --clear-token");
        assert_eq!(parsed.tokens.get("summary"), Some(&None));

        let frame = build_frame(&parsed).expect("debe construir la trama JSON");
        let v: Value = serde_json::from_str(&frame).expect("trama JSON válida");
        assert_eq!(v["params"]["tokens"]["summary"], Value::Null);
        // Las opciones no especificadas no deben incluirse en params.
        assert!(v["params"].get("source").is_none());
        assert!(v["params"].get("agent").is_none());
        assert!(v["params"].get("seq").is_none());
        assert!(v["params"].get("ttl_ms").is_none());
    }

    #[test]
    fn token_empty_value_produces_null() {
        let args = ["pane", "report-metadata", "w1:p3", "--token", "summary="];
        let parsed = parse_args(args).expect("debe analizar token con valor vacío");
        assert_eq!(parsed.tokens.get("summary"), Some(&None));

        let frame = build_frame(&parsed).expect("debe construir la trama JSON");
        let v: Value = serde_json::from_str(&frame).expect("trama JSON válida");
        assert_eq!(v["params"]["tokens"]["summary"], Value::Null);
    }

    #[test]
    fn later_flag_overrides_earlier_for_same_key() {
        // Subcaso 1: primero set, luego clear -> null
        let args1 = [
            "pane",
            "report-metadata",
            "p1",
            "--token",
            "key=initial",
            "--clear-token",
            "key",
        ];
        let p1 = parse_args(args1).expect("análisis válido");
        let f1 = build_frame(&p1).expect("construcción válida");
        let v1: Value = serde_json::from_str(&f1).unwrap();
        assert_eq!(v1["params"]["tokens"]["key"], Value::Null);

        // Subcaso 2: primero clear, luego set -> nuevo valor
        let args2 = [
            "pane",
            "report-metadata",
            "p1",
            "--clear-token",
            "key",
            "--token",
            "key=updated",
        ];
        let p2 = parse_args(args2).expect("análisis válido");
        let f2 = build_frame(&p2).expect("construcción válida");
        let v2: Value = serde_json::from_str(&f2).unwrap();
        assert_eq!(v2["params"]["tokens"]["key"], "updated");

        // Subcaso 3: primero set, luego token K= -> null
        let args3 = [
            "pane",
            "report-metadata",
            "p1",
            "--token",
            "key=initial",
            "--token",
            "key=",
        ];
        let p3 = parse_args(args3).expect("análisis válido");
        let f3 = build_frame(&p3).expect("construcción válida");
        let v3: Value = serde_json::from_str(&f3).unwrap();
        assert_eq!(v3["params"]["tokens"]["key"], Value::Null);

        // Subcaso 4: primero token K=, luego set -> nuevo valor
        let args4 = [
            "pane",
            "report-metadata",
            "p1",
            "--token",
            "key=",
            "--token",
            "key=final",
        ];
        let p4 = parse_args(args4).expect("análisis válido");
        let f4 = build_frame(&p4).expect("construcción válida");
        let v4: Value = serde_json::from_str(&f4).unwrap();
        assert_eq!(v4["params"]["tokens"]["key"], "final");
    }

    #[test]
    fn escaping_quotes_newlines_and_non_ascii() {
        let complex_value = "hello \"world\"\nñandú € 🚀 \t \\path";
        let token_arg = format!("msg={complex_value}");
        let args = ["pane", "report-metadata", "p1", "--token", &token_arg];
        let parsed = parse_args(args).expect("análisis válido");
        let frame = build_frame(&parsed).expect("construcción válida");

        assert!(
            !frame.contains('\n'),
            "el frame serializado en cable debe ser una sola línea sin saltos crudos"
        );
        let v: Value = serde_json::from_str(&frame).expect("debe ser JSON válido deserializable");
        assert_eq!(v["params"]["tokens"]["msg"], complex_value);
    }

    #[test]
    fn usage_errors_return_exit_code_2() {
        let bad_cases: &[&[&str]] = &[
            &[],
            &["pane"],
            &["other", "report-metadata", "p1", "--token", "a=b"],
            &["pane", "other", "p1", "--token", "a=b"],
            &["pane", "report-metadata"],
            &["pane", "report-metadata", "--token", "a=b"],
            &["pane", "report-metadata", "p1"],
            &[
                "pane",
                "report-metadata",
                "p1",
                "--unknown-flag",
                "val",
                "--token",
                "a=b",
            ],
            &["pane", "report-metadata", "p1", "--source"],
            &["pane", "report-metadata", "p1", "--token"],
            &["pane", "report-metadata", "p1", "--clear-token"],
            &["pane", "report-metadata", "p1", "--token", "no_equal_sign"],
            &["pane", "report-metadata", "p1", "--token", "=no_key"],
            &["pane", "report-metadata", "p1", "--clear-token", ""],
            &[
                "pane",
                "report-metadata",
                "p1",
                "--seq",
                "not_a_number",
                "--token",
                "a=b",
            ],
            &[
                "pane",
                "report-metadata",
                "p1",
                "--ttl-ms",
                "xyz",
                "--token",
                "a=b",
            ],
            &[
                "pane",
                "report-metadata",
                "p1",
                "--token",
                "a=b",
                "unexpected_arg",
            ],
        ];

        for case in bad_cases {
            let res = parse_args(*case);
            assert!(
                res.is_err(),
                "caso debe fallar en análisis sintáctico: {case:?}"
            );
            assert_eq!(
                res.unwrap_err().exit_code(),
                EXIT_USAGE_ERROR,
                "caso debe retornar código de error de uso (2): {case:?}"
            );
        }
    }

    #[test]
    fn response_evaluation() {
        // Éxito: contiene campo "result"
        assert!(evaluate_response(r#"{"id":"cli:request","result":{"type":"ok"}}"#).is_ok());
        // Error: contiene campo "error"
        let err_res = evaluate_response(
            r#"{"id":"cli:request","error":{"code":"invalid_request","message":"bad"}}"#,
        );
        assert!(err_res.is_err());
        assert_eq!(err_res.unwrap_err().exit_code(), EXIT_RUNTIME_ERROR);

        // Respuesta no reconocida / vacía
        let empty_res = evaluate_response("");
        assert!(empty_res.is_err());
        assert_eq!(empty_res.unwrap_err().exit_code(), EXIT_RUNTIME_ERROR);
    }

    #[test]
    fn exact_frame_matches_herdr_interception_contract() {
        let args = [
            "pane",
            "report-metadata",
            "p1",
            "--source",
            "port:state",
            "--seq",
            "1",
            "--ttl-ms",
            "30000",
            "--token",
            "port_state=blocked",
        ];
        let parsed = parse_args(args).expect("análisis válido");
        let frame = build_frame(&parsed).expect("construcción válida");
        let v: Value = serde_json::from_str(&frame).expect("debe ser JSON válido");
        assert_eq!(v["id"], "cli:request");
        assert_eq!(v["method"], "pane.report_metadata");
        assert_eq!(v["params"]["pane_id"], "p1");
        assert_eq!(v["params"]["source"], "port:state");
        assert_eq!(v["params"]["seq"], 1);
        assert_eq!(v["params"]["ttl_ms"], 30000);
        assert_eq!(v["params"]["tokens"]["port_state"], "blocked");
        assert!(v["params"].get("agent").is_none());
    }

    #[test]
    fn socket_communication_with_mock_server() {
        use std::os::unix::net::UnixListener;

        let socket_dir = std::env::temp_dir();
        let socket_path = socket_dir.join(format!(
            "port-herdr-test-{}-{:?}.sock",
            std::process::id(),
            std::thread::current().id()
        ));

        let _ = std::fs::remove_file(&socket_path);
        let listener = UnixListener::bind(&socket_path).expect("debe bindear socket temporal");

        let server_path = socket_path.clone();
        let server_handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("debe aceptar conexión");
            let mut reader = BufReader::new(&stream);
            let mut line = String::new();
            reader.read_line(&mut line).expect("debe leer línea");
            assert!(line.ends_with('\n'));
            let req: Value = serde_json::from_str(line.trim()).expect("debe ser JSON");
            assert_eq!(req["method"], "pane.report_metadata");

            // Responde éxito con \n
            stream
                .write_all(b"{\"id\":\"cli:request\",\"result\":{\"type\":\"ok\"}}\n")
                .expect("debe escribir respuesta");
            stream.flush().expect("debe volcar respuesta");
        });

        let res = send_frame(
            &socket_path,
            r#"{"id":"cli:request","method":"pane.report_metadata","params":{"pane_id":"p1","tokens":{"a":"b"}}}"#,
        );
        assert!(res.is_ok(), "la llamada send_frame debe tener éxito");

        server_handle
            .join()
            .expect("el hilo servidor debe finalizar");
        let _ = std::fs::remove_file(&server_path);
    }
}
