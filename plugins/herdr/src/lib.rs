//! Herdr Customization Plugin para PORT.
//!
//! Transforma PORT en el entorno visual y de flujo de trabajo de Herdr:
//! - Inicio limpio: al abrir la terminal es 100 % terminal sin barras invasivas.
//! - Con `Ctrl + Shift + T` se crea una nueva pestaña en el espacio activo, con su propio shell PTY.
//! - Con `Alt + Left` y `Alt + Right` se navega entre las pestañas del espacio activo.
//! - Con `Ctrl + Alt + T` se crea un nuevo Espacio (Space) aparte con su propia terminal y la barra lateral izquierda queda fija y visible.
//! - Barra lateral y superior completamente transparentes, compartiendo el color de la terminal.
//! - Color de acento sincronizado en tiempo real con el wallpaper de NixOS (`~/.config/mpvpaper/accent.txt`).
//! - Configurable y sincronizado a través de `~/.config/port/config.md`.

use std::path::Path;
use std::sync::{Arc, RwLock};

use gpui::prelude::*;
use gpui::{
    div, px, rgb, AnyElement, FontWeight, IntoElement, MouseButton, ParentElement, Styled, Window,
};
use port_plugin_api::{
    AppearanceHook, ConfigFile, InputHook, KeyAction, LayoutHook, Plugin, PluginConfig, SpaceHook,
};
use port_term_core::frame::Rgb;
use port_term_core::input::Key;
use port_term_core::pty::RunningApp;

/// Obtiene el color de acento del wallpaper activo en NixOS (`~/.config/mpvpaper/accent.txt`).
pub fn system_accent_color() -> Rgb {
    if let Ok(home) = std::env::var("HOME") {
        let path = std::path::Path::new(&home).join(".config/mpvpaper/accent.txt");
        if let Ok(content) = std::fs::read_to_string(path) {
            if let Some(rgb) = parse_hex_color(content.trim()) {
                return rgb;
            }
        }
    }
    // Color de acento por defecto de NixOS (#325573)
    Rgb::new(0x32, 0x55, 0x73)
}

/// Parsea una cadena hexadecimal en formato `#RRGGBB` o `RRGGBB`.
pub fn parse_hex_color(hex: &str) -> Option<Rgb> {
    let clean = hex.strip_prefix('#').unwrap_or(hex).trim();
    if clean.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&clean[0..2], 16).ok()?;
    let g = u8::from_str_radix(&clean[2..4], 16).ok()?;
    let b = u8::from_str_radix(&clean[4..6], 16).ok()?;
    Some(Rgb::new(r, g, b))
}

fn to_hsla(rgb_val: Rgb) -> gpui::Hsla {
    let packed =
        ((rgb_val.r as u32) << 16) | ((rgb_val.g as u32) << 8) | (rgb_val.b as u32);
    rgb(packed).into()
}

/// Definición de una pestaña de terminal con su sesión asociada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HerdrTab {
    pub id: usize,
    pub title: String,
    pub session_id: usize,
    /// Programa que se está ejecutando en primer plano en esta pestaña.
    pub app: Option<RunningApp>,
}

/// Identificador centinela para una pestaña que todavía no tiene sesión asignada.
/// Nunca colisiona con un id real de `SessionManager`, que empieza en 0.
pub const PENDING_SESSION: usize = usize::MAX;

/// Iconos Nerd Font (Symbols Nerd Font Mono) para los programas reconocidos.
const ICON_PI: char = '\u{f03ff}'; // md-pi
const ICON_GEMINI: char = '\u{f0a81}'; // md-zodiac_gemini
const ICON_ROBOT: char = '\u{f06a9}'; // md-robot
const ICON_CHIP: char = '\u{f061a}'; // md-chip
const ICON_HEXAGON: char = '\u{f02d8}'; // md-hexagon
const ICON_CPU: char = '\u{f0ee0}'; // md-cpu_64_bit
const ICON_MEMORY: char = '\u{f035b}'; // md-memory
const ICON_CONSOLE: char = '\u{f018d}'; // md-console
const ICON_CURSOR: char = '\u{f01bf}'; // md-cursor_default_outline

