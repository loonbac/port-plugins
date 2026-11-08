//! Máquina de estados de la animación del cursor.
//!
//! Es la mitad del plugin que no sabe nada de GPUI ni de PORT: recibe la
//! geometría del cursor en coordenadas de celda y un reloj en milisegundos, y
//! devuelve los rectángulos que hay que pintar en ese cuadro.
//!
//! Los nombres y las constantes vienen del shader `smooth_cursor.glsl` de
//! Ghostty, pero la mecánica es distinta: allí el shader interpola en el
//! espacio de píxeles de la textura ya renderizada; aquí se interpola en
//! coordenadas de celda y PORT convierte a píxeles al pintar. El resultado
//! visual es el mismo y el plugin no necesita conocer DPI, padding ni tamaño
//! de ventana.

/// Rectángulo en coordenadas de celda fraccionarias: `column = 0.0` es el borde
/// izquierdo de la primera columna, `row = 0.0` el borde superior de la primera
/// fila. Ancho y alto en celdas permiten interpolar cursores de distinta altura
/// (de una línea de texto a un bloque más alto) sin saltos.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellRect {
    pub column: f32,
    pub row: f32,
    pub width: f32,
    pub height: f32,
}

impl CellRect {
    /// Centro del rectángulo, en el mismo espacio.
    pub fn center(&self) -> (f32, f32) {
        (self.column + self.width * 0.5, self.row + self.height * 0.5)
    }

    /// Interpola dos rectángulos. El tamaño también se interpola: es lo que
    /// hace que el cursor parezca estirarse al cambiar de alto.
    pub fn lerp(from: CellRect, to: CellRect, t: f32) -> CellRect {
        CellRect {
            column: from.column + (to.column - from.column) * t,
            row: from.row + (to.row - from.row) * t,
            width: from.width + (to.width - from.width) * t,
            height: from.height + (to.height - from.height) * t,
        }
    }
}

/// Parámetros del comportamiento, todos ajustables por configuración.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tuning {
    /// Duración de la transición en milisegundos.
    pub duration_ms: u64,
    /// Opacidad máxima de la estela (0.0 la apaga).
    pub trail_strength: f32,
    /// Radio de esquina en píxeles, tal como lo configura el usuario.
    ///
    /// El acotado por tamaño de celda no es asunto del plugin: PORT lo aplica
    /// con sus métricas antes de pintar, así que aquí el valor viaja tal cual.
    pub corner_radius_px: f32,
    /// Distancia máxima, en celdas, para dibujar estela. Un salto mayor es un
    /// repintado (scroll, cambio de viewport) y no lleva trazo.
    pub max_trail_cells: f32,
}

impl Default for Tuning {
    /// Los valores del shader: `ANIMATION_DURATION 0.12`, `TRAIL_STRENGTH 0.30`,
    /// `CORNER_RADIUS 2.0`.
    fn default() -> Self {
        Self {
            duration_ms: 120,
            trail_strength: 0.30,
            corner_radius_px: 2.0,
            max_trail_cells: 2.0,
        }
    }
}

/// Lo que hay que pintar en un cuadro. `None` significa "no toques nada": PORT
/// pinta su cursor de siempre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Overlay {
    /// Rectángulo del cursor animado.
    pub rect: CellRect,
    /// Opacidad del cursor, 0.0..=1.0.
    pub opacity: f32,
    /// Radio de esquina en píxeles. PORT lo limita a media celda y lo pasa a
    /// `PaintQuad::corner_radii`, que también trabaja en píxeles.
    pub corner_radius_px: f32,
    /// Estela opcional, del mismo color y forma que el cursor.
    pub trail: Option<Trail>,
}

/// Tramo ya recorrido por el cursor, con su propia opacidad.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trail {
    pub rect: CellRect,
    pub opacity: f32,
}

/// Forma del cursor, espejo del enum de `term-core`.
///
/// `anim` no depende de `port-plugin-api`, así que el adaptador traduce en la
/// frontera. El valor por defecto es `Block` porque es lo que PORT pinta hoy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Shape {
    #[default]
    Block,
    Beam,
    Underline,
}

/// Ancho de un cursor de barra, en fracción de celda.
pub const BEAM_WIDTH: f32 = 0.15;

/// Alto de un cursor de subrayado, en fracción de celda.
pub const UNDERLINE_HEIGHT: f32 = 0.12;

