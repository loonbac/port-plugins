# Cliente móvil (PWA) de PORT

Llevar la misma interfaz de PORT al iPhone: la terminal, y encima el modelo de
espacios y pestañas con sus marcadores de estado de agente, con animaciones
completas.

Decisión de arquitectura: PORT sigue siendo una terminal rápida en Rust. El
cliente móvil **no** se mete en el núcleo: es un plugin que sirve una PWA, y el
núcleo sólo gana capacidades genéricas que hoy no tiene.

## Specs

S1. «seria bueno un pwa creo yo ah, y tendria sistema de spaces y pestanas asi
con full animaciones».

S2. «como segunda tarea quiero esta misma interfaz en mi telefono iphone».

S3. PORT sigue siendo una terminal rápida hecha en Rust y customizable al 100%
mediante plugins: nada específico del móvil entra en el núcleo.

S4. La PWA es la primera entrega; no requiere App Store ni Xcode.

S5. «y saldrian los spaces que tengo abiertos en mi terminal actual de port
obvio»: el cliente muestra **las sesiones vivas** de la instancia de PORT que ya
está corriendo, no una terminal nueva ni un conjunto de espacios propio.

## Lo que ya existe (verificado)

- El núcleo calcula la rejilla: `Session::frame()` con filas y tramos de texto.
- El núcleo conoce el programa en primer plano, el título OSC, el progreso y el
  fondo de la rejilla (`program_status`, `osc_title`, `osc_progress`,
  `screen_tail`, `output_seq`).
- El estado de agente es público y aditivo: `AgentState` y
  `SpaceHook::update_session_agent`.
- Un plugin ya puede pedir que se lance un panel con su entorno
  (`PtyConfig.env`, T1 de la intercepción).
- El modelo de espacios y pestañas vive en el plugin herdr
  (`HerdrState::spaces`), con marcadores y agregación por espacio.

## Lo que falta en el núcleo (genérico, sin saber de móviles)

La API de plugins **no expone hoy leer la rejilla ni escribir entrada**, así que
un cliente remoto no puede existir todavía. Hace falta una capacidad genérica,
del mismo tipo que `PtyConfig.env`:

- leer la rejilla de una sesión (filas y tramos) y su revisión, para enviar sólo
  lo que cambió;
- escribir entrada en una sesión y redimensionarla;
- conocer la sesión activa y el cambio de sesión activa;
- publicar el cambio de estado del agente (ya existe) y el modelo de espacios y
  pestañas (ya existe dentro del plugin).

Nada de esto menciona HTTP, WebSocket ni iOS: son primitivas de terminal.

## Arquitectura

```text
iPhone (Safari / PWA instalada)
        │  WebSocket (frame diffs, entrada, modelo de layout)
        ▼
plugin `port-plugin-mobile`  ──►  API genérica de sesiones del núcleo
        │  HTTP estático (la propia PWA)
        ▼
PORT (Rust, gpui) — sigue siendo el mismo binario rápido
```

El plugin hace tres cosas: sirve los archivos de la PWA, mantiene el WebSocket
y traduce entre el modelo del núcleo y el protocolo del cliente. Vive en su hilo
de fondo, nunca bloquea un hook.

## Sesiones compartidas: una sola instancia viva

El móvil no arranca nada: se adjunta al PORT que ya está abierto y muestra sus
mismos espacios, pestañas y sesiones. El estado sigue viviendo en un solo lugar
(el proceso de PORT), y cada cliente es una vista más de ese estado; abrir el
móvil no duplica ni reinicia una terminal.

Consecuencia que hay que decidir de forma explícita: **el foco**. Si el foco
fuera único, tocar una pestaña en el teléfono cambiaría la pestaña del
escritorio. La recomendación es lo contrario: las sesiones se comparten, pero
cada cliente tiene su propio foco (su propia pestaña activa para mirar). Así el
móvil sirve justamente para lo que se pidió: ver desde el teléfono qué agente
quedó «espera tu respuesta» y atenderlo, sin mover lo que el escritorio está
haciendo.

Como las sesiones son las mismas, el agente que corre en un panel es literalmente
el mismo proceso: atenderlo desde el móvil es atenderlo, no una copia.

## Protocolo (borrador)

- `hello` → el servidor manda `layout` completo: espacios, pestañas, sesión
  activa, tamaño de rejilla y estado de agente por sesión.
- `frame` → diffs por fila con número de revisión; el cliente descarta lo
  antiguo. Es lo que hace que se sienta rápido: se envía lo que cambió.
- `layout` → cambios de espacios, pestañas, activo y estados de agente (para
  pintar `working`, «espera tu respuesta» y `listo`).
