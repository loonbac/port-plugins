# Herdr: estado de agente por pestaña, visible en la barra lateral

Objetivo: adaptar a PORT el sistema de estado de agente de herdr
(`Idle`/`Working`/`Blocked`/`Unknown` + `seen`) para «pi en una pestaña», con la
señal calculada en el núcleo de PORT y el diseño en el plugin `herdr`. Alcance
de UI: solo la barra lateral izquierda.

## Specs

S1. «cuando un agente esta haciendo algo deberia ser distinguible de manera
visible en su seccion del panel izquierdo».

S2. «si pi me manda una notificacion sea pregunta u otra cosa deberia tambien
indicar de manera visible para poder ir ahi y atender la necesidad».

S3. «pi en una pestaña» (el programa `pi` que corre en una sesión/pestaña de
PORT; no los subagentes de la sección AGENTS).

S4. «Solo en la sidebar» (el aviso de atención vive en la barra lateral; no se
modifica la barra de pestañas ni el borde de la ventana).

S5. «descargate el repo de herdr, lee toda la implementacion que tiene y hazlo
igual pero adaptandolo a nuestro plugin, el codigo y como funciona lo ponen
ellos practicamente con su codigo y nosotros ponemos el diseno y la nueva app
con este plugin para mi terminal».

S6. Comportamiento de referencia de herdr (fuente `/home/loonbac/Proyectos/herdr-ref`):
`AgentState { Idle, Working, Blocked, Unknown }` con marca `seen` por panel;
agregación por prioridad `Blocked > Idle(sin ver) > Working > Idle(visto) >
Unknown`; detección por reglas de patrones sobre el fondo de la rejilla
(manifiestos TOML en `distribution/agent-detection/`), complementada con
`osc_title`/`osc_progress` y la actividad del PTY.

S7. Ninguna regla nueva puede introducir E/S de disco ni espera dentro de los
hooks del plugin ni del núcleo: los hooks leen memoria; el trabajo pesado vive
en hilos de fondo o en el `pump()` del núcleo (regresión ya corregida en
`1ff4626`).

S7. «quiero poder reposicionar los spaces arrastrandolos con el click y
que se ordenen asi tipo animado».

S8. «si en una pestana tengo abierto cierto cd si abro una nueva pestana se
abre otra terminal en esa misma carpeta, y que sea asi por cada space que
tenga».

S9. «lo que no me gusta es como funciona lo de el drag, la animacion es muy
simple y casi nada detallada, no se si conoces de animaciones de arrastre y
como se ve las reposiciones con animacion y movimiento etc, se puede hacer?» y
luego «si es necesario busca alguna libreria que haga algo similar a ver si lo
puedes agregar o no se, que funcione y que haga bonito el diseno».

## Tasks

- T1 · S6 · port · contrato `AgentState`/`SessionAgent` y notificación en
  `SpaceHook` (aditivo, con default) — worker — commit `597b3fb`.
- T2 · S6 · port · primitivas genéricas en `term-core::Session` (texto del fondo
  de la rejilla, título OSC, secuencia de salida) — worker — commit `597b3fb`.
- T3 · S6 · port · motor de manifiestos de herdr para agentes que NO anuncian
  OSC 7501 (claude, codex, …): `state`, `priority`, `region`, `any`/`all`,
  `contains`, `line_regex`, `visible_*`, `skip_state_update`. **Fase 2** —
  worker — commit `3e0f543`.
- T4 · S1 · S3 · port · detección y arbitraje por sesión en `port-app` usando el
  estado OSC de pi, la actividad del PTY y el programa en primer plano;
  `seen` al enfocar la sesión; notificación a plugins — worker — commit `20061fe`.
- T5 · S1 · S3 · S4 · port-plugins · estado por sesión en `HerdrState` y
  agregación por espacio — worker — commit `bd04d31`.
- T6 · S1 · S2 · S4 · port-plugins · render en la sidebar (trabajando, atención
  sin ver, terminado) con indicador visible y texto — worker — commit `bd04d31`.
- T7 · S2 · port-plugins · clic en un espacio con atención abre la pestaña que
  la requiere — worker — commit `bd04d31`.
