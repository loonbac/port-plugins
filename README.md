# PORT Plugins · Plugins de PORT

Official and community plugins for [PORT](https://github.com/loonbac/port), a
plugin-oriented terminal emulator written in Rust on GPUI.

Plugins live in their own repository so the terminal core and its extensions can
evolve independently. A plugin is a normal in-process Rust crate that depends on
`port-plugin-api` and implements one or more hooks.

Los plugins oficiales y comunitarios de
[PORT](https://github.com/loonbac/port), un emulador de terminal orientado a
plugins escrito en Rust sobre GPUI.

Los plugins viven en su propio repositorio para que el núcleo de la terminal y
sus extensiones evolucionen de forma independiente. Un plugin es un crate de Rust
en el mismo proceso que depende de `port-plugin-api` e implementa uno o más hooks.

---

## Available plugins · Plugins disponibles

### `transparency`

Window background opacity for compositor-level transparency on Wayland.

Opacidad del fondo de la ventana para transparencias a nivel de compositor.

```rust
use port_plugin_transparency::TransparencyPlugin;
registry.register(TransparencyPlugin::default()); // 85 %
```

### `font`

Font family, size and fallback list, declared as configuration rather than
hard-coded.

Familia, tamaño y lista de fuentes de respaldo, declarados como configuración y
no como valores fijos.

```rust
use port_plugin_font::FontPlugin;
registry.register(FontPlugin::new("FiraCode Nerd Font Mono"));
```

### `font-zoom`

Interactive zoom bound to the usual keys, recalculating cell geometry and
resizing the PTY live.

Zoom interactivo con los atajos habituales, recalculando la geometría de celda y
redimensionando el PTY en vivo.

| Shortcut · Atajo | Action · Acción |
|---|---|
| `Ctrl` `+` | Zoom in · Aumentar |
| `Ctrl` `-` | Zoom out · Reducir |
| `Ctrl` `0` | Reset · Reiniciar |

### `shortcuts`

Bind arbitrary key combinations to callbacks, with a compact `ctrl+shift+t`
syntax.

Asocia combinaciones arbitrarias de teclas a callbacks, con una sintaxis compacta
tipo `ctrl+shift+t`.

```rust
use port_plugin_shortcuts::ShortcutsPlugin;
let shortcuts = ShortcutsPlugin::new();
shortcuts.bind_str("ctrl+shift+k", || println!("¡Hola!"));
```

### `menu-customizer`

Patches the core plugin manager: its shortcut, its title, or the entire
rendering of the panel.

Parchea el gestor de plugins del núcleo: su atajo, su título o todo el
renderizado del panel.

```rust
use port_plugin_menu_customizer::MenuCustomizerPlugin;
registry.register(
    MenuCustomizerPlugin::default()
        .with_shortcut("ctrl+shift+p")
        .with_title("Extensions")
        .with_custom_theme(true),
);
```

### `herdr`

The workspace manager: a resizable sidebar of spaces, a tab bar, live detection of
the running program, and automatic naming from the current directory and Git
branch.

El gestor de espacios de trabajo: barra lateral de espacios ajustable, barra de
pestañas, detección en vivo del programa en ejecución y nombre automático a partir
del directorio actual y la rama de Git.

| Shortcut · Atajo | Action · Acción |
|---|---|
| `Ctrl` `Shift` `T` | New tab · Nueva pestaña |
| `Alt` `←` / `Alt` `→` | Switch tab · Cambiar de pestaña |
| `Ctrl` `W` | Close tab · Cerrar pestaña |
| `Ctrl` `Alt` `T` | New space · Nuevo espacio |
| `Alt` `1`..`9` | Switch space · Cambiar de espacio |

Notable behaviour:

- Starts as a bare terminal: no sidebar, no tab bar, full window width.
- Tabs are labelled with the Nerd Font icon of the running program (`pi`, `codex`,
  `claude`, `antigravity`, `opencode`, `gemini`, `btop`, …).
- Spaces adopt the name of the folder you are in and the active Git branch.
- Sidebar and tab bar share the terminal background and opacity, and pick up the
  accent colour extracted from your wallpaper.

Comportamiento destacado:

- Arranca como terminal pelada: sin barra lateral, sin barra de pestañas, a ancho
  completo.
- Las pestañas se etiquetan con el icono Nerd Font del programa en ejecución
  (`pi`, `codex`, `claude`, `antigravity`, `opencode`, `gemini`, `btop`, …).
- Los espacios adoptan el nombre de la carpeta actual y la rama de Git activa.
- La barra lateral y la barra de pestañas comparten el fondo y la opacidad de la
  terminal, y toman el color de acento extraído del wallpaper.

### `close-guard`

Asks for confirmation before closing the window when programs are running, with
keyboard and mouse support.

Pide confirmación antes de cerrar la ventana si hay programas en ejecución, con
soporte de teclado y ratón.

| Key · Tecla | Action · Acción |
|---|---|
| `←` `→` | Move focus · Mover el foco |
| `Enter` | Confirm · Confirmar |
| `n` / `Esc` | Keep terminal · Seguir en la terminal |
| `y` | Close anyway · Cerrar de todos modos |

---

## Hooks · Hooks

Plugins implement one or more traits from `port-plugin-api`:

| Hook | Purpose · Propósito |
|---|---|
| `AppearanceHook` | Opacity, colours, fonts · Opacidad, colores, fuentes |
| `InputHook` | Intercept and consume keys · Interceptar y consumir teclas |
| `LayoutHook` | Inject top bar, sidebar, status bar · Inyectar barras |
| `SpaceHook` | Create, select and track sessions/tabs/spaces · Crear y seguir sesiones |
| `PluginManagerHook` | Replace the plugin manager UI · Reemplazar la UI del gestor |
| `LifecycleHook` | Veto the window close · Vetar el cierre de la ventana |

Every plugin may also provide `default_config`, `load_config` and `save_config`
to persist settings in `~/.config/port/config.md`.

Cada plugin puede además aportar `default_config`, `load_config` y `save_config`
para persistir sus ajustes en `~/.config/port/config.md`.

---

## Using plugins · Usar plugins

Add the plugin to your `Cargo.toml`, then register it:

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

## Development · Desarrollo

```bash
nix-shell      # native libraries linked
cargo test     # unit tests for every plugin
cargo build --release
```

### Writing your own · Escribir tu propio

Create a crate with `port-plugin-api` as a dependency, implement `Plugin` plus
any hook, and add it to the workspace in `Cargo.toml`. The plugin is registered
from the host application; the registry handles configuration, enable/disable and
hot-reload for you.

Crea un crate con `port-plugin-api` como dependencia, implementa `Plugin` y los
hooks que necesites, y añádelo al workspace en `Cargo.toml`. El plugin se
registra desde la aplicación anfitriona; el registro se encarga de la
configuración, la activación y la recarga en caliente.

## License · Licencia

MIT
