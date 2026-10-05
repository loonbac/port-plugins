//! Tests de la lógica de subagentes. Headless: nada de GPUI y nada de disco real.

use super::agents::*;
use std::fs;
use std::path::{Path, PathBuf};

/// Escribe un task válido con los valores por defecto, sobrescribiendo lo que
/// haga falta para cada prueba.
fn task_json(overrides: &[(&str, &str)]) -> String {
    let mut fields: Vec<(&str, &str)> = vec![
        ("id", "\"t_abc123\""),
        ("agent", "\"gentle-ai-explore\""),
        ("label", "\"Map the auth module\""),
        ("status", "\"running\""),
        (
            "sessionPath",
            "\"/home/u/.pi/agent/gentle-agents/sessions/2026-10-05T12-41-58-266Z_01a10c0d-91c9-71c4-9cfb-cf218eae1d82.jsonl\"",
        ),
        ("lastActivityAt", "1732000005000"),
        ("turns", "2"),
        ("toolCalls", "3"),
    ];
    for (key, value) in overrides {
        if let Some(slot) = fields.iter_mut().find(|(k, _)| k == key) {
            slot.1 = value;
        }
    }
    let body: String = fields
        .iter()
        .map(|(k, v)| format!("\"{k}\":{v}"))
        .collect::<Vec<_>>()
        .join(",");
    format!("{{\"task\":{{{body}}}}}")
}

/// Crea un árbol con la misma forma que el real: `<raiz>/gentle-agents/tasks`.
/// Importa porque `sessions_dir()` deriva el directorio de sesiones como hermano
/// de `tasks/`, y una prueba con una ruta plana no ejercitaría esa derivación.
fn temp_dir(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("herdr-agents-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let dir = root.join("gentle-agents").join("tasks");
    fs::create_dir_all(&dir).expect("crear dir temporal");
    dir
}

fn write_task(dir: &Path, name: &str, body: &str) {
    fs::write(dir.join(format!("{name}.json")), body).expect("escribir task");
}

// ── Parseo ─────────────────────────────────────────────────────────────────

#[test]
fn parsea_un_task_real() {
    let entry = parse_task(&task_json(&[])).expect("debería parsear");
    assert_eq!(entry.id, "t_abc123");
    assert_eq!(entry.agent, "gentle-ai-explore");
    assert_eq!(entry.label, "Map the auth module");
    assert_eq!(entry.status, AgentStatus::Running);
    assert_eq!(entry.turns, 2);
    assert_eq!(entry.tool_calls, 3);
    assert_eq!(entry.last_activity_at, 1732000005000);
}

#[test]
fn extrae_el_uuid_del_nombre_de_sesion() {
    let path = Path::new(
        "/home/u/.pi/agent/gentle-agents/sessions/2026-10-05T12-41-58-266Z_01a10c0d-91c9-71c4-9cfb-cf218eae1d82.jsonl",
    );
    assert_eq!(
        uuid_from_session_path(path).as_deref(),
        Some("01a10c0d-91c9-71c4-9cfb-cf218eae1d82")
    );
}

#[test]
fn extrae_el_uuid_tambien_sin_timestamp() {
    // Si el nombre no lleva timestamp, el nombre ES el identificador.
    let path = Path::new("/x/y/01a10c0d-91c9-71c4-9cfb-cf218eae1d82.jsonl");
    assert_eq!(
        uuid_from_session_path(path).as_deref(),
        Some("01a10c0d-91c9-71c4-9cfb-cf218eae1d82")
    );
}

#[test]
fn un_json_roto_no_rompe() {
    assert!(parse_task("{ esto no es json").is_none());
    assert!(parse_task("").is_none());
    // Sin `id` no es un task reconocible.
    assert!(parse_task("{\"task\":{\"agent\":\"x\"}}").is_none());
}

#[test]
fn un_task_incompleto_usa_valores_neutros() {
    // Solo `id`: el resto debe salir con valores por defecto, no con un error.
    let entry = parse_task("{\"task\":{\"id\":\"t_x\"}}").expect("parsea");
    assert_eq!(entry.id, "t_x");
    assert_eq!(entry.agent, "agent");
    assert_eq!(entry.turns, 0);
    assert!(entry.session_uuid.is_none());
}

#[test]
fn un_directorio_inexistente_devuelve_lista_vacia() {
    let mut watcher = AgentWatcher::new("/no/existe/de/verdad/tasks");
    assert!(watcher.entries().is_empty());
}

// ── Orden y límite ─────────────────────────────────────────────────────────

#[test]
fn los_vivos_van_primero_y_despues_los_mas_recientes() {
    let dir = temp_dir("orden");
    write_task(
        &dir,
        "a-terminado-viejo",
        &task_json(&[
            ("id", "\"a\""),
            ("status", "\"completed\""),
            ("lastActivityAt", "100"),
        ]),
    );
    write_task(
        &dir,
        "b-terminado-nuevo",
        &task_json(&[
            ("id", "\"b\""),
            ("status", "\"completed\""),
            ("lastActivityAt", "900"),
        ]),
    );
    write_task(
        &dir,
        "c-vivo",
        &task_json(&[
            ("id", "\"c\""),
            ("status", "\"running\""),
            ("lastActivityAt", "500"),
        ]),
    );

    let mut watcher = AgentWatcher::new(&dir);
    let visible = watcher.visible(10);
    let ids: Vec<&str> = visible.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["c", "b", "a"],
        "vivo primero, luego por actividad"
    );
}

