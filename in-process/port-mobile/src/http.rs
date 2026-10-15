//! Capa HTTP mínima: lo justo para servir la PWA y aceptar el WebSocket.
//!
//! No es un servidor de propósito general y no pretende serlo. Interpreta una
//! petición, decide qué hacer con ella y arma la respuesta; no abre ningún
//! socket, así que se prueba entera sin red.
//!
//! La seguridad de esta superficie vive acá, y en dos piezas concretas:
//!
//! - el **token**, que guarda el socket por donde entra la escritura, y que se
//!   compara sin filtrar por tiempo cuánto coincide;
//! - la **resolución de archivos**, que no decodifica nada y sólo acepta
//!   segmentos de un alfabeto conocido, de modo que una ruta codificada no puede
//!   disfrazarse de otra y salirse de la raíz.

use std::path::{Path, PathBuf};

/// Método de la petición.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// Lectura.
    Get,
    /// Envío, usado para la entrada del cliente.
    Post,
    /// Cualquier otro: no se atiende.
    Other,
}

/// Una petición ya interpretada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// Método pedido.
    pub method: Method,
    /// Ruta sin consulta, tal como llegó.
    pub path: String,
    /// Pares de la consulta, en orden.
    pub query: Vec<(String, String)>,
    /// Cabeceras, con el nombre tal como llegó.
    pub headers: Vec<(String, String)>,
}

impl Request {
    /// Valor de una cabecera, sin distinguir mayúsculas en el nombre.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// Primer valor de la consulta con ese nombre.
    pub fn param(&self, name: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    /// `true` si la petición pide cambiar de protocolo a WebSocket.
    pub fn wants_upgrade(&self) -> bool {
        self.header("upgrade")
            .is_some_and(|value| value.eq_ignore_ascii_case("websocket"))
    }
}

/// Tope de una petición: más que esto no es una petición.
pub const MAX_REQUEST_BYTES: usize = 16 * 1024;

/// Interpreta la cabecera de una petición, hasta la línea en blanco.
///
/// Devuelve `None` si no se entiende: una petición ilegible no se responde con
/// un error del protocolo, simplemente no se atiende.
pub fn parse_request(raw: &[u8]) -> Option<Request> {
    if raw.len() > MAX_REQUEST_BYTES {
        return None;
    }
    let text = std::str::from_utf8(raw).ok()?;
    let mut lines = text.split("\r\n");
    let mut request_line = lines.next()?.split(' ');
    let method = match request_line.next()? {
        "GET" => Method::Get,
        "POST" => Method::Post,
        _ => Method::Other,
    };
    let target = request_line.next()?;
    let (path, query) = match target.split_once('?') {
        Some((path, query)) => (path, parse_query(query)),
        None => (target, Vec::new()),
    };
    if !path.starts_with('/') {
        return None;
    }

    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        let (name, value) = line.split_once(':')?;
        headers.push((name.trim().to_string(), value.trim().to_string()));
    }

    Some(Request {
        method,
        path: path.to_string(),
        query,
        headers,
    })
}

/// Pares de una consulta, sin decodificar.
///
/// No se decodifica a propósito: el token viaja tal cual lo eligió su dueño y no
/// hay ningún valor que necesite escapes.
fn parse_query(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((key, value)) => (key.to_string(), value.to_string()),
            None => (pair.to_string(), String::new()),
        })
        .collect()
}

/// Qué hacer con una petición.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// Cambiar de protocolo: el cliente entra a la terminal.
    Upgrade,
    /// Servir este archivo de la PWA, con su ruta relativa.
    Asset(String),
    /// El token no acompaña ni coincide.
    Unauthorized,
    /// No hay nada que servir ahí.
    NotFound,
}

