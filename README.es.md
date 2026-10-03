# PORT Plugins · Plugins de PORT

[English](README.md)

Plugins oficiales y comunitarios de
[PORT](https://github.com/loonbac/port), un emulador de terminal orientado a
plugins escrito en Rust sobre GPUI.

Los plugins viven en su propio repositorio para que el núcleo de la terminal y
sus extensiones evolucionen de forma independiente. Un plugin es un crate de Rust
en el mismo proceso que depende de `port-plugin-api` e implementa uno o más
hooks.

## Plugins disponibles

### `transparency`

Opacidad del fondo de la ventana para transparencias a nivel de compositor en
Wayland.

```rust
use port_plugin_transparency::TransparencyPlugin;
registry.register(TransparencyPlugin::default()); // 85 %
```

### `font`

Familia, tamaño y lista de fuentes de respaldo, declarados como configuración y
no como valores fijos.

```rust
use port_plugin_font::FontPlugin;
registry.register(FontPlugin::new("FiraCode Nerd Font Mono"));
```

### `font-zoom`

Es dueño del tamaño de fuente como estado, y nada más: informa del tamaño actual
mediante `AppearanceHook` y expone `zoom_in()`, `zoom_out()` y `reset_zoom()`.

Deliberadamente **no registra ningún atajo**. Enlazar esas acciones a teclas es
trabajo del plugin `shortcuts`, así el zoom se puede asociar a otras teclas, o
dejarlo sin atajo, sin tocar este plugin.

Expone esa capacidad como servicio para que otros plugins puedan manejarla:

| Servicio | Acción | Devuelve |
|---|---|---|
| `font-zoom` | `zoom_in` | Tamaño nuevo |
| `font-zoom` | `zoom_out` | Tamaño nuevo |
| `font-zoom` | `reset` | Tamaño base |
| `font-zoom` | `size` | Tamaño actual |

### `shortcuts`

Asocia combinaciones arbitrarias de teclas a callbacks, con una sintaxis compacta
tipo `ctrl+shift+t`. Aquí viven todos los atajos de PORT, incluido el zoom:

```rust
use port_plugin_shortcuts::ShortcutsPlugin;

let shortcuts = ShortcutsPlugin::new();

// Su propio atajo.
shortcuts.bind_str("ctrl+shift+k", || println!("¡Hola!"));

// Manejando el servicio publicado por otro plugin: sin tipos compartidos,
// se resuelve al pulsar la tecla.
shortcuts.bind_service("ctrl+=", "font-zoom", "zoom_in");
shortcuts.bind_service("ctrl+-", "font-zoom", "zoom_out");
shortcuts.bind_service("ctrl+0", "font-zoom", "reset");
```

Los bindings que usa la configuración instalada son:

| Atajo | Acción |
|---|---|
| `Ctrl` `+` / `Ctrl` `=` | Aumentar fuente |
| `Ctrl` `+` `Shift` / `Ctrl` `=` `Shift` | Aumentar fuente |
| `Ctrl` `-` | Reducir fuente |
| `Ctrl` `0` | Reiniciar tamaño de fuente |

### `menu-customizer`

Parchea el gestor de plugins del núcleo: su atajo, su título o todo el
renderizado del panel.

```rust
use port_plugin_menu_customizer::MenuCustomizerPlugin;
registry.register(
    MenuCustomizerPlugin::default()
        .with_shortcut("ctrl+shift+p")
        .with_title("Extensiones")
        .with_custom_theme(true),
);
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

Los plugins implementan uno o más traits de `port-plugin-api`:

| Hook | Propósito |
|---|---|
| `AppearanceHook` | Opacidad, colores, fuentes |
| `InputHook` | Interceptar y consumir teclas |
| `LayoutHook` | Inyectar barra superior, lateral o de estado |
| `SpaceHook` | Crear, seleccionar y seguir sesiones/pestañas/espacios |
| `PluginManagerHook` | Reemplazar la UI del gestor de plugins |
| `LifecycleHook` | Vetar el cierre de la ventana |

Cada plugin puede además aportar `default_config`, `load_config` y `save_config`
para persistir sus ajustes en `~/.config/port/config.md`.

## Usar plugins

Añade el plugin a tu `Cargo.toml` y regístralo:

```toml
[dependencies]
port-plugin-herdr = { git = "https://github.com/loonbac/port-plugins.git" }
```

```rust
use port_plugin_herdr::HerdrPlugin;

fn main() {
    let mut registry = PluginRegistry::new();
    registry.register(HerdrPlugin::new());
}
```

## Desarrollo

```bash
nix-shell      # librerías nativas enlazadas
cargo test     # 39 tests unitarios
cargo build --release
```

### Escribir tu propio plugin

Crea un crate con `port-plugin-api` como dependencia, implementa `Plugin` y los
hooks que necesites, y añádelo al workspace en `Cargo.toml`. El plugin se
registra desde la aplicación anfitriona; el registro se encarga de la
configuración, la activación y la recarga en caliente.

## Licencia

MIT
