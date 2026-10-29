//! Barra lateral del plugin herdr.
//!
//! Única responsabilidad: dibujar la lista de espacios, la lista de subagentes
//! de `pi` y el asidero que ajusta el ancho de la barra.

use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    black, div, point, px, AnyElement, BoxShadow, FontWeight, IntoElement, MouseButton,
    ParentElement, Styled, Window,
};

use port_plugin_api::AgentState;
use port_term_core::frame::Rgb;

use crate::agents;
use crate::anim::{CardFrame, CARD_GAP, CARD_HEIGHT, CARD_PITCH};
use crate::color::to_hsla;
use crate::state::{SIDEBAR_MAX_WIDTH, SIDEBAR_MIN_WIDTH};
use crate::watch::{clip_last_step, request_watch};
use crate::HerdrPlugin;

/// Zona sensible del asidero de ancho, en píxeles.
///
/// Vive enteramente DENTRO del panel: una zona centrada en el borde se solapa
/// con la columna de la terminal, y el puntero sobre esa franja queda en una
/// hitbox ambigua entre dos hermanos del layout.
const RESIZE_HANDLE_HIT: f32 = 7.0;
/// Ancho de la pista visible del asidero.
const RESIZE_GRIP_WIDTH: f32 = 2.0;

/// Color de aviso para agentes bloqueados (`Blocked`).
const COLOR_BLOCKED: Rgb = Rgb::new(0xe5, 0x53, 0x4b);
/// Color de éxito para agentes listos sin ver (`Idle` con `seen = false`).
const COLOR_IDLE_UNSEEN: Rgb = Rgb::new(0x39, 0xc5, 0xcf);
/// Nombre del espacio activo.
const COLOR_NAME_ACTIVE: Rgb = Rgb::new(0xf0, 0xf6, 0xfc);
/// Nombre de un espacio en reposo.
const COLOR_NAME: Rgb = Rgb::new(0xc9, 0xd1, 0xd9);
/// Texto secundario (rama, etiqueta de subagente).
const COLOR_MUTED: Rgb = Rgb::new(0x8b, 0x94, 0x9e);
/// Texto terciario (último paso, pistas).
const COLOR_FAINT: Rgb = Rgb::new(0x6e, 0x76, 0x81);
/// Periodo del parpadeo de aviso (`Blocked`), en milisegundos: encendido medio
/// periodo, tenue medio periodo.
const BLOCKED_BLINK_PERIOD_MS: u64 = 1000;
/// Cuadros del spinner braille para `Working`, a 80 ms por cuadro.
const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const SPINNER_FRAME_MS: u64 = 80;

/// Cuadro del spinner según el reloj: avanza solo con cada render, sin timers
/// ni estado extra, porque la sidebar se repinta a ~60 fps por el latido del PTY.
pub(crate) fn spinner_frame(now_ms: u64) -> &'static str {
    SPINNER_FRAMES[(now_ms / SPINNER_FRAME_MS) as usize % SPINNER_FRAMES.len()]
}

/// `true` durante la primera mitad de cada periodo: parpadeo simple.
pub(crate) fn blink_on(now_ms: u64, period_ms: u64) -> bool {
    period_ms > 0 && now_ms % period_ms < period_ms / 2
}

