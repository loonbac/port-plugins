//! Plugin de política de ratón y selección para PORT.
//!
//! Es el dueño de la [`MousePolicy`] que la terminal aplica a cada sesión:
//! decide cuándo un gesto pertenece al programa (clics y arrastres) y cuándo
//! vuelve a PORT para la selección local, cómo se copia al soltar y con qué
//! color y opacidad se resalta lo seleccionado.
//!
//! Corre dentro del proceso porque la política es un dato del núcleo que la UI
//! lee en cada gesto; no cruza el protocolo de los plugins de tienda.

use std::sync::RwLock;

use port_plugin_api::{MouseHook, Plugin, PluginConfig};
use port_term_core::frame::Rgb;
use port_term_core::session::MousePolicy;

/// Plugin que guarda y expone la política de ratón y selección.
pub struct SelectionPlugin {
    policy: RwLock<MousePolicy>,
}

impl SelectionPlugin {
    /// Crea una instancia con la política por defecto de PORT.
    pub fn new() -> Self {
        Self {
            policy: RwLock::new(MousePolicy::default()),
        }
    }

    /// Política vigente que el núcleo aplicará a las sesiones.
    pub fn policy(&self) -> MousePolicy {
        *self.policy.read().unwrap()
    }
}

impl Default for SelectionPlugin {
    fn default() -> Self {
        Self::new()
    }
}

/// Parsea un color hexadecimal en formato `#rrggbb` o `rrggbb`.
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

/// Aplica una clave booleana sobre `target`, que ya trae su valor por defecto.
///
/// Sin la clave no hace nada: el campo conserva el default. Un valor que no se
/// puede leer como booleano tampoco lo cambia y se reporta por stderr, igual que
/// el resto de PORT, para que una errata en el archivo no apague un ajuste en
/// silencio.
fn apply_bool(config: &PluginConfig, key: &str, target: &mut bool) {
    let Some(raw) = config.get(key) else {
        return;
    };
    match config.get_bool(key) {
        Some(value) => *target = value,
        None => eprintln!(
            "aviso: el plugin 'selection' ignoró `{key} = {raw}`: no es un booleano válido"
        ),
    }
}

/// Aplica la clave de color sobre `target`, que ya trae su valor por defecto.
///
/// Acepta `#rrggbb` (o `rrggbb`). Un color ilegible conserva el default y se
/// reporta por stderr.
fn apply_color(config: &PluginConfig, key: &str, target: &mut Rgb) {
    let Some(raw) = config.get(key) else {
        return;
    };
    match parse_hex_color(raw) {
        Some(color) => *target = color,
        None => eprintln!(
            "aviso: el plugin 'selection' ignoró `{key} = {raw}`: se esperaba un color #rrggbb"
        ),
    }
}

/// Aplica la clave de opacidad sobre `target`, que ya trae su valor por defecto.
///
/// Recorta el resultado a `0.0..=1.0`. Un valor ilegible conserva el default y
/// se reporta por stderr.
fn apply_opacity(config: &PluginConfig, key: &str, target: &mut f32) {
    let Some(raw) = config.get(key) else {
        return;
    };
    match raw.trim().parse::<f32>() {
        Ok(value) => *target = value.clamp(0.0, 1.0),
        Err(_) => {
            eprintln!("aviso: el plugin 'selection' ignoró `{key} = {raw}`: no es un número válido")
        }
    }
}

impl MouseHook for SelectionPlugin {
    /// La política que PORT aplica al ratón y a la selección.
    fn mouse_policy(&self) -> MousePolicy {
        self.policy()
    }
}

