//! Protocolo de informes de panel: la superficie de herdr que PORT intercepta.
//!
//! Única responsabilidad: entender la petición que un panel manda por el socket
//! y validarla con los mismos límites que herdr. No abre sockets ni conoce
//! hilos, así que es puro y se prueba sin infraestructura.
//!
//! Lo que **no** se valida acá es lo que necesita estado: la existencia del
//! panel, el orden de `seq` por par (panel, fuente) y el tope de tokens por
//! panel. Eso lo resuelve quien tiene el estado.

use std::collections::BTreeMap;

use port_plugin_api::AgentState;
use serde_json::{json, Value};

/// Método de herdr que PORT intercepta.
pub(crate) const METHOD_REPORT_METADATA: &str = "pane.report_metadata";
/// Token reservado que lleva el estado del agente.
///
/// Se usa un token en vez de un método propio para que el contrato interceptado
/// siga siendo el de herdr.
pub(crate) const STATE_TOKEN: &str = "port_state";

/// Límites copiados del contrato de herdr (C11 del documento de tarea).
pub(crate) const MAX_SOURCE_CHARS: usize = 80;
pub(crate) const MAX_TOKEN_KEYS: usize = 16;
pub(crate) const MAX_TOKEN_KEY_CHARS: usize = 32;
pub(crate) const MAX_TOKEN_VALUE_CHARS: usize = 80;
pub(crate) const MIN_TTL_MS: u64 = 1;
pub(crate) const MAX_TTL_MS: u64 = 86_400_000;

/// Petición ya validada y normalizada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReportRequest {
    /// Identificador que el cliente eligió; se repite en la respuesta.
    pub id: String,
    /// Panel que reporta, tal cual lo mandó el cliente.
    pub pane_id: String,
    /// Fuente que reporta; acota el orden de `seq`.
    pub source: String,
    /// Tokens a fijar. Un valor vacío significa borrar la clave.
    pub tokens: BTreeMap<String, String>,
    /// Orden por par (panel, fuente); se acepta sólo si crece.
    pub seq: Option<u64>,
    /// Vida de los tokens de esta petición, en milisegundos.
    pub ttl_ms: Option<u64>,
}

impl ReportRequest {
    /// Estado del agente declarado en los tokens, si lo hay.
    pub(crate) fn agent_state(&self) -> Option<AgentState> {
        self.tokens
            .get(STATE_TOKEN)
            .and_then(|value| parse_agent_state(value))
    }
}

/// Error de protocolo, con el código y el mensaje que espera el contrato.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReportError {
    /// La línea no es una petición entendible.
    InvalidRequest(String),
    /// El panel no existe.
    PaneNotFound(String),
    /// El campo `agent` vino vacío.
    InvalidAgent,
    /// `source` fuera de los límites o con caracteres no permitidos.
    InvalidSource,
    /// Algún token incumple sus límites.
    InvalidToken,
    /// `ttl_ms` fuera de rango.
    InvalidTtl,
    /// El panel ya tiene más tokens de los permitidos.
    TokenLimit,
}

impl ReportError {
    /// Código estable del contrato.
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::InvalidRequest(_) => "invalid_request",
            Self::PaneNotFound(_) => "pane_not_found",
            Self::InvalidAgent => "invalid_agent",
            Self::InvalidSource => "invalid_metadata_source",
            Self::InvalidToken => "invalid_metadata_token",
            Self::InvalidTtl => "invalid_metadata_ttl",
            Self::TokenLimit => "metadata_token_limit",
        }
    }

    /// Mensaje legible; el de una petición inválida viaja en el propio error.
    pub(crate) fn message(&self) -> String {
        match self {
            Self::InvalidRequest(detail) => format!("invalid request: {detail}"),
            Self::PaneNotFound(pane) => format!("pane {pane} not found"),
            Self::InvalidAgent => "agent must not be empty".to_string(),
            Self::InvalidSource => {
                format!("source must be 1-{MAX_SOURCE_CHARS} of [A-Za-z0-9:._-]")
            }
            Self::InvalidToken => format!(
                "token keys must be 1-{MAX_TOKEN_KEY_CHARS} of [A-Za-z0-9_-], at most {MAX_TOKEN_KEYS} per request"
            ),
            Self::InvalidTtl => format!("ttl_ms must be {MIN_TTL_MS}-{MAX_TTL_MS}"),
            Self::TokenLimit => "metadata token limit reached".to_string(),
        }
    }
}

