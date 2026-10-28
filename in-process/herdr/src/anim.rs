//! Movimiento de las tarjetas de espacio.
//!
//! Única responsabilidad: convertir (tiempo, tarjetas, arrastre en curso) en un
//! desplazamiento, una opacidad y un tinte por tarjeta. No conoce GPUI ni el
//! estado del plugin, así que se prueba sin ventana.
//!
//! La física viene de `gpui-ce-motion` en su forma portable
//! (`default-features = false`): el crate trae adaptadores para una copia de GPUI
//! distinta de la que usa PORT, así que acá se usan solo las matemáticas
//! (resortes, curvas e interrupción con velocidad). El reloj lo sigue poniendo el
//! render del anfitrión, que es el único que sabe cuándo hay un cuadro.
//!
//! Antes esto era un resorte casero sin identidad: un `Vec<Spring>` paralelo a la
//! lista de espacios. Al cerrar un espacio intermedio, el desplazamiento residual
//! de una tarjeta caía sobre la siguiente. Ahora el estado se guarda por **id de
//! espacio** (`Motions<usize>`), así que quitar o reordenar tarjetas no mueve a
//! las demás.

use std::collections::HashSet;
use std::sync::OnceLock;
use std::time::Duration;

use gpui_ce_motion::{Easing, Motion, MotionState, Motions, Spring, Target, Transition};

/// Alto fijo de una tarjeta de espacio, en píxeles lógicos.
///
/// Es fijo a propósito: el arrastre convierte píxeles en ranuras con
/// [`CARD_PITCH`], y una altura que dependiera del texto haría que ese paso
/// fuera una estimación.
pub(crate) const CARD_HEIGHT: f32 = 44.0;

/// Separación entre tarjetas consecutivas, en píxeles lógicos.
pub(crate) const CARD_GAP: f32 = 4.0;

/// Paso vertical exacto entre dos tarjetas: alto más separación.
///
/// El arrastre divide por este valor para decidir la ranura destino, así que
/// tiene que coincidir con el layout. Mientras fue una estimación (`45.0`), el
/// error se acumulaba en el ancla del gesto y un movimiento mínimo terminaba
/// cruzando dos ranuras. Una prueba de layout comprueba que coinciden.
pub(crate) const CARD_PITCH: f32 = CARD_HEIGHT + CARD_GAP;

/// Desplazamiento vertical desde el que entra una tarjeta nueva, en píxeles.
const ENTER_OFFSET: f32 = 10.0;

/// Resorte del reacomodo: una tarjeta que cambia de ranura.
///
/// La duración es la de llegar al 90 % del trayecto y el rebote es cero: una
/// lista que oscila se lee como un error, no como movimiento. Como el resorte
/// conserva la velocidad al cambiar de destino, un arrastre rápido encadena los
/// desplazamientos en vez de reiniciarlos en cada ranura.
fn shift() -> Transition {
    static SHIFT: OnceLock<Transition> = OnceLock::new();
    *SHIFT.get_or_init(|| {
        Transition::Spring(
            Spring::visual(Duration::from_millis(240), 0.0).expect("duración y rebote positivos"),
        )
    })
}

/// Resorte de la tarjeta agarrada: sigue al puntero, no a una ranura.
///
/// Más corto que [`shift`] a propósito: la tarjeta tiene que sentirse pegada al
/// cursor, y el resorte solo suaviza el salto entre eventos del puntero.
fn follow() -> Transition {
    static FOLLOW: OnceLock<Transition> = OnceLock::new();
    *FOLLOW.get_or_init(|| {
        Transition::Spring(
            Spring::visual(Duration::from_millis(90), 0.0).expect("duración y rebote positivos"),
        )
    })
}

/// Curva del realce de levantado y del tinte del espacio activo.
fn fade() -> Transition {
    Transition::Tween(Duration::from_millis(160), Easing::EaseOut)
}

/// Ranura a la que iría una tarjeta agarrada en `grab_index` si se soltara con
/// el puntero `pointer_dy` píxeles más abajo.
///
/// Es una función del puntero y nada más: no acumula estado entre llamadas, así
/// que repetirla con la misma posición da siempre el mismo resultado y un evento
/// de movimiento perdido no descoloca el gesto. El umbral es media tarjeta, que
/// es donde la arrastrada tapa a su vecina lo suficiente para pedir el cambio.
pub(crate) fn drop_slot(grab_index: usize, pointer_dy: f32, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let slot = grab_index as f32 + pointer_dy / CARD_PITCH;
    slot.round().clamp(0.0, (len - 1) as f32) as usize
}

