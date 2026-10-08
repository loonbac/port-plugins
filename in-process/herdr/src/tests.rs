//! Pruebas del plugin herdr.
//!
//! Única responsabilidad: verificar desde fuera de sus módulos el estado, la
//! identidad, la vigilancia y los hooks del plugin.

use std::path::Path;

use port_plugin_api::KeyAction;
use port_term_core::frame::Rgb;
use port_term_core::input::Key;
use port_term_core::pty::RunningApp;

use crate::identity::{
    app_identity, tab_identity, ICON_CHIP, ICON_CPU, ICON_GEMINI, ICON_HEXAGON, ICON_PI, ICON_ROBOT,
};
use crate::watch::{clip_last_step, AGENT_STEP_MAX_CHARS};

use super::*;

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Presencia vacía aislada del disco real.
///
/// Con la ruta real, un subagente vivo en esta máquina abriría la barra y
/// los tests que esperan una terminal limpia fallarían sin que nada en el
/// código hubiera cambiado.
fn empty_presence(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "herdr-vacio-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("crear presencia vacia");
    dir
}

#[test]
fn parse_hex_color_valid() {
    assert_eq!(parse_hex_color("#325573"), Some(Rgb::new(0x32, 0x55, 0x73)));
    assert_eq!(parse_hex_color("325573"), Some(Rgb::new(0x32, 0x55, 0x73)));
    assert_eq!(parse_hex_color("invalid"), None);
}

#[test]
fn herdr_opens_clean_terminal_by_default() {
    // Presencia vacía: sin subagentes que fuercen la barra.
    let presence = empty_presence("limpio");
    let plugin = HerdrPlugin::with_presence_dir(&presence);
    assert_eq!(plugin.id(), "herdr");
    assert_eq!(plugin.name(), "Herdr Customization Plugin");
    // Al abrir la terminal por defecto es limpia: sin sidebar ni topbar
    assert_eq!(plugin.left_sidebar_width(), 0.0);
    assert_eq!(plugin.top_bar_height(), 0.0);
    assert!(plugin.left_sidebar().is_none());
    assert!(plugin.top_bar().is_none());
    // herdr no aporta opacidad global: la decide el plugin de la tienda.
    assert_eq!(plugin.opacity(), None);
    // Sin fondo propio: la sidebar comparte el fondo de la ventana.
    assert_eq!(plugin.state.read().unwrap().opacity, 0.0);
}

#[test]
fn ctrl_shift_t_creates_tab_in_active_space() {
    let plugin = HerdrPlugin::default();
    // Inicialmente 1 tab, sin top bar
    assert_eq!(plugin.top_bar_height(), 0.0);

    // Pulsamos Ctrl+Shift+T
    let ctrl_shift_t = Key::new("t").ctrl().shift();
    assert_eq!(plugin.on_key(&ctrl_shift_t), KeyAction::Consume);

    // Ahora hay 2 tabs en el espacio activo y el top bar aparece
    assert_eq!(plugin.top_bar_height(), 38.0);
    assert!(plugin.top_bar().is_some());
    assert!(plugin.take_new_session_request());

    // Simulamos que el core asigna session_id = 1
    plugin.on_session_created(1);
    assert_eq!(plugin.active_session(), 1);

    // Navegación con Alt+Left y Alt+Right
    let alt_left = Key::new("Left").alt();
    assert_eq!(plugin.on_key(&alt_left), KeyAction::Consume);
    assert_eq!(plugin.active_session(), 0);

    let alt_right = Key::new("Right").alt();
    assert_eq!(plugin.on_key(&alt_right), KeyAction::Consume);
    assert_eq!(plugin.active_session(), 1);

    // Cerrar pestaña activa
    assert!(plugin.close_active_tab());
    assert_eq!(plugin.active_session(), 0);
}

#[test]
fn ctrl_alt_t_creates_space_and_opens_sidebar() {
    // Presencia aislada: el test espera una terminal limpia al empezar, y
    // con la presencia real un subagente vivo ya habría abierto la barra.
    let presence = empty_presence("ctrl-alt-t");
    let plugin = HerdrPlugin::with_presence_dir(&presence);
    assert_eq!(plugin.left_sidebar_width(), 0.0);

    // Pulsamos Ctrl+Alt+T
    let ctrl_alt_t = Key::new("t").ctrl().alt();
    assert_eq!(plugin.on_key(&ctrl_alt_t), KeyAction::Consume);

    // Ahora el sidebar está abierto permanente y tiene ancho 236px
    assert_eq!(plugin.left_sidebar_width(), 236.0);
    assert!(plugin.left_sidebar().is_some());
    assert_eq!(plugin.top_bar_height(), 38.0);
    assert!(plugin.top_bar().is_some());

    // Debe haber 2 espacios: space-1 (el previo) y space-2 (el nuevo)
    let spaces = plugin.state.read().unwrap().spaces.clone();
    assert_eq!(spaces.len(), 2);
    assert_eq!(spaces[0].name, "space-1");
    assert_eq!(spaces[1].name, "space-2");
    assert_eq!(plugin.state.read().unwrap().active_space_index, 1);
    assert!(plugin.take_new_session_request());
}

#[test]
fn herdr_accent_reads_system_color() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let plugin = HerdrPlugin::default();
    let accent = plugin.state.read().unwrap().effective_accent();
    assert_eq!(accent, system_accent_color());
}

