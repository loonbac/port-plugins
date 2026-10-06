# PORT Plugins · Tienda

[English](README.md)

Tienda de plugins de [PORT](https://github.com/loonbac/port), un emulador de
terminal orientado a plugins escrito en Rust sobre GPUI.

Este repositorio guarda dos clases de plugins:

- `plugins/<id>/` — plugins de **tienda**. Cada uno es un binario independiente
  que solo enlaza el SDK; `port plugin add` lo instala sin recompilar PORT.
- `in-process/<id>/` — plugins que usan features en proceso (widgets GPUI, hooks
  en caliente, el protocolo de cierre). PORT debe recompilarse con ellos; no se
  instalan desde la tienda.

## Store · Tienda

### Instalar un plugin

```bash
# Desde el repositorio (la forma de tienda).
port plugin add https://github.com/loonbac/port-plugins/plugins/font-zoom

# Desde un checkout local, mientras se desarrolla.
port plugin add /ruta/port-plugins/plugins/font-zoom
```

La primera instalación compila el plugin en la máquina anfitriona
(`cargo build --release`), así que necesita un toolchain de Rust. Instalarlo de
nuevo lo recompila y lo reemplaza: actualizar y reinstalar son la misma
operación.

Los plugins instalados viven en `~/.local/share/port/plugins/<id>`, cada uno
junto a un manifiesto que guarda su origen y las capacidades que anunció.

Para activar o desactivar un plugin, abre el menú de plugins de PORT con
`Ctrl+Shift+L` y cámbialo ahí.

### Catálogo

| id | Nombre | Qué hace | Tipo | Capacidades | Instalación |
|---|---|---|---|---|---|
| `transparency` | Transparency | Opacidad del fondo de la ventana para transparencias a nivel de compositor en Wayland. | tienda | `appearance` | `port plugin add …/plugins/transparency` |
| `font` | Font Configuration | Familia, tamaño y lista de fuentes de respaldo. | tienda | `appearance` | `port plugin add …/plugins/font` |
| `font-zoom` | Font Zoom | Es dueño del tamaño de fuente vigente y de sus atajos (`Ctrl+=`, `Ctrl+-`, `Ctrl+0`). | tienda | `appearance`, `input` | `port plugin add …/plugins/font-zoom` |
| `herdr` | Herdr Customization Plugin | Gestor de espacios: barra lateral ajustable, barra de pestañas, detección en vivo del programa y nombre a partir del directorio actual y la rama de Git. | en proceso | `AppearanceHook`, `InputHook`, `LayoutHook`, `SpaceHook` | — (recompilar PORT) |
| `shortcuts` | Custom Shortcuts | Asocia combinaciones de teclas a callbacks y a servicios publicados por otros plugins. | en proceso | `InputHook` | — (recompilar PORT) |
| `close-guard` | Close Guard | Pide confirmación antes de cerrar la ventana si hay programas en ejecución. | en proceso | `LifecycleHook` | — (recompilar PORT) |
| `selection` | Selection | Es dueño de la política de ratón y selección: reenvía clics, arrastres y movimiento al programa, Shift devuelve el gesto a PORT, y controla la copia al soltar, el doble y triple clic y el color del resaltado. | en proceso | `MouseHook` | — (recompilar PORT) |

Para los plugins de tienda la URL de instalación es
`https://github.com/loonbac/port-plugins/plugins/<id>`.

## Por qué algunos plugins van en proceso

La frontera de la tienda es el protocolo JSON Lines, y no todo cabe por ahí:

- **`herdr` dibuja elementos GPUI.** `LayoutHook` devuelve un `AnyElement`, que
  no tiene forma serializada, así que el layout se queda dentro del proceso.
- **`shortcuts` es un intermediario de servicios.** Resuelve al pulsar la tecla
  servicios publicados por otros plugins, y esos servicios son objetos de trait
  de Rust: no cruzan la frontera del proceso.
- **`close-guard` necesita una ida y vuelta de confirmación de cierre.** En el
  protocolo el ciclo de vida es solo `Shutdown`, una notificación, así que el
  núcleo no puede preguntar a un plugin si la ventana puede cerrarse.

## Referencia de plugins

### `font-zoom`

Es dueño del tamaño de fuente como estado y nada más: informa el tamaño actual
por la capacidad `appearance` y lo maneja por `input`. Los atajos son suyos, así
que el plugin funciona aunque `shortcuts` no esté cargado.

| Atajo | Acción |
|---|---|
| `Ctrl` `=` | Aumentar fuente |
| `Ctrl` `-` | Reducir fuente |
| `Ctrl` `0` | Reiniciar tamaño de fuente |

Su bloque de configuración acepta las mismas claves de tamaño que antes más las
tres plantillas de atajo:

```font-zoom
default_size = 14
step = 1.0
min_size = 6.0
max_size = 72.0
key_zoom_in = ctrl+=
key_zoom_out = ctrl+-
key_reset = ctrl+0
```

### `shortcuts`

Asocia combinaciones arbitrarias de teclas a callbacks, con una sintaxis compacta
tipo `ctrl+shift+t`. Aquí viven los atajos en proceso personalizados. Un atajo
también puede llamar a un servicio publicado por otro plugin, resuelto al pulsar
la tecla:

```rust
use port_plugin_shortcuts::ShortcutsPlugin;

let shortcuts = ShortcutsPlugin::new();

// Su propio atajo.
shortcuts.bind_str("ctrl+shift+k", || println!("¡Hola!"));

// Servicio publicado por otro plugin: sin tipos compartidos, se resuelve al
// pulsar la tecla.
shortcuts.bind_service("ctrl+shift+p", "some-plugin", "some_action");
```

### `herdr`

El gestor de espacios de trabajo: barra lateral de espacios ajustable, barra de
pestañas, detección en vivo del programa en ejecución y nombre automático a partir
del directorio actual y la rama de Git.

| Atajo | Acción |
|---|---|
| `Ctrl` `Shift` `T` | Nueva pestaña |
| `Alt` `←` / `Alt` `→` | Cambiar de pestaña |
| `Ctrl` `W` | Cerrar pestaña |
| `Ctrl` `Alt` `T` | Nuevo espacio |
| `Alt` `1`..`9` | Cambiar de espacio |

Comportamiento destacado:

- Arranca como terminal pelada: sin barra lateral, sin barra de pestañas, a ancho
  completo.
- Las pestañas se etiquetan con el icono Nerd Font del programa en ejecución
  (`pi`, `codex`, `claude`, `antigravity`, `opencode`, `gemini`, `btop`, …).
- Los espacios adoptan el nombre de la carpeta actual y la rama de Git activa.
- La barra lateral y la barra de pestañas comparten el fondo y la opacidad de la
  terminal, y toman el color de acento extraído del wallpaper.
- El borde de la barra lateral se puede arrastrar para cambiar su ancho.

### `close-guard`

Pide confirmación antes de cerrar la ventana si hay programas en ejecución, con
soporte de teclado y ratón.

| Tecla | Acción |
|---|---|
| `←` `→` | Mover el foco |
| `Enter` | Confirmar |
| `n` / `Esc` | Seguir en la terminal |
| `y` | Cerrar de todos modos |

## Hooks

Los plugins en proceso implementan uno o más traits de `port-plugin-api`:

| Hook | Propósito |
|---|---|
| `AppearanceHook` | Opacidad, colores, fuentes |
| `InputHook` | Interceptar y consumir teclas |
| `LayoutHook` | Inyectar barra superior, lateral o de estado |
| `SpaceHook` | Crear, seleccionar y seguir sesiones/pestañas/espacios |
| `LifecycleHook` | Vetar el cierre de la ventana |

Los plugins de tienda no enlazan GPUI ni `port-plugin-api`; declaran los valores
equivalentes por el protocolo del SDK. Cada plugin puede además aportar
`default_config`, `load_config` y `save_config` para persistir sus ajustes en
`~/.config/port/config.md`.

## Desarrollo

```bash
nix-shell                    # librerías nativas enlazadas
cargo test --workspace       # pruebas unitarias y de protocolo
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

### Escribir tu propio plugin

Para un plugin de tienda, crea un crate que dependa solo de `port-plugin-sdk`,
añade un `[[bin]]` y una sección `[package.metadata.port]` con el `id` y las
`capabilities`, e implementa `port_plugin_sdk::runtime::Plugin`. PORT lo instala
con `port plugin add`, lo compila una vez y habla con él por el protocolo.

Para un plugin en proceso, añade `port-plugin-api` como dependencia, implementa
`Plugin` y los hooks que necesites, y agrega el crate a `in-process/*` en el
workspace. PORT debe recompilarse con él; el registro se encarga entonces de la
configuración, la activación y la recarga en caliente.

## Licencia

MIT
