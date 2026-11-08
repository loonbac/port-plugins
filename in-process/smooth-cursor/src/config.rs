//! Configuración del plugin, leída del bloque `smooth-cursor` de
//! `~/.config/port/config.md`.
//!
//! Las tres claves nacen del shader de Ghostty (`ANIMATION_DURATION`,
//! `TRAIL_STRENGTH`, `CORNER_RADIUS`), pero se leen con nombres explícitos y en
//! unidades del mundo del usuario: milisegundos y píxeles, no segundos ni
//! fracciones de celda.

use port_plugin_api::config::PluginConfig;

use crate::anim::Tuning;

/// Claves del bloque de configuración.
pub const KEY_ENABLED: &str = "enabled";
pub const KEY_DURATION_MS: &str = "duration_ms";
pub const KEY_TRAIL_STRENGTH: &str = "trail_strength";
pub const KEY_CORNER_RADIUS: &str = "corner_radius";
pub const KEY_MAX_TRAIL_CELLS: &str = "max_trail_cells";

/// Rango aceptado para la duración, en milisegundos.
///
/// El límite superior no es arbitrario: una transición de más de medio segundo
/// deja dos cursores visibles mucho tiempo y se lee como un fallo de dibujado,
/// no como una animación.
const DURATION_MS_RANGE: (u64, u64) = (0, 600);

/// Rango aceptado para la estela.
const TRAIL_STRENGTH_RANGE: (f32, f32) = (0.0, 1.0);

/// Rango aceptado para el radio, en píxeles.
const CORNER_RADIUS_RANGE: (f32, f32) = (0.0, 16.0);

/// Rango aceptado para el salto máximo con estela, en celdas.
const MAX_TRAIL_CELLS_RANGE: (f32, f32) = (0.0, 64.0);

/// Configuración resuelta del plugin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Config {
    /// `false` deja el cursor nativo y el plugin no pide cuadros.
    pub enabled: bool,
    pub tuning: Tuning,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            tuning: Tuning::default(),
        }
    }
}

impl Config {
    /// Valores que se escriben cuando el usuario no configuró nada.
    ///
    /// Se publican como `Plugin::default_config` para que el archivo de
    /// configuración nazca con las claves visibles y commentadas por el valor,
    /// en vez de un bloque vacío que no enseña qué se puede tocar.
    pub fn to_plugin_config(self) -> PluginConfig {
        let mut config = PluginConfig::new();
        config
            .set(KEY_ENABLED, self.enabled)
            .set(KEY_DURATION_MS, self.tuning.duration_ms)
            .set(
                KEY_TRAIL_STRENGTH,
                format!("{:.2}", self.tuning.trail_strength),
            )
            .set(
                KEY_CORNER_RADIUS,
                format!("{:.1}", self.tuning.corner_radius_px),
            )
            .set(
                KEY_MAX_TRAIL_CELLS,
                format!("{:.0}", self.tuning.max_trail_cells),
            );
        config
    }

    /// Construye la configuración desde el bloque del archivo.
    ///
    /// Un valor fuera de rango o ilegible **no** invalida el bloque: se descarta
    /// esa clave y se conserva el valor por defecto. Es lo que evita que un
    /// `duration_ms = abc` deje el plugin apagado sin que el usuario sepa por
    /// qué.
    pub fn from_plugin_config(config: &PluginConfig) -> Self {
        let mut resolved = Self::default();

        if let Some(enabled) = config.get_bool(KEY_ENABLED) {
            resolved.enabled = enabled;
        }

        if let Some(duration) = config.get_u32(KEY_DURATION_MS) {
            resolved.tuning.duration_ms = clamp_u64(duration as u64, DURATION_MS_RANGE);
        }

        if let Some(strength) = config.get_f32(KEY_TRAIL_STRENGTH) {
            resolved.tuning.trail_strength = clamp_f32(strength, TRAIL_STRENGTH_RANGE);
        }

        if let Some(radius) = config.get_f32(KEY_CORNER_RADIUS) {
            resolved.tuning.corner_radius_px = clamp_f32(radius, CORNER_RADIUS_RANGE);
        }

        if let Some(cells) = config.get_f32(KEY_MAX_TRAIL_CELLS) {
            resolved.tuning.max_trail_cells = clamp_f32(cells, MAX_TRAIL_CELLS_RANGE);
        }

        resolved
    }
}

