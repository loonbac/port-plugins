//! Tests del lector de presencia. Headless: fixtures en un directorio temporal,
//! sin GPUI y sin tocar el contrato real de la máquina del desarrollador.

use super::presence::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

const HASH: &str = "5ad9a04398e3b840b64c906d0f794d3d17a140ed3ada1a0bbcded38b16de3ccc";
const OTRO_HASH: &str = "122ae8c3cd6d4a8738c9919946034a7389adaa13024606827d94bb39ee28fc58";
const INCARNATION: &str = "81a8a249-55b7-4b19-bddd-f1db0fb2dd69";
const OTRA_ENCARNACION: &str = "d345a092-4a46-4c77-a895-f4b456d1bc49";

/// Latido vivo frente a `1_005_000`.
const LATIDO_VIVO: &str = "1000000";
const AHORA: u64 = 1_005_000;

/// Construye un header válido, con los campos indicados sobrescritos.
fn header_json(overrides: &[(&str, &str)]) -> String {
    let mut fields: Vec<(&str, String)> = vec![
        ("schema", "1".to_string()),
        ("sessionHash", format!("\"{HASH}\"")),
        ("incarnation", format!("\"{INCARNATION}\"")),
        ("label", "\"sesion de prueba\"".to_string()),
        ("heartbeat", LATIDO_VIVO.to_string()),
        ("generation", "1".to_string()),
        (
            "counts",
            "{\"running\":0,\"queued\":0,\"waiting\":0,\"finished\":0}".to_string(),
        ),
        ("digest", "null".to_string()),
        ("unavailable", "null".to_string()),
    ];
    for (key, value) in overrides {
        match fields.iter_mut().find(|(k, _)| k == key) {
            Some(slot) => slot.1 = (*value).to_string(),
            None => fields.push((key, (*value).to_string())),
        }
    }
    let body = fields
        .iter()
        .map(|(k, v)| format!("\"{k}\":{v}"))
        .collect::<Vec<_>>()
        .join(",");
    format!("{{{body}}}")
}

/// Construye el `summary` de una tarea con los valores por defecto.
fn summary_json(overrides: &[(&str, &str)]) -> String {
    let mut fields: Vec<(&str, String)> = vec![
        ("id", "\"t_abc123\"".to_string()),
        ("agent", "\"gentle-ai-worker\"".to_string()),
        ("label", "\"herdr presence reader\"".to_string()),
        ("status", "\"running\"".to_string()),
        ("model", "\"test/model\"".to_string()),
        ("createdAt", "3000".to_string()),
        ("startedAt", "3001".to_string()),
        ("endedAt", "null".to_string()),
        ("lastActivityAt", "4000".to_string()),
    ];
    for (key, value) in overrides {
        match fields.iter_mut().find(|(k, _)| k == key) {
            Some(slot) => slot.1 = (*value).to_string(),
            None => fields.push((key, (*value).to_string())),
        }
    }
    let body = fields
        .iter()
        .map(|(k, v)| format!("\"{k}\":{v}"))
        .collect::<Vec<_>>()
        .join(",");
    format!("{{{body}}}")
}

/// Una fila de tarea completa, con `thread` vacío.
fn task_row(overrides: &[(&str, &str)]) -> String {
    task_row_with_thread(overrides, &[])
}

/// Una fila de tarea con los items de hilo indicados, ya serializados.
fn task_row_with_thread(overrides: &[(&str, &str)], items: &[&str]) -> String {
    format!(
        "{{\"summary\":{},\"thread\":{{\"version\":1,\"dropped\":0,\"items\":[{}]}}}}",
        summary_json(overrides),
        items.join(",")
    )
}

/// Un item de hilo a partir de sus campos.
fn item_json(fields: &[(&str, &str)]) -> String {
    let body = fields
        .iter()
        .map(|(k, v)| format!("\"{k}\":{v}"))
        .collect::<Vec<_>>()
        .join(",");
    format!("{{{body}}}")
}

/// Un archivo de actividad completo para la identidad por defecto.
fn activity_json(rows: &[String]) -> String {
    format!(
        "{{\"schema\":1,\"sessionHash\":\"{HASH}\",\"incarnation\":\"{INCARNATION}\",\"generation\":1,\"activity\":{{\"tasks\":[{}]}}}}",
        rows.join(",")
    )
}