/// Traduce el binario de un programa al icono y nombre legible que muestra la pestaña.
fn app_identity(app: &RunningApp) -> (char, String) {
    let bin = app.bin.as_str();

    // Agentes de IA
    if bin == "pi" || bin.starts_with("pi-") || bin.starts_with("pi_") {
        return (ICON_PI, "Pi Agent".to_string());
    }
    if bin.contains("codex") {
        return (ICON_CHIP, "Codex".to_string());
    }
    if bin.contains("claude") {
        return (ICON_ROBOT, "Claude Code".to_string());
    }
    if bin.contains("antigravity") {
        return (ICON_HEXAGON, "Antigravity".to_string());
    }
    if bin.contains("opencode") {
        return (ICON_CPU, "OpenCode".to_string());
    }
    if bin.contains("gemini") {
        return (ICON_GEMINI, "Gemini CLI".to_string());
    }
    if bin.contains("aider") {
        return (ICON_MEMORY, "Aider".to_string());
    }
    if bin.contains("cursor") {
        return (ICON_CURSOR, "Cursor Agent".to_string());
    }

    // Utilidades de terminal conocidas
    match bin {
        "btop" | "htop" | "top" | "bpytop" => {
            return (ICON_CPU, "System Monitor".to_string());
        }
        "nvim" | "vim" | "vi" | "nvim-qt" => {
            return (ICON_MEMORY, "Editor".to_string());
        }
        "less" | "more" | "man" => {
            return (ICON_CONSOLE, "Pager".to_string());
        }
        "python" | "python3" | "node" | "deno" | "bun" => {
            return (ICON_CHIP, "Runtime".to_string());
        }
        _ => {}
    }

    // Cualquier otro agente o programa desconocido: robot + nombre capitalizado.
    (
        ICON_ROBOT,
        humanize(bin),
    )
}

/// Convierte `opencode` en `Opencode` para mostrarlo como nombre legible.
fn humanize(bin: &str) -> String {
    let mut chars = bin.chars();
    match chars.next() {
        Some(first) => {
            let mut out: String = first.to_uppercase().collect();
            out.push_str(chars.as_str());
            out
        }
        None => bin.to_string(),
    }
}

/// Definición de un espacio de trabajo en Herdr.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HerdrSpace {
    pub name: String,
    pub branch: String,
    pub custom_color: Option<Rgb>,
    pub tabs: Vec<HerdrTab>,
    pub active_tab_index: usize,
}

/// Estado interno del plugin Herdr.
#[derive(Debug, Clone)]
pub struct HerdrState {
    pub active_space_index: usize,
    pub spaces: Vec<HerdrSpace>,
    pub opacity: f32,
    pub accent_mode: String,
    pub new_session_requested: bool,
    pub close_session_requested: Option<usize>,
    next_tab_id: usize,
    next_space_num: usize,
}

impl HerdrState {
    /// Resuelve el color de acento actual (leyendo el wallpaper si está en modo "auto").
    pub fn effective_accent(&self) -> Rgb {
        if self.accent_mode.to_lowercase() == "auto" {
            system_accent_color()
        } else if let Some(rgb) = parse_hex_color(&self.accent_mode) {
            rgb
        } else {
            system_accent_color()
        }
    }
}

impl Default for HerdrState {
    fn default() -> Self {
        Self {
            // Inicialmente 1 espacio limpio: la terminal abre pura a pantalla completa
            active_space_index: 0,
            spaces: vec![HerdrSpace {
                name: "space-1".to_string(),
                branch: "main".to_string(),
                custom_color: None,
                tabs: vec![HerdrTab {
                    id: 1,
                    title: "terminal".to_string(),
                    session_id: 0,
                    app: None,
                }],
                active_tab_index: 0,
            }],
            opacity: 0.85,
            accent_mode: "auto".to_string(),
            new_session_requested: false,
            close_session_requested: None,
            next_tab_id: 2,
            next_space_num: 2,
        }
    }
}