/// Orden resultante de mover `from` a `to` en `ids`.
///
/// Es la única fuente de verdad del reacomodo: las ranuras de cada tarjeta se
/// derivan comparando el orden de reposo con este, así que mover una tarjeta
/// corre a las de en medio sin ninguna tabla de casos aparte.
pub(crate) fn reordered(ids: &[usize], from: usize, to: usize) -> Vec<usize> {
    let mut out = ids.to_vec();
    if from < out.len() && to < out.len() && from != to {
        let moved = out.remove(from);
        out.insert(to, moved);
    }
    out
}

/// Lo que la barra sabe de una tarjeta antes de animarla.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CardInput {
    /// Id estable del espacio: es la clave del movimiento.
    pub id: usize,
    /// Si esta tarjeta es el espacio activo.
    pub active: bool,
    /// Desplazamiento del puntero mientras esta tarjeta se arrastra, en píxeles.
    ///
    /// `None` en todas las demás, que persiguen su ranura.
    pub drag_offset: Option<f32>,
    /// Ranuras que tiene que recorrer para llegar a su ranura visual.
    pub slot_delta: i32,
}

/// Lo que hay que pintar de una tarjeta en este cuadro.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct CardFrame {
    /// Desplazamiento vertical respecto de su ranura, en píxeles.
    pub offset_y: f32,
    /// Opacidad de la tarjeta, de 0 a 1.
    pub opacity: f32,
    /// Tinte de espacio activo, de 0 a 1.
    pub tint: f32,
}

/// Resultado de un cuadro de la barra lateral.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct SpaceFrame {
    /// Un fotograma por tarjeta, en el mismo orden que la lista recibida.
    pub cards: Vec<CardFrame>,
    /// Realce de levantado del arrastre, de 0 a 1.
    pub lift: f32,
    /// `true` si algo sigue en movimiento y hace falta otro cuadro.
    pub animating: bool,
}

/// Motor de movimiento de las tarjetas, con estado retenido por id.
#[derive(Debug, Default)]
pub(crate) struct CardMotion {
    offsets: Motions<usize>,
    tints: Motions<usize>,
    lift: MotionState,
    /// Ids que ya se vieron, para animar la entrada una sola vez por espacio.
    seen: HashSet<usize>,
}

impl CardMotion {
    /// Avanza un cuadro hasta `now_ms` y devuelve qué pintar.
    ///
    /// `lifting` es el gesto de arrastre en curso; `reduced_motion` llega de la
    /// preferencia del usuario cuando el anfitrión puede leerla (hoy el hook de
    /// layout no recibe contexto, así que se pasa `false`).
    pub(crate) fn sample(
        &mut self,
        now_ms: u64,
        inputs: &[CardInput],
        lifting: bool,
        reduced_motion: bool,
    ) -> SpaceFrame {
        let now = Duration::from_millis(now_ms);
        let mut cards = Vec::with_capacity(inputs.len());
        let mut animating = false;

        for input in inputs {
            let (target_y, transition) = match input.drag_offset {
                Some(offset) => (offset, follow()),
                None => (input.slot_delta as f32 * CARD_PITCH, shift()),
            };
            let mut motion =
                Motion::new(Target::default().y(target_y).opacity(1.0)).transition(transition);
            // La entrada solo existe la primera vez que aparece este id: al
            // reordenar o rebasar los resortes, la tarjeta no vuelve a entrar.
            if !self.seen.contains(&input.id) {
                self.seen.insert(input.id);
                motion = motion.initial(Target::default().y(target_y + ENTER_OFFSET).opacity(0.0));
            }
            let frame = self
                .offsets
                .sample_one(input.id, motion, now, reduced_motion);
            animating |= frame.is_animating();

            let tint = self.tints.sample_one(
                input.id,
                Motion::new(Target::default().opacity(if input.active { 1.0 } else { 0.0 }))
                    .transition(fade()),
                now,
                reduced_motion,
            );
            animating |= tint.is_animating();

            cards.push(CardFrame {
                offset_y: frame.target().y_px(),
                opacity: frame.target().alpha(),
                tint: tint.target().alpha(),
            });
        }

        // Un espacio que se fue deja de tener estado: guardarlo sería arrastrar
        // trayectorias de tarjetas que ya no existen.
        let live: HashSet<usize> = inputs.iter().map(|input| input.id).collect();
        self.seen.retain(|id| live.contains(id));
        self.offsets.retain(|id| live.contains(id));
        self.tints.retain(|id| live.contains(id));

        let lift = self.lift.sample(
            Motion::new(Target::default().opacity(if lifting { 1.0 } else { 0.0 }))
                .transition(fade()),
            now,
            reduced_motion,
        );
        animating |= lift.is_animating();

        SpaceFrame {
            cards,
            lift: lift.target().alpha(),
            animating,
        }
    }

