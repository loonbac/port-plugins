# PORT Plugins · Store

[Español](README.es.md)

Plugin store for [PORT](https://github.com/loonbac/port), a plugin-oriented
terminal emulator written in Rust on GPUI.

This repository holds two kinds of plugins:

- `plugins/<id>/` — **store** plugins. Each one is a standalone binary that
  links only the plugin SDK; `port plugin add` installs it without recompiling
  PORT.
- `in-process/<id>/` — plugins that use in-process features (GPUI widgets, live
  hooks, the close protocol). PORT must be recompiled with them; they are not
  installable from the store.

## Store · Tienda

### Installing a plugin

```bash
# From the repository (the store form).
port plugin add https://github.com/loonbac/port-plugins/plugins/font-zoom

# From a local checkout, while developing.
port plugin add /ruta/port-plugins/plugins/font-zoom
```

The first install compiles the plugin on the host (`cargo build --release`), so
it needs a Rust toolchain. Installing it again recompiles and replaces it:
updating and reinstalling are the same operation.

Installed plugins live in `~/.local/share/port/plugins/<id>`, each next to a
manifest that records its source and the capabilities it announced.

To activate or deactivate a plugin, open PORT's plugin menu with `Ctrl+Shift+L`
and toggle it there.

### Catalog

| id | Name | What it does | Type | Capabilities | Install |
|---|---|---|---|---|---|
| `transparency` | Transparency | Window background opacity for compositor-level transparency on Wayland. | store | `appearance` | `port plugin add …/plugins/transparency` |
| `font` | Font Configuration | Font family, size and fallback list. | store | `appearance` | `port plugin add …/plugins/font` |
| `font-zoom` | Font Zoom | Owns the current font size and its zoom shortcuts (`Ctrl+=`, `Ctrl+-`, `Ctrl+0`). | store | `appearance`, `input` | `port plugin add …/plugins/font-zoom` |
| `herdr` | Herdr Customization Plugin | Workspace manager: resizable sidebar of spaces, tab bar, live program detection and naming from the current directory and Git branch. | in-process | `AppearanceHook`, `InputHook`, `LayoutHook`, `SpaceHook` | — (recompile PORT) |
| `shortcuts` | Custom Shortcuts | Binds key combinations to callbacks and to services published by other plugins. | in-process | `InputHook` | — (recompile PORT) |
| `close-guard` | Close Guard | Asks for confirmation before closing the window when programs are running. | in-process | `LifecycleHook` | — (recompile PORT) |
| `selection` | Selection | Owns the mouse and selection policy: forwards clicks, drags and motion to the program, Shift returns the gesture to PORT, and copy/word/line/highlight on select. | in-process | `MouseHook` | — (recompile PORT) |

For store plugins the install URL is
`https://github.com/loonbac/port-plugins/plugins/<id>`.

## Why some plugins are in-process

The store boundary is the JSON Lines protocol, and not everything fits through
it:

- **`herdr` draws GPUI elements.** `LayoutHook` returns an `AnyElement`, which
  has no serialized form, so the layout stays inside the process.
- **`shortcuts` is a service broker.** It resolves services published by other
  plugins when a key is pressed, and those services are Rust trait objects: they
  do not cross the process boundary.
- **`close-guard` needs a close-confirmation round trip.** In the protocol the
  lifecycle is only `Shutdown`, a notification, so the core cannot ask a plugin
  whether the window may close.

## Plugin reference

### `font-zoom`

Owns font size as state and nothing else: it reports the current size through
the `appearance` capability and drives it through `input`. The shortcuts are
its own, so the plugin works even when `shortcuts` is not loaded.

| Shortcut | Action |
|---|---|
| `Ctrl` `=` | Zoom in |
| `Ctrl` `-` | Zoom out |
| `Ctrl` `0` | Reset font size |

Its configuration block accepts the same size keys as before plus the three
shortcut patterns:

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

Bind arbitrary key combinations to callbacks, with a compact `ctrl+shift+t`
syntax. This is where custom in-process bindings live. A binding can also call
a service published by another plugin, resolved on keypress:

```rust
use port_plugin_shortcuts::ShortcutsPlugin;

let shortcuts = ShortcutsPlugin::new();

// Its own binding.
shortcuts.bind_str("ctrl+shift+k", || println!("Hello!"));

// Another plugin's published service: no shared types, resolved on keypress.
shortcuts.bind_service("ctrl+shift+p", "some-plugin", "some_action");
```

### `herdr`

The workspace manager: a resizable sidebar of spaces, a tab bar, live detection
of the running program, and automatic naming from the current directory and Git
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

In-process plugins implement one or more traits from `port-plugin-api`:

| Hook | Purpose |
|---|---|
| `AppearanceHook` | Opacity, colours, fonts |
| `InputHook` | Intercept and consume keys |
| `LayoutHook` | Inject top bar, sidebar, status bar |
| `SpaceHook` | Create, select and track sessions/tabs/spaces |
| `LifecycleHook` | Veto the window close |

Store plugins do not link GPUI or `port-plugin-api`; they declare the matching
values through the SDK protocol instead. Every plugin may also provide
`default_config`, `load_config` and `save_config` to persist settings in
`~/.config/port/config.md`.

## Development

```bash
nix-shell                    # native libraries linked
cargo test --workspace       # unit + protocol tests
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

### Writing your own

For a store plugin, create a crate that depends only on `port-plugin-sdk`, add a
`[[bin]]` and a `[package.metadata.port]` section with the `id` and
`capabilities`, and implement `port_plugin_sdk::runtime::Plugin`. PORT installs
it with `port plugin add`, compiles it once and talks to it over the protocol.

For an in-process plugin, add `port-plugin-api` as a dependency, implement
`Plugin` plus any hook you need, and add the crate to `in-process/*` in the
workspace. PORT must be recompiled with it; the registry then handles
configuration, enable/disable and hot-reload.

## License

MIT