#[test]
fn aplica_el_limite_de_filas_visibles() {
    let dir = temp_dir("limite");
    for n in 0..12 {
        write_task(
            &dir,
            &format!("t{n}"),
            &task_json(&[
                ("id", &format!("\"t{n}\"")),
                ("lastActivityAt", &format!("{}", 1000 + n)),
            ]),
        );
    }
    let mut watcher = AgentWatcher::new(&dir);
    assert_eq!(watcher.visible(6).len(), 6);
    assert_eq!(watcher.visible(99).len(), 12);
}

#[test]
fn cuenta_los_vivos() {
    let dir = temp_dir("vivos");
    write_task(
        &dir,
        "a",
        &task_json(&[("id", "\"a\""), ("status", "\"running\"")]),
    );
    write_task(
        &dir,
        "b",
        &task_json(&[("id", "\"b\""), ("status", "\"queued\"")]),
    );
    write_task(
        &dir,
        "c",
        &task_json(&[("id", "\"c\""), ("status", "\"completed\"")]),
    );

    let mut watcher = AgentWatcher::new(&dir);
    assert_eq!(watcher.live_count(), 2);
}

// ── Tiempo real: la caché ──────────────────────────────────────────────────

#[test]
fn sin_cambios_no_vuelve_a_leer_el_disco() {
    let dir = temp_dir("cache-sin-cambio");
    write_task(&dir, "a", &task_json(&[("id", "\"a\"")]));

    let mut watcher = AgentWatcher::new(&dir);
    assert_eq!(watcher.entries().len(), 1, "la primera lectura carga");
    let after_first = watcher.refreshes;

    // Diez lecturas más sin tocar nada: la caché debe servir.
    for _ in 0..10 {
        assert_eq!(watcher.entries().len(), 1);
    }
    assert_eq!(
        watcher.refreshes, after_first,
        "sin cambios en la carpeta no debe releerse"
    );
}

#[test]
fn un_archivo_nuevo_se_detecta_en_la_lectura_siguiente() {
    // Este es el test que demuestra el tiempo real: escribir un subagente nuevo
    // tiene que verse en la siguiente llamada, sin que nadie lo pida.
    let dir = temp_dir("cache-con-cambio");
    write_task(
        &dir,
        "a",
        &task_json(&[("id", "\"a\""), ("status", "\"running\"")]),
    );

    let mut watcher = AgentWatcher::new(&dir);
    assert_eq!(watcher.live_count(), 1);
    let before = watcher.refreshes;

    // Llega un subagente nuevo.
    write_task(
        &dir,
        "b",
        &task_json(&[
            ("id", "\"b\""),
            ("status", "\"running\""),
            ("lastActivityAt", "9999999999999"),
        ]),
    );

    assert_eq!(
        watcher.live_count(),
        2,
        "el subagente nuevo se ve de inmediato"
    );
    assert!(
        watcher.refreshes > before,
        "hubo que releer porque la carpeta cambió"
    );
}

#[test]
fn el_comando_de_reapertura_usa_el_uuid_de_la_sesion() {
    let dir = temp_dir("comando");
    write_task(&dir, "a", &task_json(&[("id", "\"a\"")]));

    let cmd = resume_command(&sessions_dir(&dir), "01a10c0d-91c9-71c4-9cfb-cf218eae1d82");
    assert!(cmd.starts_with("pi --session-dir"), "{cmd}");
    // El UUID sale del nombre del .jsonl, no del JSON.
    assert!(
        cmd.contains("--session 01a10c0d-91c9-71c4-9cfb-cf218eae1d82"),
        "{cmd}"
    );
    // Y el directorio de sesiones es el hermano de tasks/, no tasks/ mismo.
    let expected = sessions_dir(&dir);
    assert!(
        cmd.contains(&expected.display().to_string()),
        "{cmd} \n esperado: {expected:?}"
    );
}

#[test]
fn un_task_sin_sesion_no_inventa_un_comando() {
    let dir = temp_dir("sin-sesion");
    write_task(&dir, "a", "{\"task\":{\"id\":\"a\"}}");

    let mut watcher = AgentWatcher::new(&dir);
    assert!(
        watcher.visible(10)[0].session_uuid.is_none(),
        "sin sessionPath no se puede reabrir: mejor nada que un comando roto"
    );
}
