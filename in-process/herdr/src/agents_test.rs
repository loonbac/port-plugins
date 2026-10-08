//! Tests de la lógica de subagentes. Headless: fixtures de presencia viva en un
//! directorio temporal, sin GPUI, sin tocar el contrato real de la máquina del
//! desarrollador y sin ningún registro de tarea (esa vía ya no existe).

use super::agents::*;
use super::presence::LIVE_WINDOW_MS;
use std::fs;
use std::path::{Path, PathBuf};

const HASH: &str = "5ad9a04398e3b840b64c906d0f794d3d17a140ed3ada1a0bbcded38b16de3ccc";
const OTRO_HASH: &str = "122ae8c3cd6d4a8738c9919946034a7389adaa13024606827d94bb39ee28fc58";
const INCARNATION: &str = "81a8a249-55b7-4b19-bddd-f1db0fb2dd69";
const OTRA_ENCARNACION: &str = "d345a092-4a46-4c77-a895-f4b456d1bc49";

/// Reloj inyectado en todas las pruebas.
const AHORA: u64 = 1_005_000;

// ── Fixtures ───────────────────────────────────────────────────────────────

/// Header de presencia con latido explícito.
fn header_json(hash: &str, incarnation: &str, heartbeat: u64) -> String {
    format!(
        "{{\"schema\":1,\"sessionHash\":\"{hash}\",\"incarnation\":\"{incarnation}\",\"label\":\"sesion de prueba\",\"heartbeat\":{heartbeat},\"generation\":1,\"counts\":{{\"running\":0,\"queued\":0,\"waiting\":0,\"finished\":0}},\"digest\":null,\"unavailable\":null}}"
    )
}

/// `summary` de una tarea de presencia, con los valores por defecto.
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
    let body: String = fields
        .iter()
        .map(|(k, v)| format!("\"{k}\":{v}"))
        .collect::<Vec<_>>()
        .join(",");
    format!("{{{body}}}")
}

fn task_row(overrides: &[(&str, &str)]) -> String {
    format!("{{\"summary\":{}}}", summary_json(overrides))
}

fn activity_json(hash: &str, incarnation: &str, rows: &[String]) -> String {
    format!(
        "{{\"schema\":1,\"sessionHash\":\"{hash}\",\"incarnation\":\"{incarnation}\",\"generation\":1,\"activity\":{{\"tasks\":[{}]}}}}",
        rows.join(",")
    )
}

/// Crea `<raiz>/gentle-agents/presence` y devuelve esa ruta.
fn dir(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("herdr-agents-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let presence = root.join("gentle-agents").join("presence");
    fs::create_dir_all(&presence).expect("crear presencia");
    presence
}

fn write_header(presence: &Path, hash: &str, incarnation: &str, heartbeat: u64) {
    let name = format!("{hash}.{incarnation}.header.json");
    fs::write(
        presence.join(name),
        header_json(hash, incarnation, heartbeat),
    )
    .expect("escribir header");
}

fn write_session(presence: &Path, hash: &str, incarnation: &str, heartbeat: u64, rows: &[String]) {
    write_header(presence, hash, incarnation, heartbeat);
    let name = format!("{hash}.{incarnation}.activity.json");
    fs::write(presence.join(name), activity_json(hash, incarnation, rows))
        .expect("escribir actividad");
}

/// Atajo: watcher apuntando a un directorio de presencia creado al vuelo.
fn watcher_for(presence: &Path) -> AgentWatcher {
    AgentWatcher::new(presence)
}

// ── Proyección de la presencia ─────────────────────────────────────────────

#[test]
fn la_fila_lleva_su_sesion_sin_registro() {
    // No existe ningún `tasks/<id>.json`: la identidad sale de la presencia.
    let presence = dir("identidad");
    write_session(
        &presence,
        HASH,
        INCARNATION,
        AHORA,
        &[task_row(&[("id", "\"t_abc123\"")])],
    );

    let mut watcher = watcher_for(&presence);
    let entries = watcher.entries(AHORA);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, "t_abc123");
    assert_eq!(entries[0].agent, "gentle-ai-worker");
    assert_eq!(entries[0].label, "herdr presence reader");
    assert_eq!(entries[0].status, AgentStatus::Running);
    assert_eq!(entries[0].last_activity_at, 4000);
    assert_eq!(entries[0].session_hash, HASH);
    assert_eq!(entries[0].incarnation, INCARNATION);
}

