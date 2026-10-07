//! Tests del comando visor del hilo vivo.
//!
//! Comprueban la forma exacta del `PtyConfig` que consume el núcleo: `bash -c`
//! con el script como segundo argumento, los cuatro datos como argumentos
//! posicionales separados (nunca interpolados), el único archivo que se abre
//! (`$1/$2.$3.activity.json`) y un filtro `jq` sin comillas simples. También que
//! construir el comando no toca el sistema de archivos y que el filtro pinta una
//! muestra estática con la gramática esperada.

use super::viewer::{watch_command, FILTER, SCRIPT};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

const PRESENCE_DIR: &str = "/tmp/herdr-viewer-inexistente/presence";
const HASH: &str = "5ad9a04398e3b840b64c906d0f794d3d17a140ed3ada1a0bbcded38b16de3ccc";
const INCARNATION: &str = "81a8a249-55b7-4b19-bddd-f1db0fb2dd69";
const LABEL: &str = "herdr viewer";

#[test]
fn el_comando_es_bash_con_la_forma_de_cinco_argumentos() {
    let config = watch_command(Path::new(PRESENCE_DIR), HASH, INCARNATION, LABEL);
    assert_eq!(config.command, "bash");
    assert_eq!(config.args[0], "-c");
    assert_eq!(config.args[1], SCRIPT);
    // `$0` del shell y los cuatro datos, como argumentos separados.
    assert_eq!(config.args[2], "herdr-view");
    assert_eq!(config.args.len(), 7, "bash, -c, script, $0 y cuatro datos");
    assert_eq!(config.cwd, None);
}

#[test]
fn el_script_solo_abre_el_archivo_de_actividad() {
    assert!(
        SCRIPT.contains("FILE=\"$1/$2.$3.activity.json\""),
        "la única ruta construida debe ser la de actividad: {SCRIPT}"
    );
    assert!(
        !SCRIPT.contains(".header.json"),
        "el visor no lee el header: {SCRIPT}"
    );
    assert!(
        !SCRIPT.contains(".jsonl"),
        "el visor no lee la sesión de pi: {SCRIPT}"
    );
    assert!(
        SCRIPT.contains("jq -r --unbuffered"),
        "jq debe emitir texto crudo sin buffer: {SCRIPT}"
    );
    assert!(
        SCRIPT.contains(FILTER),
        "el script debe empotrar el filtro: {SCRIPT}"
    );
}

#[test]
fn los_argumentos_viajan_separados_y_nunca_dentro_del_script() {
    let config = watch_command(Path::new(PRESENCE_DIR), HASH, INCARNATION, LABEL);
    assert_eq!(config.args[3], PRESENCE_DIR);
    assert_eq!(config.args[4], HASH);
    assert_eq!(config.args[5], INCARNATION);
    assert_eq!(config.args[6], LABEL);
    // Sin interpolación: ningún dato es parte del texto que el shell interpreta.
    assert!(!SCRIPT.contains(PRESENCE_DIR), "el dir no va en el script");
    assert!(!SCRIPT.contains(HASH), "el hash no va en el script");
    assert!(!SCRIPT.contains(LABEL), "la etiqueta no va en el script");
}

#[test]
fn el_filtro_no_lleva_comillas_simples() {
    // El filtro se empotra dentro de una cadena del shell entrecomillada con
    // comillas simples: una comilla suelta lo rompería.
    assert!(
        !FILTER.contains('\''),
        "el filtro no puede contener comillas simples"
    );
    // El script lo entrecomilla con un par de comillas simples alrededor.
    let quoted = format!("'{FILTER}'");
    assert!(
        SCRIPT.contains(&quoted),
        "el script debe entrecomillar el filtro: {SCRIPT}"
    );
}

