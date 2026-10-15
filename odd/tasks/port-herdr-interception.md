# Intercepción del contrato de herdr en PORT

PORT se hace pasar por herdr para las aplicaciones que corren dentro de sus
paneles: publica las mismas variables de entorno, mantiene un socket real y
ofrece un ejecutable compatible, de modo que las llamadas que `pi` ya hace a
`herdr` lleguen a PORT y no a un herdr instalado. Con eso el estado del agente
(`blocked`) viaja por el mismo canal que el resto de la actividad y el sidebar
puede marcar «espera tu respuesta».

Origen: el sidebar mostró `working` mientras `pi` esperaba una respuesta, aunque
el aviso de escritorio sí llegó.

## Specs

S1. «no vamos a usar herdr tenemos que interceptar las llamadas que haria a
herdr para tomarlas nosotros eso queremos».

S2. «mira la notificacion me aviso pero en la ui de port no me salio nada seguia
port plugins en working, revisa eso».

S3. El sidebar debe distinguir `working` de «espera tu respuesta» para un agente
`pi` que corre en un panel de PORT, sin depender de que haya un herdr instalado.

S4. Interceptar no debe exigir modificar la forma en que `pi` ya habla con
herdr: PORT se adapta al contrato existente. Cualquier dato que PORT necesite y
que hoy no viaje por ese contrato se agrega desde el lado que produce el dato.

S5. Sin I/O bloqueante ni de disco dentro de los hooks de layout; el trabajo
pesado vive en hilos propios o en el `pump()` del núcleo.

## Contrato a interceptar (evidencia de `herdr-ref`, sólo lectura)

Lo que `pi` consume hoy (`gentle-pi/lib/herdr-activity.ts`,
`extensions/gentle-herdr-activity.ts`):

- C1. Se activa sólo si `HERDR_ENV === "1"` y `HERDR_PANE_ID` y
  `HERDR_SOCKET_PATH` tienen contenido; si no, retorna sin hacer nada.
- C2. Exige que `statSync(HERDR_SOCKET_PATH).isSocket()` sea verdadero.
- C3. Escribe con `execFile(HERDR_BIN_PATH ?? "herdr", ["pane",
  "report-metadata", pane, "--source", "gentle:activity", "--agent", "pi",
  "--seq", N, "--ttl-ms", "30000", ...tokens])`, con `--token summary=<línea>` /
  `--token summary2=<línea>` o `--clear-token`.
- C4. **Ignora por completo la salida y el código de salida** de esa llamada.
- C5. Lee `dirname(HERDR_SOCKET_PATH)/session.json` cada 5 s como máximo y usa
  `sidebar_width` sólo si es un entero entre 6 y 512, como `width - 5` columnas.
- C6. Recorta cada token a 78 caracteres / 256 bytes porque herdr corta los
  valores en 80 caracteres.

Lo que herdr provee (a replicar):

- C7. Variables en el panel: `HERDR_ENV=1`, `HERDR_SOCKET_PATH`,
  `HERDR_BIN_PATH`, y en paneles gestionados `HERDR_WORKSPACE_ID` (`w<N>`),
  `HERDR_TAB_ID` (`<ws>:t<N>`), `HERDR_PANE_ID` (`<ws>:p<N>`).
- C8. Socket: `$XDG_CONFIG_HOME/herdr/herdr.sock`, modo `0o600`, JSON delimitado
  por saltos de línea, **una petición por conexión**, máximo 1 MiB, UTF-8.
- C9. Trama: `{"id":"cli:request","method":"pane.report_metadata","params":{…}}\n`
  y respuesta `{"id":…,"result":{"type":"ok"}}` o
  `{"id":…,"error":{"code":…,"message":…}}`.
- C10. Parámetros: `pane_id`, `source` (1–80 caracteres de `[A-Za-z0-9:._-]`),
  opcionales `agent`, `title`, `display_agent`, `state_labels`, `tokens`
  (`{clave: valor|null}`), `clear_*`, `seq: u64?`, `ttl_ms: u64?`.
- C11. Límites: 16 claves por petición, clave de 1–32 de `[A-Za-z0-9_-]`, valor
  recortado a 80 caracteres, `ttl_ms` entre 1 y 86 400 000, 32 tokens por panel.
- C12. `seq` es por par (panel, fuente): sólo se acepta si `seq > último`.
- C13. herdr escribe `session.json` con `sidebar_width: null` siempre: el ancho
  real vive en el cliente. PORT puede hacerlo mejor y publicar el ancho real.

## Tasks

- T1 · S4 · port (núcleo) · `PtyConfig.env` aditivo y aplicado al proceso
  lanzado; sin él no hay forma de inyectar las variables de C7 — pendiente.
- T2 · C7 · port-plugins · acuñar identidad de panel por sesión y armar el
  entorno de C7 en `take_spawn_session_request`; recordar panel → sesión —
  pendiente.
- T3 · C8 · C9 · C10 · C11 · C12 · port-plugins · socket real propio (modo 0600,
  una petición por conexión, tope de 1 MiB) con hilo oyente y manejador de
  `pane.report_metadata` que valide como herdr y vuelque en `HerdrState` —
  pendiente.