#[test]
fn el_paso_y_la_identidad_vienen_de_la_presencia() {
    let presence = dir("paso");
    write_session(
        &presence,
        HASH,
        INCARNATION,
        AHORA,
        &[task_row(&[
            ("id", "\"t_paso\""),
            ("lastStep", "\"reading the contract\""),
        ])],
    );

    let mut watcher = watcher_for(&presence);
    let entries = watcher.entries(AHORA);
    assert_eq!(entries[0].last_step, "reading the contract");
    assert_eq!(entries[0].session_hash, HASH);
    assert_eq!(entries[0].incarnation, INCARNATION);
}

#[test]
fn la_fila_sin_paso_queda_con_paso_vacio() {
    let presence = dir("sin-paso");
    write_session(
        &presence,
        HASH,
        INCARNATION,
        AHORA,
        &[task_row(&[("id", "\"t_sin_paso\""), ("lastStep", "null")])],
    );

    let mut watcher = watcher_for(&presence);
    let entries = watcher.entries(AHORA);
    assert_eq!(
        entries[0].last_step, "",
        "sin lastStep la fila se pinta igual, con paso vacío"
    );
}

#[test]
fn una_actividad_rota_no_rompe() {
    // Un archivo de actividad ilegible deja la sesión viva pero sin filas.
    let presence = dir("actividad-rota");
    write_header(&presence, HASH, INCARNATION, AHORA);
    fs::write(
        presence.join(format!("{HASH}.{INCARNATION}.activity.json")),
        "{ roto",
    )
    .expect("escribir actividad rota");

    let mut watcher = watcher_for(&presence);
    assert!(watcher.entries(AHORA).is_empty());
}

#[test]
fn un_directorio_inexistente_devuelve_lista_vacia() {
    let mut watcher = AgentWatcher::new("/no/existe/de/verdad/presence");
    assert!(watcher.entries(AHORA).is_empty());
}

// ── Política de visibilidad ────────────────────────────────────────────────

#[test]
fn un_subagente_activo_aparece() {
    let presence = dir("activo");
    write_session(
        &presence,
        HASH,
        INCARNATION,
        AHORA,
        &[task_row(&[("id", "\"t_vivo\""), ("status", "\"running\"")])],
    );

    let mut watcher = watcher_for(&presence);
    let entries = watcher.entries(AHORA);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, "t_vivo");
    assert_eq!(entries[0].status, AgentStatus::Running);
}

#[test]
fn waiting_cuenta_como_vivo() {
    let presence = dir("waiting");
    write_session(
        &presence,
        HASH,
        INCARNATION,
        AHORA,
        &[task_row(&[
            ("id", "\"t_espera\""),
            ("status", "\"waiting\""),
            ("endedAt", "null"),
        ])],
    );

    let mut watcher = watcher_for(&presence);
    let entries = watcher.entries(AHORA);
    assert_eq!(entries.len(), 1, "un subagente en espera sigue vivo");
    assert_eq!(entries[0].status, AgentStatus::Waiting);
    assert!(entries[0].status.is_live());
    assert_eq!(entries[0].status.label(), "waiting");
}

#[test]
fn el_terminado_aparece_solo_dentro_de_la_gracia() {
    let presence = dir("gracia");
    // Uno justo en el borde y otro un milisegundo más allá.
    write_session(
        &presence,
        HASH,
        INCARNATION,
        AHORA,
        &[
            task_row(&[
                ("id", "\"t_borde\""),
                ("status", "\"completed\""),
                ("endedAt", &(AHORA - FINISHED_GRACE_MS).to_string()),
                ("lastActivityAt", &(AHORA - FINISHED_GRACE_MS).to_string()),
            ]),
            task_row(&[
                ("id", "\"t_pasado\""),
                ("status", "\"completed\""),
                ("endedAt", &(AHORA - FINISHED_GRACE_MS - 1).to_string()),
                (
                    "lastActivityAt",
                    &(AHORA - FINISHED_GRACE_MS - 1).to_string(),
                ),
            ]),
        ],
    );

    let mut watcher = watcher_for(&presence);
    let ids: Vec<String> = watcher
        .entries(AHORA)
        .iter()
        .map(|entry| entry.id.clone())
        .collect();
    assert_eq!(
        ids,
        vec!["t_borde"],
        "la gracia vale exactamente 15 s y no uno más"
    );
}