/// Ejecuta el filtro real sobre una muestra y devuelve sus líneas.
fn render(sample: &str) -> Vec<String> {
    let mut child = Command::new("jq")
        .args([
            "-r",
            "--unbuffered",
            "--arg",
            "hash",
            HASH,
            "--arg",
            "incarnation",
            INCARNATION,
            "--argjson",
            "seen",
            "0",
            FILTER,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("jq debe estar en PATH");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(sample.as_bytes())
        .expect("escribir la muestra");
    let out = child.wait_with_output().expect("esperar a jq");
    assert!(
        out.status.success(),
        "el filtro debe aceptar la muestra: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .expect("jq emite UTF-8")
        .lines()
        .map(str::to_string)
        .collect()
}

#[test]
fn la_gramatica_pinta_una_muestra_estatica_con_los_colores_de_pi() {
    let larga = "a".repeat(400);
    let muestra = format!(
        "{{\"schema\":1,\"sessionHash\":\"{HASH}\",\"incarnation\":\"{INCARNATION}\",\"activity\":{{\"tasks\":[{{\"summary\":{{\"status\":\"running\"}},\"thread\":{{\"items\":[\
{{\"kind\":\"text\",\"text\":\"hola\"}},\
{{\"kind\":\"thinking\",\"text\":\"pienso\"}},\
{{\"kind\":\"note\",\"text\":\"nota\"}},\
{{\"kind\":\"tool\",\"name\":\"bash\",\"output\":\"salida\",\"running\":false,\"isError\":false}},\
{{\"kind\":\"tool\"}},\
{{\"kind\":\"text\",\"text\":\"{larga}\"}}\
]}}}}]}}}}"
    );

    let lineas = render(&muestra);
    assert_eq!(lineas[0], "@count=6;1", "línea de control");

    // Texto del usuario/asistente: fg-15 con su prefijo de siempre.
    assert_eq!(lineas[1], "\x1b[97mtext → hola\x1b[0m");

    // Razonamiento y nota: dim fg-7, con el texto completo de siempre.
    assert_eq!(lineas[2], "\x1b[2m\x1b[37m· piensa pienso\x1b[0m");
    assert_eq!(lineas[3], "\x1b[2m\x1b[37m· nota nota\x1b[0m");

    // Herramienta ok: cabecera bold fg-9, estado en fg-10, salida en fg-7 y
    // pie dim con el recuento real de líneas.
    assert_eq!(lineas[4], "\x1b[1;91m▸ bash \x1b[0m\x1b[92m…ok\x1b[0m");
    assert!(lineas[4].contains("\x1b[1;91m"), "cabecera bold fg-9");
    assert!(lineas[4].contains("\x1b[92m"), "estado ok en fg-10");
    assert_eq!(lineas[5], "\x1b[37m↳ salida\x1b[0m");
    assert_eq!(lineas[6], "\x1b[2m\x1b[37m  ✓ ok · 1 líneas\x1b[0m");

    // Una herramienta sin nombre ni salida no se descarta: cabecera y pie.
    assert_eq!(lineas[7], "\x1b[1;91m▸  \x1b[0m\x1b[92m…ok\x1b[0m");
    assert_eq!(lineas[8], "\x1b[2m\x1b[37m  ✓ ok · 0 líneas\x1b[0m");

    // Texto desconocido: mismo prefijo, recortado y en fg-15.
    assert!(lineas[9].starts_with("\x1b[97mtext → "), "{}", lineas[9]);
    assert!(lineas[9].ends_with("\x1b[0m"), "{}", lineas[9]);
    assert!(
        lineas[9].contains('…'),
        "el texto largo se recorta: {}",
        lineas[9]
    );
    assert_eq!(
        lineas[9].chars().count(),
        "\x1b[97m\x1b[0mtext → ".chars().count() + 301,
        "texto recortado a 300 más elipsis"
    );
    assert_eq!(
        lineas.len(),
        10,
        "control + seis items, uno de ellos con salida en 3 filas"
    );
}

#[test]
fn una_herramienta_fallida_o_corriendo_usa_su_estado_y_su_fondo() {
    let muestra = format!(
        "{{\"schema\":1,\"sessionHash\":\"{HASH}\",\"incarnation\":\"{INCARNATION}\",\"activity\":{{\"tasks\":[{{\"summary\":{{\"status\":\"running\"}},\"thread\":{{\"items\":[\
{{\"kind\":\"tool\",\"name\":\"edit\",\"output\":\"uno\\ndos\",\"running\":false,\"isError\":true}},\
{{\"kind\":\"tool\",\"name\":\"bash\",\"output\":\"parcial\",\"running\":true,\"isError\":false}}\
]}}}}]}}}}"
    );

    let lineas = render(&muestra);
    assert_eq!(lineas[0], "@count=2;1", "línea de control");

    // Herramienta fallida: cabecera bold fg-9, salidas en fg-7 sobre bg-0 y pie
    // con el recuento real de líneas (dos, no la salida en una sola fila).
    assert_eq!(lineas[1], "\x1b[1;91m▸ edit \x1b[0m\x1b[1;91m…falló\x1b[0m");
    assert_eq!(lineas[2], "\x1b[40m\x1b[37m↳ uno\x1b[0m");
    assert_eq!(lineas[3], "\x1b[40m\x1b[37m↳ dos\x1b[0m");
    assert_eq!(lineas[4], "\x1b[2m\x1b[37m  ✗ falló · 2 líneas\x1b[0m");

    // Herramienta corriendo: estado en fg-11 y pie sin recuento.
    assert_eq!(
        lineas[5],
        "\x1b[1;91m▸ bash \x1b[0m\x1b[93m…corriendo\x1b[0m"
    );
    assert_eq!(lineas[6], "\x1b[37m↳ parcial\x1b[0m");
    assert_eq!(lineas[7], "\x1b[2m\x1b[37m  …corriendo\x1b[0m");
    assert_eq!(lineas.len(), 8);
}

#[test]
fn la_herramienta_read_colapsa_su_salida_en_una_sola_fila() {
    let ruta = concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs");
    let muestra = format!(
        "{{\"schema\":1,\"sessionHash\":\"{HASH}\",\"incarnation\":\"{INCARNATION}\",\"activity\":{{\"tasks\":[{{\"summary\":{{\"status\":\"running\"}},\"thread\":{{\"items\":[\
{{\"kind\":\"tool\",\"name\":\"read\",\"arguments\":{{\"path\":\"{ruta}\"}},\"output\":\"linea uno\\n\\nlinea dos\\nlinea tres\",\"running\":false,\"isError\":false}},\
{{\"kind\":\"tool\",\"name\":\"read\",\"arguments\":{{\"file\":\"notas.md\"}},\"output\":\"solo una\",\"running\":false,\"isError\":false}},\
{{\"kind\":\"tool\",\"name\":\"bash\",\"output\":\"uno\\ndos\",\"running\":false,\"isError\":false}}\
]}}}}]}}}}"
    );

    let lineas = render(&muestra);
    assert_eq!(lineas[0], "@count=3;1", "línea de control");

    // `read` con varias líneas: cabecera con la ruta y UNA sola fila que
    // resume el resto con su recuento.
    assert_eq!(
        lineas[1],
        format!("\x1b[1;91m▸ read {ruta} \x1b[0m\x1b[92m…ok\x1b[0m")
    );
    assert_eq!(lineas[2], "\x1b[37m↳ linea uno … +2 líneas\x1b[0m");

    // `read` con una sola línea: sin sufijo de recuento.
    assert_eq!(
        lineas[3],
        "\x1b[1;91m▸ read notas.md \x1b[0m\x1b[92m…ok\x1b[0m"
    );
    assert_eq!(lineas[4], "\x1b[37m↳ solo una\x1b[0m");

    // Las demás herramientas conservan su gramática de una fila por línea.
    assert_eq!(lineas[5], "\x1b[1;91m▸ bash \x1b[0m\x1b[92m…ok\x1b[0m");
    assert_eq!(lineas[6], "\x1b[37m↳ uno\x1b[0m");
    assert_eq!(lineas[7], "\x1b[37m↳ dos\x1b[0m");
    assert_eq!(lineas[8], "\x1b[2m\x1b[37m  ✓ ok · 2 líneas\x1b[0m");
    assert_eq!(lineas.len(), 9, "control + tres herramientas");
}

#[test]
fn el_encabezado_del_script_lleva_los_colores_de_pi() {
    // El encabezado se pinta en el shell (no en jq): la etiqueta sigue siendo un
    // argumento posicional y el formato nunca la interpreta.
    assert!(
        SCRIPT.contains("● %s"),
        "el encabezado usa un formato seguro"
    );
    assert!(SCRIPT.contains("\\033[1;91m"), "encabezado bold fg-9");
    assert!(
        SCRIPT.contains("\\033[2m\\033[37m%s\\033[0m"),
        "la regla dim fg-7"
    );
    assert!(SCRIPT.contains("RULE="), "la regla se construye una vez");
}

#[test]
fn construir_el_comando_no_toca_el_sistema_de_archivos() {
    // Ruta inexistente: construir el comando no debe crearla.
    let dir = Path::new(PRESENCE_DIR);
    assert!(!dir.exists(), "la ruta de prueba no debe existir");
    let config = watch_command(dir, HASH, INCARNATION, LABEL);
    assert!(!dir.exists(), "watch_command no debe crear el directorio");
    assert_eq!(config.command, "bash");

    // Archivo real: tampoco debe leerlo ni escribirlo.
    let temp = std::env::temp_dir().join(format!(
        "herdr-viewer-sin-tocar-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&temp).expect("crear directorio temporal");
    let file = temp.join(format!("{HASH}.{INCARNATION}.activity.json"));
    std::fs::write(&file, "{\"schema\":1}").expect("escribir actividad");
    let before = std::fs::read_to_string(&file).expect("leer antes");

    let config = watch_command(&temp, HASH, INCARNATION, LABEL);

    assert_eq!(config.args[3], temp.display().to_string());
    assert_eq!(
        std::fs::read_to_string(&file).expect("leer después"),
        before,
        "el visor no debe modificar la actividad"
    );

    let _ = std::fs::remove_dir_all(&temp);
}