/// Plugin de personalización visual y estructural Herdr.
pub struct HerdrPlugin {
    state: Arc<RwLock<HerdrState>>,
}

impl HerdrPlugin {
    /// Crea una nueva instancia del plugin Herdr con valores por defecto.
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(HerdrState::default())),
        }
    }

    /// Crea un nuevo espacio aparte del actual, solicita una nueva sesión PTY y lo activa.
    pub fn create_space_and_open_sidebar(&self) {
        let mut s = self.state.write().unwrap();
        let num = s.next_space_num;
        s.next_space_num += 1;

        let next_tab = s.next_tab_id;
        s.next_tab_id += 1;

        s.spaces.push(HerdrSpace {
            name: format!("space-{num}"),
            branch: "main".to_string(),
            custom_color: None,
            tabs: vec![HerdrTab {
                id: next_tab,
                title: "terminal".to_string(),
                session_id: PENDING_SESSION, // on_session_created lo reemplaza
                app: None,
            }],
            active_tab_index: 0,
        });

        s.active_space_index = s.spaces.len() - 1;
        s.new_session_requested = true;
    }

    /// Crea una nueva pestaña dentro del espacio actualmente activo.
    pub fn create_tab_in_active_space(&self) {
        let mut s = self.state.write().unwrap();
        let tab_id = s.next_tab_id;
        s.next_tab_id += 1;

        let space_idx = s.active_space_index;
        if let Some(space) = s.spaces.get_mut(space_idx) {
            space.tabs.push(HerdrTab {
                id: tab_id,
                title: format!("term {}", space.tabs.len() + 1),
                session_id: PENDING_SESSION, // on_session_created lo reemplaza
                app: None,
            });
            space.active_tab_index = space.tabs.len() - 1;
            s.new_session_requested = true;
        }
    }

    /// Cambia a la pestaña anterior dentro del espacio actual.
    pub fn select_previous_tab(&self) -> bool {
        let mut s = self.state.write().unwrap();
        let space_idx = s.active_space_index;
        if let Some(space) = s.spaces.get_mut(space_idx) {
            if space.tabs.len() > 1 {
                if space.active_tab_index == 0 {
                    space.active_tab_index = space.tabs.len() - 1;
                } else {
                    space.active_tab_index -= 1;
                }
                return true;
            }
        }
        false
    }

    /// Cambia a la pestaña siguiente dentro del espacio actual.
    pub fn select_next_tab(&self) -> bool {
        let mut s = self.state.write().unwrap();
        let space_idx = s.active_space_index;
        if let Some(space) = s.spaces.get_mut(space_idx) {
            if space.tabs.len() > 1 {
                space.active_tab_index = (space.active_tab_index + 1) % space.tabs.len();
                return true;
            }
        }
        false
    }

    /// Selecciona un espacio por índice.
    pub fn select_space(&self, index: usize) {
        let mut s = self.state.write().unwrap();
        if index < s.spaces.len() {
            s.active_space_index = index;
        }
    }

    /// Selecciona una pestaña por índice dentro del espacio activo.
    pub fn select_tab(&self, index: usize) {
        let mut s = self.state.write().unwrap();
        let space_idx = s.active_space_index;
        if let Some(space) = s.spaces.get_mut(space_idx) {
            if index < space.tabs.len() {
                space.active_tab_index = index;
            }
        }
    }

    /// Cierra la pestaña activa en el espacio actual si hay más de una.
    pub fn close_active_tab(&self) -> bool {
        let mut s = self.state.write().unwrap();
        let space_idx = s.active_space_index;
        let mut closed_session = None;
        if let Some(space) = s.spaces.get_mut(space_idx) {
            if space.tabs.len() > 1 {
                let tab_idx = space.active_tab_index;
                let removed_tab = space.tabs.remove(tab_idx);
                closed_session = Some(removed_tab.session_id);
                if space.active_tab_index >= space.tabs.len() {
                    space.active_tab_index = space.tabs.len() - 1;
                }
            }
        }
        if let Some(sess_id) = closed_session {
            s.close_session_requested = Some(sess_id);
            true
        } else {
            false
        }
    }

    /// Guarda la configuración actual en la ruta predeterminada (`~/.config/port/config.md`).
    pub fn save_to_default_file(&self) -> std::io::Result<()> {
        if let Some(config) = self.save_config() {
            ConfigFile::save_plugin(&ConfigFile::default_path(), self.id(), &config)?;
        }
        Ok(())
    }
}

