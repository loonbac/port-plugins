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
    let plugin = HerdrPlugin::default();
    let accent = plugin.state.read().unwrap().effective_accent();
    assert_eq!(accent, system_accent_color());
}

#[test]
fn herdr_space_updates_name_and_branch_from_cwd() {
    let plugin = HerdrPlugin::default();
    let path = std::path::Path::new("/home/loonbac/Proyectos/port");
    plugin.update_session_cwd(0, path, "port");

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
        assert_eq!(previo.session_id, 0, "se guardó la sesión original");
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

    // El núcleo consume la petición una sola vez: el comando lleva la
    // identidad de la sesión y el directorio de presencia, no una ruta de
    // registro.
    let config = plugin
        .take_spawn_session_request()
        .expect("el clic debe pedir una sesión de visor");
    assert_eq!(config.command, "bash");
    assert_eq!(config.args[2], "herdr-view");
    assert_eq!(config.args[3], presence.display().to_string());
    assert_eq!(config.args[4], WATCH_HASH);
    assert_eq!(config.args[5], WATCH_INCARNATION);
    assert_eq!(config.args[6], expected_label);
    assert!(
        plugin.take_spawn_session_request().is_none(),
        "una petición de visor se consume una sola vez"
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
            0,
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
    assert!(plugin.watch_entry(&entry));
    assert!(plugin.take_spawn_session_request().is_some());
    plugin.on_session_created(7);

    // Escape sin modificadores pide cerrar la sesión del visor y consume.
    assert_eq!(plugin.on_key(&Key::new("Escape")), KeyAction::Consume);
    assert_eq!(plugin.take_close_session_request(), Some(7));
    assert!(
        plugin.state.read().unwrap().spaces[0].tabs[0]
            .watching
            .is_some(),
        "la pestaña no se borra: solo se pide cerrar la sesión del visor"
    );

    // Ctrl+W hace exactamente lo mismo, nunca borra la pestaña del usuario.
    assert_eq!(plugin.on_key(&Key::new("w").ctrl()), KeyAction::Consume);
    assert_eq!(plugin.take_close_session_request(), Some(7));
    assert_eq!(plugin.state.read().unwrap().spaces[0].tabs.len(), 1);

    // close_active_tab tampoco borra una pestaña ocupada por el visor.
    assert!(plugin.close_active_tab());
    assert_eq!(plugin.take_close_session_request(), Some(7));
    assert_eq!(plugin.state.read().unwrap().spaces[0].tabs.len(), 1);

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