/// Estado del agente a partir del token reservado.
///
/// Se acepta cualquier caja: quien reporta escribe el estado a mano.
pub(crate) fn parse_agent_state(value: &str) -> Option<AgentState> {
    match value.trim().to_ascii_lowercase().as_str() {
        "working" | "busy" => Some(AgentState::Working),
        "blocked" | "waiting" | "attention" => Some(AgentState::Blocked),
        "idle" | "done" | "ready" => Some(AgentState::Idle),
        "unknown" => Some(AgentState::Unknown),
        _ => None,
    }
}

/// `true` si el carácter está permitido en `source`.
fn source_char_ok(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, ':' | '.' | '_' | '-')
}

/// `true` si el carácter está permitido en una clave de token.
fn token_key_char_ok(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-')
}

/// Normaliza el valor de un token: sin controles, recortado y acotado.
///
/// Un resultado vacío es un borrado, igual que en herdr.
fn normalize_token_value(raw: &str) -> String {
    raw.chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .chars()
        .take(MAX_TOKEN_VALUE_CHARS)
        .collect()
}

/// Interpreta y valida una línea del socket.
pub(crate) fn parse_request(line: &str) -> Result<ReportRequest, ReportError> {
    let value: Value = serde_json::from_str(line.trim())
        .map_err(|error| ReportError::InvalidRequest(error.to_string()))?;

    let id = value
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    let method = value
        .get("method")
        .and_then(Value::as_str)
        .ok_or_else(|| ReportError::InvalidRequest("missing method".to_string()))?;
    if method != METHOD_REPORT_METADATA {
        return Err(ReportError::InvalidRequest(format!(
            "unsupported method {method}"
        )));
    }

    let params = value
        .get("params")
        .ok_or_else(|| ReportError::InvalidRequest("missing params".to_string()))?;

    let pane_id = params
        .get("pane_id")
        .and_then(Value::as_str)
        .ok_or_else(|| ReportError::InvalidRequest("missing pane_id".to_string()))?
        .to_string();

    // `agent` es opcional, pero si viene no puede quedar vacío.
    if let Some(agent) = params.get("agent").and_then(Value::as_str) {
        if agent.trim().is_empty() {
            return Err(ReportError::InvalidAgent);
        }
    }

    let source = params
        .get("source")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let source_len = source.chars().count();
    if source_len == 0 || source_len > MAX_SOURCE_CHARS || !source.chars().all(source_char_ok) {
        return Err(ReportError::InvalidSource);
    }

    let mut tokens = BTreeMap::new();
    if let Some(map) = params.get("tokens").and_then(Value::as_object) {
        if map.len() > MAX_TOKEN_KEYS {
            return Err(ReportError::InvalidToken);
        }
        for (key, raw) in map {
            let key_len = key.chars().count();
            if key_len == 0 || key_len > MAX_TOKEN_KEY_CHARS || !key.chars().all(token_key_char_ok)
            {
                return Err(ReportError::InvalidToken);
            }
            // `null` y la cadena vacía son la misma cosa: borrar la clave.
            let normalized = match raw {
                Value::Null => String::new(),
                Value::String(text) => normalize_token_value(text),
                // Un valor que no sea texto ni nulo no es un token entendible.
                _ => return Err(ReportError::InvalidToken),
            };
            tokens.insert(key.clone(), normalized);
        }
    }

    let ttl_ms = match params.get("ttl_ms") {
        None | Some(Value::Null) => None,
        Some(value) => {
            let ttl = value.as_u64().ok_or(ReportError::InvalidTtl)?;
            if !(MIN_TTL_MS..=MAX_TTL_MS).contains(&ttl) {
                return Err(ReportError::InvalidTtl);
            }
            Some(ttl)
        }
    };

    let seq = match params.get("seq") {
        None | Some(Value::Null) => None,
        Some(value) => Some(value.as_u64().ok_or_else(|| {
            ReportError::InvalidRequest("seq must be a non-negative integer".to_string())
        })?),
    };

    Ok(ReportRequest {
        id,
        pane_id,
        source,
        tokens,
        seq,
        ttl_ms,
    })
}

