# Herdr: barra lateral de spaces estable y con movimiento real

Arreglar los defectos de interacción de la barra lateral de SPACES (espacios que
cambian de lugar solos, gestos que quedan pegados al pasar el mouse) y reemplazar
la animación casera por un motor declarativo (`gpui-ce-motion`), con identidad
estable por espacio y una sola fuente de verdad para el gesto.

## Specs

S1. «tengo un problema con la barra lateral de mi plugin de herdr, se bugea mucho
los spaces a veces cambian de lado solos».

S2. «la animacion no es buena».

S3. «me gustaria que le agregaras https://github.com/1bye/gpui-motion para hacer
mejores animaciones para el plugin, asi no tendria problemas».

S4. «hay demasiados problemas con el panel lateral de spaces, tantos que ni te
puedo explicar cuales son porque son muchos y muy especificos que suceden
simplemente pasando el mouse o cambiando de de space y se notan a simple vista
pero no puedo explicartelo no se como».

S5. «asi que tienes que analizar eeso y arrglarlo».

## Diagnóstico (evidencia en el código, previo a los cambios)

D1 · Gesto pegado (S1, S4). El `mouse_down` de una tarjeta escribe `space_drag` y
pide repintar; la capa que recibe el `mouse_up` (`hooks/layout/sidebar.rs`, el
overlay de 6000 px) sólo existe en el frame siguiente. GPUI reconstruye los
`mouse_listeners` al pintar, así que un `MouseUp` que llega antes del repintado
(clic rápido, o down/move/up en el mismo lote) no encuentra ese listener y
`space_drag` queda `Some` para siempre. Desde ahí, cualquier movimiento del mouse
sobre la ventana entra por el overlay y `move_space_drag` reordena los espacios
sin ningún botón apretado: «cambian de lado solos». Además `needs_animation`
devuelve `true` mientras `space_drag.is_some()`, así que la ventana queda pidiendo
cuadros a 60 fps indefinidamente.

D2 · Identidad por índice (S1). `HerdrSpace` no tiene id. Los resortes viven en un
`Vec<Spring>` paralelo al `Vec<HerdrSpace>` y se sincronizan con
`resize(len, Spring::new())`: al cerrar un espacio intermedio el desplazamiento
residual de una tarjeta se aplica a la siguiente, y una tarjeta nueva hereda el
resorte de otra. El reordenamiento muta `spaces` y los resortes a la vez;
cualquier desincronización desplaza la tarjeta equivocada.

D3 · El activo se mueve durante el arrastre (S1, S4). `swap_space_slots`
intercambia `active_space_index` en cada intercambio: arrastrar un espacio que no
era el activo cambia el espacio activo (y con él la terminal) a mitad del gesto.

D4 · Paso estimado (S1). `SPACE_CARD_PITCH = 45.0` es una estimación; la altura
real de la tarjeta depende del texto. El ancla del arrastre se recentra sumando ese
paso en cada intercambio (`anchor_y += SPACE_CARD_PITCH`), así que el error se
acumula y, tras dos o tres cruces, un movimiento mínimo dispara varios
intercambios.

D5 · Movimiento pobre (S2). El resorte casero no interpola el tinte de la tarjeta
activa ni el estado de hover (no hay hover), el realce de levantado se rampa
linealmente y se vuelve a suavizar con `ease_out_cubic`, no hay entrada animada de
tarjetas nuevas ni del propio panel, y el ancho de la barra salta.

## Tasks

- T1 · S1 · port-plugins · identidad estable por espacio: `HerdrSpace.id` +
  `next_space_id`, reconciliación de constructores y contadores; el índice activo
  se resuelve por id, nunca por posición — commit `2f178d1`.
- T2 · S1 · S4 · port-plugins · gesto con una sola fuente de verdad: el
  `mouse_up` se captura en el armazón de la barra (presente en todos los frames,
  fase de captura sin filtro de hover), el arrastre no muta `spaces` y calcula la
  ranura destino desde el puntero (idempotente), pitch exacto de tarjeta —
  commit `2f178d1`.