/// Decide qué hacer con una petición.
///
/// Sólo el socket exige token. La cáscara de la PWA no lleva ningún secreto y el
/// navegador pide su manifiesto y sus iconos por su cuenta; si se les exigiera,
/// no podría instalarla. El control de afuera es la red de Tailscale, y el token
/// guarda lo único que sí importa: por dónde entra la escritura.
pub fn route(request: &Request, token: &str) -> Route {
    if request.path == "/ws" {
        let provided = request.param("token").map(str::to_string).or_else(|| {
            request
                .header("authorization")
                .and_then(|value| value.strip_prefix("Bearer "))
                .map(str::to_string)
        });
        return match provided {
            Some(provided) if token_matches(token, &provided) => Route::Upgrade,
            _ => Route::Unauthorized,
        };
    }
    if request.method != Method::Get {
        return Route::NotFound;
    }
    let asset = request.path.trim_start_matches('/');
    let asset = if asset.is_empty() {
        "index.html"
    } else {
        asset
    };
    Route::Asset(asset.to_string())
}

/// Compara dos tokens sin filtrar por tiempo cuánto coinciden.
///
/// Un `==` sobre cadenas corta en la primera diferencia, y ese tiempo se puede
/// medir desde afuera para adivinar el token carácter a carácter. Acá se recorre
/// entero siempre.
pub fn token_matches(expected: &str, provided: &str) -> bool {
    let expected = expected.as_bytes();
    let provided = provided.as_bytes();
    if expected.is_empty() {
        return false;
    }
    // La longitud no es secreta: se descarta antes y el recorrido queda igual.
    if expected.len() != provided.len() {
        return false;
    }
    let mut difference: u8 = 0;
    for (a, b) in expected.iter().zip(provided.iter()) {
        difference |= a ^ b;
    }
    difference == 0
}

/// Resuelve un archivo servible dentro de la raíz, o nada.
///
/// El alfabeto es deliberadamente estrecho y **no se decodifica nada**: una ruta
/// con `%2e%2e` no significa `..` acá, significa basura, y se rechaza. Esa es la
/// razón de no aceptar escapes: el guardia no puede olvidarse de un paso de
/// decodificación si no hay ninguno.
pub fn resolve_asset(root: &Path, path: &str) -> Option<PathBuf> {
    if path.is_empty() {
        return None;
    }
    let mut resolved = root.to_path_buf();
    for segment in path.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return None;
        }
        let allowed = segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
        if !allowed {
            return None;
        }
        resolved.push(segment);
    }
    Some(resolved)
}

/// Respuesta HTTP con su cuerpo.
pub fn response(status: u16, content_type: &str, body: &[u8]) -> Vec<u8> {
    let reason = match status {
        200 => "OK",
        401 => "Unauthorized",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Unknown",
    };
    let mut out = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\n\r\n",
        body.len()
    )
    .into_bytes();
    out.extend_from_slice(body);
    out
}