impl Plugin for SelectionPlugin {
    fn id(&self) -> &'static str {
        "selection"
    }

    fn name(&self) -> &'static str {
        "Selection"
    }

    fn mouse_hook(&self) -> Option<&dyn MouseHook> {
        Some(self)
    }

    /// Configuración por defecto, derivada de [`MousePolicy::default()`].
    ///
    /// Derivarla en lugar de repetir los números es lo que impide que el archivo
    /// distribuido y el default del núcleo se separen con el tiempo.
    fn default_config(&self) -> Option<PluginConfig> {
        let defaults = MousePolicy::default();
        let mut cfg = PluginConfig::new();
        // Clics y arrastres: con el programa dueño del ratón (modos de reporte
        // 1000/1002/1003) el gesto se le reenvía.
        cfg.set("forward_clicks", defaults.forward_clicks);
        cfg.set("forward_drag", defaults.forward_drag);
        cfg.set("forward_motion", defaults.forward_motion);
        // Shift devuelve el gesto a PORT aunque el programa lo haya capturado.
        cfg.set("shift_selects", defaults.shift_selects);
        // Selección local: copia al soltar y atajos de palabra y línea.
        cfg.set("copy_on_select", defaults.copy_on_select);
        cfg.set("word_on_double_click", defaults.word_on_double_click);
        cfg.set("line_on_triple_click", defaults.line_on_triple_click);
        // Aspecto del resaltado.
        cfg.set(
            "highlight_color",
            format!(
                "#{:02x}{:02x}{:02x}",
                defaults.highlight.r, defaults.highlight.g, defaults.highlight.b
            ),
        );
        cfg.set("highlight_opacity", defaults.highlight_opacity);
        Some(cfg)
    }

    /// Carga la política desde el bloque de configuración del plugin.
    ///
    /// Se parte siempre de [`MousePolicy::default()`]: una clave ausente deja su
    /// campo en el default, no en el valor de una carga anterior, así que el
    /// archivo es la única fuente de verdad.
    fn load_config(&self, config: &PluginConfig) {
        let mut policy = MousePolicy::default();
        apply_bool(config, "forward_clicks", &mut policy.forward_clicks);
        apply_bool(config, "forward_drag", &mut policy.forward_drag);
        apply_bool(config, "forward_motion", &mut policy.forward_motion);
        apply_bool(config, "shift_selects", &mut policy.shift_selects);
        apply_bool(config, "copy_on_select", &mut policy.copy_on_select);
        apply_bool(
            config,
            "word_on_double_click",
            &mut policy.word_on_double_click,
        );
        apply_bool(
            config,
            "line_on_triple_click",
            &mut policy.line_on_triple_click,
        );
        apply_color(config, "highlight_color", &mut policy.highlight);
        apply_opacity(config, "highlight_opacity", &mut policy.highlight_opacity);
        *self.policy.write().unwrap() = policy;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Construye una configuración con las claves indicadas, como la leería
    /// `load_config` del archivo `~/.config/port/config.md`.
    fn config_with(pairs: &[(&str, &str)]) -> PluginConfig {
        let mut cfg = PluginConfig::new();
        for (key, value) in pairs {
            cfg.set(*key, *value);
        }
        cfg
    }

    /// La configuración por defecto no puede separarse de `MousePolicy::default()`:
    /// si el núcleo cambia sus defaults, este plugin tiene que decir lo mismo.
    #[test]
    fn los_defaults_coinciden_con_mouse_policy_default() {
        let plugin = SelectionPlugin::new();
        let expected = MousePolicy::default();
        let config = plugin
            .default_config()
            .expect("el plugin declara sus defaults");

        assert_eq!(
            config.get_bool("forward_clicks"),
            Some(expected.forward_clicks)
        );
        assert_eq!(config.get_bool("forward_drag"), Some(expected.forward_drag));
        assert_eq!(
            config.get_bool("forward_motion"),
            Some(expected.forward_motion)
        );
        assert_eq!(
            config.get_bool("shift_selects"),
            Some(expected.shift_selects)
        );
        assert_eq!(
            config.get_bool("copy_on_select"),
            Some(expected.copy_on_select)
        );
        assert_eq!(
            config.get_bool("word_on_double_click"),
            Some(expected.word_on_double_click)
        );
        assert_eq!(
            config.get_bool("line_on_triple_click"),
            Some(expected.line_on_triple_click)
        );
        assert_eq!(
            parse_hex_color(
                config
                    .get("highlight_color")
                    .expect("clave highlight_color")
            ),
            Some(expected.highlight)
        );
        assert_eq!(
            config.get_f32("highlight_opacity"),
            Some(expected.highlight_opacity)
        );

        assert_eq!(plugin.policy(), expected);
    }

    /// `forward_clicks`: apagarlo hace que la pulsación no salga al programa.
    #[test]
    fn forward_clicks_false_se_aplica() {
        let plugin = SelectionPlugin::new();
        plugin.load_config(&config_with(&[("forward_clicks", "false")]));
        assert!(!plugin.policy().forward_clicks);
        // El resto de claves ausentes siguen en su valor por defecto.
        assert!(plugin.policy().forward_drag);
    }

    /// `forward_drag`: apagarlo deja el arrastre con botón pulsado en PORT.
    #[test]
    fn forward_drag_false_se_aplica() {
        let plugin = SelectionPlugin::new();
        plugin.load_config(&config_with(&[("forward_drag", "false")]));
        assert!(!plugin.policy().forward_drag);
    }

    /// `forward_motion`: apagarlo deja en PORT todo el movimiento (modo 1003).
    #[test]
    fn forward_motion_false_se_aplica() {
        let plugin = SelectionPlugin::new();
        plugin.load_config(&config_with(&[("forward_motion", "false")]));
        assert!(!plugin.policy().forward_motion);
    }

    /// `shift_selects`: apagarlo hace que Shift deje de ser la salida de emergencia.
    #[test]
    fn shift_selects_false_se_aplica() {
        let plugin = SelectionPlugin::new();
        plugin.load_config(&config_with(&[("shift_selects", "false")]));
        assert!(!plugin.policy().shift_selects);
    }

    /// `copy_on_select`: apagarlo desactiva la copia al soltar.
    #[test]
    fn copy_on_select_false_se_aplica() {
        let plugin = SelectionPlugin::new();
        plugin.load_config(&config_with(&[("copy_on_select", "false")]));
        assert!(!plugin.policy().copy_on_select);
    }

    /// `word_on_double_click`: apagarlo desactiva la selección por palabra.
    #[test]
    fn word_on_double_click_false_se_aplica() {
        let plugin = SelectionPlugin::new();
        plugin.load_config(&config_with(&[("word_on_double_click", "false")]));
        assert!(!plugin.policy().word_on_double_click);
    }

    /// `line_on_triple_click`: apagarlo desactiva la selección por línea.
    #[test]
    fn line_on_triple_click_false_se_aplica() {
        let plugin = SelectionPlugin::new();
        plugin.load_config(&config_with(&[("line_on_triple_click", "false")]));
        assert!(!plugin.policy().line_on_triple_click);
    }

    /// `highlight_color`: se acepta `#rrggbb` y se convierte a RGB.
    #[test]
    fn highlight_color_se_parsea_como_rrggbb() {
        let plugin = SelectionPlugin::new();
        plugin.load_config(&config_with(&[("highlight_color", "#ff8800")]));
        assert_eq!(plugin.policy().highlight, Rgb::new(0xff, 0x88, 0x00));
    }

    /// `highlight_opacity`: se parsea y se recorta al rango válido.
    #[test]
    fn highlight_opacity_se_aplica() {
        let plugin = SelectionPlugin::new();
        plugin.load_config(&config_with(&[("highlight_opacity", "0.62")]));
        assert_eq!(plugin.policy().highlight_opacity, 0.62);
    }

    /// Un booleano ilegible conserva el valor por defecto.
    #[test]
    fn un_booleano_ilegible_conserva_el_default() {
        let plugin = SelectionPlugin::new();
        plugin.load_config(&config_with(&[("forward_clicks", "quizá")]));
        assert!(plugin.policy().forward_clicks);
        assert_eq!(plugin.policy(), MousePolicy::default());
    }

    /// Un color ilegible conserva el valor por defecto.
    #[test]
    fn un_color_ilegible_conserva_el_default() {
        let plugin = SelectionPlugin::new();
        plugin.load_config(&config_with(&[("highlight_color", "azul")]));
        assert_eq!(plugin.policy().highlight, MousePolicy::default().highlight);
    }

    /// Una opacidad ilegible conserva el valor por defecto.
    #[test]
    fn una_opacidad_ilegible_conserva_el_default() {
        let plugin = SelectionPlugin::new();
        plugin.load_config(&config_with(&[("highlight_opacity", "mucho")]));
        assert_eq!(
            plugin.policy().highlight_opacity,
            MousePolicy::default().highlight_opacity
        );
    }

    /// La opacidad por debajo de cero se recorta a `0.0`.
    #[test]
    fn la_opacidad_se_recorta_a_cero_por_abajo() {
        let plugin = SelectionPlugin::new();
        plugin.load_config(&config_with(&[("highlight_opacity", "-1")]));
        assert_eq!(plugin.policy().highlight_opacity, 0.0);
    }

    /// La opacidad por encima de uno se recorta a `1.0`.
    #[test]
    fn la_opacidad_se_recorta_a_uno_por_arriba() {
        let plugin = SelectionPlugin::new();
        plugin.load_config(&config_with(&[("highlight_opacity", "2")]));
        assert_eq!(plugin.policy().highlight_opacity, 1.0);
    }

    /// Una clave ausente no toca su campo: se queda en el valor por defecto.
    #[test]
    fn una_clave_ausente_conserva_el_default() {
        let plugin = SelectionPlugin::new();
        plugin.load_config(&config_with(&[("forward_clicks", "false")]));

        let policy = plugin.policy();
        let defaults = MousePolicy::default();
        assert!(!policy.forward_clicks, "la clave presente sí se aplica");
        assert_eq!(policy.forward_drag, defaults.forward_drag);
        assert_eq!(policy.forward_motion, defaults.forward_motion);
        assert_eq!(policy.shift_selects, defaults.shift_selects);
        assert_eq!(policy.copy_on_select, defaults.copy_on_select);
        assert_eq!(policy.word_on_double_click, defaults.word_on_double_click);
        assert_eq!(policy.line_on_triple_click, defaults.line_on_triple_click);
        assert_eq!(policy.highlight, defaults.highlight);
        assert_eq!(policy.highlight_opacity, defaults.highlight_opacity);
    }

    /// El plugin publica su hook de ratón y este devuelve la política cargada.
    #[test]
    fn el_hook_de_raton_es_el_dueno_de_la_politica() {
        let plugin = SelectionPlugin::new();
        plugin.load_config(&config_with(&[("shift_selects", "false")]));

        let hook = plugin
            .mouse_hook()
            .expect("selection publica su hook de ratón");
        assert!(!hook.mouse_policy().shift_selects);
    }
}