fn header_name(hash: &str, incarnation: &str) -> String {
    format!("{hash}.{incarnation}.header.json")
}

fn activity_name(hash: &str, incarnation: &str) -> String {
    format!("{hash}.{incarnation}.activity.json")
}

/// Crea un árbol con la misma forma que el real: `<raiz>/gentle-agents/presence`.
fn temp_dir(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("herdr-presence-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let dir = root.join("gentle-agents").join("presence");
    fs::create_dir_all(&dir).expect("crear dir temporal");
    dir
}

fn write(dir: &Path, name: &str, body: &str) {
    fs::write(dir.join(name), body).expect("escribir fixture");
}

// ── Parseo puro ────────────────────────────────────────────────────────────

#[test]
fn parsea_un_header_valido() {
    let session = parse_header(&header_json(&[])).expect("debería parsear");
    assert_eq!(session.session_hash, HASH);
    assert_eq!(session.incarnation, INCARNATION);
    assert_eq!(session.label, "sesion de prueba");
    assert_eq!(session.heartbeat, 1_000_000);
    assert!(session.thread_available);
    assert!(session.tasks.is_empty());
}

#[test]
fn un_header_roto_no_rompe() {
    assert!(parse_header("{ esto no es json").is_none());
    assert!(parse_header("").is_none());
    assert!(parse_header("[]").is_none());
    // Un `schema` distinto no es este contrato.
    assert!(parse_header(&header_json(&[("schema", "2")])).is_none());
    // `generation` debe existir y ser mayor que cero.
    assert!(parse_header(&header_json(&[("generation", "0")])).is_none());
    // El latido no puede ser negativo ni faltar.
    assert!(parse_header(&header_json(&[("heartbeat", "-1")])).is_none());
    assert!(parse_header(&header_json(&[("label", "7")])).is_none());
    // La identidad tiene forma obligatoria: hash de 64 hex y UUID v4.
    assert!(parse_header(&header_json(&[("sessionHash", "\"no-es-hex\"")])).is_none());
    assert!(parse_header(&header_json(&[("incarnation", "\"no-es-uuid\"")])).is_none());
}

#[test]
fn las_claves_extra_se_ignoran() {
    let header = header_json(&[("campoFuturo", "{\"algo\":true}")]);
    let session = parse_header(&header).expect("parsea");
    let activity = activity_json(&[task_row(&[("extra", "\"x\"")])]);
    assert_eq!(
        parse_activity(&activity, &session).expect("parsea").len(),
        1
    );
}

#[test]
fn normaliza_la_etiqueta() {
    let session = parse_header(&header_json(&[(
        "label",
        "\"  hola\\u0007\\n\\t   mundo  \"",
    )]))
    .expect("parsea");
    assert_eq!(session.label, "hola mundo");

    // Se corta a 120 caracteres.
    let larga = "a".repeat(130);
    let session =
        parse_header(&header_json(&[("label", format!("\"{larga}\"").as_str())])).expect("parsea");
    assert_eq!(session.label.chars().count(), 120);
}

// ── Vivencia ───────────────────────────────────────────────────────────────

#[test]
fn lee_una_sesion_viva_con_sus_tareas() {
    let dir = temp_dir("viva");
    write(&dir, &header_name(HASH, INCARNATION), &header_json(&[]));
    write(
        &dir,
        &activity_name(HASH, INCARNATION),
        &activity_json(&[
            task_row(&[]),
            task_row(&[
                ("id", "\"t_fin\""),
                ("status", "\"completed\""),
                ("endedAt", "4200"),
            ]),
        ]),
    );

    let sessions = read_live_sessions(&dir, AHORA);
    assert_eq!(sessions.len(), 1, "la sesión viva debe salir");
    let session = &sessions[0];
    assert!(session.thread_available);
    assert_eq!(session.tasks.len(), 2);
    assert_eq!(session.tasks[0].id, "t_abc123");
    assert_eq!(session.tasks[0].status, "running");
    assert_eq!(session.tasks[0].ended_at, None);
    assert_eq!(session.tasks[0].last_activity_at, 4000);
    assert_eq!(session.tasks[1].id, "t_fin");
    assert_eq!(session.tasks[1].ended_at, Some(4200));
}

#[test]
fn el_latido_en_el_borde_de_la_ventana_sigue_vivo() {
    let dir = temp_dir("borde");
    write(&dir, &header_name(HASH, INCARNATION), &header_json(&[]));

    let justo = 1_000_000 + LIVE_WINDOW_MS;
    assert_eq!(
        read_live_sessions(&dir, justo).len(),
        1,
        "exactamente en la ventana el latido cuenta como vivo"
    );
    assert!(
        read_live_sessions(&dir, justo + 1).is_empty(),
        "un milisegundo más allá ya no está vivo"
    );
}

#[test]
fn un_latido_en_el_futuro_no_esta_vivo() {
    let dir = temp_dir("futuro");
    write(
        &dir,
        &header_name(HASH, INCARNATION),
        &header_json(&[("heartbeat", "2000000")]),
    );
    assert!(read_live_sessions(&dir, AHORA).is_empty());
}

#[test]
fn el_nombre_debe_coincidir_con_el_contenido() {
    let dir = temp_dir("nombre");
    // El contenido dice HASH/INCARNATION, pero el nombre promete otras.
    write(
        &dir,
        &header_name(OTRO_HASH, INCARNATION),
        &header_json(&[]),
    );
    write(
        &dir,
        &header_name(HASH, OTRA_ENCARNACION),
        &header_json(&[]),
    );
    assert!(
        read_live_sessions(&dir, AHORA).is_empty(),
        "un nombre que no casa con su contenido no es de esa sesión"
    );
}

#[test]
fn un_directorio_inexistente_devuelve_lista_vacia() {
    assert!(read_live_sessions(Path::new("/no/existe/de/verdad/presence"), AHORA).is_empty());
}

// ── Actividad ──────────────────────────────────────────────────────────────

#[test]
fn unavailable_activity_too_large_no_lee_la_actividad() {
    let dir = temp_dir("demasiado-grande");
    write(
        &dir,
        &header_name(HASH, INCARNATION),
        &header_json(&[
            ("unavailable", "\"activity-too-large\""),
            ("digest", "null"),
        ]),
    );
    // Hay una actividad perfectamente válida en disco: no debe leerse.
    write(
        &dir,
        &activity_name(HASH, INCARNATION),
        &activity_json(&[task_row(&[])]),
    );

    let sessions = read_live_sessions(&dir, AHORA);
    assert_eq!(sessions.len(), 1, "la sesión no se pierde");
    assert!(!sessions[0].thread_available);
    assert!(sessions[0].tasks.is_empty());
}

#[test]
fn un_unavailable_desconocido_tampoco_lee_la_actividad() {
    // Decisión fail closed: cualquier motivo de indisponibilidad que no sea
    // null deja el hilo fuera; la sesión sobrevive igual.
    let dir = temp_dir("unavailable-raro");
    write(
        &dir,
        &header_name(HASH, INCARNATION),
        &header_json(&[("unavailable", "\"motivo-futuro\"")]),
    );
    write(
        &dir,
        &activity_name(HASH, INCARNATION),
        &activity_json(&[task_row(&[])]),
    );

    let sessions = read_live_sessions(&dir, AHORA);
    assert_eq!(sessions.len(), 1);
    assert!(!sessions[0].thread_available);
    assert!(sessions[0].tasks.is_empty());
}

#[test]
fn una_actividad_ausente_no_pierde_la_sesion() {
    let dir = temp_dir("sin-actividad");
    write(&dir, &header_name(HASH, INCARNATION), &header_json(&[]));

    let sessions = read_live_sessions(&dir, AHORA);
    assert_eq!(sessions.len(), 1);
    assert!(!sessions[0].thread_available);
    assert!(sessions[0].tasks.is_empty());
}

#[test]
fn una_actividad_con_identidad_distinta_no_aporta_tareas() {
    let dir = temp_dir("identidad-distinta");
    write(&dir, &header_name(HASH, INCARNATION), &header_json(&[]));
    let ajena = format!(
        "{{\"schema\":1,\"sessionHash\":\"{OTRO_HASH}\",\"incarnation\":\"{INCARNATION}\",\"activity\":{{\"tasks\":[]}}}}"
    );
    write(&dir, &activity_name(HASH, INCARNATION), &ajena);

    let sessions = read_live_sessions(&dir, AHORA);
    assert_eq!(sessions.len(), 1);
    assert!(!sessions[0].thread_available);
    assert!(sessions[0].tasks.is_empty());
}

#[test]
fn una_actividad_rota_no_rompe() {
    let dir = temp_dir("actividad-rota");
    write(&dir, &header_name(HASH, INCARNATION), &header_json(&[]));
    write(&dir, &activity_name(HASH, INCARNATION), "{ roto");

    let sessions = read_live_sessions(&dir, AHORA);
    assert_eq!(sessions.len(), 1);
    assert!(!sessions[0].thread_available);
}

// ── Tareas ─────────────────────────────────────────────────────────────────

#[test]
fn un_estado_desconocido_omite_solo_su_tarea() {
    let dir = temp_dir("estado-desconocido");
    write(&dir, &header_name(HASH, INCARNATION), &header_json(&[]));
    write(
        &dir,
        &activity_name(HASH, INCARNATION),
        &activity_json(&[
            task_row(&[("id", "\"t_buena_1\"")]),
            task_row(&[("id", "\"t_mala\""), ("status", "\"zombie\"")]),
            task_row(&[("id", "\"t_buena_2\""), ("status", "\"timed_out\"")]),
        ]),
    );

    let sessions = read_live_sessions(&dir, AHORA);
    assert_eq!(sessions.len(), 1);
    assert!(sessions[0].thread_available);
    let ids: Vec<&str> = sessions[0]
        .tasks
        .iter()
        .map(|task| task.id.as_str())
        .collect();
    assert_eq!(ids, vec!["t_buena_1", "t_buena_2"]);
}

#[test]
fn un_id_vacio_o_mal_tipado_omite_su_tarea() {
    let dir = temp_dir("id-malo");
    write(&dir, &header_name(HASH, INCARNATION), &header_json(&[]));
    write(
        &dir,
        &activity_name(HASH, INCARNATION),
        &activity_json(&[
            task_row(&[("id", "\"\"")]),
            task_row(&[("id", "7")]),
            task_row(&[("id", "\"t_ok\"")]),
        ]),
    );

    let sessions = read_live_sessions(&dir, AHORA);
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].tasks.len(), 1);
    assert_eq!(sessions[0].tasks[0].id, "t_ok");
}