- T8 · S6 · pruebas en ambos repos (motor de reglas, arbitraje, agregación,
  render) y actualización de `README.es.md`/`README.md` — workers — 175 + 95 +
  122 tests en verde; READMEs actualizados; más `b30e9af docs(plugin)` en `port`
  con `AgentState`/`update_session_agent` como API de plataforma.
- T9 · S7 · port-plugins · reordenar spaces arrastrando tarjetas (umbral
  clic/arrastre, swaps en vivo, tarjeta pulsante, `needs_animation` en
  arrastre) — commit `b60aba9` (junto a T10).
- T10 · S8 · port-plugins · pestaña nueva hereda el cwd de la activa de su
  espacio (por `take_spawn_session_request`, sin tocar el núcleo) — commit
  `b60aba9`.
- T11 · S9 · port-plugins · arrastre con movimiento real: resorte crítico
  (`anim::Spring`), la tarjeta sigue al puntero, las vecinas se deslizan,
  sombra de levantado interpolada y asentado al soltar — commit pendiente.

## Diseño (barra lateral, solo SPACES)

Cada tarjeta de espacio agrega el estado de las sesiones de sus pestañas y lo
muestra sin ocupar más ancho del necesario:

- `Working`: spinner braille animado por reloj de render (10 cuadros a 80 ms;
  la sidebar se repinta a ~60 fps por el latido del PTY, sin timers) en color
  de acento + texto corto `working`.
- `Blocked` (atención): punto `●` con parpadeo lento (periodo 1000 ms, tenue
  a 0.25 la mitad apagada) + texto `espera tu respuesta`; borde y fondo de la
  tarjeta teñidos mientras no se haya visto.
- `Idle` sin ver (terminó y nadie lo miró): punto en color de éxito + texto
  `listo`, con el mismo realce de tarjeta que `Blocked` pero más tenue.
- `Idle` visto: sin marca especial (la tarjeta queda como hoy).
- `Unknown`: sin marca.

Cuando una tarjeta tiene atención, el clic no sólo selecciona el espacio: también
selecciona la pestaña cuya sesión la produce, para «ir ahí y atender la
necesidad» (S2).

El estado llega del núcleo por sesión; el plugin no lee disco ni la rejilla.

## Referencia herdr (verificada con `path:line`)

- `src/detect/mod.rs:11` `enum AgentState { Idle, Working, Blocked, Unknown }`.
- `src/workspace/aggregate.rs:50` prioridad de agregación exacta.
- `src/detect/manifest.rs:368` `detect_with_osc(agent, DetectionInput{screen, osc_title, osc_progress})`.
- `src/detect/manifest.rs:446` evaluación de reglas; gana la de mayor
  `priority`; en empate, la primera de la lista.
- `distribution/agent-detection/pi.toml` reglas `working_literal` y
  `working_border` (regiones `whole_recent` y `bottom_non_empty_lines(12)`).
- `src/terminal/state.rs:339` `set_detected_state_with_visible_blocker`:
  arbitraje entre regla visible, integración y actividad del PTY.

## Log

L1 (pedido original, verbatim): «quiero editar el plugin de herdr para mi
terminal port, cuando un agente esta haciendo algo deberia ser distinguible de
manera visible en su seccion del panel izquierdo, tambien si pi me manda una
notificacion sea pregunta u otra cosa deberia tambien indicar de manera visible
para poder ir ahi y atender la necesidad».

L2 (respuesta 1, verbatim): «sabes que, descargate el repo de herdr, lee toda la
implementacion que tiene y hazlo igual pero adaptandolo a nuestro plugin, el
codigo y como funciona lo ponen ellos practicamente con su codigo y nosotros
ponemos el diseno y la nueva app con este plugin para mi terminal».

L3 (respuesta 2, verbatim): «pi en una pestaña».

L4 (respuesta 3, verbatim): «Solo en la sidebar».

L5 (exploración, evidencia): el plugin hoy solo lee la presencia de subagentes
en `~/.pi/agent/gentle-agents/presence/` (`in-process/herdr/src/presence.rs`),
que no contiene estado de pi ni avisos. PORT no parsea OSC y no expone a los
plugins el contenido de la rejilla
(`crates/term-core/src/session/mod.rs:76` solo atiende `PtyWrite`,
`ColorRequest` y `TextAreaSizeRequest`). Por eso el estado de «pi en una
pestaña» debe originarse en el núcleo.