/// Respuesta de éxito, tal como la espera el contrato.
pub(crate) fn ok_response(id: &str) -> String {
    json!({ "id": id, "result": { "type": "ok" } }).to_string()
}

/// Respuesta de error, con el código y el mensaje del contrato.
pub(crate) fn error_response(id: &str, error: &ReportError) -> String {
    json!({
        "id": id,
        "error": { "code": error.code(), "message": error.message() },
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(params: Value) -> String {
        json!({ "id": "cli:request", "method": METHOD_REPORT_METADATA, "params": params })
            .to_string()
    }

    /// Una petición completa y válida se interpreta entera.
    #[test]
    fn peticion_valida_se_interpreta() {
        let raw = line(json!({
            "pane_id": "w1:p2",
            "agent": "pi",
            "source": "gentle:activity",
            "tokens": { "summary": "  leyendo el mapa  ", "port_state": "blocked" },
            "seq": 7,
            "ttl_ms": 30_000,
        }));
        let request = parse_request(&raw).expect("petición válida");

        assert_eq!(request.id, "cli:request");
        assert_eq!(request.pane_id, "w1:p2");
        assert_eq!(request.source, "gentle:activity");
        assert_eq!(request.seq, Some(7));
        assert_eq!(request.ttl_ms, Some(30_000));
        assert_eq!(
            request.tokens.get("summary").map(String::as_str),
            Some("leyendo el mapa")
        );
        assert_eq!(request.agent_state(), Some(AgentState::Blocked));
    }

    /// Un token nulo o vacío significa borrar la clave; no se pierde el dato.
    #[test]
    fn token_vacio_es_borrado() {
        let raw = line(json!({
            "pane_id": "w1:p2",
            "source": "gentle:activity",
            "tokens": { "summary": null, "otro": "   " },
        }));
        let request = parse_request(&raw).expect("petición válida");
        assert_eq!(request.tokens.get("summary").map(String::as_str), Some(""));
        assert_eq!(request.tokens.get("otro").map(String::as_str), Some(""));
        assert_eq!(request.agent_state(), None);
    }

    /// Los límites de `source` se aplican: vacío, demasiado largo y caracteres
    /// fuera del alfabeto permitido.
    #[test]
    fn source_fuera_de_limites_falla() {
        for bad in ["", "   ", "con espacio", "acentué"] {
            let raw = line(json!({ "pane_id": "w1:p2", "source": bad }));
            assert_eq!(parse_request(&raw), Err(ReportError::InvalidSource));
        }
        let largo = "a".repeat(MAX_SOURCE_CHARS + 1);
        let raw = line(json!({ "pane_id": "w1:p2", "source": largo }));
        assert_eq!(parse_request(&raw), Err(ReportError::InvalidSource));
        // Justo en el límite sí entra.
        let justo = "a".repeat(MAX_SOURCE_CHARS);
        let raw = line(json!({ "pane_id": "w1:p2", "source": justo }));
        assert!(parse_request(&raw).is_ok());
    }

    /// La clave del token tiene su propio alfabeto y su propio tope.
    #[test]
    fn clave_de_token_invalida_falla() {
        for bad in ["con espacio", "con:dos", ""] {
            let raw = line(json!({
                "pane_id": "w1:p2",
                "source": "s",
                "tokens": { bad: "valor" },
            }));
            assert_eq!(parse_request(&raw), Err(ReportError::InvalidToken));
        }
        let larga = "k".repeat(MAX_TOKEN_KEY_CHARS + 1);
        let raw = line(json!({
            "pane_id": "w1:p2",
            "source": "s",
            "tokens": { larga: "valor" },
        }));
        assert_eq!(parse_request(&raw), Err(ReportError::InvalidToken));
    }

    /// Más claves de las permitidas no entra.
    #[test]
    fn demasiadas_claves_fallan() {
        let mut tokens = serde_json::Map::new();
        for index in 0..=MAX_TOKEN_KEYS {
            tokens.insert(format!("k{index}"), Value::String("v".to_string()));
        }
        let raw = line(json!({ "pane_id": "w1:p2", "source": "s", "tokens": tokens }));
        assert_eq!(parse_request(&raw), Err(ReportError::InvalidToken));
    }

    /// El valor se recorta a 80 caracteres y pierde los controles.
    #[test]
    fn valor_de_token_se_normaliza() {
        let raw = line(json!({
            "pane_id": "w1:p2",
            "source": "s",
            "tokens": { "summary": "a\u{7}b" },
        }));
        let request = parse_request(&raw).expect("petición válida");
        assert_eq!(
            request.tokens.get("summary").map(String::as_str),
            Some("ab")
        );

        let larguisimo = "x".repeat(200);
        let raw = line(json!({
            "pane_id": "w1:p2",
            "source": "s",
            "tokens": { "summary": larguisimo },
        }));
        let request = parse_request(&raw).expect("petición válida");
        assert_eq!(
            request
                .tokens
                .get("summary")
                .map(|value| value.chars().count()),
            Some(MAX_TOKEN_VALUE_CHARS)
        );
    }

    /// `ttl_ms` fuera de rango se rechaza; ausente se acepta.
    #[test]
    fn ttl_fuera_de_rango_falla() {
        for bad in [0_u64, MAX_TTL_MS + 1] {
            let raw = line(json!({ "pane_id": "w1:p2", "source": "s", "ttl_ms": bad }));
            assert_eq!(parse_request(&raw), Err(ReportError::InvalidTtl));
        }
        let raw = line(json!({ "pane_id": "w1:p2", "source": "s" }));
        assert_eq!(parse_request(&raw).expect("válida").ttl_ms, None);
    }

    /// Un método distinto o un JSON roto no son peticiones entendibles.
    #[test]
    fn peticion_ilegible_falla() {
        assert!(matches!(
            parse_request("no es json"),
            Err(ReportError::InvalidRequest(_))
        ));
        let raw = json!({ "id": "x", "method": "pane.close", "params": {} }).to_string();
        assert!(matches!(
            parse_request(&raw),
            Err(ReportError::InvalidRequest(_))
        ));
        // Campos desconocidos se ignoran, como en herdr.
        let raw = line(json!({ "pane_id": "w1:p2", "source": "s", "futuro": 1 }));
        assert!(parse_request(&raw).is_ok());
    }

    /// Las respuestas llevan el identificador y el código del contrato.
    #[test]
    fn respuestas_respetan_el_contrato() {
        let ok: Value = serde_json::from_str(&ok_response("cli:request")).expect("json");
        assert_eq!(ok["id"], "cli:request");
        assert_eq!(ok["result"]["type"], "ok");

        let error: Value =
            serde_json::from_str(&error_response("cli:request", &ReportError::InvalidSource))
                .expect("json");
        assert_eq!(error["error"]["code"], "invalid_metadata_source");
        assert!(error["error"]["message"].as_str().is_some());

        let pane: Value = serde_json::from_str(&error_response(
            "cli:request",
            &ReportError::PaneNotFound("w9:p9".to_string()),
        ))
        .expect("json");
        assert_eq!(pane["error"]["code"], "pane_not_found");
        assert_eq!(pane["error"]["message"], "pane w9:p9 not found");
    }

    /// El token reservado entiende alias y cualquier caja.
    #[test]
    fn estado_del_agente_se_interpreta() {
        assert_eq!(parse_agent_state(" BLOCKED "), Some(AgentState::Blocked));
        assert_eq!(parse_agent_state("waiting"), Some(AgentState::Blocked));
        assert_eq!(parse_agent_state("Working"), Some(AgentState::Working));
        assert_eq!(parse_agent_state("idle"), Some(AgentState::Idle));
        assert_eq!(parse_agent_state("listo"), None);
    }
}