#[test]
fn el_acento_no_se_relee_en_cada_render() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let temp_home = std::env::temp_dir().join(format!(
        "herdr-accent-memo-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&temp_home);
    let accent_dir = temp_home.join(".config/mpvpaper");
    std::fs::create_dir_all(&accent_dir).expect("crear config");
    let accent_file = accent_dir.join("accent.txt");
    std::fs::write(&accent_file, "#112233\n").expect("escribir acento 1");

    let prev_home = std::env::var("HOME").ok();
    std::env::set_var("HOME", &temp_home);

    let plugin = HerdrPlugin::default();
    let c1 = plugin.state.read().unwrap().effective_accent();
    assert_eq!(c1, Rgb::new(0x11, 0x22, 0x33));

    // Cambiamos el archivo de acento en disco
    std::fs::write(&accent_file, "#445566\n").expect("escribir acento 2");

    // Segunda llamada: debe seguir en caché y no releer el archivo inmediatamente
    let c2 = plugin.state.read().unwrap().effective_accent();
    assert_eq!(
        c2,
        Rgb::new(0x11, 0x22, 0x33),
        "el acento debe mantenerse en cache dentro del TTL"
    );

    // Esperamos a que pase el TTL y refresque el hilo de fondo
    std::thread::sleep(std::time::Duration::from_millis(crate::state::ACCENT_TTL_MS + 350));

    let c3 = plugin.state.read().unwrap().effective_accent();
    assert_eq!(
        c3,
        Rgb::new(0x44, 0x55, 0x66),
        "tras vencer el TTL debe releer el nuevo acento"
    );

    if let Some(h) = prev_home {
        std::env::set_var("HOME", h);
    } else {
        std::env::remove_var("HOME");
    }
    let _ = std::fs::remove_dir_all(&temp_home);
}

#[test]
fn una_rama_git_memorizada_no_se_vuelve_a_leer() {
    let repo_dir = std::env::temp_dir().join(format!(
        "herdr-git-memo-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&repo_dir);
    let git_dir = repo_dir.join(".git");
    std::fs::create_dir_all(&git_dir).expect("crear .git");
    std::fs::write(git_dir.join("HEAD"), "ref: refs/heads/feature-memo\n").expect("escribir HEAD");

    let plugin = HerdrPlugin::default();
    plugin.update_session_cwd(0, &repo_dir, "test-repo");

    let start = std::time::Instant::now();
    while plugin.state.read().unwrap().spaces[0].branch != "feature-memo"
        && start.elapsed() < std::time::Duration::from_millis(800)
    {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    {
        let s = plugin.state.read().unwrap();
        assert_eq!(s.spaces[0].name, "test-repo");
        assert_eq!(s.spaces[0].branch, "feature-memo");
    }

    // Borramos .git/HEAD
    std::fs::remove_file(git_dir.join("HEAD")).expect("borrar HEAD");

    // Segunda llamada con el MISMO cwd: no debe releer el disco y la rama debe mantenerse
    plugin.update_session_cwd(0, &repo_dir, "test-repo");

    let s = plugin.state.read().unwrap();
    assert_eq!(
        s.spaces[0].branch, "feature-memo",
        "la rama memorizada debe conservarse sin volver a leer el disco"
    );

    let _ = std::fs::remove_dir_all(&repo_dir);
}

#[test]
fn los_hooks_de_layout_leen_la_instantanea_sin_acceder_al_disco_ni_bloquear() {
    let presence = seed_watch_agent("snapshot-no-io");
    let plugin = HerdrPlugin::with_presence_dir(&presence);

    // Los hooks devuelven el layout con la instantánea en memoria
    assert!(plugin.left_sidebar_width() > 0.0);
    assert!(plugin.left_sidebar().is_some());

    // Borramos el directorio de presencia completamente
    let _ = std::fs::remove_dir_all(&presence);

    // Los hooks siguen respondiendo sin bloquearse ni fallar porque no tocan el disco
    for _ in 0..10 {
        assert!(plugin.left_sidebar_width() > 0.0);
        assert!(plugin.left_sidebar().is_some());
    }
}

#[test]
fn el_acento_del_hook_no_relee_el_disco() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let temp_home = std::env::temp_dir().join(format!(
        "herdr-accent-no-io-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&temp_home);
    let accent_dir = temp_home.join(".config/mpvpaper");
    std::fs::create_dir_all(&accent_dir).expect("crear config");
    let accent_file = accent_dir.join("accent.txt");
    std::fs::write(&accent_file, "#112233\n").expect("escribir acento inicial");

    let prev_home = std::env::var("HOME").ok();
    std::env::set_var("HOME", &temp_home);

    let plugin = HerdrPlugin::default();
    assert_eq!(plugin.state.read().unwrap().effective_accent(), Rgb::new(0x11, 0x22, 0x33));

    // Cambiamos o borramos el archivo en disco
    std::fs::write(&accent_file, "#445566\n").expect("escribir nuevo acento");

    // Las llamadas del hook a effective_accent() deben devolver la instantánea publicada
    // sin hacer E/S de disco
    for _ in 0..10 {
        let val = plugin.state.read().unwrap().effective_accent();
        assert_eq!(
            val,
            Rgb::new(0x11, 0x22, 0x33),
            "effective_accent() debe ser lectura pura en memoria y no releer el disco"
        );
    }

    // Tras vencer el TTL y refrescar el hilo en segundo plano, el nuevo acento debe publicarse
    std::thread::sleep(std::time::Duration::from_millis(crate::state::ACCENT_TTL_MS + 350));
    let refreshed_val = plugin.state.read().unwrap().effective_accent();
    assert_eq!(
        refreshed_val,
        Rgb::new(0x44, 0x55, 0x66),
        "el hilo debe publicar el nuevo valor preservando frescura"
    );

    if let Some(h) = prev_home {
        std::env::set_var("HOME", h);
    } else {
        std::env::remove_var("HOME");
    }
    let _ = std::fs::remove_dir_all(&temp_home);
}

#[test]
fn la_rama_del_hook_no_relee_el_disco() {
    let repo_dir = std::env::temp_dir().join(format!(
        "herdr-git-no-hook-io-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let other_dir = std::env::temp_dir().join(format!(
        "herdr-git-no-hook-other-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&repo_dir);
    let _ = std::fs::remove_dir_all(&other_dir);
    let git_dir = repo_dir.join(".git");
    std::fs::create_dir_all(&git_dir).expect("crear .git");
    std::fs::create_dir_all(&other_dir).expect("crear other");
    std::fs::write(git_dir.join("HEAD"), "ref: refs/heads/feature-async\n").expect("escribir HEAD");

    let plugin = HerdrPlugin::default();
    plugin.update_session_cwd(0, &repo_dir, "test-repo");

    // Esperamos a que el hilo en segundo plano publique la rama detectada
    let start = std::time::Instant::now();
    while plugin.state.read().unwrap().spaces[0].branch != "feature-async"
        && start.elapsed() < std::time::Duration::from_millis(800)
    {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(plugin.state.read().unwrap().spaces[0].branch, "feature-async");

    // Borramos .git por completo
    std::fs::remove_dir_all(&git_dir).expect("borrar .git");

    let reads_before = crate::identity::GIT_BRANCH_READS.load(std::sync::atomic::Ordering::SeqCst);

    // Llamamos update_session_cwd para ese cwd y para otro desconocido
    for _ in 0..10 {
        plugin.update_session_cwd(0, &repo_dir, "test-repo");
        plugin.update_session_cwd(0, &other_dir, "other-repo");
    }

    let reads_after = crate::identity::GIT_BRANCH_READS.load(std::sync::atomic::Ordering::SeqCst);

    // El hook update_session_cwd NO debe invocar detect_git_branch en ningún caso
    assert_eq!(
        reads_after, reads_before,
        "update_session_cwd no debe realizar lecturas de git sincrónicas"
    );

    let _ = std::fs::remove_dir_all(&repo_dir);
    let _ = std::fs::remove_dir_all(&other_dir);
}

#[test]
fn herdr_space_updates_name_and_branch_from_cwd() {
    let plugin = HerdrPlugin::default();
    let path = std::path::Path::new("/home/loonbac/Proyectos/port");
    plugin.update_session_cwd(0, path, "port");

    let start = std::time::Instant::now();
    while plugin.state.read().unwrap().spaces[0].branch != "master"
        && start.elapsed() < std::time::Duration::from_millis(800)
    {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    let s = plugin.state.read().unwrap();
    assert_eq!(s.spaces[0].name, "port");
    assert_eq!(s.spaces[0].branch, "master");
}

#[test]
fn ai_agents_get_their_nerd_font_icon_and_readable_name() {
    let cases = [
        ("pi", ICON_PI, "Pi Agent"),
        ("codex", ICON_CHIP, "Codex"),
        ("claude", ICON_ROBOT, "Claude Code"),
        ("antigravity", ICON_HEXAGON, "Antigravity"),
        ("opencode", ICON_CPU, "OpenCode"),
        ("gemini", ICON_GEMINI, "Gemini CLI"),
    ];
    for (bin, icon, label) in cases {
        let app = RunningApp {
            pid: 1,
            bin: bin.to_string(),
        };
        let (got_icon, got_label) = app_identity(&app);
        assert_eq!(got_icon, icon, "icono incorrecto para {bin}");
        assert_eq!(got_label, label, "nombre incorrecto para {bin}");
    }
}

#[test]
fn a_new_tab_never_inherits_the_app_of_another_session() {
    let plugin = HerdrPlugin::default();

    // La primera pestaña corresponde a la sesión 0 real.
    let pi = RunningApp {
        pid: 1,
        bin: "pi".to_string(),
    };
    plugin.update_session_app(0, Some(&pi));

    // Se abre una pestaña nueva: nace sin sesión asignada.
    let ctrl_shift_t = Key::new("t").ctrl().shift();
    plugin.on_key(&ctrl_shift_t);
    assert!(plugin.take_new_session_request());
    plugin.on_session_created(1);

    // La sesión 0 sigue siendo pi, pero la pestaña nueva no puede heredarlo.
    plugin.update_session_app(1, None);

    let s = plugin.state.read().unwrap();
    assert_eq!(s.spaces[0].tabs[0].app, Some(pi));
    assert_eq!(
        s.spaces[0].tabs[1].app, None,
        "una pestaña nueva no debe mostrar la app de otra sesión"
    );
    assert_eq!(s.spaces[0].tabs[1].session_id, 1);
}

#[test]
fn sidebar_width_is_clamped_to_usable_bounds() {
    let plugin = HerdrPlugin::default();
    assert_eq!(plugin.sidebar_width(), SIDEBAR_DEFAULT_WIDTH);

    plugin.set_sidebar_width(9999.0);
    assert_eq!(plugin.sidebar_width(), SIDEBAR_MAX_WIDTH);

    plugin.set_sidebar_width(10.0);
    assert_eq!(plugin.sidebar_width(), SIDEBAR_MIN_WIDTH);

    plugin.set_sidebar_width(320.0);
    assert_eq!(plugin.sidebar_width(), 320.0);
}

#[test]
fn sidebar_width_is_persisted_in_config() {
    let plugin = HerdrPlugin::default();
    plugin.set_sidebar_width(300.0);
    let saved = plugin.save_config().unwrap();
    assert_eq!(saved.get_f32("sidebar_width"), Some(300.0));

    // Una sesión nueva debe recuperar el ancho guardado.
    let other = HerdrPlugin::default();
    other.load_config(&saved);
    assert_eq!(other.sidebar_width(), 300.0);
}

#[test]
fn dragging_the_handle_changes_the_width_from_the_cursor_delta() {
    let plugin = HerdrPlugin::default();
    plugin.set_sidebar_width(236.0);
    plugin.begin_resize(236.0);
    assert!(plugin.is_resizing());

    // Arrastrar 60 px a la derecha ensancha la barra 60 px.
    assert!(plugin.update_resize(296.0));
    assert_eq!(plugin.sidebar_width(), 296.0);

    // Arrastrar de vuelta la devuelve al ancho original.
    assert!(plugin.update_resize(236.0));
    assert_eq!(plugin.sidebar_width(), 236.0);

    plugin.end_resize();
    assert!(!plugin.is_resizing());
    assert!(
        !plugin.update_resize(500.0),
        "sin arrastre activo no debe cambiar"
    );
    assert_eq!(plugin.sidebar_width(), 236.0);
}

#[test]
fn dragging_left_narrows_the_sidebar_down_to_the_limit() {
    let plugin = HerdrPlugin::default();
    plugin.set_sidebar_width(300.0);
    plugin.begin_resize(300.0);

    // Estrechar: el cursor avanza en X negativas respecto al asidero.
    assert!(plugin.update_resize(260.0));
    assert_eq!(plugin.sidebar_width(), 260.0);

    assert!(plugin.update_resize(200.0));
    assert_eq!(plugin.sidebar_width(), 200.0);

    // Más allá del mínimo queda fijado en el límite inferior.
    assert!(plugin.update_resize(40.0));
    assert_eq!(plugin.sidebar_width(), SIDEBAR_MIN_WIDTH);
    assert!(plugin.update_resize(0.0));
    assert_eq!(plugin.sidebar_width(), SIDEBAR_MIN_WIDTH);
}

#[test]
fn closing_a_tab_removes_it_and_deletes_the_space_when_empty() {
    let plugin = HerdrPlugin::default();
    assert_eq!(plugin.state.read().unwrap().spaces.len(), 1);

    // Se abre una segunda pestaña con su propia sesión.
    let ctrl_shift_t = Key::new("t").ctrl().shift();
    plugin.on_key(&ctrl_shift_t);
    assert!(plugin.take_new_session_request());
    plugin.on_session_created(1);
    assert_eq!(
        plugin.state.read().unwrap().spaces[0].tabs.len(),
        2,
        "deben existir dos pestañas en el espacio"
    );

    // Se cierra la segunda pestaña: el espacio conserva su otra pestaña.
    plugin.on_session_closed(1);
    let s = plugin.state.read().unwrap();
    assert_eq!(s.spaces.len(), 1, "el espacio sigue vivo con una pestaña");
    assert_eq!(s.spaces[0].tabs.len(), 1);

    // Se cierra la última: el espacio deja de existir y se vuelve al inicial.
    drop(s);
    plugin.on_session_closed(0);
    let s = plugin.state.read().unwrap();
    assert_eq!(
        s.spaces[0].tabs.len(),
        1,
        "un espacio sin pestañas no debe quedar en la barra lateral"
    );
    assert_eq!(s.active_space_index, 0);
}

#[test]
fn unknown_agents_fall_back_to_robot_and_humanized_name() {
    let app = RunningApp {
        pid: 1,
        bin: "miagente".to_string(),
    };
    let (icon, label) = app_identity(&app);
    assert_eq!(icon, ICON_ROBOT);
    assert_eq!(label, "Miagente");
}

#[test]
fn running_app_updates_the_matching_tab() {
    let plugin = HerdrPlugin::default();
    // Se crean dos pestañas en el espacio 1
    let ctrl_shift_t = Key::new("t").ctrl().shift();
    plugin.on_key(&ctrl_shift_t);
    assert!(plugin.take_new_session_request());
    plugin.on_session_created(1);

    let app = RunningApp {
        pid: 42,
        bin: "pi".to_string(),
    };
    plugin.update_session_app(1, Some(&app));

    let s = plugin.state.read().unwrap();
    assert_eq!(s.spaces[0].tabs[1].app, Some(app));
    assert_eq!(s.spaces[0].tabs[0].app, None);
}

// ── Visor por clic ─────────────────────────────────────────────────────

const WATCH_HASH: &str = "5ad9a04398e3b840b64c906d0f794d3d17a140ed3ada1a0bbcded38b16de3ccc";
const WATCH_INCARNATION: &str = "81a8a249-55b7-4b19-bddd-f1db0fb2dd69";

/// Presencia viva con una tarea, en un directorio temporal.
///
/// No hay ningún `tasks/<id>.json`: la identidad y el paso salen de la
/// presencia, que es la única fuente que existe mientras el subagente corre.
/// Devuelve la ruta de presencia que espera el plugin.
fn seed_watch_agent(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "herdr-viewer-{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let presence = root.join("gentle-agents").join("presence");
    std::fs::create_dir_all(&presence).expect("crear presencia");

    let now = agents::now_ms();
    let header = format!(
        "{{\"schema\":1,\"sessionHash\":\"{WATCH_HASH}\",\"incarnation\":\"{WATCH_INCARNATION}\",\"label\":\"sesion de prueba\",\"heartbeat\":{now},\"generation\":1,\"unavailable\":null}}"
    );
    std::fs::write(
        presence.join(format!("{WATCH_HASH}.{WATCH_INCARNATION}.header.json")),
        header,
    )
    .expect("escribir header");
    let activity = format!(
        "{{\"schema\":1,\"sessionHash\":\"{WATCH_HASH}\",\"incarnation\":\"{WATCH_INCARNATION}\",\"generation\":1,\"activity\":{{\"tasks\":[{{\"summary\":{{\"id\":\"t_abc123\",\"agent\":\"gentle-ai-worker\",\"label\":\"herdr viewer\",\"status\":\"running\",\"lastStep\":\"reading the presence\",\"createdAt\":1,\"startedAt\":2,\"endedAt\":null,\"lastActivityAt\":{now}}}}}]}}}}"
    );
    std::fs::write(
        presence.join(format!("{WATCH_HASH}.{WATCH_INCARNATION}.activity.json")),
        activity,
    )
    .expect("escribir actividad");

    presence
}

/// Hash e incarnación del segundo subagente de las pruebas de doble clic.
const WATCH_HASH_B: &str = "b7c1e5a9d3f24780b6c2e8d4a1f35c9e7b0d2a4c6e8f1b3d5a7c9e0f2b4d6a8c";
const WATCH_INCARNATION_B: &str = "2b9d1e77-0f3a-4c6e-8b21-9d4f6a2c5e70";

/// Presencia viva con DOS subagentes distintos, para distinguir dos clics.
///
/// La identidad de sesión y la etiqueta difieren, así que la petición del visor
/// se puede atribuir a uno u otro sin ambigüedad.
fn seed_two_watch_agents(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "herdr-viewer2-{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let presence = root.join("gentle-agents").join("presence");
    std::fs::create_dir_all(&presence).expect("crear presencia");

    let now = agents::now_ms();
    for (hash, incarnation, label, task) in [
        (WATCH_HASH, WATCH_INCARNATION, "herdr viewer A", "t_a"),
        (WATCH_HASH_B, WATCH_INCARNATION_B, "herdr viewer B", "t_b"),
    ] {
        let header = format!(
            "{{\"schema\":1,\"sessionHash\":\"{hash}\",\"incarnation\":\"{incarnation}\",\"label\":\"sesion de prueba\",\"heartbeat\":{now},\"generation\":1,\"unavailable\":null}}"
        );
        std::fs::write(
            presence.join(format!("{hash}.{incarnation}.header.json")),
            header,
        )
        .expect("escribir header");
        let activity = format!(
            "{{\"schema\":1,\"sessionHash\":\"{hash}\",\"incarnation\":\"{incarnation}\",\"generation\":1,\"activity\":{{\"tasks\":[{{\"summary\":{{\"id\":\"{task}\",\"agent\":\"gentle-ai-worker\",\"label\":\"{label}\",\"status\":\"running\",\"lastStep\":\"reading the presence\",\"createdAt\":1,\"startedAt\":2,\"endedAt\":null,\"lastActivityAt\":{now}}}}}]}}}}"
        );
        std::fs::write(
            presence.join(format!("{hash}.{incarnation}.activity.json")),
            activity,
        )
        .expect("escribir actividad");
    }

    presence
}

#[test]
fn el_ultimo_paso_se_recorta_a_ochenta_caracteres() {
    assert_eq!(
        clip_last_step("  reading   the  presence "),
        "reading the presence",
        "una sola línea, sin extremos"
    );
    let largo = "a".repeat(120);
    let corto = clip_last_step(&largo);
    assert_eq!(corto.chars().count(), AGENT_STEP_MAX_CHARS + 1);
    assert!(corto.ends_with('…'), "se corta con elipsis");
    assert!(!clip_last_step("queued").ends_with('…'));
}

#[test]
fn con_un_subagente_vivo_la_sidebar_reserva_su_ancho() {
    let presence = seed_watch_agent("ancho");
    let plugin = HerdrPlugin::with_presence_dir(&presence);

    assert_eq!(
        plugin.state.read().unwrap().spaces.len(),
        1,
        "el caso que fallaba es el de un solo espacio"
    );
    // La barra se pinta, así que tiene que reservar ancho. Sin este ancho
    // PORT la dibuja fuera de la caja del layout: se ve, pero ninguna de sus
    // filas recibe el clic (el ancho y el dibujo tienen que coincidir).
    assert!(
        plugin.left_sidebar_width() > 0.0,
        "una sidebar visible tiene que reservar su ancho"
    );
    assert!(plugin.left_sidebar().is_some());
}

#[test]
fn hacer_clic_en_un_subagente_ocupa_la_pestana_activa_sin_crear_nada() {
    let presence = seed_watch_agent("clic");
    let plugin = HerdrPlugin::with_presence_dir(&presence);

    // La fila visible es la que el clic sigue.
    let entry = plugin
        .visible_agents(1)
        .into_iter()
        .next()
        .expect("un subagente visible");
    assert_eq!(entry.session_hash, WATCH_HASH);
    assert_eq!(entry.incarnation, WATCH_INCARNATION);
    assert_eq!(entry.last_step, "reading the presence");
    let expected_label = entry.label.clone();
    assert_eq!(expected_label, "herdr viewer");

    let (espacios_antes, pestanas_antes, id_antes, titulo_antes) = {
        let s = plugin.state.read().unwrap();
        let space = &s.spaces[0];
        (
            s.spaces.len(),
            space.tabs.len(),
            space.tabs[0].id,
            space.tabs[0].title.clone(),
        )
    };
    assert!(plugin.watch_entry(&entry));

    // El clic no crea nada: mismo espacio, misma pestaña, mismo id.
    {
        let s = plugin.state.read().unwrap();
        assert_eq!(
            s.spaces.len(),
            espacios_antes,
            "el visor no crea un espacio"
        );
        assert_eq!(s.active_space_index, 0, "seguimos en el espacio activo");
        let space = &s.spaces[s.active_space_index];
        assert_eq!(
            space.tabs.len(),
            pestanas_antes,
            "el visor no agrega pestañas"
        );
        assert_eq!(
            space.active_tab_index, 0,
            "la pestaña activa es la de siempre"
        );
        let tab = &space.tabs[0];
        assert_eq!(tab.id, id_antes, "la pestaña conserva su identidad");
        // Aún no existe la sesión del visor: la pestaña sigue mostrando el
        // terminal original, pero ya guarda lo necesario para volver a él.
        assert_eq!(tab.session_id, 0, "el terminal original sigue a la vista");
        let previo = tab.watching.as_ref().expect("la pestaña apunta al visor");
        assert_eq!(previo.session_id, Some(0), "se guardó la sesión original");
        assert_eq!(previo.title, titulo_antes, "se guardó el título original");
        assert!(!previo.title_locked, "se guardó el bloqueo original");
        assert_eq!(
            s.pending_watch
                .as_ref()
                .expect("la petición del visor")
                .agent_label,
            expected_label,
            "la petición lleva la etiqueta del subagente"
        );
        // El resto del espacio conserva su terminal de siempre.
        assert_eq!(space.name, "space-1");
    }
    assert!(
        !plugin.take_new_session_request(),
        "el visor nunca pide un shell por defecto"
    );

    // El núcleo consume la petición: el comando lleva la identidad de la
    // sesión y el directorio de presencia, no una ruta de registro.
    let config = plugin
        .take_spawn_session_request()
        .expect("el clic debe pedir una sesión de visor");
    assert_eq!(config.command, "bash");
    assert_eq!(config.args[2], "herdr-view");
    assert_eq!(config.args[3], presence.display().to_string());
    assert_eq!(config.args[4], WATCH_HASH);
    assert_eq!(config.args[5], WATCH_INCARNATION);
    assert_eq!(config.args[6], expected_label);
    // Mientras el núcleo no cree la sesión, la petición sigue en vuelo y el
    // host puede reintentarla; se agota recién al crear la sesión.
    assert!(
        plugin.take_spawn_session_request().is_some(),
        "la petición viva se reintenta"
    );
    plugin.on_session_created(7);
    assert!(
        plugin.take_spawn_session_request().is_none(),
        "una petición de visor se consume al crear su sesión"
    );
}

#[test]
fn crear_la_sesion_del_visor_enlaza_la_pestana_activa_y_bloquea_su_titulo() {
    let presence = seed_watch_agent("enlace");
    let plugin = HerdrPlugin::with_presence_dir(&presence);
    let entry = plugin
        .visible_agents(1)
        .into_iter()
        .next()
        .expect("un subagente visible");
    let expected_label = entry.label.clone();
    assert!(plugin.watch_entry(&entry));
    assert!(plugin.take_spawn_session_request().is_some());

    // El núcleo crea la sesión del visor: la MISMA pestaña activa toma su id
    // y la etiqueta del subagente, y queda con el título bloqueado.
    plugin.on_session_created(7);
    {
        let s = plugin.state.read().unwrap();
        let space = &s.spaces[0];
        assert_eq!(space.tabs.len(), 1, "no aparece ninguna pestaña nueva");
        let tab = &space.tabs[0];
        assert_eq!(
            tab.session_id, 7,
            "la pestaña activa pasa a mostrar el visor"
        );
        assert_eq!(tab.title, expected_label, "el título ES el subagente");
        assert!(tab.title_locked, "la pestaña del visor no se renombra");
        assert_eq!(
            tab.watching
                .as_ref()
                .expect("sigue apuntando al original")
                .session_id,
            Some(0),
            "el terminal original sigue guardado para volver a él"
        );
        assert!(s.pending_viewer_title.is_none(), "la etiqueta se agota");
    }
    // El área principal dibuja solo el visor desde que su sesión existe.
    assert_eq!(
        plugin.active_session(),
        7,
        "el núcleo dibuja la sesión del visor, no la del terminal original"
    );
}

#[test]
fn cerrar_la_sesion_del_visor_restaura_el_terminal_original() {
    let presence = seed_watch_agent("restaura");
    let plugin = HerdrPlugin::with_presence_dir(&presence);
    // Título original no trivial: la restauración tiene que devolverlo tal cual.
    plugin.state.write().unwrap().spaces[0].tabs[0].title = "mi terminal".to_string();

    let entry = plugin
        .visible_agents(1)
        .into_iter()
        .next()
        .expect("un subagente visible");
    assert!(plugin.watch_entry(&entry));
    assert!(plugin.take_spawn_session_request().is_some());
    plugin.on_session_created(7);

    plugin.on_session_closed(7);

    let s = plugin.state.read().unwrap();
    assert_eq!(
        s.spaces.len(),
        1,
        "el espacio no se borra al salir del visor"
    );
    let space = &s.spaces[0];
    assert_eq!(space.tabs.len(), 1, "la pestaña se restaura, no se cierra");
    let tab = &space.tabs[0];
    assert_eq!(tab.session_id, 0, "vuelve el terminal original");
    assert_eq!(tab.title, "mi terminal", "vuelve el título original");
    assert!(!tab.title_locked, "vuelve el bloqueo original");
    assert!(
        tab.watching.is_none(),
        "la pestaña deja de apuntar al visor"
    );
}

#[test]
fn escape_y_ctrl_w_en_el_visor_piden_cerrar_su_sesion() {
    let presence = seed_watch_agent("salida");
    let plugin = HerdrPlugin::with_presence_dir(&presence);
    let entry = plugin
        .visible_agents(1)
        .into_iter()
        .next()
        .expect("un subagente visible");

    // Escape sin modificadores pide cerrar la sesión del visor, consume y
    // restaura la pestaña sin esperar al callback del núcleo.
    assert!(plugin.watch_entry(&entry));
    assert!(plugin.take_spawn_session_request().is_some());
    plugin.on_session_created(7);
    assert_eq!(plugin.on_key(&Key::new("Escape")), KeyAction::Consume);
    assert_eq!(plugin.take_close_session_request(), Some(7));
    {
        let s = plugin.state.read().unwrap();
        assert_eq!(s.spaces[0].tabs.len(), 1, "la pestaña no se borra");
        assert!(s.spaces[0].tabs[0].watching.is_none(), "vuelve al terminal");
        assert_eq!(s.spaces[0].tabs[0].session_id, 0);
    }

    // Ctrl+W hace exactamente lo mismo.
    assert!(plugin.watch_entry(&entry));
    assert!(plugin.take_spawn_session_request().is_some());
    plugin.on_session_created(7);
    assert_eq!(plugin.on_key(&Key::new("w").ctrl()), KeyAction::Consume);
    assert_eq!(plugin.take_close_session_request(), Some(7));
    assert_eq!(plugin.state.read().unwrap().spaces[0].tabs.len(), 1);
    assert_eq!(plugin.state.read().unwrap().spaces[0].tabs[0].session_id, 0);

    // close_active_tab tampoco borra una pestaña ocupada por el visor.
    assert!(plugin.watch_entry(&entry));
    assert!(plugin.take_spawn_session_request().is_some());
    plugin.on_session_created(7);
    assert!(plugin.close_active_tab());
    assert_eq!(plugin.take_close_session_request(), Some(7));
    assert_eq!(plugin.state.read().unwrap().spaces[0].tabs.len(), 1);
    assert_eq!(plugin.state.read().unwrap().spaces[0].tabs[0].session_id, 0);

    // Escape con modificadores no es la salida del visor.
    assert_eq!(plugin.on_key(&Key::new("Escape").ctrl()), KeyAction::Pass);
    assert!(plugin.take_close_session_request().is_none());
}

#[test]
fn los_atajos_normales_de_pestana_no_cambian() {
    let plugin = HerdrPlugin::default();
    // Escape en una pestaña normal sigue su curso hacia el PTY.
    assert_eq!(plugin.on_key(&Key::new("Escape")), KeyAction::Pass);
    // Ctrl+W sigue cerrando la pestaña activa cuando hay más de una.
    plugin.create_tab_in_active_space();
    assert!(plugin.take_new_session_request());
    plugin.on_session_created(1);
    assert_eq!(plugin.on_key(&Key::new("w").ctrl()), KeyAction::Consume);
    assert_eq!(plugin.take_close_session_request(), Some(1));
    let s = plugin.state.read().unwrap();
    assert_eq!(s.spaces[0].tabs.len(), 1, "la pestaña normal sí se cierra");
    assert_eq!(s.spaces[0].tabs[0].session_id, 0);
}

#[test]
fn una_fila_sin_identidad_no_pide_visor() {
    let presence = seed_watch_agent("sin-identidad");
    let plugin = HerdrPlugin::with_presence_dir(&presence);

    // La presencia siempre trae identidad; se fuerza una fila sin ella para
    // ejercitar la guarda del clic.
    let entry = agents::AgentEntry {
        id: "t_huerfana".to_string(),
        agent: "gentle-ai-worker".to_string(),
        label: "huerfana".to_string(),
        status: agents::AgentStatus::Running,
        last_activity_at: 1,
        last_step: String::new(),
        session_hash: String::new(),
        incarnation: String::new(),
    };
    assert!(
        !plugin.watch_entry(&entry),
        "sin identidad no hay nada que ver"
    );
    assert!(
        plugin.take_spawn_session_request().is_none(),
        "sin identidad no se pide sesión"
    );
    assert_eq!(
        plugin.state.read().unwrap().spaces[0].tabs.len(),
        1,
        "tampoco se crea una pestaña vacía"
    );
}

#[test]
fn el_directorio_y_la_app_del_visor_no_tocan_la_pestana_ocupada() {
    let presence = seed_watch_agent("titulo");
    let plugin = HerdrPlugin::with_presence_dir(&presence);
    let entry = plugin
        .visible_agents(1)
        .into_iter()
        .next()
        .expect("un subagente visible");
    let expected_label = entry.label.clone();
    assert!(plugin.watch_entry(&entry));
    assert!(plugin.take_spawn_session_request().is_some());
    plugin.on_session_created(7);

    // El shell del visor informa su carpeta y su app: ni el espacio del
    // usuario se renombra ni la pestaña ocupada pierde el nombre del subagente.
    plugin.update_session_cwd(7, Path::new("/home/loonbac/Proyectos/port"), "port");
    plugin.update_session_app(
        7,
        Some(&RunningApp {
            pid: 1,
            bin: "bash".to_string(),
        }),
    );

    let s = plugin.state.read().unwrap();
    let space = &s.spaces[0];
    assert_eq!(
        space.name, "space-1",
        "el cwd de la sesión visora no renombra el espacio del usuario"
    );
    let tab = &space.tabs[0];
    assert_eq!(
        tab.title, expected_label,
        "el título bloqueado sigue siendo el subagente"
    );
    assert_eq!(tab.app, None, "la app del visor no sustituye al subagente");
    drop(s);

    // Un terminal normal (sin visor) sí sigue renombrando su espacio.
    let normal = HerdrPlugin::default();
    normal.update_session_cwd(0, Path::new("/home/loonbac/Proyectos/port"), "port");
    assert_eq!(normal.state.read().unwrap().spaces[0].name, "port");
}

#[test]
fn la_pestana_con_titulo_bloqueado_ignora_la_app_en_primer_plano() {
    // Regresión de "Sleep": el visor corre `bash -c tail|jq`, así que su
    // app en primer plano no debe sustituir el nombre del subagente que
    // pinta la píldora de la barra superior.
    let bloqueada = HerdrTab {
        id: 1,
        title: "Sleep".to_string(),
        session_id: 7,
        app: Some(RunningApp {
            pid: 1,
            bin: "bash".to_string(),
        }),
        title_locked: true,
        watching: None,
    };
    assert_eq!(
        tab_identity(&bloqueada),
        (None, "Sleep".to_string()),
        "el título fijado manda sobre la app en primer plano"
    );

    // Una pestaña normal conserva icono y nombre derivados de su app.
    let normal = HerdrTab {
        title_locked: false,
        ..bloqueada.clone()
    };
    assert_eq!(
        tab_identity(&normal),
        (Some(ICON_ROBOT), "Bash".to_string())
    );
}

// ── Enlace de sesiones por id ──────────────────────────────────────────

#[test]
fn cambiar_de_espacio_antes_de_crear_la_sesion_enlaza_la_pestana_que_la_pidio() {
    let plugin = HerdrPlugin::default();

    // Ctrl+Alt+T crea space-2 con una pestaña pendiente que pidió su shell.
    plugin.create_space_and_open_sidebar();
    // El usuario se cambia a space-1 antes de que el núcleo cree la sesión.
    plugin.select_space(0);

    // El núcleo crea la sesión del id que la pidió, no de la pestaña activa.
    assert!(plugin.take_new_session_request());
    plugin.on_session_created(1);

    let s = plugin.state.read().unwrap();
    assert_eq!(
        s.spaces[1].tabs[0].session_id, 1,
        "la sesión va a la pestaña que la pidió, no a la activa"
    );
    assert_eq!(
        s.spaces[0].tabs[0].session_id, 0,
        "la terminal de space-1 conserva su sesión"
    );
    assert_eq!(s.active_space_index, 0, "seguimos en space-1");
    drop(s);
    assert_eq!(
        plugin.active_session(),
        0,
        "el área principal sigue dibujando la sesión de space-1"
    );
}

#[test]
fn el_visor_no_deja_la_pestana_sin_sesion_cuando_su_terminal_aun_no_existe() {
    let presence = seed_watch_agent("pendiente");
    let plugin = HerdrPlugin::with_presence_dir(&presence);

    // space-2 nace con una pestaña pendiente y el usuario pulsa la fila de
    // AGENTS antes de que su terminal exista.
    plugin.create_space_and_open_sidebar();
    let entry = plugin
        .visible_agents(1)
        .into_iter()
        .next()
        .expect("un subagente visible");
    assert!(plugin.watch_entry(&entry));

    // Frame del núcleo: primero el shell pendiente del espacio, después el
    // visor que retargeteó la misma pestaña.
    assert!(plugin.take_new_session_request());
    plugin.on_session_created(1);
    assert!(plugin.take_spawn_session_request().is_some());
    plugin.on_session_created(7);

    // El visor se cierra: la pestaña no tenía terminal, así que pide uno fresco
    // en lugar de quedarse con el centinela.
    plugin.on_session_closed(7);

    // El núcleo crea el shell fresco que la pestaña pidió.
    assert!(
        plugin.take_new_session_request(),
        "la pestaña restaurada debe pedir un shell"
    );
    plugin.on_session_created(8);

    let s = plugin.state.read().unwrap();
    assert_eq!(s.spaces.len(), 2, "los dos espacios siguen vivos");
    let space = s.spaces.get(1).expect("space-2 sigue existiendo");
    let tab = &space.tabs[0];
    assert_ne!(
        tab.session_id, PENDING_SESSION,
        "la pestaña nunca queda con el centinela pegado"
    );
    assert_eq!(tab.session_id, 8, "la pestaña recibe un terminal real");
}

#[test]
fn cerrar_una_sesion_reajusta_los_indices_activos() {
    let plugin = HerdrPlugin::default();

    // Tres espacios con sesiones vivas.
    plugin.create_space_and_open_sidebar();
    plugin.create_space_and_open_sidebar();
    assert!(plugin.take_new_session_request());
    plugin.on_session_created(1);
    assert!(plugin.take_new_session_request());
    plugin.on_session_created(2);

    // El usuario mira el espacio del medio.
    plugin.select_space(1);
    let activo_antes = plugin.state.read().unwrap().spaces[1].name.clone();
    assert_eq!(activo_antes, "space-2");

    // Se cierra la sesión del primer espacio: su pestaña y el espacio se van.
    plugin.on_session_closed(0);

    let s = plugin.state.read().unwrap();
    assert_eq!(s.spaces.len(), 2, "queda un espacio por cada sesión viva");
    assert_eq!(
        s.spaces[s.active_space_index].name, activo_antes,
        "el espacio activo es el MISMO, no el que quedó en su índice"
    );
    for space in &s.spaces {
        assert!(
            space.active_tab_index < space.tabs.len(),
            "el índice de pestaña activa no puede quedar fuera de rango en {}",
            space.name
        );
    }
}

#[test]
fn el_boton_mas_del_topbar_no_hereda_la_sesion_de_otra_pestana() {
    let plugin = HerdrPlugin::default();

    // El `+` del topbar crea su pestaña por el mismo camino del estado.
    plugin
        .state
        .write()
        .unwrap()
        .create_pending_tab_in_active_space();

    {
        let s = plugin.state.read().unwrap();
        let tab = s.spaces[0].tabs.last().expect("la pestaña nueva");
        assert_eq!(
            tab.session_id, PENDING_SESSION,
            "la pestaña del topbar no puede nacer con la sesión de otra"
        );
    }

    // El núcleo le asigna su propia sesión.
    assert!(plugin.take_new_session_request());
    plugin.on_session_created(1);

    let s = plugin.state.read().unwrap();
    let tab = s.spaces[0].tabs.last().expect("la pestaña nueva");
    assert_eq!(tab.session_id, 1, "la sesión real llega a la pestaña nueva");
    assert_eq!(
        s.spaces[0].tabs[0].session_id, 0,
        "la terminal original conserva la 0"
    );
}

#[test]
fn cerrar_una_pestana_pendiente_no_deja_su_shell_en_otra_pestana() {
    let plugin = HerdrPlugin::default();

    // El topbar abre una pestaña pendiente.
    plugin
        .state
        .write()
        .unwrap()
        .create_pending_tab_in_active_space();
    // El usuario la cierra antes de que el núcleo cree su shell.
    assert!(plugin.close_active_tab());

    assert!(
        !plugin.take_new_session_request(),
        "la petición de la pestaña cerrada no debe sobrevivir"
    );
    assert_eq!(
        plugin.state.read().unwrap().spaces[0].tabs[0].session_id, 0,
        "el terminal preexistente conserva su sesión"
    );

    // Una pestaña legítima nueva sí pide y recibe su propia sesión.
    plugin
        .state
        .write()
        .unwrap()
        .create_pending_tab_in_active_space();
    assert!(plugin.take_new_session_request());
    plugin.on_session_created(9);

    let s = plugin.state.read().unwrap();
    let tabs = &s.spaces[0].tabs;
    assert_eq!(tabs.len(), 2, "solo sobreviven la vieja y la nueva");
    assert_eq!(
        tabs.last().unwrap().session_id,
        9,
        "la sesión cae en la pestaña nueva"
    );
    assert_eq!(tabs[0].session_id, 0, "la pestaña vieja no se toca");
}

#[test]
fn cerrar_la_pestana_del_visor_antes_de_crear_su_sesion_cancela_el_visor() {
    let presence = seed_watch_agent("cancela");
    let plugin = HerdrPlugin::with_presence_dir(&presence);

    // Una segunda pestaña para poder cerrar la que el visor va a ocupar.
    plugin
        .state
        .write()
        .unwrap()
        .create_pending_tab_in_active_space();
    let entry = plugin
        .visible_agents(1)
        .into_iter()
        .next()
        .expect("un subagente visible");
    assert!(plugin.watch_entry(&entry));

    // La pestaña se cierra antes de que el núcleo pida la sesión del visor.
    assert!(plugin.close_active_tab());

    assert!(
        plugin.take_spawn_session_request().is_none(),
        "sin pestaña vigente no se crea una sesión de visor"
    );
}

#[test]
fn cerrar_una_pestana_pendiente_no_pide_cerrar_una_sesion_inexistente() {
    let plugin = HerdrPlugin::default();
    plugin
        .state
        .write()
        .unwrap()
        .create_pending_tab_in_active_space();
    assert!(plugin.close_active_tab());

    let requested = plugin.take_close_session_request();
    assert_ne!(
        requested,
        Some(PENDING_SESSION),
        "no se pide cerrar el centinela de una pestaña pendiente"
    );
    assert!(
        requested.is_none(),
        "una pestaña pendiente no tiene sesión que cerrar"
    );
}

#[test]
fn el_visor_no_restaura_una_sesion_que_ya_se_cerro() {
    let presence = seed_watch_agent("cerrada");
    let plugin = HerdrPlugin::with_presence_dir(&presence);
    let entry = plugin
        .visible_agents(1)
        .into_iter()
        .next()
        .expect("un subagente visible");

    // El visor ocupa la pestaña que mostraba el terminal original (sesión 0).
    assert!(plugin.watch_entry(&entry));
    assert!(plugin.take_spawn_session_request().is_some());
    plugin.on_session_created(7);

    // El shell original muere mientras el visor está abierto: su pestaña no se
    // restaura todavía (la sesión del visor sigue viva).
    plugin.on_session_closed(0);

    // Al cerrar el visor, la sesión original ya no existe: la pestaña no puede
    // volver a un id muerto, así que pide un shell fresco.
    plugin.on_session_closed(7);

    let s = plugin.state.read().unwrap();
    let tab = &s.spaces[0].tabs[0];
    assert_ne!(
        tab.session_id, 0,
        "una sesión ya cerrada no puede volver a la pestaña"
    );
    assert_eq!(tab.session_id, PENDING_SESSION, "la pestaña queda pendiente");
    drop(s);

    assert!(
        plugin.take_new_session_request(),
        "la pestaña pide un shell fresco"
    );
    plugin.on_session_created(8);
    assert_eq!(
        plugin.state.read().unwrap().spaces[0].tabs[0].session_id, 8,
        "el shell fresco llega a la pestaña restaurada"
    );
}

#[test]
fn cerrar_un_espacio_no_cambia_al_activo_entre_nombres_repetidos() {
    let plugin = HerdrPlugin::default();

    // Tres espacios con sesión viva, todos con el MISMO nombre.
    plugin.create_space_and_open_sidebar();
    plugin.create_space_and_open_sidebar();
    assert!(plugin.take_new_session_request());
    plugin.on_session_created(1);
    assert!(plugin.take_new_session_request());
    plugin.on_session_created(2);
    {
        let mut s = plugin.state.write().unwrap();
        for (i, space) in s.spaces.iter_mut().enumerate() {
            space.name = "mismo".to_string();
            space.branch = format!("b{i}");
        }
    }

    // El usuario mira el espacio del medio y se cierra el tercero.
    plugin.select_space(1);
    plugin.on_session_closed(2);

    let s = plugin.state.read().unwrap();
    assert_eq!(s.spaces.len(), 2, "el tercer espacio desaparece");
    let activo = &s.spaces[s.active_space_index];
    assert_eq!(
        activo.branch, "b1",
        "el activo sigue siendo el mismo espacio, no el primero con su nombre"
    );
    assert_eq!(activo.tabs[0].session_id, 1);
}

#[test]
fn un_spawn_fallido_no_desvia_la_siguiente_sesion_a_otra_pestana() {
    let plugin = HerdrPlugin::default();

    // El principal pide un shell para la primera pestaña, pero el spawn falla:
    // el núcleo no llama a `on_session_created`.
    plugin
        .state
        .write()
        .unwrap()
        .create_pending_tab_in_active_space();
    assert!(plugin.take_new_session_request());

    // Llega una segunda petición mientras la primera sigue en vuelo.
    plugin
        .state
        .write()
        .unwrap()
        .create_pending_tab_in_active_space();
    assert!(plugin.take_new_session_request());
    // El reintento sigue apuntando a la primera: no se llevó por delante la
    // petición de la segunda.
    {
        let s = plugin.state.read().unwrap();
        assert!(
            s.pending_shell_tabs.contains(&3),
            "la petición de la segunda pestaña sigue encolada"
        );
    }

    plugin.on_session_created(9);
    {
        let s = plugin.state.read().unwrap();
        assert_eq!(s.spaces[0].tabs[0].session_id, 0, "la terminal base no se toca");
        assert_eq!(
            s.spaces[0].tabs[1].session_id, 9,
            "la sesión cae en la pestaña que la pidió"
        );
        assert_eq!(
            s.spaces[0].tabs[2].session_id, PENDING_SESSION,
            "la segunda pestaña pendiente no hereda la sesión ajena"
        );
    }

    // La petición de la segunda pestaña se entrega en su propio turno.
    assert!(plugin.take_new_session_request());
    plugin.on_session_created(10);
    assert_eq!(
        plugin.state.read().unwrap().spaces[0].tabs[2].session_id,
        10,
        "la segunda pestaña termina con su sesión real"
    );
}

#[test]
fn la_pestana_pendiente_reintenta_su_shell_si_la_creacion_falla() {
    let plugin = HerdrPlugin::default();
    plugin
        .state
        .write()
        .unwrap()
        .create_pending_tab_in_active_space();

    // El host reintenta la misma pestaña: vuelve a pedir sin encolar otra.
    assert!(plugin.take_new_session_request());
    assert!(plugin.take_new_session_request());
    plugin.on_session_created(4);

    assert_eq!(
        plugin.state.read().unwrap().spaces[0].tabs[1].session_id,
        4,
        "la pestaña pendiente termina con su sesión real"
    );
}

#[test]
fn el_visor_espera_a_que_resuelva_el_shell_en_vuelo() {
    let presence = seed_watch_agent("en-vuelo");
    let plugin = HerdrPlugin::with_presence_dir(&presence);
    let entry = plugin
        .visible_agents(1)
        .into_iter()
        .next()
        .expect("un subagente visible");
    assert!(plugin.watch_entry(&entry));

    // Un shell pendiente queda en vuelo.
    plugin
        .state
        .write()
        .unwrap()
        .create_pending_tab_in_active_space();
    assert!(plugin.take_new_session_request());

    // Mientras el shell está en vuelo, el visor no se intercala.
    assert!(
        plugin.take_spawn_session_request().is_none(),
        "el visor espera al shell en vuelo"
    );

    // El shell resuelve y el visor se entrega en el render siguiente.
    plugin.on_session_created(5);
    assert!(plugin.take_spawn_session_request().is_some());
    plugin.on_session_created(6);

    let s = plugin.state.read().unwrap();
    assert_eq!(s.spaces[0].tabs[1].session_id, 5, "el shell va a la pestaña nueva");
    assert_eq!(
        s.spaces[0].tabs[0].session_id, 6,
        "el visor ocupa la pestaña del clic"
    );
}

#[test]
fn una_configuracion_con_varios_espacios_no_inventa_sesiones() {
    let plugin = HerdrPlugin::default();
    let mut cfg = port_plugin_api::PluginConfig::new();
    cfg.set("spaces", "a:main,b:main,c:main");
    plugin.load_config(&cfg);

    {
        let s = plugin.state.read().unwrap();
        assert_eq!(s.spaces.len(), 3, "los tres espacios de la config");
        assert_eq!(s.spaces[0].tabs[0].session_id, 0, "solo la sesión 0 existe");
        assert_eq!(
            s.spaces[1].tabs[0].session_id, PENDING_SESSION,
            "el segundo espacio nace pendiente"
        );
        assert_eq!(
            s.spaces[2].tabs[0].session_id, PENDING_SESSION,
            "el tercer espacio nace pendiente"
        );
    }

    // El núcleo crea una por render y las enlaza a la pestaña correcta.
    assert!(plugin.take_new_session_request());
    plugin.on_session_created(1);
    assert!(plugin.take_new_session_request());
    plugin.on_session_created(2);
    assert!(!plugin.take_new_session_request(), "no queda ninguna más");

    let s = plugin.state.read().unwrap();
    assert_eq!(s.spaces[0].tabs[0].session_id, 0);
    assert_eq!(s.spaces[1].tabs[0].session_id, 1, "la sesión 1 va al segundo espacio");
    assert_eq!(s.spaces[2].tabs[0].session_id, 2, "la sesión 2 va al tercero");
}

#[test]
fn cerrar_todas_las_sesiones_no_deja_una_pestana_con_una_sesion_muerta() {
    let plugin = HerdrPlugin::default();

    plugin.on_session_closed(0);

    {
        let s = plugin.state.read().unwrap();
        assert_eq!(s.spaces.len(), 1, "queda el espacio reconstruido");
        let tab = &s.spaces[0].tabs[0];
        assert_ne!(tab.session_id, 0, "no puede apuntar a la sesión que murió");
        assert_eq!(tab.session_id, PENDING_SESSION);
    }

    assert!(plugin.take_new_session_request(), "pide un shell fresco");
    plugin.on_session_created(5);
    assert_eq!(
        plugin.state.read().unwrap().spaces[0].tabs[0].session_id,
        5,
        "el shell fresco llega a la pestaña"
    );
}

#[test]
fn la_configuracion_no_duplica_ids_de_pestana() {
    let plugin = HerdrPlugin::default();
    let mut cfg = port_plugin_api::PluginConfig::new();
    cfg.set("spaces", "a:main,b:main,c:main");
    plugin.load_config(&cfg);

    // Una pestaña nueva por el mismo camino del topbar.
    plugin
        .state
        .write()
        .unwrap()
        .create_pending_tab_in_active_space();

    // El id nuevo continúa la numeración y no repite ninguno declarado.
    let (nuevo, repetido) = {
        let s = plugin.state.read().unwrap();
        let nuevo = s.spaces[s.active_space_index]
            .tabs
            .last()
            .expect("la pestaña nueva")
            .id;
        let repetido = s
            .spaces
            .iter()
            .flat_map(|sp| sp.tabs.iter())
            .filter(|t| t.id == nuevo)
            .count();
        (nuevo, repetido)
    };
    assert_eq!(repetido, 1, "el id de la pestaña nueva es único");
    assert!(nuevo > 3, "el id nuevo continúa la numeración declarada");

    // Cada shell encolado enlaza a una pestaña distinta y correcta.
    for session in [10usize, 11, 12] {
        assert!(plugin.take_new_session_request());
        plugin.on_session_created(session);
    }
    assert!(!plugin.take_new_session_request(), "no queda ninguna más");

    let s = plugin.state.read().unwrap();
    let mut enlazadas: Vec<(usize, usize)> = Vec::new();
    for sp in &s.spaces {
        for t in &sp.tabs {
            if t.session_id >= 10 {
                enlazadas.push((t.id, t.session_id));
            }
        }
    }
    enlazadas.sort_unstable();
    assert_eq!(enlazadas, vec![(2, 10), (3, 11), (4, 12)]);
}

#[test]
fn cerrar_la_pestana_del_visor_desde_el_topbar_restaura_el_terminal_original() {
    let presence = seed_watch_agent("topbar");
    let plugin = HerdrPlugin::with_presence_dir(&presence);
    let entry = plugin
        .visible_agents(1)
        .into_iter()
        .next()
        .expect("un subagente visible");
    assert!(plugin.watch_entry(&entry));
    assert!(plugin.take_spawn_session_request().is_some());
    plugin.on_session_created(7);

    // El topbar cierra la pestaña que muestra el visor: no se borra.
    assert!(plugin.state.write().unwrap().close_tab_at(0, 0));
    assert_eq!(plugin.state.read().unwrap().spaces[0].tabs.len(), 1);
    assert_eq!(
        plugin.take_close_session_request(),
        Some(7),
        "se cierra la sesión del visor, no la pestaña"
    );

    plugin.on_session_closed(7);
    assert_eq!(
        plugin.state.read().unwrap().spaces[0].tabs[0].session_id,
        0,
        "vuelve el terminal original"
    );
}

#[test]
fn cerrar_una_pestana_normal_con_el_helper_deja_el_indice_valido() {
    let plugin = HerdrPlugin::default();
    plugin.create_tab_in_active_space();
    assert!(plugin.take_new_session_request());
    plugin.on_session_created(1);

    assert!(plugin.state.write().unwrap().close_tab_at(0, 1));

    let (len, active, session) = {
        let s = plugin.state.read().unwrap();
        (
            s.spaces[0].tabs.len(),
            s.spaces[0].active_tab_index,
            s.spaces[0].tabs[0].session_id,
        )
    };
    assert_eq!(len, 1);
    assert!(active < len, "el índice activo queda dentro de rango");
    assert_eq!(session, 0);
    assert_eq!(plugin.take_close_session_request(), Some(1));
}

#[test]
fn un_segundo_clic_en_agents_cambia_el_visor_sin_perder_el_terminal_original() {
    let presence = seed_watch_agent("segundo");
    let plugin = HerdrPlugin::with_presence_dir(&presence);

    let a = plugin
        .visible_agents(1)
        .into_iter()
        .next()
        .expect("un subagente visible");
    assert!(plugin.watch_entry(&a));
    assert!(plugin.take_spawn_session_request().is_some());
    plugin.on_session_created(7);

    // Segundo clic: conserva el terminal original (0), no el visor (7).
    let b = plugin
        .visible_agents(1)
        .into_iter()
        .next()
        .expect("un subagente visible");
    assert!(plugin.watch_entry(&b));
    assert_eq!(
        plugin.state.read().unwrap().spaces[0].tabs[0]
            .watching
            .as_ref()
            .expect("sigue apuntando al original")
            .session_id,
        Some(0),
        "el terminal original no se pierde"
    );
    assert_eq!(
        plugin.take_close_session_request(),
        Some(7),
        "se pide cerrar el visor anterior"
    );

    // La sesión del visor nuevo reemplaza a la vieja.
    assert!(plugin.take_spawn_session_request().is_some());
    plugin.on_session_created(8);
    // El cierre del visor viejo ya no coincide con la pestaña.
    plugin.on_session_closed(7);
    assert_eq!(plugin.state.read().unwrap().spaces[0].tabs[0].session_id, 8);

    plugin.on_session_closed(8);
    let s = plugin.state.read().unwrap();
    let tab = &s.spaces[0].tabs[0];
    assert_eq!(tab.session_id, 0, "vuelve el terminal original");
    assert_eq!(tab.title, "terminal", "vuelve el título original");
    assert!(!tab.title_locked);
}

#[test]
fn un_visor_en_vuelo_cuya_pestana_desaparece_no_enlaza_la_sesion_a_otra_pestana() {
    let presence = seed_watch_agent("vuelo-caido");
    let plugin = HerdrPlugin::with_presence_dir(&presence);

    // Dos pestañas con sesión; el visor toma la primera (sesión 0).
    plugin.create_tab_in_active_space();
    assert!(plugin.take_new_session_request());
    plugin.on_session_created(5);
    plugin.select_tab(0);

    let entry = plugin
        .visible_agents(1)
        .into_iter()
        .next()
        .expect("un subagente visible");
    assert!(plugin.watch_entry(&entry));
    assert!(plugin.take_spawn_session_request().is_some());

    // La pestaña del visor desaparece antes de que su sesión se cree.
    plugin.on_session_closed(0);

    assert!(
        plugin.take_spawn_session_request().is_none(),
        "el visor no puede redirigirse a otra pestaña"
    );
    plugin.on_session_created(9);
    assert_eq!(
        plugin.state.read().unwrap().spaces[0].tabs[0].session_id,
        5,
        "la pestaña restante no recibe la sesión del visor"
    );
}

#[test]
fn una_sesion_que_herdr_no_pidio_no_toca_sus_pestanas() {
    let plugin = HerdrPlugin::default();
    {
        let mut s = plugin.state.write().unwrap();
        s.spaces[0].tabs[0].watching = Some(WatchedTab {
            session_id: Some(0),
            title: "terminal".to_string(),
            title_locked: false,
        });
        s.pending_viewer_title = Some("no tocar".to_string());
    }
    let antes = {
        let s = plugin.state.read().unwrap();
        s.spaces
            .iter()
            .flat_map(|sp| sp.tabs.iter().map(|t| (t.session_id, t.title.clone(), t.title_locked)))
            .collect::<Vec<_>>()
    };

    plugin.on_session_created(42);

    let despues = {
        let s = plugin.state.read().unwrap();
        s.spaces
            .iter()
            .flat_map(|sp| sp.tabs.iter().map(|t| (t.session_id, t.title.clone(), t.title_locked)))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        antes, despues,
        "una sesión ajena no se enlaza a las pestañas de herdr"
    );
    assert_eq!(
        plugin.state.read().unwrap().pending_viewer_title.as_deref(),
        Some("no tocar"),
        "no se consume el título del visor"
    );
}

#[test]
fn recargar_una_configuracion_con_la_sesion_cero_muerta_no_la_restaura() {
    let plugin = HerdrPlugin::default();

    // Un segundo espacio mantiene vivo el estado tras cerrar la sesión 0.
    plugin.create_space_and_open_sidebar();
    assert!(plugin.take_new_session_request());
    plugin.on_session_created(1);
    plugin.on_session_closed(0);

    let mut cfg = port_plugin_api::PluginConfig::new();
    cfg.set("spaces", "a:main,b:main");
    plugin.load_config(&cfg);

    {
        let s = plugin.state.read().unwrap();
        assert_eq!(s.spaces.len(), 2);
        assert_ne!(
            s.spaces[0].tabs[0].session_id, 0,
            "no restaura la sesión que ya murió"
        );
        assert_eq!(s.spaces[0].tabs[0].session_id, PENDING_SESSION);
        assert_eq!(s.spaces[1].tabs[0].session_id, PENDING_SESSION);
    }

    // Las dos pestañas piden su shell y lo reciben en orden.
    assert!(plugin.take_new_session_request());
    plugin.on_session_created(3);
    assert!(plugin.take_new_session_request());
    plugin.on_session_created(4);
    let s = plugin.state.read().unwrap();
    assert_eq!(s.spaces[0].tabs[0].session_id, 3);
    assert_eq!(s.spaces[1].tabs[0].session_id, 4);
}

#[test]
fn recargar_una_configuracion_deja_el_espacio_activo_dentro_de_rango() {
    let plugin = HerdrPlugin::default();
    plugin.create_space_and_open_sidebar();
    plugin.create_space_and_open_sidebar();
    assert!(plugin.take_new_session_request());
    plugin.on_session_created(1);
    assert!(plugin.take_new_session_request());
    plugin.on_session_created(2);
    assert_eq!(plugin.state.read().unwrap().active_space_index, 2);

    let mut cfg = port_plugin_api::PluginConfig::new();
    cfg.set("spaces", "a:main");
    plugin.load_config(&cfg);

    let s = plugin.state.read().unwrap();
    assert_eq!(s.spaces.len(), 1);
    assert_eq!(
        s.active_space_index, 0,
        "el índice activo queda dentro del vector nuevo"
    );
    drop(s);
    assert_eq!(
        plugin.active_session(),
        0,
        "la sesión activa es la del espacio declarado, no una ajena"
    );
}

#[test]
fn la_sesion_activa_no_cae_en_la_cero_cuando_no_hay_pestana_activa() {
    let plugin = HerdrPlugin::default();
    assert_eq!(plugin.active_session(), 0, "la pestaña normal sigue en 0");

    plugin.state.write().unwrap().active_space_index = 5;
    assert_eq!(
        plugin.active_session(),
        PENDING_SESSION,
        "sin pestaña activa no se cae en la sesión 0"
    );

    plugin.state.write().unwrap().spaces.clear();
    assert_eq!(plugin.active_session(), PENDING_SESSION);
}

#[test]
fn un_segundo_clic_mientras_el_visor_esta_en_vuelo_no_pierde_la_peticion() {
    let presence = seed_two_watch_agents("en-vuelo-clic");
    let plugin = HerdrPlugin::with_presence_dir(&presence);

    let entries = plugin.visible_agents(2);
    let a = entries
        .iter()
        .find(|e| e.session_hash == WATCH_HASH)
        .expect("subagente A")
        .clone();
    let b = entries
        .iter()
        .find(|e| e.session_hash == WATCH_HASH_B)
        .expect("subagente B")
        .clone();

    assert!(plugin.watch_entry(&a));
    assert!(plugin.take_spawn_session_request().is_some());
    // Segundo clic sobre la MISMA pestaña mientras la sesión de A está en vuelo.
    assert!(plugin.watch_entry(&b));

    {
        let s = plugin.state.read().unwrap();
        assert_eq!(
            s.in_flight,
            Some(crate::state::InFlight::Viewer(1)),
            "el visor de A sigue en vuelo"
        );
        assert_eq!(
            s.pending_watch
                .as_ref()
                .expect("petición viva")
                .session_hash,
            WATCH_HASH,
            "la petición en vuelo sigue siendo la de A"
        );
    }

    plugin.on_session_created(7);
    plugin.on_session_closed(7);
    assert_eq!(
        plugin.state.read().unwrap().spaces[0].tabs[0].session_id,
        0,
        "vuelve el terminal original"
    );
}

#[test]
fn el_visor_no_borra_la_pestana_si_su_watching_se_perdio_en_el_camino() {
    let presence = seed_watch_agent("watching-perdido");
    let plugin = HerdrPlugin::with_presence_dir(&presence);
    let entry = plugin
        .visible_agents(1)
        .into_iter()
        .next()
        .expect("un subagente visible");

    // Primer visor (A) sobre la pestaña 1.
    assert!(plugin.watch_entry(&entry));
    assert!(plugin.take_spawn_session_request().is_some());
    plugin.on_session_created(7);

    // Segundo clic (B) pide cerrar el visor 7 y conserva el original.
    assert!(plugin.watch_entry(&entry));
    assert_eq!(plugin.take_close_session_request(), Some(7));

    // Un shell en vuelo retrasa el spawn de B.
    plugin.create_tab_in_active_space();
    assert!(plugin.take_new_session_request());
    assert!(
        plugin.take_spawn_session_request().is_none(),
        "B queda diferido por el shell en vuelo"
    );

    // El cierre del visor 7 se procesa antes de que B enlace: la pestaña
    // pierde su `watching` y vuelve al terminal original.
    plugin.on_session_closed(7);
    assert!(
        plugin.state.read().unwrap().spaces[0].tabs[0]
            .watching
            .is_none(),
        "el camino de vuelta se perdió en el intermedio"
    );

    // El shell resuelve y B enlaza: debe sintetizar un `watching` nuevo.
    plugin.on_session_created(5);
    assert!(plugin.take_spawn_session_request().is_some());
    plugin.on_session_created(8);

    plugin.on_session_closed(8);
    let s = plugin.state.read().unwrap();
    assert!(
        s.spaces[0].tabs.iter().any(|t| t.id == 1),
        "la pestaña del usuario no se borra"
    );
    let tab1 = s.spaces[0].tabs.iter().find(|t| t.id == 1).unwrap();
    assert_eq!(tab1.session_id, 0, "vuelve el terminal original");
}

#[test]
fn un_clic_en_otra_pestana_mientras_el_visor_esta_en_vuelo_no_pisa_la_peticion() {
    let presence = seed_two_watch_agents("otra-pestana");
    let plugin = HerdrPlugin::with_presence_dir(&presence);

    let entries = plugin.visible_agents(2);
    let a = entries
        .iter()
        .find(|e| e.session_hash == WATCH_HASH)
        .expect("subagente A")
        .clone();
    let b = entries
        .iter()
        .find(|e| e.session_hash == WATCH_HASH_B)
        .expect("subagente B")
        .clone();

    // Dos pestañas con sesión propia.
    plugin.create_tab_in_active_space();
    assert!(plugin.take_new_session_request());
    plugin.on_session_created(5);
    plugin.select_tab(0);

    // El clic A deja su visor en vuelo sobre la pestaña 1.
    assert!(plugin.watch_entry(&a));
    assert!(plugin.take_spawn_session_request().is_some());

    // El usuario cambia a la pestaña 2 y vuelve a pulsar: no se honra.
    plugin.select_tab(1);
    assert!(plugin.watch_entry(&b));

    {
        let s = plugin.state.read().unwrap();
        assert_eq!(
            s.in_flight,
            Some(crate::state::InFlight::Viewer(1)),
            "el visor de A sigue en vuelo sobre la pestaña 1"
        );
        assert_eq!(
            s.pending_watch
                .as_ref()
                .expect("petición viva")
                .session_hash,
            WATCH_HASH,
            "la petición en vuelo sigue siendo la de A"
        );
        assert_eq!(
            s.pending_viewer_tab,
            Some(1),
            "sigue apuntando a la pestaña 1"
        );
    }

    plugin.on_session_created(7);
    plugin.on_session_closed(7);

    let s = plugin.state.read().unwrap();
    let tab1 = s.spaces[0].tabs.iter().find(|t| t.id == 1).expect("pestaña 1");
    let tab2 = s.spaces[0].tabs.iter().find(|t| t.id == 2).expect("pestaña 2");
    assert_eq!(tab1.session_id, 0, "la pestaña 1 vuelve a su terminal");
    assert_eq!(tab2.session_id, 5, "la pestaña 2 no se toca");
}

#[test]
fn salir_del_visor_restaura_el_terminal_sin_esperar_al_nucleo() {
    let presence = seed_watch_agent("salir-sin-nucleo");
    let plugin = HerdrPlugin::with_presence_dir(&presence);
    let entry = plugin
        .visible_agents(1)
        .into_iter()
        .next()
        .expect("un subagente visible");
    assert!(plugin.watch_entry(&entry));
    assert!(plugin.take_spawn_session_request().is_some());
    plugin.on_session_created(7);

    // Salir del visor restaura YA: el host cierra por petición sin llamar a
    // `on_session_closed`.
    assert!(plugin.request_viewer_close());
    {
        let s = plugin.state.read().unwrap();
        assert_eq!(s.spaces.len(), 1);
        let tab = &s.spaces[0].tabs[0];
        assert_eq!(tab.session_id, 0, "el terminal original vuelve en el acto");
        assert_eq!(tab.title, "terminal");
        assert!(!tab.title_locked);
        assert!(tab.watching.is_none());
    }
    assert_eq!(plugin.active_session(), 0);
    assert_eq!(plugin.take_close_session_request(), Some(7));

    // El callback tardío (si llegara) debe ser inofensivo.
    plugin.on_session_closed(7);
    let s = plugin.state.read().unwrap();
    assert_eq!(s.spaces.len(), 1, "el espacio sigue vivo");
    assert_eq!(s.spaces[0].tabs[0].session_id, 0);
    assert!(s.spaces[0].tabs[0].watching.is_none());
}

#[test]
fn cerrar_con_la_equis_el_visor_restaura_el_terminal_sin_esperar_al_nucleo() {
    let presence = seed_watch_agent("equis-sin-nucleo");
    let plugin = HerdrPlugin::with_presence_dir(&presence);
    let entry = plugin
        .visible_agents(1)
        .into_iter()
        .next()
        .expect("un subagente visible");
    assert!(plugin.watch_entry(&entry));
    assert!(plugin.take_spawn_session_request().is_some());
    plugin.on_session_created(7);

    // La `×` del topbar pasa por el mismo cierre compartido.
    assert!(plugin.state.write().unwrap().close_tab_at(0, 0));
    {
        let s = plugin.state.read().unwrap();
        assert_eq!(s.spaces[0].tabs.len(), 1, "la pestaña no se borra");
        let tab = &s.spaces[0].tabs[0];
        assert_eq!(tab.session_id, 0, "restaura en el acto");
        assert!(tab.watching.is_none());
    }
    assert_eq!(plugin.take_close_session_request(), Some(7));
}

#[test]
fn salir_de_un_visor_sin_terminal_pide_un_shell() {
    let presence = seed_watch_agent("visor-pendiente");
    let plugin = HerdrPlugin::with_presence_dir(&presence);

    // La pestaña nace pendiente y el visor la ocupa antes de tener terminal.
    plugin
        .state
        .write()
        .unwrap()
        .create_pending_tab_in_active_space();
    let entry = plugin
        .visible_agents(1)
        .into_iter()
        .next()
        .expect("un subagente visible");
    assert!(plugin.watch_entry(&entry));

    // El shell del espacio se crea primero, como en el núcleo real.
    assert!(plugin.take_new_session_request());
    plugin.on_session_created(5);
    assert!(plugin.take_spawn_session_request().is_some());
    plugin.on_session_created(7);

    assert!(plugin.request_viewer_close());
    {
        let s = plugin.state.read().unwrap();
        let tab = s.spaces[0].tabs.iter().find(|t| t.id == 2).unwrap();
        assert_eq!(tab.session_id, PENDING_SESSION, "sin terminal al que volver");
        assert!(tab.watching.is_none());
    }
    assert!(plugin.take_new_session_request(), "pide un shell fresco");
    plugin.on_session_created(8);
    assert_eq!(
        plugin.state.read().unwrap().spaces[0]
            .tabs
            .iter()
            .find(|t| t.id == 2)
            .unwrap()
            .session_id,
        8
    );
}

#[test]
fn la_pestana_que_salio_del_visor_se_puede_cerrar() {
    let presence = seed_watch_agent("cerrable");
    let plugin = HerdrPlugin::with_presence_dir(&presence);

    // Una segunda pestaña con su sesión para que Ctrl+W tenga qué cerrar.
    plugin.create_tab_in_active_space();
    assert!(plugin.take_new_session_request());
    plugin.on_session_created(5);
    plugin.select_tab(0);

    let entry = plugin
        .visible_agents(1)
        .into_iter()
        .next()
        .expect("un subagente visible");
    assert!(plugin.watch_entry(&entry));
    assert!(plugin.take_spawn_session_request().is_some());
    plugin.on_session_created(7);
    assert!(plugin.request_viewer_close());

    // Ya no es una pestaña de visor: Ctrl+W la cierra normalmente.
    assert_eq!(plugin.on_key(&Key::new("w").ctrl()), KeyAction::Consume);
    assert_eq!(
        plugin.take_close_session_request(),
        Some(0),
        "cierra su terminal real, no el visor muerto"
    );
    assert_eq!(plugin.state.read().unwrap().spaces[0].tabs.len(), 1);
}