impl Default for HerdrPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl AppearanceHook for HerdrPlugin {
    fn background_tint(&self, base: Rgb) -> Rgb {
        base
    }

    fn opacity(&self) -> Option<f32> {
        Some(self.state.read().unwrap().opacity)
    }
}

impl SpaceHook for HerdrPlugin {
    fn active_session(&self) -> usize {
        let s = self.state.read().unwrap();
        if let Some(space) = s.spaces.get(s.active_space_index) {
            if let Some(tab) = space.tabs.get(space.active_tab_index) {
                return tab.session_id;
            }
        }
        0
    }

    fn active_space(&self) -> usize {
        self.state.read().unwrap().active_space_index
    }

    fn take_new_session_request(&self) -> bool {
        let mut s = self.state.write().unwrap();
        let req = s.new_session_requested;
        s.new_session_requested = false;
        req
    }

    fn on_session_created(&self, session_id: usize) {
        let mut s = self.state.write().unwrap();
        let space_idx = s.active_space_index;
        if let Some(space) = s.spaces.get_mut(space_idx) {
            let tab_idx = space.active_tab_index;
            if let Some(tab) = space.tabs.get_mut(tab_idx) {
                tab.session_id = session_id;
            }
        }
    }

    fn update_session_app(&self, session_id: usize, app: Option<&RunningApp>) {
        let mut s = self.state.write().unwrap();
        for space in s.spaces.iter_mut() {
            for tab in space.tabs.iter_mut() {
                // Una pestaña sin sesión asignada no puede recibir datos: si se
                // aceptara, heredaría la app de otra pestaña por colisión de id.
                if tab.session_id == session_id && tab.session_id != PENDING_SESSION {
                    tab.app = app.cloned();
                }
            }
        }
    }

    fn take_close_session_request(&self) -> Option<usize> {
        let mut s = self.state.write().unwrap();
        s.close_session_requested.take()
    }

    fn update_session_cwd(&self, session_id: usize, cwd: &Path, folder_name: &str) {
        let mut s = self.state.write().unwrap();
        for space in s.spaces.iter_mut() {
            // Solo el espacio propietario de la sesión se renombra.
            let contains_session = space
                .tabs
                .iter()
                .any(|t| t.session_id == session_id && t.session_id != PENDING_SESSION);
            if !contains_session {
                continue;
            }

            if !folder_name.is_empty() {
                space.name = folder_name.to_string();
            }

            // Si la pestaña tiene nombre por defecto, se actualiza con la carpeta.
            for tab in space.tabs.iter_mut() {
                if tab.session_id == session_id
                    && tab.session_id != PENDING_SESSION
                    && (tab.title == "terminal" || tab.title.starts_with("term "))
                {
                    tab.title = folder_name.to_string();
                }
            }

            // Detecta automáticamente la rama git si la carpeta está en un repositorio.
            space.branch = detect_git_branch(cwd).unwrap_or_else(|| "local".to_string());
            break;
        }
    }
}

