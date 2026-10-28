//! Pruebas de layout del plugin herdr.
//!
//! Única responsabilidad: medir, con la ventana de prueba de GPUI, que el texto
//! que el plugin no controla (nombre de carpeta, rama, nombre y etiqueta de
//! subagente, título de pestaña) se dibuja dentro del panel lateral y de la
//! barra superior, por largos que sean esos textos.
//!
//! Las claves que busca `debug_bounds` son las mismas que declara cada hook con
//! `debug_selector` (ver `hooks/layout/sidebar.rs` y `hooks/layout/topbar.rs`).
//! Si alguna se renombra, la prueba falla al no encontrar los límites en lugar
//! de pasar en silencio.

use std::rc::Rc;

use gpui::{
    point, px, size, AnyElement, Bounds, Context, IntoElement, Pixels, Render, TestAppContext,
    Window,
};
use port_plugin_api::LayoutHook;
use port_term_core::pty::RunningApp;

use crate::anim::{CARD_HEIGHT, CARD_PITCH};
use crate::state::{HerdrSpace, HerdrTab};
use crate::tests::{empty_presence, seed_watch_agent_with};
use crate::{HerdrPlugin, SIDEBAR_DEFAULT_WIDTH};

// ── Claves de `debug_selector` ─────────────────────────────────────────
const PANEL_LATERAL: &str = "herdr-sidebar";
const TARJETA_0: &str = "herdr-sidebar-space-card-0";
const TARJETA_1: &str = "herdr-sidebar-space-card-1";
const TARJETA_2: &str = "herdr-sidebar-space-card-2";
const NOMBRE_ESPACIO: &str = "herdr-sidebar-space-name-0";
const RAMA_ESPACIO: &str = "herdr-sidebar-space-branch-0";
const AGENTE_NOMBRE: &str = "herdr-sidebar-agent-name-t_largo";
const AGENTE_ETIQUETA: &str = "herdr-sidebar-agent-label-t_largo";
const AGENTE_PASO: &str = "herdr-sidebar-agent-step-t_largo";
const BARRA_SUPERIOR: &str = "herdr-topbar";
const PESTANA: &str = "herdr-topbar-tab-0";
const ETIQUETA_PESTANA: &str = "herdr-topbar-tab-label-0";
const BOTON_NUEVA_PESTANA: &str = "herdr-topbar-new-tab";

/// Alto que reserva la barra superior (ver `top_bar_height`).
const ALTO_BARRA_SUPERIOR: f32 = 38.0;

/// Espacio con el nombre y la rama indicados.
fn espacio(id: usize, nombre: &str, branch: &str, tabs: Vec<HerdrTab>) -> HerdrSpace {
    HerdrSpace {
        id,
        name: nombre.to_string(),
        branch: branch.to_string(),
        custom_color: None,
        tabs,
        active_tab_index: 0,
    }
}

/// Pestaña en reposo (sin app en primer plano y sin título fijado) con el texto
/// que se quiere ver en la barra.
fn pestana(id: usize, titulo: &str) -> HerdrTab {
    HerdrTab {
        id,
        title: titulo.to_string(),
        session_id: 0,
        app: None,
        title_locked: false,
        watching: None,
    }
}

/// Dibuja `element` como elemento raíz de una ventana de prueba del tamaño
/// dado y devuelve los límites de cada clave, en el mismo orden.
///
/// Sirve para árboles sin estilos de cursor: GPUI los pinta contra la vista en
/// curso y un elemento dibujado así no tiene vista (usar `medir_en_vista`).
fn medir(
    cx: &mut TestAppContext,
    ancho: f32,
    alto: f32,
    element: impl FnOnce() -> AnyElement,
    claves: &[&'static str],
) -> Vec<Bounds<Pixels>> {
    let cx = cx.add_empty_window();
    // El elemento se construye dentro del cierre: GPUI lo asigna al arena del
    // frame en curso, y construirlo antes de la ventana deja una referencia
    // al arena ya limpiado.
    cx.draw(
        point(px(0.0), px(0.0)),
        size(px(ancho), px(alto)),
        |_window, _cx| element(),
    );
    claves
        .iter()
        .map(|clave| {
            cx.debug_bounds(clave).unwrap_or_else(|| {
                panic!("sin límites para `{clave}`: ¿cambió su `debug_selector`?")
            })
        })
        .collect()
}

/// Vista mínima de prueba: dibuja como raíz de la ventana el elemento que se
/// quiere medir. Hace falta para el panel lateral, que lleva estilos de cursor
/// (`cursor_pointer`, `cursor_col_resize`).
struct VistaDePrueba<F: Fn() -> AnyElement + 'static>(F);

impl<F: Fn() -> AnyElement + 'static> Render for VistaDePrueba<F> {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        (self.0)()
    }
}