L6 (decisión de arquitectura): el motor de detección se porta al núcleo de PORT
(primitivas genéricas en `term-core`, detección en `port-app`, contrato en
`plugin-api`) y el plugin `herdr` conserva únicamente el diseño y el render.
Motivo: el plugin no tiene acceso al contenido de la terminal, y la detección es
lógica de dominio del terminal, no presentación. Repositorio de referencia
clonado en `/home/loonbac/Proyectos/herdr-ref` (solo lectura).

L7 (decisión de secuencia): pi anunció su estado a la terminal por OSC 7501
(`state=working`, `state=blocked:kind=question`), que es exacto y no exige
`regex`/`toml` ni depender del parseo del dibujo. El camino de pi (S1–S4) se
entrega primero sobre OSC + actividad del PTY + modelo de estados de herdr; el
motor de manifiestos (T3) queda como fase 2 para agentes que no anuncian OSC.
Referencia del mensaje OSC en el bundle de pi: `formatProgramStatus`
emite `ESC]7501;state=...:app=pi:kind=...:msg=<base64>ST` y sólo activa el
protocolo si el terminal responde a la consulta `ESC]7501;?`.

L8 (evidencia T1+T2+T4, commits en `port` rama `feat/herdr-agent-state`):
`597b3fb feat(agent): AgentState contract and OSC program-status primitives` y
`20061fe feat(agent): per-session agent state detection and plugin
notification`. Verificación: 175 tests verde (T1+T2), 82 verde (T4), clippy sin
warnings. Desviaciones aceptadas frente al brief: `WORKING_WINDOW = 1000 ms`
(en vez de 600) e `is_known_agent` incluye `aider`. El churn de `Cargo.lock`
(quita el `source` git por el patch local) y los archivos con cambios locales
del usuario (`Cargo.toml`, `view/paint.rs`, etc.) quedaron FUERA de los
commits.

L9 (evidencia T5+T6+T7, commit en `port-plugins` rama `feat/herdr-agent-state`):
`bd04d31 feat(herdr): per-space agent state with attention in the sidebar`.
Verificación propia del orquestador: `cargo test -p port-plugin-herdr` → 122
passed. El `cargo fmt --all -- --check` global falla sólo por `agents.rs`
preexistente sin formatear (sin modificar, fuera de superficie). Se eliminó un
archivo extraviado `crates/port-app/src/agent.rs` (copia sin referencias,
fuera del workspace) antes de commitear.

L10 (evidencia T3, commit en `port` rama `feat/herdr-agent-state`):
`3e0f543 feat(agent): herdr manifest engine for agents without OSC`. Motor con
`priority`, regiones (`whole_recent`, `bottom/top_lines`, `osc_title`,
`osc_progress`), puertas anidadas y `visible_*`/`skip`; 10 manifiestos copiados
verbatim; arbitraje OSC → `visible_blocker` → manifiesto → actividad/reposo;
evaluación acelerada (200/1000 ms). Verificación propia: `cargo test -p port
--bin port` → 76 passed. Nota: `port-app/Cargo.toml` SÍ se commiteó (su diff
eran solo las 6 líneas de `regex`/`serde`/`toml`); `Cargo.lock`, el `Cargo.toml`
raíz y el resto de archivos con cambios locales del usuario quedaron fuera.
Regiones exóticas no implementadas (vacío documentado): las de marcadores de
prompt de codex y `prompt_box_body`/`after_last_horizontal_rule` de claude.

L11 (plataforma): a petición del usuario, `AgentState` y
`SpaceHook::update_session_agent` quedan documentados como API de plataforma
para cualquier plugin en `port/README.md` y `README.es.md` (sección
“Estado de agente por sesión”, con ejemplo mínimo), commit `b30e9af`. La
terminal sigue simple; cada plugin decide qué mostrar.

L12 (animación, a petición verbatim «queria tambien algo tipo animacion
parpadeanto algo asi para ver que esta en eso»): `Working` lleva spinner
braille animado y `Blocked` parpadeo lento; `Idle` sin ver queda estático.
Commit en `port-plugins`: `b407a79`. Tests: 124 en verde (122 + 2 nuevos de
`spinner_frame`/`blink_on`).