/// Detecta la rama activa de git leyendo directamente `.git/HEAD` sin invocar subprocesos.
fn detect_git_branch(cwd: &Path) -> Option<String> {
    let mut current = Some(cwd);
    while let Some(dir) = current {
        let git_head = dir.join(".git").join("HEAD");
        if git_head.exists() {
            if let Ok(content) = std::fs::read_to_string(&git_head) {
                let trimmed = content.trim();
                if let Some(branch) = trimmed.strip_prefix("ref: refs/heads/") {
                    return Some(branch.to_string());
                } else if trimmed.len() >= 7 {
                    return Some(trimmed[..7].to_string());
                }
            }
            break;
        }
        current = dir.parent();
    }
    None
}

impl LayoutHook for HerdrPlugin {
    fn left_sidebar_width(&self) -> f32 {
        let s = self.state.read().unwrap();
        if s.spaces.len() > 1 {
            236.0
        } else {
            0.0
        }
    }

    fn top_bar_height(&self) -> f32 {
        let s = self.state.read().unwrap();
        let space_tabs_len = s
            .spaces
            .get(s.active_space_index)
            .map(|sp| sp.tabs.len())
            .unwrap_or(0);

        if space_tabs_len > 1 || s.spaces.len() > 1 {
            38.0
        } else {
            0.0
        }
    }

    fn left_sidebar(&self) -> Option<AnyElement> {
        let state = self.state.read().unwrap();
        // Si solo hay un espacio, la terminal es limpia y no dibuja barra lateral
        if state.spaces.len() <= 1 {
            return None;
        }

        let active_space_idx = state.active_space_index;
        let spaces = state.spaces.clone();
        let opacity = state.opacity;
        let accent = to_hsla(state.effective_accent());
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

            let state_for_click = Arc::clone(&self.state);
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
                .on_mouse_down(MouseButton::Left, move |_event, window: &mut Window, _cx| {
                    let mut s = state_for_click.write().unwrap();
                    s.active_space_index = i;
                    window.refresh();
                })
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(10.0))
                        .child(
                            div()
                                .w(px(8.0))
                                .h(px(8.0))
                                .rounded(px(4.0))
                                .bg(dot_color),
                        )
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

        // Sidebar con fondo idéntico al de la terminal y compartiendo su opacidad
        let sidebar = div()
            .w(px(236.0))
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
                    .child(spaces_list),
            )
            .child(
                div()
                    .px(px(6.0))
                    .py(px(6.0))
                    .text_size(px(11.0))
                    .text_color(rgb(0x6e7681))
                    .child("Ctrl+Alt+T Nuevo espacio"),
            );

        Some(sidebar.into_any_element())
    }

    fn top_bar(&self) -> Option<AnyElement> {
        let state = self.state.read().unwrap();
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

            let state_for_click = Arc::clone(&self.state);
            let state_for_close = Arc::clone(&self.state);
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
                .on_mouse_down(MouseButton::Left, move |_event, window: &mut Window, _cx| {
                    let mut s = state_for_click.write().unwrap();
                    let space_idx = s.active_space_index;
                    if let Some(sp) = s.spaces.get_mut(space_idx) {
                        sp.active_tab_index = i;
                    }
                    window.refresh();
                })
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
                .child(match tab.app.as_ref() {
                    // Hay un programa en primer plano: icono Nerd Font + nombre legible
                    Some(app) => {
                        let (icon, label) = app_identity(app);
                        div()
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
                            )
                    }
                    // Terminal en reposo: se muestra la carpeta de la sesión
                    None => div()
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
                        .child(tab.title.clone()),
                })
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(rgb(0x6e7681))
                        .on_mouse_down(MouseButton::Left, move |_event, window: &mut Window, _cx| {
                            let mut s = state_for_close.write().unwrap();
                            let space_idx = s.active_space_index;
                            let mut closed_session = None;
                            if let Some(sp) = s.spaces.get_mut(space_idx) {
                                if sp.tabs.len() > 1 && i < sp.tabs.len() {
                                    let removed = sp.tabs.remove(i);
                                    closed_session = Some(removed.session_id);
                                    if sp.active_tab_index >= sp.tabs.len() {
                                        sp.active_tab_index = sp.tabs.len() - 1;
                                    }
                                }
                            }
                            if let Some(sess_id) = closed_session {
                                s.close_session_requested = Some(sess_id);
                                window.refresh();
                            }
                        })
                        .child("×"),
                );

            tabs_row = tabs_row.child(tab_pill);
        }

        let state_for_new = Arc::clone(&self.state);
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
            .on_mouse_down(MouseButton::Left, move |_event, window: &mut Window, _cx| {
                let mut s = state_for_new.write().unwrap();
                let next_id = s.next_tab_id;
                s.next_tab_id += 1;
                let space_idx = s.active_space_index;
                if let Some(sp) = s.spaces.get_mut(space_idx) {
                    sp.tabs.push(HerdrTab {
                        id: next_id,
                        title: format!("term {}", sp.tabs.len() + 1),
                        session_id: 0,
                        app: None,
                    });
                    sp.active_tab_index = sp.tabs.len() - 1;
                    s.new_session_requested = true;
                }
                window.refresh();
            })
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(accent)
                    .child("+"),
            );

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
}