- T4 · C3 · C4 · port-plugins · ejecutable compatible `herdr` que traduzca
  `pane report-metadata …` a la trama del socket y respete el contrato de salida
  (0 éxito, 1 error de API, 2 uso) — pendiente.
- T5 · C5 · C13 · port-plugins · escribir `session.json` junto al socket con el
  `sidebar_width` real de PORT — pendiente.
- T6 · S4 · ~/.nixos/pkgs/pi · publicar el estado del agente por el contrato
  interceptado (reutilizando el evento `herdr:blocked` que ya existe y hoy sólo
  consume el servicio de notificaciones) — pendiente.
- T7 · S3 · port-plugins · mapear el estado interceptado a `AgentState`
  (`SpaceHook::update_session_agent` ya existe) y cubrirlo con pruebas —
  pendiente.
- T8 · S5 · port-plugins · pruebas (socket, validación, shim, mapeo) y
  documentación de la superficie — pendiente.

## Diseño

Cada panel de PORT es una sesión PTY y recibe una identidad `w<ws>:p<n>`. Al
lanzar el shell, el plugin arma el entorno de C7 apuntando al socket propio de
la instancia y al ejecutable compatible, así la extensión de `pi` se activa y
escribe contra PORT.

El ejecutable compatible es un cliente delgado, igual que el de herdr: conecta
al socket y manda una petición. El socket lo escucha un hilo del plugin, que
valida con los límites de C11 y C12 y vuelca el resultado en el estado que ya
alimenta el sidebar.

El estado del agente viaja como token reservado (`port_state`) en la misma
petición `pane.report_metadata`, en vez de agregar un método propio: así el
contrato interceptado sigue siendo el de herdr y no hay que inventar superficie.

## Log

L1 (pedido verbatim, S1): «no vamos a usar herdr tenemos que interceptar las
llamadas que haria a herdr para tomarlas nosotros eso queremos».

L2 (síntoma verbatim, S2): «mira la notificacion me aviso pero en la ui de port
no me salio nada seguia port plugins en working, revisa eso».

L3 (diagnóstico): el estado `Blocked` es inalcanzable para `pi` hoy.
`crates/port-app/assets/agent-detection/pi.toml` tiene 27 líneas y sólo dos
reglas `working`: es el único manifiesto sin regla `blocked` (claude 10, grok 9,
codex 6, antigravity 4, amp 3, cursor 2, droid 2, gemini 1, opencode 1).
`rg 7501` en todo gentle-pi no encuentra nada: nadie emite la señal OSC que el
núcleo sí parsea. El aviso que sí llegó sale de
`gentle-pi/lib/notification-events.ts` (`NOTIFICATION_ATTENTION_EVENT =
"herdr:blocked"`), producido por preguntas y confirmaciones y unificado en
`gentle-ai.ts`; su único consumidor es el servicio de notificaciones. Y PORT no
implementa ninguna parte de la superficie de herdr: no hay `HERDR_*` en
`port/crates` ni en `port-plugins`, no hay sockets, y `herdr` no está instalado.

L4 (decisión del usuario, S1): PORT intercepta el contrato en vez de usar herdr
ni de inventar una señal propia.

L5 (mapa del contrato, C1–C13): exploración delegada sobre
`/home/loonbac/Proyectos/herdr-ref`. Hallazgos que cambian el diseño: el
consumidor **ignora la salida y el código de salida** (C4), exige un socket real
aunque sólo lo use como portero (C2), y herdr publica `sidebar_width: null` de
modo que el ancho real nunca llega a la aplicación (C13). También que
`PtyConfig` no tiene `env`: hace falta T1.

L6 (T1 hecho y verificado): `PtyConfig.env` se aplica después de `TERM`,
`COLORTERM` y `TERM_PROGRAM`, así quien lanza un panel puede también
reemplazarlos. La prueba `el_entorno_extra_llega_al_proceso` corre sobre un
shell y un PTY reales y se comprobó que muerde: sin el bucle de aplicación
falla, restaurado pasa. Suites de `port-term-core` y `port-plugin-api` en verde,
clippy limpio. Commit `103bf6d`.

L7 (dos aristas de T2 que reveló el código):

1. El shell por defecto no pasa por ningún `PtyConfig` que el plugin controle:
el anfitrión llama `session_mgr.spawn_session()`, que clona el `default_config`
del gestor. Y ese `default_config` sí puede venir de la configuración del
usuario (otro shell, otros argumentos), así que hacer que el plugin devuelva su
propio `PtyConfig::default()` lo reemplazaría en silencio: una regresión contra
«customizable al 100%». Hace falta que el entorno del plugin se **sume** al
config por defecto sin sustituirlo (T2a). `spawn_session` y
`spawn_session_with` comparten `spawn_session_config`, así que la única
diferencia es cuál config se usa.

2. `HERDR_PANE_ID` tiene que existir **antes** de lanzar el proceso, porque el
entorno se fija en el spawn, pero el id de sesión lo asigna el gestor después.
El plugin ya sabe qué petición está en vuelo (`InFlight::Shell(tab_id)`), así que
puede acuñar el id de panel en ese momento, devolverlo en el entorno y enlazar
panel → sesión al recibir `on_session_created`. El id de panel no necesita ser el
id de sesión.