#[test]
fn un_task_sin_thread_sigue_siendo_valido() {
    // El hilo no se parsea a propósito: la fila no necesita traerlo.
    let session = parse_header(&header_json(&[])).expect("parsea");
    let activity = format!(
        "{{\"schema\":1,\"sessionHash\":\"{HASH}\",\"incarnation\":\"{INCARNATION}\",\"activity\":{{\"tasks\":[{{\"summary\":{}}}]}}}}",
        summary_json(&[])
    );
    let tasks = parse_activity(&activity, &session).expect("parsea");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].id, "t_abc123");
    assert_eq!(tasks[0].label, "herdr presence reader");
}

// ── Hilo (thread) ──────────────────────────────────────────────────────────

#[test]
fn un_item_de_herramienta_tolera_cada_campo_ausente() {
    let session = parse_header(&header_json(&[])).expect("parsea");
    let completos: [(&str, &str); 6] = [
        ("kind", "\"tool\""),
        ("callId", "\"c1\""),
        ("name", "\"bash\""),
        ("output", "\"salida\""),
        ("running", "true"),
        ("isError", "true"),
    ];
    for ausente in ["callId", "name", "output", "running", "isError"] {
        let campos: Vec<(&str, &str)> = completos
            .iter()
            .copied()
            .filter(|(clave, _)| *clave != ausente)
            .collect();
        let item = item_json(&campos);
        let activity = activity_json(&[task_row_with_thread(&[], &[item.as_str()])]);
        let tasks = parse_activity(&activity, &session).expect("parsea");
        let AgentThreadItem::Tool {
            call_id,
            name,
            output,
            running,
            is_error,
        } = &tasks[0].thread[0]
        else {
            panic!("el item debe seguir siendo una herramienta");
        };
        assert_eq!(call_id, if ausente == "callId" { "" } else { "c1" });
        assert_eq!(name, if ausente == "name" { "" } else { "bash" });
        assert_eq!(output, if ausente == "output" { "" } else { "salida" });
        assert_eq!(*running, ausente != "running");
        assert_eq!(*is_error, ausente != "isError");
    }

    // Un `null` explícito cae igual que un campo ausente.
    let con_null = item_json(&[
        ("kind", "\"tool\""),
        ("name", "null"),
        ("running", "null"),
        ("isError", "null"),
    ]);
    let activity = activity_json(&[task_row_with_thread(&[], &[con_null.as_str()])]);
    let tasks = parse_activity(&activity, &session).expect("parsea");
    assert_eq!(
        tasks[0].thread[0],
        AgentThreadItem::Tool {
            call_id: String::new(),
            name: String::new(),
            output: String::new(),
            running: false,
            is_error: false,
        }
    );
}