    /// Alinea los resortes con una lista nueva sin cambiar lo que se ve.
    ///
    /// Se llama justo después de aplicar un orden pendiente: en ese instante
    /// cada tarjeta ya está visualmente en su ranura nueva, así que el
    /// desplazamiento correcto es cero. Descartar los resortes es exacto (no hay
    /// salto) y evita tener que reposicionarlos, que es lo que el motor no
    /// permite hacer sin perder la velocidad.
    pub(crate) fn rebase(&mut self) {
        self.offsets.clear();
    }

    /// Olvida todo el movimiento. Se usa cuando la barra deja de verse, para
    /// que al volver las tarjetas entren animadas en vez de aparecer quietas.
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(id: usize) -> CardInput {
        CardInput {
            id,
            active: false,
            drag_offset: None,
            slot_delta: 0,
        }
    }

    /// Sin movimiento, el primer cuadro ya está quieto y no pide más.
    #[test]
    fn en_reposo_no_pide_cuadros() {
        let mut motion = CardMotion::default();
        // Primer cuadro: la tarjeta entra, así que se mueve una vez.
        let first = motion.sample(0, &[card(1)], false, false);
        assert!(first.animating);
        // Agotada la entrada, no hay nada que animar.
        let settled = motion.sample(5_000, &[card(1)], false, false);
        assert!(!settled.animating, "quedó pidiendo cuadros en reposo");
        assert_eq!(settled.cards[0].offset_y, 0.0);
        assert_eq!(settled.cards[0].opacity, 1.0);
    }

    /// Una tarjeta nueva entra desde abajo y transparente.
    #[test]
    fn una_tarjeta_nueva_entra_animada() {
        let mut motion = CardMotion::default();
        let frame = motion.sample(0, &[card(7)], false, false);
        assert!(frame.cards[0].opacity < 0.5, "no entró con fundido");
        assert!(frame.cards[0].offset_y > 0.0, "no entró desde abajo");
        // Al asentarse queda opaca y en su ranura.
        let settled = motion.sample(5_000, &[card(7)], false, false);
        assert_eq!(settled.cards[0].opacity, 1.0);
        assert_eq!(settled.cards[0].offset_y, 0.0);
    }

    /// El desplazamiento de una vecina llega a una ranura completa y se detiene.
    #[test]
    fn una_vecina_recorre_una_ranura_y_se_detiene() {
        let mut motion = CardMotion::default();
        // Se da de alta quieta.
        motion.sample(0, &[card(1)], false, false);
        let mut input = card(1);
        input.slot_delta = -1;
        let mut t = 1_000;
        let mut frame = motion.sample(t, &[input], false, false);
        let mut steps = 0;
        while frame.animating {
            t += 16;
            frame = motion.sample(t, &[input], false, false);
            steps += 1;
            assert!(steps < 240, "el reacomodo no se detuvo");
        }
        assert_eq!(frame.cards[0].offset_y, -CARD_PITCH);
    }

    /// Cambiar el destino a mitad del recorrido continúa desde donde está y no
    /// reinicia el movimiento: es lo que hace el arrastre al cruzar ranuras.
    #[test]
    fn cambiar_de_destino_no_reinicia_el_movimiento() {
        let mut motion = CardMotion::default();
        motion.sample(0, &[card(1)], false, false);
        let mut down = card(1);
        down.slot_delta = 1;
        motion.sample(1_000, &[down], false, false);
        motion.sample(1_080, &[down], false, false);
        let parcial = motion.sample(1_120, &[down], false, false).cards[0].offset_y;
        assert!(
            parcial > 0.0 && parcial < CARD_PITCH,
            "no iba en camino: {parcial}"
        );
        // El nuevo destino es su propia ranura. El resorte conserva posición Y
        // velocidad: sigue subiendo un poco antes de darse la vuelta.
        let mut back = card(1);
        back.slot_delta = 0;
        let siguiente = motion.sample(1_140, &[back], false, false).cards[0].offset_y;
        assert!(
            (siguiente - parcial).abs() < CARD_PITCH * 0.25,
            "teleportó al nuevo destino en vez de continuar: {siguiente} vs {parcial}"
        );
        // Y termina donde se le pidió, no donde iba.
        let mut frame = SpaceFrame::default();
        let mut t = 1_140;
        for _ in 0..240 {
            t += 16;
            frame = motion.sample(t, &[back], false, false);
        }
        assert_eq!(frame.cards[0].offset_y, 0.0);
        assert!(!frame.animating);
    }

