//! El plugin: estado compartido, configuración y el puente hacia PORT.
//!
//! Este módulo no menciona ningún tipo de `port-plugin-api` salvo en el
//! adaptador final ([`SmoothCursorPlugin`] implementa `Plugin` y `OverlayHook`),
//! para que la lógica se pueda probar sin construir un `FrameView` ni un
//! `Overlay` del host. Los métodos `begin_frame` / `quads` / `wants_frames`
//! reciben primitivas y devuelven primitivas.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use port_plugin_api::config::PluginConfig;

use crate::anim::{shape_rect, Animator, CellRect, Shape};
use crate::config::Config;

/// Identificador con el que PORT registra el plugin.
pub const PLUGIN_ID: &str = "smooth-cursor";

/// Nombre legible.
pub const PLUGIN_NAME: &str = "Smooth Cursor";

/// Un quad listo para convertir en `OverlayQuad`, sin depender del tipo del host.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quad {
    pub rect: CellRect,
    pub opacity: f32,
    pub corner_radius_px: f32,
}

/// Plugin de cursor animado.
pub struct SmoothCursorPlugin {
    config: Mutex<Config>,
    animator: Mutex<Animator>,
    /// Resultado del último cuadro, para responder `needs_animation()`.
    ///
    /// El trait del host no recibe el reloj, así que el plugin no puede
    /// recalcular la respuesta mientras se le pregunta. Guarda la del último
    /// `overlay()`: el registro pregunta, luego el host pinta e invoca el hook,
    /// y ese cuadro deja la respuesta del siguiente. Cuesta un cuadro de retraso
    /// en apagar el canal, que es exactamente la ventana de gracia.
    last_wants_frames: AtomicBool,
}

impl SmoothCursorPlugin {
    pub fn new() -> Self {
        Self {
            config: Mutex::new(Config::default()),
            animator: Mutex::new(Animator::new()),
            last_wants_frames: AtomicBool::new(false),
        }
    }

    /// Aplica la configuración y devuelve una instantánea.
    pub fn apply_config(&self, config: &PluginConfig) {
        let resolved = Config::from_plugin_config(config);
        *self.config.lock().expect("config envenenada") = resolved;
    }

    pub fn config(&self) -> Config {
        *self.config.lock().expect("config envenenada")
    }

    /// Registra la celda del cursor en este cuadro.
    ///
    /// `rect: None` resetea el estado: es lo que evita que un cursor que
    /// desaparece y reaparece en otro sitio dibuje una estela entre dos
    /// posiciones que nunca fueron consecutivas.
    pub fn begin_frame(&self, rect: Option<CellRect>, now_ms: u64) -> bool {
        let config = self.config();
        let mut animator = self.animator.lock().expect("animator envenenado");
        if !config.enabled {
            animator.observe(None, now_ms);
            return false;
        }
        animator.observe(rect, now_ms)
    }

    /// `true` si hay que seguir pidiendo cuadros.
    ///
    /// Debe caer a `false` cuando la transición termina: un `true` persistente
    /// es refresco permanente y recupera justo el coste que este feature evita.
    pub fn wants_frames(&self, now_ms: u64) -> bool {
        let config = self.config();
        if !config.enabled {
            return false;
        }
        self.animator
            .lock()
            .expect("animator envenenado")
            .needs_animation(now_ms, &config.tuning)
    }

    /// Procesa un cuadro del host: registra el cursor y devuelve los quads.
    ///
    /// El orden importa y es el motivo de que este método exista en vez de
    /// dejar que el adaptador encadene las llamadas: primero se registra el
    /// cursor (o el reset), después se **consume** el resultado —lo que
    /// actualiza la respuesta de `needs_animation()`— y solo entonces se
    /// decide si hay algo que pintar. Si el adaptador se saltara el consumo
    /// cuando no hay cursor, el flag quedaría pegado en `true` y volvería el
    /// refresco permanente.
    pub fn frame(&self, rect: Option<CellRect>, shape: Shape, now_ms: u64) -> Option<Vec<Quad>> {
        self.begin_frame(rect, now_ms);
        self.quads_for(shape, now_ms)
    }

    /// Respuesta a `OverlayHook::needs_animation()`: la del último cuadro.
    pub fn wants_frames_cached(&self) -> bool {
        self.last_wants_frames.load(Ordering::Relaxed)
    }