#[test]
fn parsea_los_items_textuales() {
    let session = parse_header(&header_json(&[])).expect("parsea");
    let items = [
        item_json(&[("kind", "\"text\""), ("text", "\"hola\"")]),
        item_json(&[("kind", "\"thinking\""), ("text", "\"pienso\"")]),
        item_json(&[("kind", "\"note\""), ("text", "\"nota\"")]),
    ];
    let refs: Vec<&str> = items.iter().map(String::as_str).collect();
    let activity = activity_json(&[task_row_with_thread(&[], &refs)]);
    let tasks = parse_activity(&activity, &session).expect("parsea");
    assert_eq!(
        tasks[0].thread,
        vec![
            AgentThreadItem::Text {
                kind: "text".to_string(),
                text: "hola".to_string()
            },
            AgentThreadItem::Text {
                kind: "thinking".to_string(),
                text: "pienso".to_string()
            },
            AgentThreadItem::Text {
                kind: "note".to_string(),
                text: "nota".to_string()
            },
        ]
    );
}

#[test]
fn un_item_textual_sin_texto_se_omite() {
    let session = parse_header(&header_json(&[])).expect("parsea");
    let items = [
        item_json(&[("kind", "\"text\""), ("text", "null")]),
        item_json(&[("kind", "\"thinking\"")]),
        item_json(&[("kind", "\"text\""), ("text", "7")]),
        item_json(&[("kind", "\"text\""), ("text", "\"legible\"")]),
    ];
    let refs: Vec<&str> = items.iter().map(String::as_str).collect();
    let activity = activity_json(&[task_row_with_thread(&[], &refs)]);
    let tasks = parse_activity(&activity, &session).expect("parsea");
    assert_eq!(tasks[0].thread.len(), 1, "solo el item con texto legible");
    assert_eq!(
        tasks[0].thread[0],
        AgentThreadItem::Text {
            kind: "text".to_string(),
            text: "legible".to_string()
        }
    );
}