pub(super) fn left_sidebar(plugin: &HerdrPlugin) -> Option<AnyElement> {
    // Un solo candado por cuadro: el mismo préstamo avanza la animación y copia
    // lo que el render necesita. La ventana de escritura es corta a propósito
    // (se cierra al copiar el estado, antes de construir los elementos).
    let now_ms = agents::now_ms();
    let mut state = plugin.state.write().unwrap();
    // El mismo criterio que reserva el ancho de la barra decide si se pinta
    // (ver `sidebar_shown`). Si sólo hay un espacio y no hay subagentes
    // visibles, la terminal es limpia.
    if !plugin.sidebar_shown(&state) {
        // Sin panel no hay gesto ni movimiento que recordar: al volver, las
        // tarjetas tienen que entrar animadas en vez de aparecer quietas donde
        // quedaron la última vez.
        state.space_drag = None;
        state.resize_anchor_x = None;
        state.pending_order = None;
        state.space_motion.reset();
        state.card_frame = Default::default();
        return None;
    }
    // La animación avanza aquí, una vez por cuadro, con el reloj del render: el
    // plugin no tiene temporizadores propios. El mismo instante se reutiliza
    // abajo para el spinner y los parpadeos.
    state.step_space_motion(now_ms);

    let active_space_idx = state.active_space_index;
    let spaces = state.spaces.clone();
    let opacity = state.opacity;
    let accent = to_hsla(state.effective_accent());
    let state_sidebar_width = state.sidebar_width;
    let state_is_resizing = state.resize_anchor_x.is_some();
    let drag_id = state.space_drag.map(|drag| drag.id);
    // El fotograma que calculó `step_space_motion`: un desplazamiento, una
    // opacidad y un tinte por tarjeta, en el mismo orden que `spaces`.
    let frame = state.card_frame.clone();
    let space_agent_states: Vec<Option<(AgentState, bool)>> = spaces
        .iter()
        .map(|space| space.aggregated_agent_state(&state.agents))
        .collect();
    drop(state);

    let warn_hsla = to_hsla(COLOR_BLOCKED);
    let success_hsla = to_hsla(COLOR_IDLE_UNSEEN);
    let term_bg = to_hsla(Rgb::DEFAULT_BG);
    let mut cards: Vec<AnyElement> = Vec::new();
    // La tarjeta que sigue al puntero, si hay un arrastre en curso. Se pinta
    // aparte porque va fuera del flujo (ver el final del bucle).
    let mut dragged_card: Option<AnyElement> = None;

    for (i, space) in spaces.iter().enumerate() {
        let is_active = i == active_space_idx;
        let dot_color = space.custom_color.map(to_hsla).unwrap_or(accent);
        let agent_state = space_agent_states[i];
        let is_dragged = drag_id == Some(space.id);
        let card = frame.cards.get(i).copied().unwrap_or(CardFrame {
            offset_y: 0.0,
            opacity: 1.0,
            tint: if is_active { 1.0 } else { 0.0 },
        });
        // La tarjeta arrastrada se levanta: sombra, tinte pleno y borde cerrado.
        let lifted = if is_dragged { frame.lift } else { 0.0 };
        let alpha = card.opacity;

        // Realce de tarjeta: Blocked o Idle sin ver tiñen borde y fondo con su
        // color; el resto usa el tinte del espacio activo, que se interpola en
        // vez de encenderse de golpe al cambiar de espacio.
        let (card_bg, card_border) = match agent_state {
            Some((AgentState::Blocked, _)) => (
                warn_hsla.opacity(0.20 * alpha),
                warn_hsla.opacity(0.55 * alpha),
            ),
            Some((AgentState::Idle, false)) => (
                success_hsla.opacity(0.12 * alpha),
                success_hsla.opacity(0.40 * alpha),
            ),
            _ => (
                accent.opacity(0.20 * card.tint * alpha),
                accent.opacity(0.55 * card.tint * alpha),
            ),
        };
        // Arrastrada: el tinte sube con el levantado y el borde se cierra del
        // todo, para que se lea separada del resto de la lista.
        let (card_bg, card_border) = if is_dragged {
            (
                accent.opacity((0.10 + 0.20 * lifted) * alpha),
                accent.opacity((0.45 + 0.55 * lifted) * alpha),
            )
        } else {
            (card_bg, card_border)
        };

        // Hover: la tarjeta se aclara bajo el puntero. Se calcula acá, con el
        // tinte incluido, porque un realce de hover fijo pisaría el del espacio
        // activo en vez de sumarse.
        let hover_bg = match agent_state {
            Some((AgentState::Blocked, _)) => warn_hsla.opacity(0.28),
            Some((AgentState::Idle, false)) => success_hsla.opacity(0.20),
            _ => accent.opacity(0.10 + 0.10 * card.tint),
        };
        let hover_border = match agent_state {
            Some((AgentState::Blocked, _)) => warn_hsla.opacity(0.70),
            Some((AgentState::Idle, false)) => success_hsla.opacity(0.55),
            _ => accent.opacity(0.40 + 0.30 * card.tint),
        };

        // Animación por reloj de render (ver `spinner_frame`/`blink_on`).
        // Marcador a la derecha de la fila del nombre: glifo + texto corto según
        // estado.
        let marker = match agent_state {
            Some((AgentState::Working, _)) => {
                Some((spinner_frame(now_ms), "working", accent.opacity(alpha)))
            }
            Some((AgentState::Blocked, _)) => {
                let color = if blink_on(now_ms, BLOCKED_BLINK_PERIOD_MS) {
                    warn_hsla.opacity(alpha)
                } else {
                    warn_hsla.opacity(0.25 * alpha)
                };
                Some(("●", "espera tu respuesta", color))
            }
            Some((AgentState::Idle, false)) => Some(("✓", "listo", success_hsla.opacity(alpha))),
            Some((AgentState::Idle, true)) | Some((AgentState::Unknown, _)) | None => None,
        };

        let name_color = to_hsla(if is_active {
            COLOR_NAME_ACTIVE
        } else {
            COLOR_NAME
        });
        let branch_color = if is_active {
            accent
        } else {
            to_hsla(COLOR_MUTED)
        };
        let space_id = space.id;
        let state_for_click = Arc::clone(&plugin.state);
        let mut item = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(8.0))
            // Alto fijo: el paso del arrastre (`CARD_PITCH`) se deriva de este
            // número, así que no puede depender del texto (ver `anim`).
            .h(px(CARD_HEIGHT))
            .px(px(10.0))
            .rounded(px(6.0))
            .cursor_grab()
            .overflow_hidden()
            .hover(move |style| style.bg(hover_bg).border_color(hover_border))
            .bg(card_bg)
            .border_1()
            .border_color(card_border)
            // Clave de las pruebas de layout (ver `layout_test`): mide el alto
            // de la tarjeta y el paso real entre tarjetas.
            .debug_selector(move || format!("herdr-sidebar-space-card-{i}"))
            .on_mouse_down(MouseButton::Left, move |event, window: &mut Window, _cx| {
                let y: f32 = event.position.y.into();
                // No selecciona todavía: si el puntero no se mueve es un clic y
                // `end_space_drag` seleccionará al soltar; si se mueve, empieza
                // el reordenamiento.
                state_for_click
                    .write()
                    .unwrap()
                    .begin_space_drag(space_id, y);
                window.refresh();
            })
            .child(
                // El ancho lo decide la barra, nunca el nombre: `flex_1` toma
                // el espacio disponible, `min_w_0` deja encogerse por debajo
                // del contenido y `truncate` recorta con `…` en vez de
                // desbordar el panel.
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .flex_shrink_0()
                            .w(px(8.0))
                            .h(px(8.0))
                            .rounded(px(4.0))
                            .bg(dot_color.opacity(alpha)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(px(13.0))
                                    .font_weight(FontWeight::BOLD)
                                    .truncate()
                                    // Clave de las pruebas de layout (ver
                                    // `layout_test`): permite medir que el
                                    // texto no sale del panel.
                                    .debug_selector(move || format!("herdr-sidebar-space-name-{i}"))
                                    .text_color(name_color.opacity(alpha))
                                    .child(space.name.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .truncate()
                                    .debug_selector(move || {
                                        format!("herdr-sidebar-space-branch-{i}")
                                    })
                                    .text_color(branch_color.opacity(alpha))
                                    .child(space.branch.clone()),
                            ),
                    ),
            );

        if let Some((glyph, text, color)) = marker {
            item = item.child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(4.0))
                    .debug_selector(move || format!("herdr-sidebar-space-agent-{i}"))
                    .child(div().text_size(px(10.0)).text_color(color).child(glyph))
                    .child(
                        div()
                            .text_size(px(10.0))
                            .font_weight(FontWeight::BOLD)
                            .truncate()
                            .text_color(color)
                            .debug_selector(move || format!("herdr-sidebar-space-agent-text-{i}"))
                            .child(text),
                    ),
            );
        }

        // La sombra entra con el levantado, en vez de encenderse entera en el
        // primer cuadro. Un preset no se puede interpolar, así que la sombra se
        // arma a mano con el alfa y el desenfoque del realce.
        if lifted > 0.0 {
            item = item.shadow(vec![BoxShadow {
                color: black().opacity(0.42 * lifted),
                offset: point(px(0.0), px(3.0)),
                blur_radius: px(14.0 * lifted),
                spread_radius: px(0.0),
            }]);
        }

        if is_dragged {
            // En su ranura queda un marcador del mismo alto: si la tarjeta
            // saliera del flujo, las demás subirían una posición.
            cards.push(div().flex_shrink_0().h(px(CARD_HEIGHT)).into_any_element());
            // Y la tarjeta se pinta fuera del flujo, anclada a esa ranura más lo
            // que la movió el puntero. Antes se la movía al final de los hijos
            // del `flex_col` para que quedara por encima, y eso no cambia el
            // orden de pintado: cambia el layout, así que la arrastrada se
            // dibujaba al pie de la lista, debajo de todas.
            dragged_card = Some(
                item.absolute()
                    .left(px(0.0))
                    .right(px(0.0))
                    .top(px(i as f32 * CARD_PITCH + card.offset_y))
                    .into_any_element(),
            );
        } else {
            // El desplazamiento entra como margen arriba y su negativo abajo:
            // la tarjeta se corre sin empujar a las demás fuera de su ranura,
            // que es lo que hace legible el reordenamiento en vivo.
            cards.push(
                item.mt(px(card.offset_y))
                    .mb(px(-card.offset_y))
                    .into_any_element(),
            );
        }
    }

    // El contenedor es relativo para que la tarjeta arrastrada se ancle a él, y
    // va al final para pintarse sobre las demás.
    let spaces_list = div()
        .relative()
        .flex()
        .flex_col()
        .gap(px(CARD_GAP))
        .children(cards)
        .children(dragged_card);

    // ── Sección AGENTS ──────────────────────────────────────────────────
    // Lista de subagentes de pi, en vivo. Los vivos van arriba y llevan el
    // punto encendido; los terminados, en apagado, para que la diferencia
    // se lea de un vistazo sin leer una sola palabra.
    //
    // Cada fila es pulsable: abre el visor de su sesión. La copia del
    // estado se toma fuera del cierre y se escribe bajo el candado dentro,
    // igual que el resto de filas pulsables de este archivo.
    let state_for_agent_rows = Arc::clone(&plugin.state);
    let agent_rows = {
        let guard = plugin
            .agents
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // El clic necesita el directorio de presencia para construir el
        // comando del visor; se copia antes del préstamo.
        let presence_dir = guard.presence_dir().to_path_buf();
        guard
            .snapshot_visible(agents::DEFAULT_VISIBLE_LIMIT)
            .into_iter()
            .map(|entry| {
                let dot = if entry.status.is_live() {
                    accent
                } else {
                    to_hsla(COLOR_FAINT)
                };
                // El cierre se puede invocar más de una vez, así que la
                // fila se captura por valor en lugar de prestarse.
                let row_entry = entry.clone();
                let state_for_click = Arc::clone(&state_for_agent_rows);
                let presence_dir_for_click = presence_dir.clone();
                // Claves de las pruebas de layout: identifican cada texto de la
                // tarjeta por el id de la tarea (ver `layout_test`).
                let name_key = format!("herdr-sidebar-agent-name-{}", entry.id);
                let label_key = format!("herdr-sidebar-agent-label-{}", entry.id);
                let step_key = format!("herdr-sidebar-agent-step-{}", entry.id);
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .px(px(8.0))
                    .py(px(6.0))
                    .rounded(px(6.0))
                    .bg(accent.opacity(0.10))
                    .border_1()
                    .border_color(accent.opacity(0.28))
                    .cursor_pointer()
                    .hover(move |style| style.bg(accent.opacity(0.18)))
                    .on_mouse_down(
                        MouseButton::Left,
                        move |_event, window: &mut Window, _cx| {
                            // Sin identidad de sesión no hay nada que seguir.
                            if request_watch(&state_for_click, &presence_dir_for_click, &row_entry)
                            {
                                window.refresh();
                            }
                        },
                    )
                    .child(
                        // Igual que en SPACES: el nombre cede ancho antes de
                        // empujar el estado fuera de la tarjeta. `flex_shrink_0`
                        // en el punto y en el estado los deja intactos.
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .gap(px(6.0))
                            .min_w_0()
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap(px(6.0))
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .flex_shrink_0()
                                            .w(px(6.0))
                                            .h(px(6.0))
                                            .rounded(px(3.0))
                                            .bg(dot),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.0))
                                            .font_weight(FontWeight::BOLD)
                                            .truncate()
                                            .debug_selector(move || name_key)
                                            .text_color(to_hsla(if entry.status.is_live() {
                                                COLOR_NAME_ACTIVE
                                            } else {
                                                COLOR_MUTED
                                            }))
                                            .child(entry.agent.clone()),
                                    ),
                            )
                            .child(
                                // El estado se pinta entero cuando es corto
                                // ("running"), pero `Unknown` trae texto crudo
                                // de la presencia: se acota para que no empuje
                                // el nombre fuera de la tarjeta.
                                div()
                                    .flex_shrink_0()
                                    .max_w(px(72.0))
                                    .truncate()
                                    .text_size(px(10.0))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(if entry.status.is_live() {
                                        accent
                                    } else {
                                        to_hsla(COLOR_FAINT)
                                    })
                                    .child(entry.status.label()),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .truncate()
                            .debug_selector(move || label_key)
                            .text_color(to_hsla(COLOR_MUTED))
                            .child(if entry.label.is_empty() {
                                entry.id.clone()
                            } else {
                                entry.label.clone()
                            }),
                    )
                    .child(
                        div()
                            .text_size(px(10.0))
                            .truncate()
                            .debug_selector(move || step_key)
                            .text_color(to_hsla(COLOR_FAINT))
                            .child(clip_last_step(&entry.last_step)),
                    )
            })
            .collect::<Vec<_>>()
    };

    // Fondo de la sidebar: una capa opcional sobre el fondo que el armazón ya
    // pinta detrás del cromo de un plugin (el mismo de la rejilla, con su
    // opacidad). En 0 (default) el panel se ve exactamente como la terminal;
    // subirlo lo tiñe con el fondo del tema —y componer dos capas translúcidas
    // lo deja más opaco que el resto de la ventana—. El plugin no puede pintar
    // el fondo de la aplicación: ni el color del tema ni la opacidad efectiva
    // están en el contrato del hook de layout.
    let width = state_sidebar_width;
    let resizing = state_is_resizing;
    let sidebar = div()
        .w(px(width))
        .h_full()
        .bg(term_bg.opacity(opacity))
        .border_r(px(1.0))
        .border_color(accent.opacity(0.30))
        .p(px(12.0))
        // Red de seguridad: aunque una fila futura olvide recortar su texto,
        // nada se dibuja fuera del borde del panel.
        .overflow_hidden()
        // Clave de las pruebas de layout (ver `layout_test`).
        .debug_selector(|| "herdr-sidebar".to_string())
        .flex()
        .flex_col()
        .justify_between()
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(10.0))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .px(px(6.0))
                        .child(
                            div()
                                .text_size(px(11.0))
                                .font_weight(FontWeight::BOLD)
                                .text_color(to_hsla(COLOR_MUTED))
                                .child("SPACES"),
                        )
                        .child(
                            div()
                                .px(px(6.0))
                                .py(px(1.0))
                                .rounded(px(4.0))
                                .bg(accent.opacity(0.20))
                                .border_1()
                                .border_color(accent.opacity(0.40))
                                .text_size(px(10.0))
                                .font_weight(FontWeight::BOLD)
                                .text_color(accent)
                                .child(format!("{}", spaces.len())),
                        ),
                )
                .child(spaces_list)
                // Subagentes de pi. Solo ocupa sitio si hay alguno, y solo
                // muestra los primeros: hay cientos en el historico y todos
                // a la vez no aportan nada.
                .when(!agent_rows.is_empty(), |this| {
                    this.child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .px(px(6.0))
                            .pt(px(10.0))
                            .mt(px(4.0))
                            .border_t(px(1.0))
                            .border_color(accent.opacity(0.25))
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(to_hsla(COLOR_MUTED))
                                    .child("AGENTS"),
                            )
                            .child(
                                div()
                                    .px(px(6.0))
                                    .py(px(1.0))
                                    .rounded(px(4.0))
                                    .bg(accent.opacity(0.20))
                                    .border_1()
                                    .border_color(accent.opacity(0.40))
                                    .text_size(px(10.0))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(accent)
                                    .child(format!("{}", agent_rows.len())),
                            ),
                    )
                })
                .when(!agent_rows.is_empty(), |this| {
                    this.child(div().flex().flex_col().gap(px(4.0)).children(agent_rows))
                }),
        )
        .child(
            div()
                .px(px(6.0))
                .py(px(6.0))
                .text_size(px(11.0))
                .truncate()
                .text_color(to_hsla(COLOR_FAINT))
                .child("Ctrl+Alt+T Nuevo espacio"),
        );

    // Contenedor relativo: el asidero y la capa de arrastre se posicionan
    // contra su borde derecho.
    let mut shell = div().relative().w(px(width)).h_full().child(sidebar);

    // Salida única de los dos gestos de la barra.
    //
    // Vive en el armazón —que existe en todos los cuadros— y en fase de
    // captura, así que llega aunque el puntero esté sobre la terminal y aunque el
    // `mouse_up` ocurra antes de que se pinte la capa de arrastre. Con el
    // listener en esa capa, un clic rápido dejaba el gesto pegado: el siguiente
    // movimiento del mouse reordenaba los espacios sin ningún botón apretado y
    // la ventana quedaba pidiendo cuadros para siempre.
    let state_for_up = Arc::clone(&plugin.state);
    shell = shell.capture_any_mouse_up(move |_event, window: &mut Window, _cx| {
        let ended = {
            let mut s = state_for_up.write().unwrap();
            let resizing = s.resize_anchor_x.take().is_some();
            let dragging = s.space_drag.is_some();
            if dragging {
                s.end_space_drag();
            }
            resizing || dragging
        };
        if ended {
            window.refresh();
        }
    });

    // Asidero de ancho: toda la zona sensible queda dentro del panel, con una
    // pista visible para que se sepa que el borde se puede agarrar.
    let state_for_begin = Arc::clone(&plugin.state);
    let handle = div()
        .absolute()
        .left(px(width - RESIZE_HANDLE_HIT))
        .top(px(0.0))
        .h_full()
        .w(px(RESIZE_HANDLE_HIT))
        .cursor_col_resize()
        .hover(move |style| style.bg(accent.opacity(0.18)))
        .child(
            // Pista visual de que el borde es agarrable.
            div()
                .absolute()
                .right(px(0.0))
                .top(px(0.0))
                .h_full()
                .w(px(RESIZE_GRIP_WIDTH))
                .bg(accent.opacity(if resizing { 0.85 } else { 0.35 })),
        )
        .on_mouse_down(MouseButton::Left, move |event, window, _cx| {
            let x: f32 = event.position.x.into();
            {
                let mut s = state_for_begin.write().unwrap();
                s.resize_anchor_x = Some(x);
                s.resize_start_width = s.sidebar_width;
            }
            window.refresh();
        });

    shell = shell.child(handle);

    // Mientras se arrastra el ancho, una capa invisible cubre la ventana para
    // seguir recibiendo movimiento. Debe abarcar también hacia la IZQUIERDA
    // del asidero: estrechar implica mover el cursor hacia atrás, y si la
    // capa empezara en el borde el rastreo se perdería al primer paso. El
    // final del gesto lo recibe el armazón, no esta capa.
    if resizing {
        let state_for_move = Arc::clone(&plugin.state);
        let overlay = div()
            .absolute()
            .left(px(0.0))
            .top(px(0.0))
            .h_full()
            .w(px(6000.0))
            .cursor_col_resize()
            .on_mouse_move(move |event, window, _cx| {
                // El botón pudo soltarse fuera de la ventana: si el puntero
                // vuelve sin él, el gesto terminó y no hay nada que arrastrar.
                if event.pressed_button != Some(MouseButton::Left) {
                    if state_for_move
                        .write()
                        .unwrap()
                        .resize_anchor_x
                        .take()
                        .is_some()
                    {
                        window.refresh();
                    }
                    return;
                }
                let x: f32 = event.position.x.into();
                let mut s = state_for_move.write().unwrap();
                // `resize_anchor_x` es None cuando el puntero no esta
                // arrastrando la barra. Se comprueba antes de escribir
                // para no ensuciar el estado con un movimiento suelto.
                let anchor = match s.resize_anchor_x {
                    Some(anchor) => anchor,
                    None => return,
                };
                s.sidebar_width = (s.resize_start_width + (x - anchor))
                    .clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH);
                drop(s);
                window.refresh();
            });
        shell = shell.child(overlay);
    }

    // Mientras se arrastra una tarjeta, una capa invisible cubre la ventana para
    // seguir al puntero; el `mouse_up` lo recibe el armazón (ver arriba).
    if drag_id.is_some() {
        let state_for_move = Arc::clone(&plugin.state);
        let overlay = div()
            .absolute()
            .left(px(0.0))
            .top(px(0.0))
            .h_full()
            .w(px(6000.0))
            .cursor_grabbing()
            .on_mouse_move(move |event, window, _cx| {
                // Soltar el botón fuera de la ventana no siempre llega como
                // `mouse_up`; el primer movimiento que vuelve sin el botón
                // cierra el gesto en vez de dejarlo pegado.
                if event.pressed_button != Some(MouseButton::Left) {
                    let mut s = state_for_move.write().unwrap();
                    if s.space_drag.is_some() {
                        s.end_space_drag();
                        drop(s);
                        window.refresh();
                    }
                    return;
                }
                let y: f32 = event.position.y.into();
                let changed = state_for_move.write().unwrap().move_space_drag(y);
                if changed {
                    window.refresh();
                }
            });
        shell = shell.child(overlay);
    }

    Some(shell.into_any_element())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spinner_avanza_por_cuadros_de_80ms_y_envuelve() {
        assert_eq!(spinner_frame(0), "⠋");
        assert_eq!(spinner_frame(79), "⠋");
        assert_eq!(spinner_frame(80), "⠙");
        assert_eq!(spinner_frame(799), "⠏");
        assert_eq!(spinner_frame(800), "⠋");
    }

    #[test]
    fn parpadeo_encendido_solo_la_primera_mitad() {
        assert!(blink_on(0, 1000));
        assert!(blink_on(499, 1000));
        assert!(!blink_on(500, 1000));
        assert!(!blink_on(999, 1000));
        assert!(blink_on(1000, 1000));
        assert!(!blink_on(10, 0));
    }
}
