//! Arranque del servidor móvil: de dónde sale el token, de dónde salen los
//! archivos y cuándo se enciende.
//!
//! Única responsabilidad: resolver lo que el servidor necesita para existir.
//! No abre sockets ni lee la red; eso lo hace `server` una vez que esto le dice
//! que hay algo que servir.

use std::io;
use std::net::SocketAddr;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::JoinHandle;

use crate::bridge::Bridge;
use crate::plugin::MobilePlugin;
use crate::server::{self, ServerConfig};
use crate::socket::ViewProvider;

/// Variable que fuerza el token, para noches de depuración y flotas gestionadas.
///
/// Un valor vacío se ignora igual que si no estuviera: un secreto en blanco no
/// protege nada y el servidor se niega a arrancar sin token.
pub const TOKEN_ENV: &str = "PORT_MOBILE_TOKEN";
/// Variable que fuerza el directorio de la PWA.
pub const ASSETS_ENV: &str = "PORT_MOBILE_ASSETS";
/// Nombre del archivo del token dentro del directorio de configuración.
const TOKEN_FILE: &str = "mobile-token";
/// Cuántos bytes aleatorios lleva el token: 256 bits, en hexadecimal.
///
/// Hexadecimal y no base64 a propósito: viaja en la consulta de la URL y en una
/// cabecera, y así no hay rellenos ni caracteres que escapar ni validar de más.
const TOKEN_BYTES: usize = 32;

/// Servidor en marcha: dónde escucha. Al soltarse, el hilo se desprende y sigue
/// sirviendo hasta que el proceso termine: el escucha vive dentro del hilo.
pub struct ServerHandle {
    /// Dirección real donde quedó escuchando (puerto efímero resuelto).
    pub addr: SocketAddr,
    // Se guarda para documentar que el hilo sigue vivo: al soltar el handle
    // el hilo se desprende en vez de terminarse.
    _handle: JoinHandle<()>,
}

impl MobilePlugin {
    /// Arranca el servidor móvil si hay algo que servir.
    ///
    /// Devuelve el plugin para registrar y el servidor en marcha. Sin token o
    /// sin archivos no hay servidor: una terminal sin PWA instalada se comporta
    /// exactamente igual que antes.
    pub fn start() -> Option<(Self, ServerHandle)> {
        let token = resolve_token()?;
        let assets = resolve_assets()?;
        Self::start_with(token, assets)
    }

    /// Arranque con token y archivos ya resueltos. Existe para poder probarlo
    /// sin tocar el entorno del proceso.
    pub(crate) fn start_with(token: String, assets: PathBuf) -> Option<(Self, ServerHandle)> {
        // Cinturón y tirantes: aunque `start()` ya filtra por `resolve_assets`,
        // quien llame directo no debe poder levantar un servidor sin nada que servir.
        if !assets.is_dir() {
            return None;
        }
        let bridge = Arc::new(Bridge::new());
        let plugin = Self::new(bridge.clone());
        let slot = plugin.services_slot();
        let view: ViewProvider = Arc::new(move || MobilePlugin::initial_view(&slot));
        let config = ServerConfig::new(token, assets);
        let (addr, handle) = server::spawn(config, bridge, view).ok()?;
        Some((
            plugin,
            ServerHandle {
                addr,
                _handle: handle,
            },
        ))
    }
}

/// Token con el que se anuncia el servidor: explícito o del archivo.
fn resolve_token() -> Option<String> {
    if let Some(explicit) = std::env::var_os(TOKEN_ENV).map(|v| v.to_string_lossy().into_owned()) {
        let explicit = explicit.trim().to_string();
        if !explicit.is_empty() {
            return Some(explicit);
        }
    }
    let path = token_file_path()?;
    load_or_create_token(&path).ok()
}

/// Dónde vive el token: directorio de configuración, estable entre reinicios.
///
/// A propósito NO es el directorio de sesión: ese se limpia al cerrar sesión y
/// obligaría a emparejar el teléfono en cada arranque. El archivo en sí va con
/// 0600, que es lo que lo protege.
fn token_file_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join("port").join(TOKEN_FILE))
}

/// Lee el token del archivo, o lo genera si falta o no vale.
///
/// Un contenido que no cumple el formato se reemplaza sin preguntar: un secreto
/// a medias es peor que uno nuevo, y el teléfono pide el token de todos modos
/// la primera vez que se empareja.
pub fn load_or_create_token(path: &Path) -> io::Result<String> {
    if let Ok(existing) = std::fs::read_to_string(path) {
        let existing = existing.trim().to_string();
        if is_valid_token(&existing) {
            return Ok(existing);
        }
    }
    let token = generate_token()?;
    if let Some(parent) = path.parent() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)?;
    }
    {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(token.as_bytes())?;
    }
    // Por si el archivo ya existía con permisos amplios: el secreto manda.
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(token)
}

/// Un token vale si tiene la forma que generamos: 64 hexadecimales.
fn is_valid_token(token: &str) -> bool {
    token.len() == TOKEN_BYTES * 2 && token.bytes().all(|b| b.is_ascii_hexdigit())
}

/// 256 bits de `/dev/urandom`, en hexadecimal.
///
/// Sin dependencias a propósito: añadir `rand` movería el hash de vendorización
/// de Nix por algo que el sistema operativo ya da.
fn generate_token() -> io::Result<String> {
    use std::io::Read;
    let mut bytes = [0u8; TOKEN_BYTES];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(hex_encode(&bytes))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

/// Directorio de la PWA: explícito o junto al binario.
///
/// Sin directorio no hay nada que servir y el servidor no arranca. Es lo que
/// hace que una terminal sin la PWA instalada se comporte igual que antes.
pub fn resolve_assets() -> Option<PathBuf> {
    resolve_assets_from(
        std::env::var_os(ASSETS_ENV).map(PathBuf::from),
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf)),
    )
}

