//! Barra superior del plugin herdr.
//!
//! Única responsabilidad: dibujar las pestañas del espacio activo y el botón
//! que crea una pestaña nueva.

use std::sync::Arc;

use gpui::prelude::*;
use gpui::{
    div, px, rgb, AnyElement, FontWeight, IntoElement, MouseButton, ParentElement, Styled, Window,
};

use port_term_core::frame::Rgb;

use crate::color::to_hsla;
use crate::identity::tab_identity;
use crate::HerdrPlugin;

pub(super) fn top_bar(plugin: &HerdrPlugin) -> Option<AnyElement> {
    let state = plugin.state.read().unwrap();
    let space_tabs = match state.spaces.get(state.active_space_index) {
        Some(sp) => sp.tabs.clone(),
        None => Vec::new(),
    };
    let active_tab_idx = state
        .spaces
        .get(state.active_space_index)
        .map(|sp| sp.active_tab_index)
        .unwrap_or(0);

    // Si solo hay una pestaña y un solo espacio, no dibujamos barra superior (terminal limpia)
    if space_tabs.len() <= 1 && state.spaces.len() <= 1 {
        return None;
    }

    let opacity = state.opacity;
    let accent = to_hsla(state.effective_accent());
    let term_bg = to_hsla(Rgb::DEFAULT_BG);
    drop(state);

    let mut tabs_row = div().flex().flex_row().items_center().gap(px(6.0));

    for (i, tab) in space_tabs.iter().enumerate() {
        let is_active = i == active_tab_idx;
        let (bg_col, border_col) = if is_active {
            (accent.opacity(0.22), accent.opacity(0.55))
        } else {
            (term_bg.opacity(opacity), accent.opacity(0.18))
        };

        let state_for_click = Arc::clone(&plugin.state);
        let state_for_close = Arc::clone(&plugin.state);
        let tab_pill = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .px(px(10.0))
            .py(px(4.0))
            .rounded(px(6.0))
            .bg(bg_col)
            .border_1()
            .border_color(border_col)
            .on_mouse_down(
                MouseButton::Left,
                move |_event, window: &mut Window, _cx| {
                    let mut s = state_for_click.write().unwrap();
                    let space_idx = s.active_space_index;
                    if let Some(sp) = s.spaces.get_mut(space_idx) {
                        sp.active_tab_index = i;
                    }
                    window.refresh();
                },
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .text_color(if is_active {
                        accent
                    } else {
                        rgb(0x6e7681).into()
                    })
                    .child(">"),
            )
            .child(match tab_identity(tab) {
                // Hay un programa en primer plano: icono Nerd Font + nombre legible
                (Some(icon), label) => div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(accent)
                            .child(icon.to_string()),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .font_weight(if is_active {
                                FontWeight::BOLD
                            } else {
                                FontWeight::NORMAL
                            })
                            .text_color(if is_active {
                                rgb(0xf0f6fc)
                            } else {
                                rgb(0x8b949e)
                            })
                            .child(label),
                    ),
                // Terminal en reposo o título fijado por el plugin: el texto
                // de la pestaña (nombre del subagente en el visor).
                (None, label) => div()
                    .text_size(px(12.0))
                    .font_weight(if is_active {
                        FontWeight::BOLD
                    } else {
                        FontWeight::NORMAL
                    })
                    .text_color(if is_active {
                        rgb(0xf0f6fc)
                    } else {
                        rgb(0x8b949e)
                    })
                    .child(label),
            })
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(rgb(0x6e7681))
                    .on_mouse_down(
                        MouseButton::Left,
                        move |_event, window: &mut Window, _cx| {
                            let mut s = state_for_close.write().unwrap();
                            let space_idx = s.active_space_index;
                            // Mismo cierre que `Ctrl+W`/`close_active_tab`, en un
                            // solo lugar: una pestaña del visor se restaura, no
                            // se borra.
                            if s.close_tab_at(space_idx, i) {
                                window.refresh();
                            }
                        },
                    )
                    .child("×"),
            );

        tabs_row = tabs_row.child(tab_pill);
    }

    let state_for_new = Arc::clone(&plugin.state);
    let add_tab_btn = div()
        .flex()
        .items_center()
        .justify_center()
        .w(px(24.0))
        .h(px(24.0))
        .rounded(px(4.0))
        .bg(accent.opacity(0.15))
        .border_1()
        .border_color(accent.opacity(0.35))
        .on_mouse_down(
            MouseButton::Left,
            move |_event, window: &mut Window, _cx| {
                let mut s = state_for_new.write().unwrap();
                // La pestaña nace pendiente: nunca hereda la sesión 0 de otra
                // pestaña mientras el núcleo no le crea la suya.
                s.create_pending_tab_in_active_space();
                window.refresh();
            },
        )
        .child(div().text_size(px(13.0)).text_color(accent).child("+"));

    tabs_row = tabs_row.child(add_tab_btn);

    let top_bar = div()
        .h(px(38.0))
        .w_full()
        .bg(term_bg.opacity(opacity))
        .border_b(px(1.0))
        .border_color(accent.opacity(0.30))
        .px(px(10.0))
        .flex()
        .flex_row()
        .items_center()
        .child(tabs_row);

    Some(top_bar.into_any_element())
}