- `input` → teclas, pegado y `resize`.
- `attention` → aviso cuando un agente queda bloqueado: badging, sonido y aviso
  del sistema.

## Interfaz: el mismo modelo con animaciones completas

- **Barra lateral de espacios**: las mismas tarjetas, con el mismo reordenamiento
  por arrastre. El resorte crítico ya está escrito y probado en Rust
  (`anim::Spring`); se porta a TypeScript con las mismas constantes para que el
  gesto se sienta idéntico. En iOS se suma el gesto de deslizar entre espacios,
  que es el mismo movimiento con otra entrada.
- **Pestañas**: las mismas fichas, con los mismos marcadores y colores
  (`● working`, `● espera tu respuesta`, `✓ listo`).
- **Animaciones**: spinner braille, parpadeo de aviso y levantado de la tarjeta
  arrastrada, todos por reloj de render igual que en el escritorio. Sólo
  `transform` y `opacity` para que compongan en la GPU y sostengan 120 Hz.
- **Respeta `prefers-reduced-motion`**: sin movimiento, pero con los mismos
  estados.
- **Áreas seguras y teclado**: `env(safe-area-inset-*)` y `visualViewport` para
  que el teclado no tape la línea activa.

## Seguridad (no negociable)

Una terminal es ejecución remota de código por diseño, así que:

- Escuchar sólo en la interfaz de la VPN (Tailscale o WireGuard) o en
  `localhost` detrás de un túnel SSH. **Nunca** expuesto a Internet.
- Token por dispositivo, comparado en tiempo constante; sesión que caduca.
- TLS propio o, mejor, confiar en la VPN y no publicar el puerto.
- Sin tokens ni contenido de la terminal en los registros.

## Fases

- F1 · Espejo de sólo lectura: capacidad genérica de rejilla en el núcleo,
  servidor del plugin, `hello` + `frame` + `layout`, token y VPN. Se comprueba
  en Safari del iPhone.
- F2 · Entrada: teclado, pegado, `resize` y barra de accesorios.
- F3 · PWA: manifiesto, service worker, iconos, instalación en la pantalla de
  inicio, `wake lock`, reconexión.
- F4 · El modelo de la interfaz: cajón de espacios con arrastre y resortes,
  fichas de pestañas, marcadores de agente y aviso de atención.
- F5 · Pulido: movimiento reducido, varias sesiones a la vez, temas y tipografía
  sincronizados, indicador de desconexión.

## Tasks

- M1 · S3 · port · capacidad genérica: leer la rejilla de una sesión, escribir
  entrada y redimensionar, más la sesión activa — pendiente.
- M2 · S3 · port-plugins · plugin móvil: servidor HTTP + WebSocket, `hello`,
  diffs de `frame` y `layout` — pendiente.
- M3 · S4 · port-plugins · PWA: shell, manifiesto, service worker, iconos —
  pendiente.
- M4 · S1 · port-plugins · interfaz: cajón de espacios con arrastre y resortes
  portados, fichas de pestañas, marcadores y animaciones — pendiente.
- M5 · S1 · port-plugins · entrada y teclado en iOS, áreas seguras — pendiente.
- M6 · S2 · seguridad: token por dispositivo, enlace a la VPN, caducidad —
  pendiente.
- M7 · pruebas: protocolo, diffs, reconexión y el portado del resorte —
  pendiente.

## Log

L1 (pedido verbatim, S1): «seria bueno un pwa creo yo ah, y tendria sistema de
spaces y pestanas asi con full animaciones».

L2 (pedido verbatim, S2): «como segunda tarea quiero esta misma interfaz en mi
telefono iphone como podriamos hacerlo?, explicame y armame un plan».

L3 (restricción del usuario, S3): «siempre que respete la vision de que port es
una terminal rapida hecha en rust customizable para poder cargar plugins y
customizarlo al 100».

L4 (hallazgo que define la fase 1): la API de plugins no expone hoy ni leer la
rejilla ni escribir entrada, así que M1 es un requisito previo y no un detalle
de implementación. Es la misma clase de capacidad genérica que `PtyConfig.env`.

L5 (cierre de M2, verificado): arranque del plugin móvil (`MobilePlugin::start()`):
token por dispositivo en `~/.config/port/mobile-token` (0600, 32 bytes, se regenera
si está roto, override `PORT_MOBILE_TOKEN`); assets en `PORT_MOBILE_ASSETS` o
`<exe>/../share/port/mobile`, cerrado sin directorio; registro en PORT (`main.rs`).
43 pruebas + clippy + fmt en verde, `cargo build -p port` en verde (todo bajo nix-shell).