/// Núcleo puro de la resolución, con las entradas inyectadas para probarlo sin
/// tocar el entorno del proceso.
pub(crate) fn resolve_assets_from(
    env: Option<PathBuf>,
    exe_dir: Option<PathBuf>,
) -> Option<PathBuf> {
    if let Some(dir) = env {
        // Configuración explícita que apunta a nada: fallar cerrado es mejor que
        // servir otro directorio por sorpresa.
        return dir.is_dir().then_some(dir);
    }
    let mut share = exe_dir?;
    // Se sube de `bin/` a la raíz de instalación y se baja a `share/…`:
    // sin `..` en la ruta no hay nada que normalizar después.
    share.pop();
    share.push("share/port/mobile");
    share.is_dir().then_some(share)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Directorio temporal único por prueba.
    fn tempdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "port-mobile-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("crear temporal");
        dir
    }

    /// El token sobrevive entre arranques: la segunda lectura devuelve el mismo.
    #[test]
    fn el_token_sobrevive_entre_arranques() {
        let dir = tempdir("token-persiste");
        let path = dir.join(TOKEN_FILE);
        let primero = load_or_create_token(&path).expect("generar");
        assert_eq!(primero.len(), TOKEN_BYTES * 2);
        assert!(primero.bytes().all(|b| b.is_ascii_hexdigit()));
        let segundo = load_or_create_token(&path).expect("leer");
        assert_eq!(primero, segundo);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Un contenido que no cumple el formato se reemplaza por uno válido.
    #[test]
    fn el_token_roto_se_regenera() {
        let dir = tempdir("token-roto");
        let path = dir.join(TOKEN_FILE);
        std::fs::write(&path, "basura").expect("escribir");
        let nuevo = load_or_create_token(&path).expect("regenerar");
        assert_ne!(nuevo, "basura");
        assert!(is_valid_token(&nuevo));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// El archivo del token queda sólo lectura del dueño, y su directorio igual.
    #[cfg(unix)]
    #[test]
    fn el_token_queda_solo_para_el_dueno() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir("token-permisos").join("sub").join("dir");
        let path = dir.join(TOKEN_FILE);
        load_or_create_token(&path).expect("generar");
        let modo = std::fs::metadata(&path).expect("meta").permissions().mode() & 0o777;
        assert_eq!(modo, 0o600, "el archivo debe ser 0600, es {modo:o}");
        let modo_dir = std::fs::metadata(&dir)
            .expect("meta dir")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(
            modo_dir, 0o700,
            "el directorio debe ser 0700, es {modo_dir:o}"
        );
        let _ = std::fs::remove_dir_all(dir.ancestors().nth(2).unwrap());
    }

    /// La resolución prefiere lo explícito, luego lo instalado, luego nada.
    #[test]
    fn los_archivos_se_resuelven_en_orden() {
        let base = tempdir("assets-orden");
        let explicito = base.join("explicito");
        let share_parent = base.join("bin");
        // El directorio del binario siempre existe en producción; el test lo modela.
        std::fs::create_dir_all(&share_parent).unwrap();
        let instalado = base.join("share").join("port").join("mobile");
        std::fs::create_dir_all(&explicito).unwrap();
        std::fs::create_dir_all(&instalado).unwrap();

        // Explícito y existente gana aunque haya instalación.
        assert_eq!(
            resolve_assets_from(Some(explicito.clone()), Some(share_parent.clone())),
            Some(explicito.clone())
        );
        // Explícito a nada: cerrado, no se cae al instalado.
        assert_eq!(
            resolve_assets_from(Some(base.join("no-existe")), Some(share_parent.clone())),
            None
        );
        // Sin variable, el share junto al binario.
        assert_eq!(
            resolve_assets_from(None, Some(share_parent)),
            Some(instalado)
        );
        // Sin nada: no hay servidor.
        assert_eq!(resolve_assets_from(None, None), None);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Sin archivos no hay servidor, aunque el token esté bien.
    #[test]
    fn sin_archivos_no_hay_servidor() {
        let dir = tempdir("sin-assets");
        assert!(MobilePlugin::start_with("a".repeat(63) + "0", dir.join("no-existe"),).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Con archivos y token el servidor escucha y sirve.
    #[test]
    fn con_archivos_el_servidor_escucha() {
        use std::io::{BufRead, Read, Write};
        use std::net::TcpStream;

        let dir = tempdir("sirve");
        std::fs::write(dir.join("index.html"), b"<h1>PORT</h1>").unwrap();
        let token = "a".repeat(63) + "0";
        let Some((_plugin, server)) = MobilePlugin::start_with(token, dir.clone()) else {
            panic!("debería arrancar");
        };
        let mut stream = TcpStream::connect(server.addr).expect("conectar");
        stream
            .write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n")
            .expect("pedir");
        let mut respuesta = String::new();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        std::io::BufReader::new(&stream)
            .read_line(&mut respuesta)
            .expect("leer");
        assert!(
            respuesta.starts_with("HTTP/1.1 200"),
            "esperaba 200, llegó: {respuesta}"
        );
        // El cuerpo lo lee el navegador; acá basta con que responda.
        let _ = stream.read(&mut [0u8; 64]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