    /// La tarjeta agarrada persigue al puntero y el levantado sube.
    #[test]
    fn la_tarjeta_agarrada_sigue_al_puntero() {
        let mut motion = CardMotion::default();
        motion.sample(0, &[card(1)], false, false);
        let mut dragged = card(1);
        dragged.drag_offset = Some(30.0);
        let mut t = 1_000;
        let mut frame = SpaceFrame::default();
        for _ in 0..120 {
            t += 16;
            frame = motion.sample(t, &[dragged], true, false);
        }
        assert!((frame.cards[0].offset_y - 30.0).abs() < 0.5);
        assert!(frame.lift > 0.9, "el levantado no llegó: {}", frame.lift);
    }

    /// Al soltar, el levantado baja solo aunque la tarjeta ya no se arrastre.
    #[test]
    fn el_levantado_baja_al_soltar() {
        let mut motion = CardMotion::default();
        let mut t = 0;
        let mut dragged = card(1);
        dragged.drag_offset = Some(20.0);
        for _ in 0..120 {
            t += 16;
            motion.sample(t, &[dragged], true, false);
        }
        let mut released = card(1);
        released.slot_delta = 0;
        let mut frame = SpaceFrame::default();
        for _ in 0..120 {
            t += 16;
            frame = motion.sample(t, &[released], false, false);
        }
        assert_eq!(frame.lift, 0.0);
        assert!(!frame.animating);
    }

    /// El tinte del espacio activo se interpola en vez de encenderse de golpe.
    #[test]
    fn el_tinte_del_activo_se_interpola() {
        let mut motion = CardMotion::default();
        motion.sample(0, &[card(1)], false, false);
        let mut active = card(1);
        active.active = true;
        // El primer cuadro fija el destino; el tinte avanza en los siguientes.
        motion.sample(1_000, &[active], false, false);
        let mid = motion.sample(1_080, &[active], false, false).cards[0].tint;
        assert!(mid > 0.0 && mid < 1.0, "el tinte saltó: {mid}");
        let mut frame = SpaceFrame::default();
        let mut t = 1_000;
        for _ in 0..120 {
            t += 16;
            frame = motion.sample(t, &[active], false, false);
        }
        assert_eq!(frame.cards[0].tint, 1.0);
    }

    /// Quitar una tarjeta intermedia no desplaza a las demás: el estado está
    /// indexado por id, no por posición.
    #[test]
    fn quitar_una_tarjeta_no_desplaza_a_las_vecinas() {
        let mut motion = CardMotion::default();
        motion.sample(0, &[card(1), card(2), card(3)], false, false);
        let a = motion.sample(1_000, &[card(1), card(2), card(3)], false, false);
        let b = motion.sample(1_000, &[card(1), card(3)], false, false);
        for frame in [&a, &b] {
            assert!(frame.cards.iter().all(|card| card.offset_y == 0.0));
        }
    }

    /// La ranura destino se redondea al medio paso y se recorta en los bordes.
    #[test]
    fn la_ranura_destino_redondea_al_medio_paso() {
        assert_eq!(drop_slot(0, 0.0, 3), 0);
        assert_eq!(drop_slot(0, CARD_PITCH * 0.4, 3), 0);
        assert_eq!(drop_slot(0, CARD_PITCH * 0.6, 3), 1);
        assert_eq!(drop_slot(0, CARD_PITCH * 2.0, 3), 2);
        assert_eq!(drop_slot(0, CARD_PITCH * 9.0, 3), 2);
        assert_eq!(drop_slot(2, -CARD_PITCH * 9.0, 3), 0);
        assert_eq!(drop_slot(0, 0.0, 0), 0);
    }

    /// La misma posición del puntero da siempre la misma ranura: sin ancla
    /// acumulada, un evento perdido no descoloca el gesto.
    #[test]
    fn la_ranura_destino_es_idempotente() {
        let y = CARD_PITCH * 1.7;
        let first = drop_slot(0, y, 4);
        for _ in 0..5 {
            assert_eq!(drop_slot(0, y, 4), first);
        }
    }

    /// Reordenar por id mueve una sola tarjeta y conserva el resto.
    #[test]
    fn reordenar_mueve_una_sola_tarjeta() {
        assert_eq!(reordered(&[10, 11, 12], 0, 2), vec![11, 12, 10]);
        assert_eq!(reordered(&[10, 11, 12], 2, 0), vec![12, 10, 11]);
        assert_eq!(reordered(&[10, 11, 12], 1, 1), vec![10, 11, 12]);
        assert_eq!(reordered(&[10], 0, 5), vec![10]);
    }

    /// El paso de tarjeta es exactamente alto más separación: el arrastre
    /// depende de ese número.
    #[test]
    fn el_paso_es_alto_mas_separacion() {
        assert_eq!(CARD_PITCH, CARD_HEIGHT + CARD_GAP);
    }
}
