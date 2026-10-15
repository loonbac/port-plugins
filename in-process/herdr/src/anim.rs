//! Movimiento: un resorte crítico y curvas de aceleración.
//!
//! Única responsabilidad: convertir tiempo en desplazamiento. No conoce
//! espacios, tarjetas ni eventos; por eso es puro y se prueba sin ventana.
//!
//! El plugin no puede usar temporizadores para animar: la barra lateral se
//! repinta cuando el anfitrión pide un cuadro. Así que el movimiento se calcula
//! como una simulación paso a paso (`step`) sobre el tiempo transcurrido entre
//! cuadros, y `needs_animation` mantiene los cuadros fluyendo mientras algo se
//! mueva.

/// Resta respecto al objetivo por debajo de la cual el resorte se considera
/// detenido, en píxeles.
const SETTLE_DISTANCE: f32 = 0.05;
/// Velocidad por debajo de la cual el resorte se considera detenido, en
/// píxeles por segundo.
const SETTLE_SPEED: f32 = 1.5;
/// Paso máximo que se integra de una vez, en segundos.
///
/// Un cuadro perdido (ventana tapada, compilación, suspensión) llega como un
/// salto de tiempo enorme; integrarlo de golpe haría que la tarjeta aparezca
/// teletransportada, o peor, que el resorte explote. Se integra a lo sumo este
/// paso y se descarta el resto.
const MAX_STEP_S: f32 = 1.0 / 30.0;

/// Resorte crítico sobre un solo eje.
///
/// Crítico significa sin rebote: llega al objetivo lo más rápido posible sin
/// pasarse. Es lo que hace que una tarjeta se sienta "pesada" en vez de
/// oscilar, y es el único parámetro que hay que elegir (`stiffness`): más
/// rígido, más pegada al puntero; más blando, más flotante.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Spring {
    pos: f32,
    vel: f32,
}

impl Spring {
    /// Resorte detenido en el origen.
    pub(crate) const fn new() -> Self {
        Self { pos: 0.0, vel: 0.0 }
    }

    /// Desplazamiento actual, en píxeles.
    pub(crate) fn value(&self) -> f32 {
        self.pos
    }

    /// Coloca el resorte en una posición concreta sin velocidad.
    ///
    /// Se usa para iniciar el movimiento: al intercambiar dos tarjetas, la que
    /// llega a la ranura nueva arranca desde donde estaba su vecina, y desde
    /// ahí el resorte la lleva a su lugar.
    pub(crate) fn place(&mut self, pos: f32) {
        self.pos = pos;
        self.vel = 0.0;
    }

    /// Avanza la simulación hacia `target` durante `dt_s` segundos y devuelve
    /// `true` si el resorte todavía se mueve.
    ///
    /// La rigidez fija el amortiguamiento en el valor crítico
    /// (`2 * sqrt(stiffness)`), que es lo que evita el rebote.
    pub(crate) fn step(&mut self, target: f32, dt_s: f32, stiffness: f32) -> bool {
        let dt = dt_s.clamp(0.0, MAX_STEP_S);
        let damping = 2.0 * stiffness.sqrt();
        let accel = stiffness * (target - self.pos) - damping * self.vel;
        self.vel += accel * dt;
        self.pos += self.vel * dt;

        if (target - self.pos).abs() < SETTLE_DISTANCE && self.vel.abs() < SETTLE_SPEED {
            self.pos = target;
            self.vel = 0.0;
            false
        } else {
            true
        }
    }
}

/// Curva de salida cúbica: rápida al principio, suave al final.
///
/// Para el realce de "levantar" la tarjeta, donde un resorte sería excesivo
/// porque no hay una posición que perseguir, sino un valor que llega de 0 a 1.
pub(crate) fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El resorte detenido en el objetivo no se mueve ni pide cuadros.
    #[test]
    fn resorte_en_reposo_no_anima() {
        let mut s = Spring::new();
        assert!(!s.step(0.0, 1.0 / 60.0, 300.0));
        assert_eq!(s.value(), 0.0);
    }

    /// Partiendo de un desplazamiento, el resorte llega al objetivo y se detiene.
    #[test]
    fn resorte_llega_y_se_detiene() {
        let mut s = Spring::new();
        s.place(45.0);
        let mut moved = true;
        let mut frames = 0;
        while moved {
            moved = s.step(0.0, 1.0 / 60.0, 300.0);
            frames += 1;
            assert!(frames < 240, "el resorte no se detuvo");
        }
        assert_eq!(s.value(), 0.0);
    }

    /// Un resorte crítico no se pasa del objetivo (sin rebote).
    #[test]
    fn resorte_critico_no_rebota() {
        let mut s = Spring::new();
        s.place(45.0);
        let mut min = f32::MAX;
        for _ in 0..120 {
            s.step(0.0, 1.0 / 60.0, 300.0);
            min = min.min(s.value());
        }
        assert!(min >= -1.0, "rebotó por debajo del objetivo: {min}");
    }

    /// Un salto de tiempo enorme se recorta y no hace explotar al resorte.
    #[test]
    fn salto_de_tiempo_no_explota() {
        let mut s = Spring::new();
        s.place(45.0);
        s.step(0.0, 10.0, 400.0);
        assert!(s.value().is_finite());
        assert!(s.value().abs() <= 45.0);
    }

    /// La curva de salida va de 0 a 1 y es monótona.
    #[test]
    fn curva_de_salida_va_de_cero_a_uno() {
        assert_eq!(ease_out_cubic(0.0), 0.0);
        assert_eq!(ease_out_cubic(1.0), 1.0);
        assert!(
            ease_out_cubic(0.5) > 0.5,
            "la salida es rápida al principio"
        );
        assert!(ease_out_cubic(0.3) < ease_out_cubic(0.6));
    }
}