#[test]
fn un_task_sin_thread_queda_con_hilo_vacio() {
    let session = parse_header(&header_json(&[])).expect("parsea");
    // `thread` ausente no es un error: la tarea queda con hilo vacío.
    let sin_thread = format!(
        "{{\"schema\":1,\"sessionHash\":\"{HASH}\",\"incarnation\":\"{INCARNATION}\",\"activity\":{{\"tasks\":[{{\"summary\":{}}}]}}}}",
        summary_json(&[])
    );
    let tasks = parse_activity(&sin_thread, &session).expect("parsea");
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].thread.is_empty());

    // `thread.items` mal tipado tampoco invalida la tarea.
    let raro = activity_json(&[format!(
        "{{\"summary\":{},\"thread\":{{\"items\":7}}}}",
        summary_json(&[])
    )]);
    let tasks = parse_activity(&raro, &session).expect("parsea");
    assert!(tasks[0].thread.is_empty());
}

#[test]
fn parsea_una_actividad_realista_con_hilos() {
    let session = parse_header(&header_json(&[])).expect("parsea");
    let items = [
        item_json(&[("kind", "\"text\""), ("text", "\"leyendo el contrato\"")]),
        item_json(&[
            ("kind", "\"tool\""),
            ("callId", "\"c1\""),
            ("name", "\"read\""),
            ("output", "\"contenido\""),
            ("running", "false"),
            ("isError", "false"),
        ]),
        item_json(&[("kind", "\"thinking\""), ("text", "\"decido el shape\"")]),
        item_json(&[
            ("kind", "\"tool\""),
            ("callId", "\"c2\""),
            ("name", "\"bash\""),
            ("output", "\"\""),
            ("running", "true"),
            ("isError", "false"),
        ]),
        item_json(&[
            ("kind", "\"note\""),
            ("text", "\"sin salida\""),
            ("extra", "true"),
        ]),
        item_json(&[
            ("kind", "\"tool\""),
            ("callId", "\"c3\""),
            ("name", "\"grep\""),
            ("output", "\"boom\""),
            ("running", "false"),
            ("isError", "true"),
        ]),
    ];
    let refs: Vec<&str> = items.iter().map(String::as_str).collect();
    let activity = activity_json(&[task_row_with_thread(&[("id", "\"t_hilo\"")], &refs)]);
    let tasks = parse_activity(&activity, &session).expect("parsea");
    assert_eq!(tasks[0].id, "t_hilo");
    assert_eq!(tasks[0].thread.len(), 6);
    assert_eq!(
        tasks[0].thread[1],
        AgentThreadItem::Tool {
            call_id: "c1".to_string(),
            name: "read".to_string(),
            output: "contenido".to_string(),
            running: false,
            is_error: false,
        }
    );
    assert_eq!(
        tasks[0].thread[3],
        AgentThreadItem::Tool {
            call_id: "c2".to_string(),
            name: "bash".to_string(),
            output: String::new(),
            running: true,
            is_error: false,
        }
    );
    assert_eq!(
        tasks[0].thread[5],
        AgentThreadItem::Tool {
            call_id: "c3".to_string(),
            name: "grep".to_string(),
            output: "boom".to_string(),
            running: false,
            is_error: true,
        }
    );
}