/// Tipo de contenido probable de un archivo de la PWA.
pub fn content_type_for(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        // El manifiesto de una PWA se sirve como JSON y suele no llevar extensión
        // reconocible; sin esto el navegador no la instala.
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peticion(linea: &str) -> Option<Request> {
        parse_request(format!("{linea}\r\nHost: x\r\n\r\n").as_bytes())
    }

    /// Una petición normal se interpreta con su ruta y su consulta.
    #[test]
    fn una_peticion_normal_se_interpreta() {
        let request = peticion("GET /index.html?a=1&b=2 HTTP/1.1").expect("petición");
        assert_eq!(request.method, Method::Get);
        assert_eq!(request.path, "/index.html");
        assert_eq!(request.param("a"), Some("1"));
        assert_eq!(request.param("b"), Some("2"));
        assert_eq!(request.param("c"), None);
        assert_eq!(request.header("host"), Some("x"));
    }

    /// Una petición ilegible no se atiende.
    #[test]
    fn una_peticion_ilegible_no_se_atiende() {
        assert!(parse_request(b"basura").is_none());
        assert!(parse_request(b"GET relativo HTTP/1.1\r\n\r\n").is_none());
        let larga = vec![b'a'; MAX_REQUEST_BYTES + 1];
        assert!(parse_request(&larga).is_none());
    }

    /// El socket exige token; la cáscara de la PWA no.
    #[test]
    fn el_socket_exige_token() {
        assert_eq!(
            route(&peticion("GET /ws?token=abc HTTP/1.1").unwrap(), "abc"),
            Route::Upgrade
        );
        assert_eq!(
            route(&peticion("GET /ws?token=nope HTTP/1.1").unwrap(), "abc"),
            Route::Unauthorized
        );
        assert_eq!(
            route(&peticion("GET /ws HTTP/1.1").unwrap(), "abc"),
            Route::Unauthorized
        );
        // La cáscara se sirve sin token: el navegador pide su manifiesto solo.
        assert_eq!(
            route(&peticion("GET /app.js HTTP/1.1").unwrap(), "abc"),
            Route::Asset("app.js".to_string())
        );
        // La raíz sirve el índice.
        assert_eq!(
            route(&peticion("GET / HTTP/1.1").unwrap(), "abc"),
            Route::Asset("index.html".to_string())
        );
    }

    /// El token también viaja en la cabecera, que es lo que usa el cliente.
    #[test]
    fn el_token_tambien_viaja_en_la_cabecera() {
        let request = parse_request(b"GET /ws HTTP/1.1\r\nAuthorization: Bearer secreto\r\n\r\n")
            .expect("petición");
        assert_eq!(route(&request, "secreto"), Route::Upgrade);
    }

    /// La comparación del token no acepta lo que no coincide.
    #[test]
    fn la_comparacion_del_token_es_estricta() {
        assert!(token_matches("secreto", "secreto"));
        assert!(!token_matches("secreto", "secret0"));
        assert!(!token_matches("secreto", "secret"));
        assert!(!token_matches("secreto", ""));
        assert!(!token_matches("", ""));
    }

    /// Sólo se sirven archivos de dentro de la raíz.
    #[test]
    fn no_se_puede_salir_de_la_raiz() {
        let root = Path::new("/srv/pwa");
        assert_eq!(
            resolve_asset(root, "app.js"),
            Some(PathBuf::from("/srv/pwa/app.js"))
        );
        assert_eq!(
            resolve_asset(root, "vendor/x.css"),
            Some(PathBuf::from("/srv/pwa/vendor/x.css"))
        );
        for malo in [
            "../secreto",
            "a/../../secreto",
            "/etc/passwd",
            "%2e%2e/secreto",
            "a//b",
            "a/./b",
            "con espacio.js",
        ] {
            assert_eq!(resolve_asset(root, malo), None, "no debía aceptar {malo}");
        }
    }

    /// La respuesta lleva estado, tipo y longitud.
    #[test]
    fn la_respuesta_lleva_lo_necesario() {
        let bytes = response(200, "text/html; charset=utf-8", b"hola");
        let texto = String::from_utf8(bytes).expect("utf8");
        assert!(texto.starts_with("HTTP/1.1 200 OK\r\n"), "{texto}");
        assert!(texto.contains("Content-Length: 4\r\n"), "{texto}");
        assert!(texto.ends_with("hola"), "{texto}");

        let negada = String::from_utf8(response(401, "text/plain", b"")).expect("utf8");
        assert!(
            negada.starts_with("HTTP/1.1 401 Unauthorized\r\n"),
            "{negada}"
        );
    }

    /// Cada archivo de la PWA se anuncia con su tipo.
    #[test]
    fn cada_archivo_se_anuncia_con_su_tipo() {
        assert!(content_type_for("index.html").starts_with("text/html"));
        assert!(content_type_for("app.js").starts_with("text/javascript"));
        assert!(content_type_for("app.css").starts_with("text/css"));
        assert_eq!(content_type_for("manifest.json"), "application/json");
        assert_eq!(content_type_for("icono.png"), "image/png");
        assert_eq!(
            content_type_for("desconocido.bin"),
            "application/octet-stream"
        );
    }
}