#[test]
fn un_latido_viejo_no_pinta_filas() {
    // El fantasma que pintaba el diseño anterior: una tarea en `running` sin
    // ninguna sesión de pi latiendo detrás.
    let presence = dir("latido-viejo");
    write_session(
        &presence,
        HASH,
        INCARNATION,
        AHORA - LIVE_WINDOW_MS - 1,
        &[task_row(&[("id", "\"t_fantasma\"")])],
    );

    let mut watcher = watcher_for(&presence);
    assert!(
        watcher.entries(AHORA).is_empty(),
        "sin latido vivo no hay fila, aunque la tarea diga running"
    );
}

#[test]
fn un_latido_en_el_futuro_no_pinta_filas() {
    let presence = dir("latido-futuro");
    write_session(
        &presence,
        HASH,
        INCARNATION,
        AHORA + 1,
        &[task_row(&[("id", "\"t_futuro\"")])],
    );

    let mut watcher = watcher_for(&presence);
    assert!(
        watcher.entries(AHORA).is_empty(),
        "un latido posterior a ahora no está vivo"
    );
}

#[test]
fn sin_presencia_no_hay_filas() {
    // Aislamiento: con la carpeta vacía no se pinta nada, aunque en la máquina
    // del desarrollador haya subagentes reales.
    let presence = dir("vacio");
    let mut watcher = watcher_for(&presence);
    assert!(watcher.entries(AHORA).is_empty());
    assert!(watcher.visible(AHORA, DEFAULT_VISIBLE_LIMIT).is_empty());
}

#[test]
fn el_layout_no_escanea_la_presencia_en_cada_llamada() {
    let presence = dir("layout-sin-escaneo");
    write_session(
        &presence,
        HASH,
        INCARNATION,
        AHORA,
        &[task_row(&[("id", "\"a\"")])],
    );

    let mut watcher = watcher_for(&presence);
    assert_eq!(watcher.entries(AHORA).len(), 1);
    let refreshes = watcher.refreshes;
    assert_eq!(refreshes, 1);

    // Borramos el directorio de presencia
    fs::remove_dir_all(&presence).expect("borrar presencia");

    // El accessor de la instantánea que usan los hooks no debe tocar el disco ni incrementar refreshes
    for _ in 0..10 {
        assert_eq!(watcher.snapshot().len(), 1);
        assert_eq!(watcher.snapshot_visible(6).len(), 1);
    }
    assert_eq!(
        watcher.refreshes, refreshes,
        "el accessor de la instantánea no debe incrementar refreshes ni releer el disco"
    );
}

// ── Orden y límite ─────────────────────────────────────────────────────────

#[test]
fn los_vivos_van_primero_y_despues_los_mas_recientes() {
    let presence = dir("orden");
    write_session(
        &presence,
        HASH,
        INCARNATION,
        AHORA,
        &[
            task_row(&[
                ("id", "\"a\""),
                ("status", "\"running\""),
                ("lastActivityAt", "100"),
            ]),
            task_row(&[
                ("id", "\"b\""),
                ("status", "\"completed\""),
                ("endedAt", &(AHORA - 1_000).to_string()),
                ("lastActivityAt", "900"),
            ]),
            task_row(&[
                ("id", "\"c\""),
                ("status", "\"running\""),
                ("lastActivityAt", "500"),
            ]),
        ],
    );

    let mut watcher = watcher_for(&presence);
    let ids: Vec<String> = watcher
        .entries(AHORA)
        .iter()
        .map(|entry| entry.id.clone())
        .collect();
    assert_eq!(
        ids,
        vec!["c", "a", "b"],
        "vivos primero, y dentro de cada grupo por actividad reciente"
    );
}