/// Ajusta la huella de celda completa a la forma real del cursor.
///
/// PORT solo pinta bloque, e `CursorState.rect` es siempre esa huella. Un
/// usuario que configura `Beam` o `Underline` espera una barra, no un bloque:
/// estrechar aquí es lo que evita un cursor visualmente más grueso de lo pedido.
/// El quad es geometría libre, así que el host lo pinta tal cual.
pub fn shape_rect(rect: CellRect, shape: Shape) -> CellRect {
    match shape {
        Shape::Block => rect,
        Shape::Beam => CellRect {
            width: rect.width * BEAM_WIDTH,
            ..rect
        },
        Shape::Underline => CellRect {
            row: rect.row + rect.height * (1.0 - UNDERLINE_HEIGHT),
            height: rect.height * UNDERLINE_HEIGHT,
            ..rect
        },
    }
}

/// Easing del shader: arranca rápido y frena suave.
pub fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// Estado de la animación.
///
/// Guarda la celda anterior y la actual, y desde cuándo corre la transición.
/// Nada más: no retiene la rejilla, ni el frame, ni colores.
#[derive(Debug, Default)]
pub struct Animator {
    previous: Option<CellRect>,
    current: Option<CellRect>,
    /// Instante del último cambio de celda, en el reloj del host.
    changed_at_ms: u64,
}

/// Ventana de gracia tras el final de la transición, en milisegundos.
///
/// Dos cuadros a 120 Hz: suficiente para que el host repinte con el cursor
/// nativo restaurado, y demasiado corta para notarse.
pub const HANDOFF_GRACE_MS: u64 = 32;

impl Animator {
    pub fn new() -> Self {
        Self::default()
    }

    /// `true` mientras la transición está en curso.
    pub fn is_animating(&self, now_ms: u64, tuning: &Tuning) -> bool {
        self.previous.is_some()
            && self.current.is_some()
            && self.previous != self.current
            && now_ms.saturating_sub(self.changed_at_ms) < tuning.duration_ms
    }

    /// `true` mientras hay que seguir pidiendo cuadros.
    ///
    /// Incluye una ventana de gracia después de que la transición termine: en
    /// ese último cuadro el plugin deja de devolver overlay y el cursor nativo
    /// vuelve a pintarse. Sin la gracia, el último cuadro animado quedaría en
    /// pantalla con el cursor nativo suprimido y el cursor se vería detenido
    /// a medio camino hasta el siguiente cambio de la terminal.
    pub fn needs_animation(&self, now_ms: u64, tuning: &Tuning) -> bool {
        self.is_animating(now_ms, tuning) || self.handoff_pending(now_ms, tuning)
    }

    /// `true` durante la ventana de gracia posterior a la transición.
    fn handoff_pending(&self, now_ms: u64, tuning: &Tuning) -> bool {
        let Some(previous) = self.previous else {
            return false;
        };
        if self.current.is_none() || previous == self.current.unwrap() {
            return false;
        }
        let elapsed = now_ms.saturating_sub(self.changed_at_ms);
        elapsed >= tuning.duration_ms && elapsed < tuning.duration_ms + HANDOFF_GRACE_MS
    }

    /// Progreso de la transición, 0.0 al salir y 1.0 al llegar.
    pub fn progress(&self, now_ms: u64, tuning: &Tuning) -> f32 {
        if tuning.duration_ms == 0 {
            return 1.0;
        }
        let elapsed = now_ms.saturating_sub(self.changed_at_ms);
        (elapsed as f32 / tuning.duration_ms as f32).clamp(0.0, 1.0)
    }

    /// Registra la celda que PORT dice que ocupa el cursor en este cuadro.
    ///
    /// Devuelve `true` si empezó una transición nueva. Un cursor que desaparece
    /// (`None`) corta la animación sin estela: no tiene sentido arrastrar algo
    /// que ya no está.
    pub fn observe(&mut self, rect: Option<CellRect>, now_ms: u64) -> bool {
        match rect {
            None => {
                self.previous = None;
                self.current = None;
                false
            }
            Some(next) => {
                if self.current == Some(next) {
                    return false;
                }
                // La primera aparición no anima: no hay de dónde venir, y
                // animarla pintaría un cursor deslizándose desde el origen de
                // la rejilla.
                let started = self.current.is_some();
                self.previous = self.current;
                self.current = Some(next);
                self.changed_at_ms = now_ms;
                started
            }
        }
    }