/// Dibuja el elemento como raíz de una vista y devuelve los límites pedidos.
fn medir_en_vista(
    cx: &mut TestAppContext,
    element: impl Fn() -> AnyElement + 'static,
    claves: &[&'static str],
) -> Vec<Bounds<Pixels>> {
    let (_, cx) = cx.add_window_view(move |_window, _cx| VistaDePrueba(element));
    // Marca la ventana como sucia y deja que `flush_effects` la dibuje.
    cx.update(|window, _cx| window.refresh());
    claves
        .iter()
        .map(|clave| {
            cx.debug_bounds(clave).unwrap_or_else(|| {
                panic!("sin límites para `{clave}`: ¿cambió su `debug_selector`?")
            })
        })
        .collect()
}

/// Afirma que `dentro` no sobresale del borde horizontal de `contenedor`.
fn assert_dentro(dentro: &Bounds<Pixels>, contenedor: &Bounds<Pixels>, que: &str) {
    assert!(
        dentro.size.width > px(0.0),
        "`{que}` no se pintó: {dentro:?}"
    );
    assert!(
        dentro.left() >= contenedor.left() && dentro.right() <= contenedor.right(),
        "`{que}` se sale de su contenedor: {dentro:?} fuera de {contenedor:?}"
    );
}

// ── Panel lateral ──────────────────────────────────────────────────────

/// Un nombre de carpeta largo no se sale del panel lateral.
#[gpui::test]
async fn un_nombre_de_espacio_largo_no_se_sale_del_panel_lateral(cx: &mut TestAppContext) {
    let presence = empty_presence("layout-nombre");
    let plugin = HerdrPlugin::with_presence_dir(&presence);
    let largo = "proyecto-con-un-nombre-de-carpeta-absurdamente-largo-para-el-panel";
    {
        let mut s = plugin.state.write().unwrap();
        let tabs = s.spaces[0].tabs.clone();
        s.spaces[0] = espacio(1, largo, &format!("feature/{largo}"), tabs);
        // Un segundo espacio mantiene la barra abierta sin depender de la
        // presencia de subagentes de esta máquina.
        s.spaces.push(espacio(2, "space-2", "main", Vec::new()));
    }

    let plugin = Rc::new(plugin);
    let barra = Rc::clone(&plugin);
    let bounds = medir_en_vista(
        cx,
        move || barra.left_sidebar().expect("la barra lateral se pinta"),
        &[PANEL_LATERAL, NOMBRE_ESPACIO, RAMA_ESPACIO],
    );

    assert_eq!(
        bounds[0].size.width,
        px(SIDEBAR_DEFAULT_WIDTH),
        "el panel mide el ancho que reserva el plugin"
    );
    assert_dentro(&bounds[1], &bounds[0], "el nombre del espacio");
    assert_dentro(&bounds[2], &bounds[0], "la rama del espacio");
}

/// El nombre, la etiqueta y el último paso de un subagente no se salen del
/// panel, aunque el último paso llegue ya recortado a `AGENT_STEP_MAX_CHARS`.
#[gpui::test]
async fn un_agente_con_textos_largos_no_se_sale_del_panel_lateral(cx: &mut TestAppContext) {
    let presence = seed_watch_agent_with(
        "layout-agentes",
        &"a".repeat(60),
        &"e".repeat(60),
        &"p".repeat(120),
        "t_largo",
    );
    let plugin = HerdrPlugin::with_presence_dir(&presence);

    let plugin = Rc::new(plugin);
    let barra = Rc::clone(&plugin);
    let bounds = medir_en_vista(
        cx,
        move || barra.left_sidebar().expect("la barra lateral se pinta"),
        &[PANEL_LATERAL, AGENTE_NOMBRE, AGENTE_ETIQUETA, AGENTE_PASO],
    );

    assert_dentro(&bounds[1], &bounds[0], "el nombre del subagente");
    assert_dentro(&bounds[2], &bounds[0], "la etiqueta del subagente");
    assert_dentro(&bounds[3], &bounds[0], "el último paso del subagente");
}

// ── Barra superior ─────────────────────────────────────────────────────

/// Un título de pestaña largo no se sale de la barra superior: se recorta con
/// `…` dentro de la pestaña.
#[gpui::test]
async fn un_titulo_de_pestana_largo_no_se_sale_de_la_barra_superior(cx: &mut TestAppContext) {
    let presence = empty_presence("layout-titulo");
    let plugin = HerdrPlugin::with_presence_dir(&presence);
    let largo = "una-pestana-con-un-titulo-absurdamente-largo-para-la-barra-superior";
    {
        let mut s = plugin.state.write().unwrap();
        // Dos pestañas: es la condición que pinta la barra.
        s.spaces[0].tabs = vec![pestana(1, largo), pestana(2, largo)];
    }

    let bounds = medir(
        cx,
        500.0,
        ALTO_BARRA_SUPERIOR,
        || plugin.top_bar().expect("la barra superior se pinta"),
        &[BARRA_SUPERIOR, PESTANA, ETIQUETA_PESTANA],
    );

    assert_dentro(&bounds[1], &bounds[0], "la pestaña");
    assert_dentro(&bounds[2], &bounds[0], "el título de la pestaña");
}

/// Una pestaña con un programa en primer plano (icono más nombre legible)
/// tampoco se sale: es la rama con más ancho fijo dentro de la pestaña.
#[gpui::test]
async fn una_pestana_con_icono_no_se_sale_de_la_barra_superior(cx: &mut TestAppContext) {
    let presence = empty_presence("layout-icono");
    let plugin = HerdrPlugin::with_presence_dir(&presence);
    {
        let mut s = plugin.state.write().unwrap();
        let mut con_icono = pestana(1, "sin titulo");
        con_icono.app = Some(RunningApp {
            pid: 4242,
            bin: "un-programa-con-un-nombre-absurdamente-largo".to_string(),
        });
        s.spaces[0].tabs = vec![con_icono, pestana(2, "bash")];
    }

    let bounds = medir(
        cx,
        500.0,
        ALTO_BARRA_SUPERIOR,
        || plugin.top_bar().expect("la barra superior se pinta"),
        &[BARRA_SUPERIOR, PESTANA, ETIQUETA_PESTANA],
    );

    assert_dentro(&bounds[1], &bounds[0], "la pestaña con icono");
    assert_dentro(&bounds[2], &bounds[0], "el nombre del programa");
}

/// Con muchas pestañas de título largo, el botón de nueva pestaña sigue dentro
/// de la barra: no lo empuja la tira de pestañas.
#[gpui::test]
async fn muchas_pestanas_no_empujan_el_boton_de_nueva_pestana_fuera_de_la_barra(
    cx: &mut TestAppContext,
) {
    let presence = empty_presence("layout-muchas");
    let plugin = HerdrPlugin::with_presence_dir(&presence);
    {
        let mut s = plugin.state.write().unwrap();
        s.spaces[0].tabs = (1..=12)
            .map(|i| pestana(i, &format!("pestana-{i}-con-nombre-de-carpeta-largo")))
            .collect();
    }

    let bounds = medir(
        cx,
        420.0,
        ALTO_BARRA_SUPERIOR,
        || plugin.top_bar().expect("la barra superior se pinta"),
        &[BARRA_SUPERIOR, BOTON_NUEVA_PESTANA],
    );

    assert_dentro(&bounds[1], &bounds[0], "el botón de nueva pestaña");
}

/// Una pestaña corta conserva su ancho natural: el arreglo recorta lo que no
/// cabe, no estira lo que sí cabe.
#[gpui::test]
async fn una_pestana_corta_conserva_su_ancho_natural(cx: &mut TestAppContext) {
    let presence = empty_presence("layout-corta");
    let plugin = HerdrPlugin::with_presence_dir(&presence);
    {
        let mut s = plugin.state.write().unwrap();
        s.spaces[0].tabs = vec![pestana(1, "port"), pestana(2, "bash"), pestana(3, "zsh")];
    }

    let bounds = medir(
        cx,
        900.0,
        ALTO_BARRA_SUPERIOR,
        || plugin.top_bar().expect("la barra superior se pinta"),
        &[BARRA_SUPERIOR, PESTANA],
    );

    assert_dentro(&bounds[1], &bounds[0], "la pestaña corta");
    assert!(
        bounds[1].size.width < px(160.0),
        "la pestaña se estiró hasta ocupar la barra: {:?}",
        bounds[1]
    );
}

/// El paso entre tarjetas es exactamente alto más separación.
///
/// El arrastre convierte píxeles en ranuras dividiendo por `CARD_PITCH`, así que
/// este número tiene que ser el del layout real. Mientras fue una estimación
/// (`45.0`) el error se acumulaba en el ancla del gesto y un movimiento mínimo
/// cruzaba dos ranuras. Si un estilo mueve la altura o la separación, esta
/// prueba falla antes de que el arrastre empiece a descolocarse.
#[gpui::test]
async fn las_tarjetas_miden_un_paso_exacto(cx: &mut TestAppContext) {
    let presence = empty_presence("layout-paso");
    let plugin = HerdrPlugin::with_presence_dir(&presence);
    {
        let mut s = plugin.state.write().unwrap();
        let tabs = s.spaces[0].tabs.clone();
        s.spaces[0] = espacio(1, "space-1", "main", tabs);
        for id in 2..=3 {
            s.spaces
                .push(espacio(id, &format!("space-{id}"), "main", Vec::new()));
        }
    }

    let plugin = Rc::new(plugin);
    let barra = Rc::clone(&plugin);
    let bounds = medir_en_vista(
        cx,
        move || barra.left_sidebar().expect("la barra lateral se pinta"),
        &[PANEL_LATERAL, TARJETA_0, TARJETA_1, TARJETA_2],
    );

    for (i, tarjeta) in bounds[1..].iter().enumerate() {
        assert_eq!(
            tarjeta.size.height,
            px(CARD_HEIGHT),
            "la tarjeta {i} no mide el alto declarado"
        );
        assert_dentro(tarjeta, &bounds[0], "la tarjeta");
    }
    for par in bounds[1..].windows(2) {
        assert_eq!(
            par[1].top() - par[0].top(),
            px(CARD_PITCH),
            "las tarjetas no están a un paso exacto: {:?} y {:?}",
            par[0],
            par[1]
        );
    }
}

/// Una tarjeta con un agente en estado `Blocked` pinta su marcador de atención
/// (`herdr-sidebar-space-agent-{i}`) dentro del panel lateral.
#[gpui::test]
async fn una_tarjeta_con_agente_blocked_pinta_su_marcador(cx: &mut TestAppContext) {
    let presence = empty_presence("layout-blocked");
    let plugin = HerdrPlugin::with_presence_dir(&presence);
    {
        let mut s = plugin.state.write().unwrap();
        let tabs = s.spaces[0].tabs.clone();
        s.spaces[0] = espacio(1, "space-1", "main", tabs);
        // Segundo espacio para que la barra se dibuje fija
        let mut p = pestana(10, "pi");
        p.session_id = 10;
        s.spaces.push(espacio(2, "space-2", "main", vec![p]));
        s.agents
            .insert(10, (port_plugin_api::AgentState::Blocked, false));
    }

    let plugin = Rc::new(plugin);
    let barra = Rc::clone(&plugin);
    let bounds = medir_en_vista(
        cx,
        move || barra.left_sidebar().expect("la barra lateral se pinta"),
        &[PANEL_LATERAL, "herdr-sidebar-space-agent-1"],
    );

    assert_eq!(
        bounds[0].size.width,
        px(SIDEBAR_DEFAULT_WIDTH),
        "el panel mide el ancho que reserva el plugin"
    );
    assert_dentro(&bounds[1], &bounds[0], "el marcador de agente blocked");
}