    /// Quads a pintar en este cuadro, en la forma que pidió el host.
    ///
    /// La estela y el cursor comparten forma: si el cursor es una barra, la
    /// estela es una barra del mismo ancho, no un bloque.
    pub fn quads_for(&self, shape: Shape, now_ms: u64) -> Option<Vec<Quad>> {
        let quads = self.quads(now_ms)?;
        Some(
            quads
                .into_iter()
                .map(|quad| Quad {
                    rect: shape_rect(quad.rect, shape),
                    ..quad
                })
                .collect(),
        )
    }

    /// Quads a pintar en este cuadro. `None` deja el cursor nativo como está.
    pub fn quads(&self, now_ms: u64) -> Option<Vec<Quad>> {
        let config = self.config();
        if !config.enabled {
            self.last_wants_frames.store(false, Ordering::Relaxed);
            return None;
        }
        // Se deja lista la respuesta para la siguiente consulta al registro.
        let wants = self
            .animator
            .lock()
            .expect("animator envenenado")
            .needs_animation(now_ms, &config.tuning);
        self.last_wants_frames.store(wants, Ordering::Relaxed);

        let animator = self.animator.lock().expect("animator envenenado");
        let overlay = animator.overlay(now_ms, &config.tuning)?;

        let mut quads = Vec::with_capacity(2);
        // La estela va primero: queda debajo del cursor, que es la forma más
        // reciente y la que el usuario debe ver nítida.
        if let Some(trail) = overlay.trail {
            quads.push(Quad {
                rect: trail.rect,
                opacity: trail.opacity,
                corner_radius_px: overlay.corner_radius_px,
            });
        }
        quads.push(Quad {
            rect: overlay.rect,
            opacity: overlay.opacity,
            corner_radius_px: overlay.corner_radius_px,
        });
        Some(quads)
    }
}

impl SmoothCursorPlugin {
    /// Valores que PORT escribe cuando el usuario no configuró nada.
    pub fn default_plugin_config(&self) -> PluginConfig {
        Config::default().to_plugin_config()
    }

    /// Configuración actual, para que el host la persista.
    pub fn current_plugin_config(&self) -> PluginConfig {
        self.config().to_plugin_config()
    }
}