impl InputHook for HerdrPlugin {
    fn on_key(&self, key: &Key) -> KeyAction {
        // Ctrl + Shift + T: crear nueva pestaña de terminal en el espacio activo
        if key.ctrl && key.shift && !key.alt && key.key.to_lowercase() == "t" {
            self.create_tab_in_active_space();
            return KeyAction::Consume;
        }

        // Alt + Flecha Izquierda: cambiar a la pestaña anterior dentro del espacio actual
        if key.alt && !key.ctrl && !key.shift && (key.key == "Left" || key.key == "left") {
            if self.select_previous_tab() {
                return KeyAction::Consume;
            }
        }

        // Alt + Flecha Derecha: cambiar a la pestaña siguiente dentro del espacio actual
        if key.alt && !key.ctrl && !key.shift && (key.key == "Right" || key.key == "right") {
            if self.select_next_tab() {
                return KeyAction::Consume;
            }
        }

        // Ctrl + Alt + T: crear un nuevo espacio aparte del actual
        if key.ctrl && key.alt && !key.shift && key.key.to_lowercase() == "t" {
            self.create_space_and_open_sidebar();
            return KeyAction::Consume;
        }

        // Ctrl + W: cerrar pestaña activa en el espacio actual
        if key.ctrl && !key.alt && !key.shift && key.key.to_lowercase() == "w" {
            if self.close_active_tab() {
                return KeyAction::Consume;
            }
        }

        // Alt + 1..9: cambiar rápidamente de espacio
        if key.alt && !key.ctrl && !key.shift {
            if let Ok(num) = key.key.parse::<usize>() {
                if (1..=9).contains(&num) {
                    self.select_space(num - 1);
                    return KeyAction::Consume;
                }
            }
        }

        KeyAction::Pass
    }
}

