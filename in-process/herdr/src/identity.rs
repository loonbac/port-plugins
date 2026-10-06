//! Identidad y nombres visibles del plugin herdr.
//!
//! Única responsabilidad: decidir con qué icono y con qué texto se muestra un
//! programa, una pestaña y la rama git de una carpeta.

use std::path::Path;

use port_term_core::pty::RunningApp;

use crate::state::HerdrTab;

/// Iconos Nerd Font (Symbols Nerd Font Mono) para los programas reconocidos.
pub(crate) const ICON_PI: char = '\u{f03ff}'; // md-pi
pub(crate) const ICON_GEMINI: char = '\u{f0a81}'; // md-zodiac_gemini
pub(crate) const ICON_ROBOT: char = '\u{f06a9}'; // md-robot
pub(crate) const ICON_CHIP: char = '\u{f061a}'; // md-chip
pub(crate) const ICON_HEXAGON: char = '\u{f02d8}'; // md-hexagon
pub(crate) const ICON_CPU: char = '\u{f0ee0}'; // md-cpu_64_bit
const ICON_MEMORY: char = '\u{f035b}'; // md-memory
const ICON_CONSOLE: char = '\u{f018d}'; // md-console
const ICON_CURSOR: char = '\u{f01bf}'; // md-cursor_default_outline

/// Traduce el binario de un programa al icono y nombre legible que muestra la pestaña.
pub(crate) fn app_identity(app: &RunningApp) -> (char, String) {
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
    (ICON_ROBOT, humanize(bin))
}

/// Identidad visible de una pestaña en la barra superior: `(icono, texto)`.
///
/// El título fijado por el plugin manda siempre sobre la app en primer plano:
/// la pestaña del visor corre `bash -c tail|jq`, y esa app no significa nada
/// para el usuario. Sin título fijado, manda la app (icono + nombre legible) y,
/// en reposo, manda el título de la pestaña.
pub(crate) fn tab_identity(tab: &HerdrTab) -> (Option<char>, String) {
    if tab.title_locked {
        return (None, tab.title.clone());
    }
    match tab.app.as_ref() {
        Some(app) => {
            let (icon, label) = app_identity(app);
            (Some(icon), label)
        }
        None => (None, tab.title.clone()),
    }
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

/// Detecta la rama activa de git leyendo directamente `.git/HEAD` sin invocar subprocesos.
pub(crate) fn detect_git_branch(cwd: &Path) -> Option<String> {
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