#[test]
fn aplica_el_limite_de_filas_visibles() {
    let presence = dir("limite");
    let rows: Vec<String> = (0..12)
        .map(|n| {
            task_row(&[
                ("id", &format!("\"t{n}\"")),
                ("status", "\"running\""),
                ("lastActivityAt", &format!("{}", 1000 + n)),
            ])
        })
        .collect();
    write_session(&presence, HASH, INCARNATION, AHORA, &rows);

    let mut watcher = watcher_for(&presence);
    assert_eq!(watcher.visible(AHORA, 6).len(), 6);
    assert_eq!(watcher.visible(AHORA, 99).len(), 12);
    // El tope por defecto de la sidebar sigue existiendo.
    assert_eq!(DEFAULT_VISIBLE_LIMIT, 6);
}

// ── Caché y TTL ────────────────────────────────────────────────────────────

#[test]
fn sin_cambios_no_vuelve_a_leer_el_disco() {
    let presence = dir("cache-sin-cambio");
    write_session(
        &presence,
        HASH,
        INCARNATION,
        AHORA,
        &[task_row(&[("id", "\"a\"")])],
    );

    let mut watcher = watcher_for(&presence);
    assert_eq!(watcher.entries(AHORA).len(), 1, "la primera lectura carga");
    let after_first = watcher.refreshes;

    for _ in 0..10 {
        assert_eq!(watcher.entries(AHORA).len(), 1);
    }
    assert_eq!(
        watcher.refreshes, after_first,
        "sin cambios ni TTL cumplido no debe releerse"
    );
}

#[test]
fn un_subagente_nuevo_se_detecta_en_la_lectura_siguiente() {
    // El mtime del directorio cambia al crear un archivo nuevo: eso dispara el
    // refresco sin esperar al TTL.
    let presence = dir("cache-con-cambio");
    write_session(
        &presence,
        HASH,
        INCARNATION,
        AHORA,
        &[task_row(&[
            ("id", "\"a\""),
            ("status", "\"running\""),
            ("lastActivityAt", "10"),
        ])],
    );

    let mut watcher = watcher_for(&presence);
    assert_eq!(watcher.entries(AHORA).len(), 1);
    let before = watcher.refreshes;

    write_session(
        &presence,
        OTRO_HASH,
        OTRA_ENCARNACION,
        AHORA,
        &[task_row(&[
            ("id", "\"b\""),
            ("status", "\"running\""),
            ("lastActivityAt", "20"),
        ])],
    );

    assert_eq!(
        watcher.entries(AHORA).len(),
        2,
        "el subagente nuevo se ve de inmediato"
    );
    assert!(
        watcher.refreshes > before,
        "hubo que releer porque la carpeta cambió"
    );
}

#[test]
fn con_el_mtime_congelado_una_fila_caducada_desaparece_tras_el_ttl() {
    // El mtime no cambia, pero el tiempo sí: sin TTL la fila quedaría pintada
    // para siempre, que es el otro síntoma del fantasma.
    let presence = dir("ttl");
    write_session(
        &presence,
        HASH,
        INCARNATION,
        AHORA,
        &[task_row(&[
            ("id", "\"t_caduca\""),
            ("status", "\"completed\""),
            ("endedAt", &(AHORA - FINISHED_GRACE_MS).to_string()),
            ("lastActivityAt", &(AHORA - FINISHED_GRACE_MS).to_string()),
        ])],
    );

    let mut watcher = watcher_for(&presence);
    assert_eq!(
        watcher.entries(AHORA).len(),
        1,
        "al principio la gracia la mantiene visible"
    );
    let after_first = watcher.refreshes;

    // Medio segundo después la gracia ya expiró, pero el TTL todavía no.
    assert_eq!(
        watcher.entries(AHORA + REFRESH_TTL_MS - 1).len(),
        1,
        "dentro del TTL la caché sigue sirviendo"
    );
    assert_eq!(watcher.refreshes, after_first, "no se releyó todavía");

    // Al cumplirse el TTL se relee y la fila caducada desaparece.
    assert!(
        watcher.entries(AHORA + REFRESH_TTL_MS).is_empty(),
        "cumplido el TTL la fila obsoleta se descarta"
    );
    assert!(watcher.refreshes > after_first);
}