impl Default for SmoothCursorPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(column: f32, row: f32) -> CellRect {
        CellRect {
            column,
            row,
            width: 1.0,
            height: 1.0,
        }
    }

    fn moved_plugin() -> SmoothCursorPlugin {
        let plugin = SmoothCursorPlugin::new();
        plugin.begin_frame(Some(rect(0.0, 0.0)), 0);
        plugin.begin_frame(Some(rect(0.0, 1.0)), 0);
        plugin
    }

    /// S1: con la animación apagada no pinta nada ni pide cuadros.
    #[test]
    fn disabled_plugin_is_passthrough() {
        let plugin = SmoothCursorPlugin::new();
        let mut raw = PluginConfig::new();
        raw.set(crate::config::KEY_ENABLED, "false");
        plugin.apply_config(&raw);

        plugin.begin_frame(Some(rect(0.0, 0.0)), 0);
        plugin.begin_frame(Some(rect(0.0, 1.0)), 0);

        assert_eq!(plugin.quads(0), None);
        assert!(!plugin.wants_frames(0));
    }

    /// S3: el orden de los quads pone la estela debajo del cursor.
    #[test]
    fn trail_is_painted_before_the_cursor() {
        let plugin = moved_plugin();
        let quads = plugin.quads(0).expect("animando");

        assert_eq!(quads.len(), 2, "estela más cursor");
        assert!(
            quads[0].opacity < quads[1].opacity,
            "la estela es más tenue que el cursor"
        );
        assert_eq!(quads[1].opacity, 1.0);
    }

    /// S3: sin estela queda solo el cursor.
    #[test]
    fn zero_strength_paints_only_the_cursor() {
        let plugin = SmoothCursorPlugin::new();
        let mut raw = PluginConfig::new();
        raw.set(crate::config::KEY_TRAIL_STRENGTH, "0");
        plugin.apply_config(&raw);

        plugin.begin_frame(Some(rect(0.0, 0.0)), 0);
        plugin.begin_frame(Some(rect(0.0, 1.0)), 0);

        assert_eq!(plugin.quads(0).expect("animando").len(), 1);
    }

    /// El trait del host no pasa el reloj, así que la respuesta cacheada tiene
    /// que apagarse sola y no quedarse en `true` para siempre.
    #[test]
    fn cached_frame_request_stops_after_the_transition() {
        let plugin = moved_plugin();
        let m = crate::anim::HANDOFF_GRACE_MS;

        assert!(plugin.quads(0).is_some());
        assert!(
            plugin.wants_frames_cached(),
            "pidiendo cuadros mientras anima"
        );

        // Cuadro posterior a la gracia: no pinta y deja el canal apagado.
        assert_eq!(plugin.quads(120 + m + 1), None);
        assert!(
            !plugin.wants_frames_cached(),
            "un true pegajoso es refresco permanente"
        );
    }

    /// Un cursor que desaparece a mitad de transición apaga el canal en ese
    /// mismo cuadro. Saltarse el consumo con `cursor: None` dejaría el flag
    /// pegado en `true`.
    #[test]
    fn disappearing_cursor_does_not_leave_frames_requested() {
        let plugin = moved_plugin();
        assert!(plugin
            .frame(Some(rect(0.0, 1.0)), Shape::Block, 10)
            .is_some());
        assert!(plugin.wants_frames_cached(), "animando");

        assert_eq!(
            plugin.frame(None, Shape::Block, 20),
            None,
            "sin cursor no hay quads"
        );
        assert!(
            !plugin.wants_frames_cached(),
            "el cuadro sin cursor también consume el resultado"
        );
    }

    /// El presupuesto de cuadros está acotado.
    ///
    /// Es la propiedad que cuesta batería si se rompe: una transición completa
    /// debe pedir un número finito y pequeño de cuadros, y después dejar de
    /// pedir para siempre. Se simula el host a 60 Hz.
    #[test]
    fn a_transition_requests_a_bounded_number_of_frames() {
        let plugin = moved_plugin();
        let mut now = 0u64;
        let mut frames = 0;

        loop {
            let painted = plugin.frame(Some(rect(0.0, 1.0)), Shape::Block, now);
            if painted.is_none() && !plugin.wants_frames_cached() {
                break;
            }
            frames += 1;
            now += 16;
            assert!(frames < 100, "la transición no termina: {frames} cuadros");
        }

        assert!(
            frames >= 5,
            "120 ms a 60 Hz son unos 8 cuadros, hubo {frames}"
        );
        assert!(
            frames <= 15,
            "120 ms más la gracia son unos 10 cuadros, hubo {frames}"
        );

        // Y una vez quieto, no vuelve a pedir nada jamás.
        for _ in 0..50 {
            now += 16;
            assert_eq!(plugin.frame(Some(rect(0.0, 1.0)), Shape::Block, now), None);
            assert!(
                !plugin.wants_frames_cached(),
                "un cursor quieto no puede pedir cuadros"
            );
        }
    }

    /// La estela hereda la forma: con un Beam no se pinta un bloque.
    #[test]
    fn trail_follows_the_cursor_shape() {
        let plugin = moved_plugin();
        let quads = plugin.quads_for(Shape::Beam, 0).expect("animando");

        for quad in &quads {
            assert!(
                (quad.rect.width - crate::anim::BEAM_WIDTH).abs() < 1e-6,
                "tanto la estela como el cursor son barras"
            );
        }
    }

    /// Un cursor quieto no pide cuadros nunca.
    #[test]
    fn idle_cursor_does_not_request_frames() {
        let plugin = SmoothCursorPlugin::new();
        plugin.begin_frame(Some(rect(2.0, 3.0)), 0);
        assert_eq!(plugin.quads(0), None);
        assert!(!plugin.wants_frames_cached());
    }

    /// S5: el tick se apaga solo, no queda pidiendo cuadros para siempre.
    #[test]
    fn frames_stop_when_the_transition_ends() {
        let plugin = moved_plugin();
        assert!(plugin.wants_frames(0));
        assert!(
            !plugin.wants_frames(10_000),
            "un true persistente es refresco permanente"
        );
    }

    /// S4: pasada la transición no hay quads, así que el nativo vuelve.
    #[test]
    fn no_quads_after_the_transition() {
        let plugin = moved_plugin();
        assert_eq!(plugin.quads(10_000), None);
    }

    /// Un cursor oculto resetea: al reaparecer lejos no hay estela fantasma.
    #[test]
    fn hidden_cursor_resets_before_reappearing() {
        let plugin = moved_plugin();
        plugin.begin_frame(None, 10);
        assert!(!plugin.wants_frames(10));

        plugin.begin_frame(Some(rect(0.0, 30.0)), 20);
        assert_eq!(
            plugin.quads(20),
            None,
            "una aparición no anima desde el origen"
        );
    }

    /// El radio llega en píxeles sin conversión.
    #[test]
    fn corner_radius_is_passed_through() {
        let plugin = SmoothCursorPlugin::new();
        let mut raw = PluginConfig::new();
        raw.set(crate::config::KEY_CORNER_RADIUS, "2.0");
        plugin.apply_config(&raw);

        plugin.begin_frame(Some(rect(0.0, 0.0)), 0);
        plugin.begin_frame(Some(rect(0.0, 1.0)), 0);

        assert_eq!(plugin.quads(0).unwrap()[1].corner_radius_px, 2.0);
    }

    /// La configuración se puede leer de vuelta tal como se cargó.
    #[test]
    fn config_round_trips_through_the_plugin() {
        let plugin = SmoothCursorPlugin::new();
        let mut raw = PluginConfig::new();
        raw.set(crate::config::KEY_DURATION_MS, "300");
        plugin.apply_config(&raw);

        let saved = plugin.current_plugin_config();
        assert_eq!(saved.get(crate::config::KEY_DURATION_MS), Some("300"));
    }
}