fn clamp_f32(value: f32, (min, max): (f32, f32)) -> f32 {
    // `NaN` no es comparable: un valor no finito cae al mínimo del rango.
    if !value.is_finite() {
        return min;
    }
    value.clamp(min, max)
}

fn clamp_u64(value: u64, (min, max): (u64, u64)) -> u64 {
    value.clamp(min, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// S6: sin configuración, los valores son los del shader.
    #[test]
    fn defaults_match_the_shader() {
        let config = Config::default();
        assert!(config.enabled);
        assert_eq!(config.tuning.duration_ms, 120);
        assert!((config.tuning.trail_strength - 0.30).abs() < 1e-6);
        assert!((config.tuning.corner_radius_px - 2.0).abs() < 1e-6);
    }

    /// S6: ida y vuelta por el bloque de configuración sin pérdida.
    #[test]
    fn round_trip_through_plugin_config() {
        let original = Config {
            enabled: false,
            tuning: Tuning {
                duration_ms: 200,
                trail_strength: 0.5,
                corner_radius_px: 3.5,
                max_trail_cells: 4.0,
            },
        };
        let parsed = Config::from_plugin_config(&original.to_plugin_config());

        assert!(!parsed.enabled);
        assert_eq!(parsed.tuning.duration_ms, 200);
        assert!((parsed.tuning.trail_strength - 0.5).abs() < 1e-6);
        assert!((parsed.tuning.corner_radius_px - 3.5).abs() < 1e-6);
        assert!((parsed.tuning.max_trail_cells - 4.0).abs() < 1e-6);
    }

    /// Un valor ilegible no invalida el resto del bloque.
    #[test]
    fn unreadable_values_fall_back_to_defaults() {
        let mut raw = PluginConfig::new();
        raw.set(KEY_DURATION_MS, "abc")
            .set(KEY_CORNER_RADIUS, "no-es-un-numero")
            .set(KEY_TRAIL_STRENGTH, "0.4");

        let parsed = Config::from_plugin_config(&raw);
        assert_eq!(parsed.tuning.duration_ms, 120, "cae al default");
        assert!((parsed.tuning.corner_radius_px - 2.0).abs() < 1e-6);
        assert!(
            (parsed.tuning.trail_strength - 0.4).abs() < 1e-6,
            "la clave válida sí se aplica"
        );
    }

    /// Los valores fuera de rango se acotan, no se ignoran.
    #[test]
    fn out_of_range_values_are_clamped() {
        let mut raw = PluginConfig::new();
        raw.set(KEY_DURATION_MS, "99999")
            .set(KEY_TRAIL_STRENGTH, "7.0")
            .set(KEY_CORNER_RADIUS, "-3");

        let parsed = Config::from_plugin_config(&raw);
        assert_eq!(parsed.tuning.duration_ms, 600);
        assert!((parsed.tuning.trail_strength - 1.0).abs() < 1e-6);
        assert_eq!(parsed.tuning.corner_radius_px, 0.0);
    }

    /// `NaN` e infinito no se propagan al pintado.
    #[test]
    fn non_finite_values_fall_to_minimum() {
        let mut raw = PluginConfig::new();
        raw.set(KEY_TRAIL_STRENGTH, "NaN")
            .set(KEY_CORNER_RADIUS, "inf");

        let parsed = Config::from_plugin_config(&raw);
        assert_eq!(parsed.tuning.trail_strength, 0.0);
        assert_eq!(parsed.tuning.corner_radius_px, 0.0);
    }

    /// `duration_ms = 0` es una petición válida: desactiva la animación.
    #[test]
    fn zero_duration_is_allowed_and_disables_animation() {
        let mut raw = PluginConfig::new();
        raw.set(KEY_DURATION_MS, "0");

        let parsed = Config::from_plugin_config(&raw);
        assert_eq!(parsed.tuning.duration_ms, 0);
    }
}