impl Plugin for HerdrPlugin {
    fn id(&self) -> &'static str {
        "herdr"
    }

    fn name(&self) -> &'static str {
        "Herdr Customization Plugin"
    }

    fn version(&self) -> &'static str {
        "0.1.0"
    }

    fn appearance_hook(&self) -> Option<&dyn AppearanceHook> {
        Some(self)
    }

    fn space_hook(&self) -> Option<&dyn SpaceHook> {
        Some(self)
    }

    fn layout_hook(&self) -> Option<&dyn LayoutHook> {
        Some(self)
    }

    fn input_hook(&self) -> Option<&dyn InputHook> {
        Some(self)
    }

    fn default_config(&self) -> Option<PluginConfig> {
        let mut cfg = PluginConfig::new();
        cfg.set("opacity", 0.85);
        cfg.set("accent", "auto");
        cfg.set("spaces", "");
        Some(cfg)
    }

    fn load_config(&self, config: &PluginConfig) {
        let mut s = self.state.write().unwrap();
        if let Some(op) = config.get_f32("opacity") {
            s.opacity = op;
        }
        if let Some(acc) = config.get("accent") {
            s.accent_mode = acc.to_string();
        }
        if let Some(spaces_str) = config.get("spaces") {
            let mut parsed_spaces = Vec::new();
            for (idx, entry) in spaces_str.split(',').enumerate() {
                if let Some((name, branch)) = entry.trim().split_once(':') {
                    if !name.trim().is_empty() {
                        parsed_spaces.push(HerdrSpace {
                            name: name.trim().to_string(),
                            branch: branch.trim().to_string(),
                            custom_color: None,
                            tabs: vec![HerdrTab {
                                id: idx + 1,
                                title: "terminal".to_string(),
                                session_id: idx,
                                app: None,
                            }],
                            active_tab_index: 0,
                        });
                    }
                }
            }
            if !parsed_spaces.is_empty() {
                s.spaces = parsed_spaces;
            }
        }
    }

    fn save_config(&self) -> Option<PluginConfig> {
        let s = self.state.read().unwrap();
        let mut cfg = PluginConfig::new();
        cfg.set("opacity", s.opacity);
        cfg.set("accent", s.accent_mode.clone());
        let spaces_str = s
            .spaces
            .iter()
            .map(|sp| format!("{}:{}", sp.name, sp.branch))
            .collect::<Vec<_>>()
            .join(",");
        cfg.set("spaces", spaces_str);
        Some(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_color_valid() {
        assert_eq!(
            parse_hex_color("#325573"),
            Some(Rgb::new(0x32, 0x55, 0x73))
        );
        assert_eq!(
            parse_hex_color("325573"),
            Some(Rgb::new(0x32, 0x55, 0x73))
        );
        assert_eq!(parse_hex_color("invalid"), None);
    }

    #[test]
    fn herdr_opens_clean_terminal_by_default() {
        let plugin = HerdrPlugin::default();
        assert_eq!(plugin.id(), "herdr");
        assert_eq!(plugin.name(), "Herdr Customization Plugin");
        // Al abrir la terminal por defecto es limpia: sin sidebar ni topbar
        assert_eq!(plugin.left_sidebar_width(), 0.0);
        assert_eq!(plugin.top_bar_height(), 0.0);
        assert!(plugin.left_sidebar().is_none());
        assert!(plugin.top_bar().is_none());
        assert_eq!(plugin.opacity(), Some(0.85));
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
        assert_eq!(plugin.take_new_session_request(), true);

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
        let plugin = HerdrPlugin::default();
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
        assert_eq!(plugin.take_new_session_request(), true);
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
            let app = RunningApp { pid: 1, bin: bin.to_string() };
            let (got_icon, got_label) = app_identity(&app);
            assert_eq!(got_icon, icon, "icono incorrecto para {bin}");
            assert_eq!(got_label, label, "nombre incorrecto para {bin}");
        }
    }

    #[test]
    fn a_new_tab_never_inherits_the_app_of_another_session() {
        let plugin = HerdrPlugin::default();

        // La primera pestaña corresponde a la sesión 0 real.
        let pi = RunningApp { pid: 1, bin: "pi".to_string() };
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
    fn unknown_agents_fall_back_to_robot_and_humanized_name() {
        let app = RunningApp { pid: 1, bin: "miagente".to_string() };
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

        let app = RunningApp { pid: 42, bin: "pi".to_string() };
        plugin.update_session_app(1, Some(&app));

        let s = plugin.state.read().unwrap();
        assert_eq!(s.spaces[0].tabs[1].app, Some(app));
        assert_eq!(s.spaces[0].tabs[0].app, None);
    }
}
