//! Barra lateral del plugin herdr.
//!
//! Única responsabilidad: dibujar la lista de espacios, la lista de subagentes
//! de `pi` y el asidero que ajusta el ancho de la barra.

use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    div, px, rgb, AnyElement, FontWeight, IntoElement, MouseButton, ParentElement, Styled, Window,
};

use port_plugin_api::AgentState;
use port_term_core::frame::Rgb;

use crate::agents;
use crate::color::to_hsla;
use crate::state::{SIDEBAR_MAX_WIDTH, SIDEBAR_MIN_WIDTH};
use crate::watch::{clip_last_step, request_watch};
use crate::HerdrPlugin;

/// Zona sensible del asidero, en píxeles. Centrada en el borde, se extiende
/// medio ancho a cada lado para que el arrastre no sea preciso al píxel.
const RESIZE_HANDLE_HIT: f32 = 7.0;

/// Color de aviso para agentes bloqueados (`Blocked`).
const COLOR_BLOCKED: Rgb = Rgb::new(0xe5, 0x53, 0x4b);
/// Color de éxito para agentes listos sin ver (`Idle` con `seen = false`).
const COLOR_IDLE_UNSEEN: Rgb = Rgb::new(0x39, 0xc5, 0xcf);
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
    let state = plugin.state.read().unwrap();
    // El mismo criterio que reserva el ancho de la barra decide si se pinta
    // (ver `sidebar_shown`). Si sólo hay un espacio y no hay subagentes
    // visibles, la terminal es limpia.
    if !plugin.sidebar_shown(&state) {
        return None;
    }

    let active_space_idx = state.active_space_index;
    let spaces = state.spaces.clone();
    let opacity = state.opacity;
    let accent = to_hsla(state.effective_accent());
    let state_sidebar_width = state.sidebar_width;
    let state_is_resizing = state.resize_anchor_x.is_some();
    let drag_index = state.space_drag.map(|drag| drag.index);
    let space_agent_states: Vec<Option<(AgentState, bool)>> = spaces
        .iter()
        .map(|space| space.aggregated_agent_state(&state.agents))
        .collect();
    drop(state);

    let warn_hsla = to_hsla(COLOR_BLOCKED);
    let success_hsla = to_hsla(COLOR_IDLE_UNSEEN);
    let term_bg = to_hsla(Rgb::DEFAULT_BG);
    let mut spaces_list = div().flex().flex_col().gap(px(4.0));

    for (i, space) in spaces.iter().enumerate() {
        let is_active = i == active_space_idx;
        let dot_color = space.custom_color.map(to_hsla).unwrap_or(accent);
        let agent_state = space_agent_states[i];
        let is_dragged = drag_index == Some(i);

        // Realce de tarjeta: Blocked o Idle sin ver tiñen borde y fondo de la tarjeta con su color;
        // la tarjeta activa conserva el tinte de acento cuando no hay aviso.
        let (card_bg, card_border) = match agent_state {
            Some((AgentState::Blocked, _)) => (warn_hsla.opacity(0.20), warn_hsla.opacity(0.55)),
            Some((AgentState::Idle, false)) => {
                (success_hsla.opacity(0.12), success_hsla.opacity(0.40))
            }
            _ if is_active => (accent.opacity(0.20), accent.opacity(0.55)),
            _ => (term_bg.opacity(0.0), term_bg.opacity(0.0)),
        };
        // Tarjeta arrastrada: levantada y pulsante para seguirla con la vista
        // mientras se reordena en vivo.
        let (card_bg, card_border) = if is_dragged {
            let lift = if blink_on(agents::now_ms(), 600) {
                0.34
            } else {
                0.20
            };
            (accent.opacity(lift), accent)
        } else {
            (card_bg, card_border)
        };

        // Animación por reloj de render (ver `spinner_frame`/`blink_on`).
        let now_ms = agents::now_ms();
        // Marcador a la derecha de la fila del nombre: glifo + texto corto según estado
        let marker = match agent_state {
            Some((AgentState::Working, _)) => Some((spinner_frame(now_ms), "working", accent)),
            Some((AgentState::Blocked, _)) => {
                let color = if blink_on(now_ms, BLOCKED_BLINK_PERIOD_MS) {
                    warn_hsla
                } else {
                    warn_hsla.opacity(0.25)
                };
                Some(("●", "espera tu respuesta", color))
            }
            Some((AgentState::Idle, false)) => Some(("✓", "listo", success_hsla)),
            Some((AgentState::Idle, true)) | Some((AgentState::Unknown, _)) | None => None,
        };

        let state_for_click = Arc::clone(&plugin.state);
        let mut item = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(8.0))
            .px(px(10.0))
            .py(px(6.0))
            .rounded(px(6.0))
            .bg(card_bg)
            .border_1()
            .border_color(card_border)
            .on_mouse_down(MouseButton::Left, move |event, window: &mut Window, _cx| {
                let y: f32 = event.position.y.into();
                {
                    let mut s = state_for_click.write().unwrap();
                    // No selecciona todavía: si el puntero no se mueve es un
                    // clic y `end_space_drag` seleccionará al soltar; si se
                    // mueve, empieza el reordenamiento en vivo.
                    s.begin_space_drag(i, y);
                }
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
                            .bg(dot_color),
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
                                    .truncate()
                                    .debug_selector(move || {
                                        format!("herdr-sidebar-space-branch-{i}")
                                    })
                                    .text_color(if is_active {
                                        accent
                                    } else {
                                        rgb(0x8b949e).into()
                                    })
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
                    rgb(0x6e7681).into()
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
                                            .text_color(if entry.status.is_live() {
                                                rgb(0xf0f6fc)
                                            } else {
                                                rgb(0x8b949e)
                                            })
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
                                        rgb(0x6e7681).into()
                                    })
                                    .child(entry.status.label()),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .truncate()
                            .debug_selector(move || label_key)
                            .text_color(rgb(0x8b949e))
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
                .truncate()
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

    // Mientras se arrastra una tarjeta, una capa invisible cubre la ventana
    // para seguir al puntero hasta el `mouse_up`, con el mismo patrón del
    // asidero de ancho.
    if drag_index.is_some() {
        let state_for_move = Arc::clone(&plugin.state);
        let state_for_end = Arc::clone(&plugin.state);
        let overlay = div()
            .absolute()
            .left(px(0.0))
            .top(px(0.0))
            .h_full()
            .w(px(6000.0))
            .on_mouse_move(move |event, window, _cx| {
                let y: f32 = event.position.y.into();
                let changed = state_for_move.write().unwrap().move_space_drag(y);
                if changed {
                    window.refresh();
                }
            })
            .on_mouse_up(MouseButton::Left, move |_event, window, _cx| {
                state_for_end.write().unwrap().end_space_drag();
                window.refresh();
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