#[cfg(test)]
mod host_order_tests {
    use super::*;
    use crate::anim::Shape;

    fn r(c: f32, row: f32) -> CellRect {
        CellRect {
            column: c,
            row,
            width: 1.0,
            height: 1.0,
        }
    }

    /// PORT invierte el orden desde el fix del host: invoca `overlay()` y
    /// **después** pregunta `needs_animation()`. Consecuencia: el flag que
    /// devuelve `needs_animation()` ya refleja el cuadro actual, no el anterior.
    ///
    /// Eso es lo que hace que el PRIMER cuadro de un salto anime. Con el orden
    /// viejo (preguntar antes de invocar) el flag venía del cuadro quieto,
    /// decía `false`, no se pedía repintado y el cursor pintado en la posición
    /// vieja se quedaba clavado hasta que otra tecla forzara un cuadro.
    #[test]
    fn the_jump_frame_paints_and_asks_for_another_frame() {
        let plugin = SmoothCursorPlugin::new();

        // Cuadro 1: cursor quieto al final del texto.
        plugin.begin_frame(Some(r(30.0, 2.0)), 0);
        assert!(plugin.quads(0).is_none());
        assert!(!plugin.wants_frames_cached(), "quieto: no pide cuadros");

        // Alt+Backspace: el cursor salta a la izquierda. El host invoca primero.
        let painted = plugin.frame(Some(r(25.0, 2.0)), Shape::Block, 100);
        assert!(painted.is_some(), "el salto se pinta en este mismo cuadro");

        // Y solo después pregunta: la respuesta ya es la del cuadro actual.
        assert!(
            plugin.wants_frames_cached(),
            "debe pedir el siguiente cuadro o la animación no corre"
        );
    }

    /// La animación avanza en cuadros sucesivos y se apaga sola al terminar.
    #[test]
    fn the_animation_advances_then_stops() {
        let plugin = SmoothCursorPlugin::new();
        plugin.begin_frame(Some(r(30.0, 2.0)), 0);
        let _ = plugin.quads(0);

        let first = plugin.frame(Some(r(25.0, 2.0)), Shape::Block, 100);
        assert!(first.is_some());
        assert!(plugin.wants_frames_cached());

        let mid = plugin
            .frame(Some(r(25.0, 2.0)), Shape::Block, 160)
            .expect("a mitad");
        assert!(
            mid[0].rect.column > 25.0 && mid[0].rect.column < 30.0,
            "interpola entre origen y destino: {}",
            mid[0].rect.column
        );

        // Pasada la gracia no pinta y deja de pedir cuadros.
        let late = crate::anim::HANDOFF_GRACE_MS + 121;
        assert_eq!(
            plugin.frame(Some(r(25.0, 2.0)), Shape::Block, 100 + late),
            None
        );
        assert!(!plugin.wants_frames_cached());
    }

    /// Un cursor oculto sigue reseteando aunque el orden haya cambiado.
    #[test]
    fn hidden_cursor_still_resets_and_stops_frames() {
        let plugin = SmoothCursorPlugin::new();
        plugin.begin_frame(Some(r(30.0, 2.0)), 0);
        let _ = plugin.quads(0);
        let _ = plugin.frame(Some(r(25.0, 2.0)), Shape::Block, 100);
        assert!(plugin.wants_frames_cached());

        assert_eq!(plugin.frame(None, Shape::Block, 120), None);
        assert!(
            !plugin.wants_frames_cached(),
            "sin cursor no se piden cuadros"
        );
    }
}
