//! Barra lateral del plugin herdr.
//!
//! Única responsabilidad: dibujar la lista de espacios, la lista de subagentes
//! de `pi` y el asidero que ajusta el ancho de la barra.

use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    div, px, rgb, AnyElement, FontWeight, IntoElement, MouseButton, ParentElement, Styled, Window,
};

use port_term_core::frame::Rgb;

use crate::agents;
use crate::color::to_hsla;
use crate::state::{SIDEBAR_MAX_WIDTH, SIDEBAR_MIN_WIDTH};
use crate::watch::{clip_last_step, request_watch};
use crate::HerdrPlugin;

/// Zona sensible del asidero, en píxeles. Centrada en el borde, se extiende
/// medio ancho a cada lado para que el arrastre no sea preciso al píxel.
const RESIZE_HANDLE_HIT: f32 = 7.0;

pub(super) fn left_sidebar(plugin: &HerdrPlugin) -> Option<AnyElement> {
    let state = plugin.state.read().unwrap();
    // Subagentes de pi, leidos en vivo. Se consultan ANTES del gate: si
    // hay alguno visible, la sidebar tiene que aparecer aunque solo haya un
    // espacio, que es el estado normal de una terminal recien abierta.
    // El reloj se toma una sola vez por render y se reutiliza en la lista.
    let now = agents::now_ms();

    // El mismo criterio que reserva el ancho de la barra decide si se pinta
    // (ver `sidebar_shown`). Si sólo hay un espacio y no hay subagentes
    // visibles, la terminal es limpia.
    if !plugin.sidebar_shown(&state, now) {
        return None;
    }

    let active_space_idx = state.active_space_index;
    let spaces = state.spaces.clone();
    let opacity = state.opacity;
    let accent = to_hsla(state.effective_accent());
    let state_sidebar_width = state.sidebar_width;
    let state_is_resizing = state.resize_anchor_x.is_some();
    drop(state);

    let term_bg = to_hsla(Rgb::DEFAULT_BG);
    let mut spaces_list = div().flex().flex_col().gap(px(4.0));

    for (i, space) in spaces.iter().enumerate() {
        let is_active = i == active_space_idx;
        let dot_color = space.custom_color.map(to_hsla).unwrap_or(accent);

        // Fondo y borde del espacio: transparente con tinte del acento si está activo
        let (card_bg, card_border) = if is_active {
            (accent.opacity(0.20), accent.opacity(0.55))
        } else {
            (term_bg.opacity(0.0), term_bg.opacity(0.0))
        };

        let state_for_click = Arc::clone(&plugin.state);
        let item = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px(px(10.0))
            .py(px(6.0))
            .rounded(px(6.0))
            .bg(card_bg)
            .border_1()
            .border_color(card_border)
            .on_mouse_down(
                MouseButton::Left,
                move |_event, window: &mut Window, _cx| {
                    let mut s = state_for_click.write().unwrap();
                    s.active_space_index = i;
                    window.refresh();
                },
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .child(div().w(px(8.0)).h(px(8.0)).rounded(px(4.0)).bg(dot_color))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(13.0))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(if is_active {
                                        rgb(0xf0f6fc)
                                    } else {
                                        rgb(0xc9d1d9)
                                    })
                                    .child(space.name.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(if is_active {
                                        accent
                                    } else {
                                        rgb(0x8b949e).into()
                                    })
                                    .child(space.branch.clone()),
                            ),
                    ),
            );

        spaces_list = spaces_list.child(item);
    }

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
        let mut guard = plugin
            .agents
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // El clic necesita el directorio de presencia para construir el
        // comando del visor; se copia antes del préstamo mutable.
        let presence_dir = guard.presence_dir().to_path_buf();
        guard
            .visible(now, agents::DEFAULT_VISIBLE_LIMIT)
            .into_iter()
            .map(|entry| {
                let dot = if entry.status.is_live() {
                    accent
                } else {
                    rgb(0x6e7681).into()
                };
                // El cierre se puede invocar más de una vez, así que la
                // fila se captura por valor en lugar de prestarse.
                let row_entry = entry.clone();
                let state_for_click = Arc::clone(&state_for_agent_rows);
                let presence_dir_for_click = presence_dir.clone();
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
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap(px(6.0))
                                    .child(div().w(px(6.0)).h(px(6.0)).rounded(px(3.0)).bg(dot))
                                    .child(
                                        div()
                                            .text_size(px(12.0))
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(if entry.status.is_live() {
                                                rgb(0xf0f6fc)
                                            } else {
                                                rgb(0x8b949e)
                                            })
                                            .child(entry.agent.clone()),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(if entry.status.is_live() {
                                        accent
                                    } else {
                                        rgb(0x6e7681).into()
                                    })
                                    .child(entry.status.label()),
                            ),
                    )
                    .child(div().text_size(px(11.0)).text_color(rgb(0x8b949e)).child(
                        if entry.label.is_empty() {
                            entry.id.clone()
                        } else {
                            entry.label.clone()
                        },
                    ))
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(rgb(0x6e7681))
                            .child(clip_last_step(&entry.last_step)),
                    )
            })
            .collect::<Vec<_>>()
    };

    // Fondo de la sidebar: con `opacity` en 0 (default) no pinta nada y
    // se ve el fondo de la ventana, idéntico al de la terminal.
    let width = state_sidebar_width;
    let resizing = state_is_resizing;
    let sidebar = div()
        .w(px(width))
        .h_full()
        .bg(term_bg.opacity(opacity))
        .border_r(px(1.0))
        .border_color(accent.opacity(0.30))
        .p(px(12.0))
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
                                .text_color(rgb(0x8b949e))
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
                                    .text_color(rgb(0x8b949e))
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
                .text_color(rgb(0x6e7681))
                .child("Ctrl+Alt+T Nuevo espacio"),
        );

    // Contenedor relativo: el asidero y la capa de arrastre se posicionan
    // contra su borde derecho.
    let mut shell = div().relative().w(px(width)).h_full().child(sidebar);

    // Asidero de ancho: zona sensible centrada en el borde derecho.
    let state_for_begin = Arc::clone(&plugin.state);
    let handle = div()
        .absolute()
        .left(px(width - RESIZE_HANDLE_HIT))
        .top(px(0.0))
        .h_full()
        .w(px(RESIZE_HANDLE_HIT * 2.0))
        .cursor_col_resize()
        .child(
            // Pista visual de que el borde es agarrable.
            div()
                .absolute()
                .right(px(0.0))
                .top(px(0.0))
                .h_full()
                .w(px(2.0))
                .bg(accent.opacity(0.0)),
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

    // Mientras se arrastra, una capa invisible cubre toda la ventana para
    // seguir recibiendo movimiento. Debe abarcar también hacia la IZQUIERDA
    // del asidero: estrechar implica mover el cursor hacia atrás, y si la
    // capa empezara en el borde el rastreo se perdería al primer paso.
    if resizing {
        let state_for_move = Arc::clone(&plugin.state);
        let state_for_end = Arc::clone(&plugin.state);
        let overlay = div()
            .absolute()
            .left(px(0.0))
            .top(px(0.0))
            .h_full()
            .w(px(6000.0))
            .cursor_col_resize()
            .on_mouse_move(move |event, window, _cx| {
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
            })
            .on_mouse_up(MouseButton::Left, move |_event, window, _cx| {
                state_for_end.write().unwrap().resize_anchor_x = None;
                window.refresh();
            });
        shell = shell.child(overlay);
    }

    Some(shell.into_any_element())
}