- T3 · S2 · S3 · port-plugins · motor declarativo: dependencia `gpui-ce-motion`
  (sólo rasgos portables, `default-features = false`), `anim.rs` reescrito sobre
  `Motions<id>` con resortes visuales, interrupción y entrada animada — commit
  `2f178d1`.
- T4 · S2 · S4 · port-plugins · diseño: hover por tarjeta, tinte del espacio
  activo interpolado, altura fija (pitch exacto), sombra de levantado desde el
  fotograma y asidero de ancho agarrable dentro del panel — commit `2f178d1`.
- T5 · S5 · port-plugins · pruebas: mapeo de ranuras, arrastre idempotente y sin
  mutar el orden, activo quieto, commit al asentar, gesto que no queda pegado,
  movimiento que se detiene, y una prueba de layout que fija
  `pitch == alto + separación` — commit `2f178d1` (178 pruebas verdes).
- T6 · S5 · port-plugins · documentos: `README.md`/`README.es.md` del plugin con la
  dependencia nueva y el requisito de re-vendorizar en Nix — este commit.

## Log

L1 (pedido original, verbatim): «tengo un problema con la barra lateral de mi
plugin de herdr, se bugea mucho los spaces a veces cambian de lado solos, la
animacion no es buena, entre otras cosas, me gustaria que le agregaras
https://github.com/1bye/gpui-motion para hacer mejores animaciones para el plugin,
asi no tendria problemas, hay demasiados problemas con el panel lateral de spaces,
tantos que ni te puedo explicar cuales son porque son muchos y muy especificos que
suceden simplemente pasando el mouse o cambiando de de space y se notan a simple
vista pero no puedo explicartelo no se como, asi que tienes que analizar eeso y
arrglarlo».

L2 (diagnóstico, evidencia en el código previo): ver D1–D5. El defecto que el
usuario describe como «se notan a simple vista» al pasar el mouse es D1: el
listener del `mouse_up` vivía en la capa que sólo existe en el frame siguiente
al `mouse_down` (`hooks/layout/sidebar.rs`), y GPUI reconstruye los
`mouse_listeners` al pintar (`gpui-0.2.2/src/window.rs:3691`), así que un clic
rápido dejaba `space_drag` pegado y el siguiente movimiento reordenaba sin botón
apretado.

L3 (verificación, en la caja `instabox port`, Debian, mismo `gpui 0.2.2` y el
`port` hermano por `[patch]`): `cargo test -p port-plugin-herdr --lib` → 178
pasadas, 0 fallos; `cargo clippy -p port-plugin-herdr --all-targets` → sin
advertencias nuevas (sólo el aviso preexistente de `proc-macro-error2`);
`cargo fmt -p port-plugin-herdr -- --check` → limpio. `cargo build -p port` en el
checkout de PORT compila todas las dependencias de `port-app`, incluido este
plugin, y falla después en un import local ajeno (`MobilePlugin` en
`crates/port-app/src/main.rs`, con cambios locales sin commitear en esa caja). No
se pudo correr `cargo test --workspace`: `plugins/font-zoom/tests/protocol.rs`
falla al compilar por `CARGO_BIN_EXE_font-zoom` sin definir, un defecto
preexistente de ese crate (no tocado por este cambio).

L4 (proceso, TDD): el motor y sus pruebas se escribieron juntos y el primer rojo
observado fue de 5 pruebas (`cambiar_de_destino_no_reinicia_el_movimiento`,
`el_tinte_del_activo_se_interpola`, `arrastrar_no_cambia_el_espacio_activo`,
`las_vecinas_se_deslizan_en_vez_de_saltar`,
`soltar_asienta_la_tarjeta_y_la_animacion_termina`), corregidas contra el
comportamiento real del motor (la interrupción conserva velocidad, el tween
arranca en el cuadro del cambio de destino, la ventana de atención de
`needs_animation` y la dirección del arrastre en dos pruebas). No hubo un rojo
por aserción antes de cada línea implementada.

L5 (cambio de contrato declarado): el reordenamiento ya no muta `spaces` durante
el gesto. Quien lea el orden de `spaces` a mitad de un arrastre ve el orden viejo
hasta que el movimiento asienta (~300 ms). Ningún consumidor del repo lo hace:
`layout_json` y los hooks leen la lista en reposo.