L13 (congelamiento en reposo, reporte verbatim «parece que la animacion se
corta abruptamente y al mover el mouse como que continua»): causa raíz — GPUI
solo repinta a demanda y sin salida del PTY ni gestos no hay cuadros.
Arreglo: `LayoutHook::needs_animation()` (default `false`) +
`PluginRegistry::needs_animation()` (OR) + la vista pide
`window.request_animation_frame()` mientras haya marcador animado; herdr lo
enciende solo con sidebar visible y algún espacio en `Working`/`Blocked`.
Commits: `port@dde0514`, `port-plugins@dc22071`. Tests: registro OR y
`needs_animation_solo_con_marcador_animado_a_la_vista` en verde.

L14 (publicación, autorización verbatim «puedes publicar pushear mergear lo
que haga falta y yo reiniciare port»): `port-plugins` fusionado a `master` y
pusheado (`a7822a1..dc22071`); `port` fusionado a `master` y pusheado
(`4a742eb..dde0514`), con los archivos locales del usuario sin commitear
intactos. NOTA para el reinicio: el `port` en uso es build de Nix
(`/run/current-system/sw/bin/port`); reiniciar solo no trae lo nuevo: hay que
correr `scripts/bump-port-plugins.sh` (mueve el pin + hash de vendor al master
recién pusheado) y luego `nixos-rebuild switch --update-input port`.

L15 (reordenar spaces con el clic, pedido verbatim S7): umbral clic/arrastre,
intercambios en vivo con ancla recentrada, tarjeta arrastrada pulsante y
`needs_animation` durante el arrastre para cuadros continuos.

L16 (entrega T9+T10): 132 tests en verde, clippy/fmt limpios salvo
`agents.rs` preexistente, commit `b60aba9` pusheado a `master`, build de
integración `cargo build -p port` en verde. INCIDENTE: un `pkill -x port` del
orquestador mató también la ventana principal del usuario (mismo nombre de
proceso `port` que la instancia de pruebas), perdiendo sus shells abiertos.
Lección registrada: jamás gestionar procesos por nombre; solo por PID exacto
de instancias propias, y no lanzar ni cerrar ventanas sin pedido explícito.

L17 (bump a master publicado): `scripts/bump-port-plugins.sh` movió el pin a
`b60aba9` con vendor verificado y paquete construido, pero rehusó aplicar por
las aristas `regex`/`serde`/`toml` de T3 sin commitear en `Cargo.lock`. Se
commitearon aisladas (`070d701`) preservando el churn del patch para desarrollo
local, y se repitió el bump: pin + vendor pusheados (`dde0514..6efd10c`).
Queda del lado del usuario (requiere root): stash en `port`, `nix flake update
port` en su config, `sudo nixos-rebuild switch`, stash pop.

L18 (animación del arrastre, S9): se descartó agregar una dependencia (la
física de un resorte crítico son ~40 líneas y solo el plugin puede avanzar la
animación con el reloj del render; además una dep nueva obligaría a repetir el
bump del vendor en Nix). Se agregó `src/anim.rs` con `Spring` (crítico, con
recorte de paso) y `ease_out_cubic`. La tarjeta arrastrada sigue al puntero, las
vecinas se deslizan desde su ranura anterior y la tarjeta asienta al soltar;
la sombra de levantado se arma a mano con alfa y desenfoque interpolados porque
un preset de gpui no se puede interpolar. El desplazamiento se aplica como
`mt(off)` + `mb(-off)` para no empujar a las demás fuera de su ranura, y la
tarjeta arrastrada se pinta al final para quedar por encima.

HALLAZGO (defecto preexistente, no una regresión):
`los_hooks_de_layout_leen_la_instantanea_sin_acceder_al_disco_ni_bloquear`
asertaba `left_sidebar_width() > 0` después de borrar el directorio de
presencia. Pero `presence::read_live_sessions` devuelve vacío cuando
`read_dir` falla, así que el vigilante vacía la instantánea en su propio hilo
en cuanto el directorio desaparece: la aserción dependía de que el sondeo no
cayera dentro del bucle. Medido: baseline (sin estos cambios) 4/4 en verde;
con los cambios, 2/3. Se reescribió la aserción para comparar la instantánea
alrededor de cada llamada (si tenía sesiones antes y después, la barra debe
pintarse), que es determinista y prueba lo mismo: que la barra sigue a la
memoria y no al disco. Después: 5/5 corridas en verde.