#[test]
fn lee_el_ultimo_paso_crudo() {
    let session = parse_header(&header_json(&[])).expect("parsea");
    let con_paso = activity_json(&[task_row_with_thread(
        &[("lastStep", "\"reading\""), ("extra", "\"x\"")],
        &[],
    )]);
    assert_eq!(
        parse_activity(&con_paso, &session).expect("parsea")[0].last_step,
        "reading"
    );

    // Ausente o `null`: cadena vacía, nunca un error.
    let ausente = activity_json(&[task_row_with_thread(&[], &[])]);
    assert_eq!(
        parse_activity(&ausente, &session).expect("parsea")[0].last_step,
        ""
    );
    let con_null = activity_json(&[task_row_with_thread(&[("lastStep", "null")], &[])]);
    assert_eq!(
        parse_activity(&con_null, &session).expect("parsea")[0].last_step,
        ""
    );
}

#[test]
fn un_started_at_o_ended_at_invalido_omite_su_tarea() {
    let session = parse_header(&header_json(&[])).expect("parsea");
    let activity = activity_json(&[
        task_row(&[("id", "\"t_mala_1\""), ("startedAt", "\"ayer\"")]),
        task_row(&[("id", "\"t_mala_2\""), ("endedAt", "1.5")]),
        task_row(&[("id", "\"t_ok\""), ("endedAt", "5000")]),
    ]);
    let tasks = parse_activity(&activity, &session).expect("parsea");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].id, "t_ok");
    assert_eq!(tasks[0].ended_at, Some(5000));
}

// ── Prefiltro por mtime ────────────────────────────────────────────────────

#[test]
fn el_prefiltro_de_mtime_omite_los_archivos_viejos() {
    let dir = temp_dir("mtime");
    let nombre = header_name(HASH, INCARNATION);
    let ruta = dir.join(&nombre);
    // Latido fresco en el contenido: si se leyera, la sesión saldría viva.
    write(&dir, &nombre, &header_json(&[]));

    let viejo = AHORA.saturating_sub(LIVE_WINDOW_MS + STALE_FILE_MARGIN_MS + 1_000);
    let archivo = fs::File::options()
        .write(true)
        .open(&ruta)
        .expect("abrir fixture");
    archivo
        .set_modified(UNIX_EPOCH + Duration::from_millis(viejo))
        .expect("fijar mtime viejo");
    assert!(
        read_live_sessions(&dir, AHORA).is_empty(),
        "un archivo más viejo que la ventana más el margen ni se lee"
    );

    // El mismo archivo, con mtime reciente, sí se lee: el contenido era válido.
    archivo
        .set_modified(UNIX_EPOCH + Duration::from_millis(AHORA))
        .expect("fijar mtime reciente");
    assert_eq!(read_live_sessions(&dir, AHORA).len(), 1);
}

// ── Rutas ──────────────────────────────────────────────────────────────────

#[test]
fn deriva_el_directorio_de_presencia() {
    let root = Path::new("/tmp/gentle-agents");
    assert_eq!(presence_dir(root), Path::new("/tmp/gentle-agents/presence"));
}