    /// Rectángulos a pintar en este cuadro, o `None` para dejarlo todo como está.
    ///
    /// El llamador ya decidió que hay cursor visible y que la animación está
    /// activa; aquí solo se resuelve la geometría.
    pub fn overlay(&self, now_ms: u64, tuning: &Tuning) -> Option<Overlay> {
        let current = self.current?;
        let radius = tuning.corner_radius_px.max(0.0);

        // Pasada la transición no se pinta nada: el cursor nativo vuelve a su
        // sitio y el plugin no gasta un quad. Es la idea 1 del shader, y de
        // paso garantiza que un plugin roto no pueda hacer desaparecer el
        // cursor.
        let previous = self.previous.filter(|p| *p != current)?;

        let raw = self.progress(now_ms, tuning);
        if raw >= 1.0 {
            return None;
        }

        let eased = ease_out_cubic(raw);
        let rect = CellRect::lerp(previous, current, eased);

        Some(Overlay {
            rect,
            opacity: 1.0,
            corner_radius_px: radius,
            trail: Self::trail(previous, current, raw, eased, tuning),
        })
    }

    /// Estela: el tramo entre los dos centros que el cursor ya recorrió.
    ///
    /// Se pinta dos veces la misma forma con distinta opacidad, porque PORT
    /// dibuja quads y no un SDF con una máscara por píxel. La primera pasada es
    /// la estela completa; la segunda recorta la parte ya recorrida, de modo que
    /// queda visible solo `previous_center → animated_center`. Es equivalente al
    /// `if (h <= t)` del shader en el único caso que importa: movimiento
    /// rectilíneo, que es todo lo que hace un cursor de terminal.
    fn trail(
        previous: CellRect,
        current: CellRect,
        raw: f32,
        eased: f32,
        tuning: &Tuning,
    ) -> Option<Trail> {
        let strength = tuning.trail_strength;
        if strength <= 0.0 {
            return None;
        }

        let (from_x, from_y) = previous.center();
        let (to_x, to_y) = current.center();
        let (mid_x, mid_y) = (
            from_x + (to_x - from_x) * eased,
            from_y + (to_y - from_y) * eased,
        );

        // Un salto de más de un par de celdas no es un desplazamiento del
        // cursor: es un repintado (scroll, cambio de viewport, `clear`).
        // Arrastrar ahí sería un trazo largo que no corresponde a ningún gesto.
        let distance = (to_x - from_x).abs().max((to_y - from_y).abs());
        let max_distance = tuning.max_trail_cells.max(0.0);
        if max_distance > 0.0 && distance > max_distance {
            return None;
        }

        // El fade apaga la estela a medida que el cursor llega.
        let opacity = (1.0 - raw).clamp(0.0, 1.0) * strength;
        if opacity <= 0.0 {
            return None;
        }

        // Rectángulo que cubre el tramo recorrido, con el tamaño del cursor
        // actual para que la estela no engorde en el eje perpendicular.
        let width_cells = (mid_x - from_x).abs();
        let height_cells = (mid_y - from_y).abs();
        let rect = CellRect {
            column: from_x.min(mid_x) - current.width * 0.5,
            row: from_y.min(mid_y) - current.height * 0.5,
            width: width_cells + current.width,
            height: height_cells + current.height,
        };

        Some(Trail { rect, opacity })
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

    /// S1: la primera aparición no anima; el segundo avistamiento sí.
    #[test]
    fn first_sighting_does_not_animate() {
        let mut a = Animator::new();
        assert!(!a.observe(Some(rect(0.0, 0.0)), 1_000));
        assert!(!a.is_animating(1_000, &Tuning::default()));
    }

    /// S2: un cambio de celda abre una transición y arranca el reloj.
    #[test]
    fn moving_starts_the_transition() {
        let mut a = Animator::new();
        a.observe(Some(rect(0.0, 0.0)), 1_000);
        assert!(a.observe(Some(rect(0.0, 1.0)), 1_100));

        let t = Tuning::default();
        assert!(a.is_animating(1_100, &t));
        assert_eq!(a.progress(1_100, &t), 0.0);
        assert!((a.progress(1_160, &t) - 0.5).abs() < 1e-4);
        assert_eq!(a.progress(1_220, &t), 1.0);
        assert!(!a.is_animating(1_220, &t));
    }

    /// S4: pasada la duración, no se pinta estela ni se interpola nada.
    #[test]
    fn finished_transition_is_passthrough() {
        let mut a = Animator::new();
        a.observe(Some(rect(2.0, 0.0)), 0);
        a.observe(Some(rect(2.0, 1.0)), 10);

        // Pasada la transición el plugin no pinta nada: vuelve el nativo.
        assert_eq!(a.overlay(10 + 500, &Tuning::default()), None);
    }

    /// S2: el tamaño se interpola, no solo la posición. Es lo que hace que el
    /// cursor parezca estirarse al pasar de una línea a un bloque.
    #[test]
    fn size_interpolates_with_position() {
        let mut a = Animator::new();
        let small = CellRect {
            column: 0.0,
            row: 0.0,
            width: 1.0,
            height: 1.0,
        };
        let tall = CellRect {
            column: 1.0,
            row: 3.0,
            width: 1.0,
            height: 2.0,
        };
        a.observe(Some(small), 0);
        a.observe(Some(tall), 0);

        let mid = a.overlay(120 / 2, &Tuning::default()).unwrap();
        // ease_out_cubic(0.5) = 1 - 0.5^3 = 0.875
        let eased = ease_out_cubic(0.5);
        assert!((mid.rect.row - (0.0 + 3.0 * eased)).abs() < 1e-4);
        assert!((mid.rect.height - (1.0 + 1.0 * eased)).abs() < 1e-4);
    }

    /// S3: la estela existe mientras hay transición y se apaga al llegar.
    #[test]
    fn trail_fades_as_cursor_arrives() {
        let mut a = Animator::new();
        a.observe(Some(rect(0.0, 0.0)), 0);
        a.observe(Some(rect(0.0, 1.0)), 0);

        let t = Tuning::default();
        let early = a.overlay(0, &t).unwrap().trail.unwrap();
        let late = a.overlay(110, &t).unwrap().trail.unwrap();

        assert!(early.opacity > late.opacity);
        // La estela temprana es casi TRAIL_STRENGTH; la tardía, casi nada.
        assert!(early.opacity <= t.trail_strength + 1e-4);
        assert!(late.opacity < early.opacity * 0.5);
    }

    /// S3: `trail_strength = 0` la apaga del todo.
    #[test]
    fn zero_strength_removes_trail() {
        let mut a = Animator::new();
        a.observe(Some(rect(0.0, 0.0)), 0);
        a.observe(Some(rect(0.0, 2.0)), 0);

        let tuning = Tuning {
            trail_strength: 0.0,
            ..Tuning::default()
        };
        assert_eq!(a.overlay(0, &tuning).unwrap().trail, None);
    }

    /// S3/design: un salto grande es un repintado, no un desplazamiento.
    #[test]
    fn long_jump_has_no_trail() {
        let mut a = Animator::new();
        a.observe(Some(rect(0.0, 0.0)), 0);
        a.observe(Some(rect(0.0, 24.0)), 0);

        let overlay = a.overlay(0, &Tuning::default()).unwrap();
        assert_eq!(overlay.trail, None);
        // Pero el cursor sí se interpola: el salto es real, solo la estela
        // mentiría sobre el recorrido.
        assert_ne!(overlay.rect, rect(0.0, 24.0));
    }

    /// Un cursor que desaparece corta la animación sin dejar estela colgada.
    #[test]
    fn hidden_cursor_cancels_animation() {
        let mut a = Animator::new();
        a.observe(Some(rect(0.0, 0.0)), 0);
        a.observe(Some(rect(0.0, 1.0)), 0);
        assert!(a.is_animating(0, &Tuning::default()));

        a.observe(None, 10);
        assert!(!a.is_animating(10, &Tuning::default()));
        assert_eq!(a.overlay(10, &Tuning::default()), None);
    }

    /// Reobservar la misma celda no reinicia el reloj: si el PTY repinta sin
    /// mover el cursor, la animación no vuelve a empezar.
    #[test]
    fn same_cell_does_not_restart_the_clock() {
        let mut a = Animator::new();
        a.observe(Some(rect(0.0, 0.0)), 0);
        a.observe(Some(rect(0.0, 1.0)), 100);

        assert!(!a.observe(Some(rect(0.0, 1.0)), 150));
        let t = Tuning::default();
        // El progreso sigue contando desde 100, no desde 150.
        assert!((a.progress(200, &t) - (100.0 / 120.0)).abs() < 1e-4);
    }

    /// El radio viaja en píxeles tal como lo configuró el usuario: el acotado
    /// por tamaño de celda lo hace PORT con sus propias métricas.
    #[test]
    fn corner_radius_is_passed_through_in_pixels() {
        let tuning = Tuning {
            corner_radius_px: 2.0,
            ..Tuning::default()
        };
        let mut a = Animator::new();
        a.observe(Some(rect(0.0, 0.0)), 0);
        a.observe(Some(rect(0.0, 1.0)), 0);

        assert_eq!(a.overlay(0, &tuning).unwrap().corner_radius_px, 2.0);
    }

    /// Un radio negativo se satura a cero en vez de llegar al pintado.
    #[test]
    fn negative_corner_radius_is_floored() {
        let tuning = Tuning {
            corner_radius_px: -4.0,
            ..Tuning::default()
        };
        let mut a = Animator::new();
        a.observe(Some(rect(0.0, 0.0)), 0);
        a.observe(Some(rect(0.0, 1.0)), 0);

        assert_eq!(a.overlay(0, &tuning).unwrap().corner_radius_px, 0.0);
    }

    /// `duration_ms = 0` desactiva la animación sin casos raros de división.
    #[test]
    fn zero_duration_is_passthrough() {
        let mut a = Animator::new();
        a.observe(Some(rect(0.0, 0.0)), 0);
        a.observe(Some(rect(0.0, 3.0)), 1_000);

        let tuning = Tuning {
            duration_ms: 0,
            ..Tuning::default()
        };
        assert!(!a.is_animating(1_000, &tuning));
        assert_eq!(a.overlay(1_000, &tuning), None);
    }

    /// El tick sobrevive al final de la transición lo justo para que el
    /// cursor nativo vuelva: sin esto el último cuadro animado se queda
    /// congelado con el nativo suprimido.
    #[test]
    fn handoff_keeps_one_extra_frame_then_stops() {
        let mut a = Animator::new();
        a.observe(Some(rect(0.0, 0.0)), 0);
        a.observe(Some(rect(0.0, 1.0)), 0);

        let t = Tuning::default();
        assert!(a.needs_animation(60, &t), "a mitad de transición");
        assert!(a.needs_animation(130, &t), "en la ventana de gracia");
        assert!(
            !a.needs_animation(120 + HANDOFF_GRACE_MS + 1, &t),
            "cerrada la gracia, deja de pedir cuadros"
        );
    }

    /// La gracia no se pide si nunca hubo transición: un cursor quieto no
    /// redibuja nada.
    #[test]
    fn idle_cursor_never_requests_frames() {
        let mut a = Animator::new();
        a.observe(Some(rect(3.0, 2.0)), 0);
        assert!(!a.needs_animation(0, &Tuning::default()));
        assert!(!a.needs_animation(10_000, &Tuning::default()));
    }

    /// Un bloque no cambia de geometría.
    #[test]
    fn block_keeps_the_full_cell_footprint() {
        let full = rect(2.0, 3.0);
        assert_eq!(shape_rect(full, Shape::Block), full);
    }

    /// Una barra es estrecha y arranca en la columna, no centrada.
    #[test]
    fn beam_is_narrow_and_left_aligned() {
        let narrow = shape_rect(rect(2.0, 3.0), Shape::Beam);
        assert!((narrow.width - BEAM_WIDTH).abs() < 1e-6);
        assert_eq!(narrow.column, 2.0, "la barra va al borde izquierdo");
        assert_eq!(narrow.height, 1.0, "ocupa el alto completo");
    }

    /// Un subrayado se pega al borde inferior de la celda.
    #[test]
    fn underline_sits_at_the_bottom_of_the_cell() {
        let line = shape_rect(rect(2.0, 3.0), Shape::Underline);
        assert!((line.height - UNDERLINE_HEIGHT).abs() < 1e-6);
        assert_eq!(line.width, 1.0, "ocupa el ancho completo");
        assert!(
            (line.row + line.height - 4.0).abs() < 1e-6,
            "su base toca el borde inferior de la celda"
        );
    }

    /// `ease_out_cubic` cumple los extremos y es monótona.
    #[test]
    fn easing_endpoints_and_monotonicity() {
        assert_eq!(ease_out_cubic(0.0), 0.0);
        assert_eq!(ease_out_cubic(1.0), 1.0);
        assert_eq!(ease_out_cubic(-1.0), 0.0);
        assert_eq!(ease_out_cubic(2.0), 1.0);

        let mut last = 0.0;
        for step in 0..=20 {
            let value = ease_out_cubic(step as f32 / 20.0);
            assert!(value >= last, "no monótona en {step}");
            last = value;
        }
        // Arranca más rápido de lo que frena: a mitad de camino ya superó 0.5.
        assert!(ease_out_cubic(0.5) > 0.5);
    }
}
