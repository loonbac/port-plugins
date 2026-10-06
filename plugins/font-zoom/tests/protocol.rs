//! Pruebas de contrato y de saludo del plugin de zoom de fuente.
//!
//! El plugin es un proceso aparte: aquí se arranca el binario real, se le envía
//! el `hello` del protocolo por stdin y se comprueba la respuesta `ready` por
//! stdout. Además se contrasta lo declarado con `[package.metadata.port]`, que
//! es lo que PORT lee al instalar.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use port_plugin_sdk::protocol::{Binding, Capability, HostRequest, PluginReply, PROTOCOL_VERSION};

/// Lo que PORT usa al instalar: identificador, binario y capacidades.
struct Manifest {
    id: String,
    bin: String,
    capabilities: Vec<String>,
}

/// Lee `[package.metadata.port]` y `[[bin]]` del `Cargo.toml` del plugin.
fn manifest() -> Manifest {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let text = std::fs::read_to_string(&path).expect("el Cargo.toml debe leerse");

    let mut section = String::new();
    let mut id = None;
    let mut bin = None;
    let mut capabilities = Vec::new();

    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('[') {
            section = line.to_string();
            continue;
        }
        match section.as_str() {
            "[package.metadata.port]" => {
                if let Some(value) = line.strip_prefix("id =") {
                    id = Some(value.trim().trim_matches('"').to_string());
                } else if let Some(value) = line.strip_prefix("capabilities =") {
                    capabilities = parse_list(value);
                }
            }
            "[[bin]]" => {
                if let Some(value) = line.strip_prefix("name =") {
                    bin = Some(value.trim().trim_matches('"').to_string());
                }
            }
            _ => {}
        }
    }

    Manifest {
        id: id.expect("el manifiesto debe declarar id"),
        bin: bin.expect("el manifiesto debe declarar un binario"),
        capabilities,
    }
}

/// Convierte `["appearance"]` en la lista de cadenas.
fn parse_list(raw: &str) -> Vec<String> {
    raw.trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|item| item.trim().trim_matches('"').to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

/// Nombres de las capacidades tal como viajan en el saludo.
fn capability_names(capabilities: &[Capability]) -> Vec<String> {
    capabilities
        .iter()
        .map(|capability| {
            serde_json::to_value(capability)
                .expect("la capacidad debe serializarse")
                .as_str()
                .expect("la capacidad debe ser una cadena")
                .to_string()
        })
        .collect()
}

/// Descripción de una combinación tal como viaja en el saludo.
fn describe(binding: &Binding) -> (String, bool, bool, bool, String) {
    (
        binding.key.to_lowercase(),
        binding.ctrl,
        binding.alt,
        binding.shift,
        binding.action.clone(),
    )
}

/// Arranca el binario, lo saluda y devuelve la respuesta `ready`.
fn handshake() -> PluginReply {
    let mut child = Command::new(env!("CARGO_BIN_EXE_font-zoom"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("el binario debe arrancar");

    let mut stdin = child.stdin.take().expect("stdin disponible");
    let stdout = child.stdout.take().expect("stdout disponible");
    let mut lines = BufReader::new(stdout).lines();

    let hello = serde_json::to_string(&HostRequest::Hello {
        api: PROTOCOL_VERSION,
    })
    .expect("el saludo debe serializarse");
    writeln!(stdin, "{hello}").expect("enviar el saludo");
    stdin.flush().expect("vaciar stdin");

    let line = lines
        .next()
        .expect("el plugin debe contestar al saludo")
        .expect("la línea debe ser utf-8");
    let reply: PluginReply = serde_json::from_str(&line).expect("la respuesta debe ser JSON");

    let shutdown = serde_json::to_string(&HostRequest::Shutdown).expect("shutdown serializable");
    writeln!(stdin, "{shutdown}").expect("enviar el cierre");
    let _ = stdin.flush();
    let _ = child.wait();

    reply
}

#[test]
fn el_manifiesto_coincide_con_el_saludo() {
    let manifest = manifest();
    assert_eq!(manifest.id, "font-zoom");
    assert_eq!(
        manifest.bin, manifest.id,
        "el binario instalable debe llevar el id del manifiesto"
    );

    match handshake() {
        PluginReply::Ready { capabilities, .. } => {
            assert_eq!(
                capability_names(&capabilities),
                manifest.capabilities,
                "las capacidades del saludo deben ser las del manifiesto"
            );
        }
        other => panic!("mensaje equivocado: {other:?}"),
    }
}

#[test]
fn el_saludo_declara_la_apariencia() {
    match handshake() {
        PluginReply::Ready {
            name,
            version,
            appearance,
            ..
        } => {
            assert_eq!(name, "Font Zoom");
            assert_eq!(version, env!("CARGO_PKG_VERSION"));
            assert_eq!(appearance.font_size, Some(14.0));
        }
        other => panic!("mensaje equivocado: {other:?}"),
    }
}

#[test]
fn el_saludo_declara_los_atajos_de_zoom() {
    match handshake() {
        PluginReply::Ready { bindings, .. } => {
            let described: Vec<_> = bindings.iter().map(describe).collect();
            assert_eq!(
                described,
                vec![
                    ("=".to_string(), true, false, false, "zoom_in".to_string()),
                    ("-".to_string(), true, false, false, "zoom_out".to_string()),
                    ("0".to_string(), true, false, false, "reset".to_string()),
                ],
                "el plugin resuelve sus propios atajos: ctrl+=, ctrl+- y ctrl+0"
            );
        }
        other => panic!("mensaje equivocado: {other:?}"),
    }
}
