# PORT Plugins

[Español](README.es.md)

Official and community plugins for [PORT](https://github.com/loonbac/port), a
plugin-oriented terminal emulator written in Rust on GPUI.

Plugins live in their own repository so the terminal core and its extensions can
evolve independently. A plugin is a normal in-process Rust crate that depends on
`port-plugin-api` and implements one or more hooks.

## Available plugins

### `transparency`

Window background opacity for compositor-level transparency on Wayland.

```rust
use port_plugin_transparency::TransparencyPlugin;
registry.register(TransparencyPlugin::default()); // 85 %
```

### `font`

Font family, size and fallback list, declared as configuration rather than
hard-coded.

```rust
use port_plugin_font::FontPlugin;
registry.register(FontPlugin::new("FiraCode Nerd Font Mono"));
```

### `font-zoom`

Owns font size as state, and nothing else: it reports the current size through
`AppearanceHook` and exposes `zoom_in()`, `zoom_out()` and `reset_zoom()`.

It deliberately registers **no key bindings**. Wiring those actions to keys is
the job of the `shortcuts` plugin, so the zoom can be bound to different keys, or
to none at all, without touching this plugin.

It exposes that capability as a service, so other plugins can drive it:

| Service · Servicio | Action · Acción | Returns · Devuelve |
|---|---|---|
| `font-zoom` | `zoom_in` | New size · Tamaño nuevo |
| `font-zoom` | `zoom_out` | New size · Tamaño nuevo |
| `font-zoom` | `reset` | Base size · Tamaño base |
| `font-zoom` | `size` | Current size · Tamaño actual |

### `shortcuts`

Bind arbitrary key combinations to callbacks, with a compact `ctrl+shift+t`
syntax. This is where every key binding in PORT lives, including the font zoom:

```rust
use port_plugin_shortcuts::ShortcutsPlugin;

let shortcuts = ShortcutsPlugin::new();

// Its own binding.
shortcuts.bind_str("ctrl+shift+k", || println!("Hello!"));

// Driving another plugin's published service: no types shared, resolved on keypress.
shortcuts.bind_service("ctrl+=", "font-zoom", "zoom_in");
shortcuts.bind_service("ctrl+-", "font-zoom", "zoom_out");
shortcuts.bind_service("ctrl+0", "font-zoom", "reset");
```

The bindings used by the shipped configuration are:

| Shortcut | Action |
|---|---|
| `Ctrl` `+` / `Ctrl` `=` | Zoom in |
| `Ctrl` `+` `Shift` / `Ctrl` `=` `Shift` | Zoom in |
| `Ctrl` `-` | Zoom out |
| `Ctrl` `0` | Reset font size |

### `menu-customizer`

Patches the core plugin manager: its shortcut, its title, or the entire
rendering of the panel.

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

| Shortcut | Action |
|---|---|
| `Ctrl` `Shift` `T` | New tab |
| `Alt` `←` / `Alt` `→` | Switch tab |
| `Ctrl` `W` | Close tab |
| `Ctrl` `Alt` `T` | New space |
| `Alt` `1`..`9` | Switch space |

Notable behaviour:

- Starts as a bare terminal: no sidebar, no tab bar, full window width.
- Tabs are labelled with the Nerd Font icon of the running program (`pi`, `codex`,
  `claude`, `antigravity`, `opencode`, `gemini`, `btop`, …).
- Spaces adopt the name of the folder you are in and the active Git branch.
- Sidebar and tab bar share the terminal background and opacity, and pick up the
  accent colour extracted from your wallpaper.
- The sidebar edge is draggable to resize it.

### `close-guard`

Asks for confirmation before closing the window when programs are running, with
keyboard and mouse support.

| Key | Action |
|---|---|
| `←` `→` | Move focus |
| `Enter` | Confirm |
| `n` / `Esc` | Keep terminal |
| `y` | Close anyway |

## Hooks

Plugins implement one or more traits from `port-plugin-api`:

| Hook | Purpose |
|---|---|
| `AppearanceHook` | Opacity, colours, fonts |
| `InputHook` | Intercept and consume keys |
| `LayoutHook` | Inject top bar, sidebar, status bar |
| `SpaceHook` | Create, select and track sessions/tabs/spaces |
| `PluginManagerHook` | Replace the plugin manager UI |
| `LifecycleHook` | Veto the window close |

Every plugin may also provide `default_config`, `load_config` and `save_config`
to persist settings in `~/.config/port/config.md`.

## Using plugins

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

## Development

```bash
nix-shell      # native libraries linked
cargo test     # 39 unit tests
cargo build --release
```

### Writing your own

Create a crate with `port-plugin-api` as a dependency, implement `Plugin` plus
any hook, and add it to the workspace in `Cargo.toml`. The plugin is registered
from the host application; the registry handles configuration, enable/disable and
hot-reload for you.

## License

MIT
