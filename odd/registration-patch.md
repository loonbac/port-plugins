# Registro de `smooth-cursor` en PORT — parche aplicable

Preparado contra `/tmp/port-overlay` (rama `feature/overlay-hook`), sin tocar ese
repo. Verifica el orden real de resolución antes de usarlo.

## Corrección importante al orden que acordamos

PORT tiene en su `Cargo.toml`:

```toml
[patch."https://github.com/loonbac/port.git"]
port-term-core = { path = "crates/term-core" }
port-plugin-api = { path = "crates/plugin-api" }
```

Ese `[patch]` redirige `port-plugin-api` al **crate local de PORT**, con el
comentario explícito: "Sin esto, `Plugin` se resuelve a dos crates distintos y el
registro no compila."

Consecuencia: mientras el `[patch]` esté activo (es decir, siempre en desarrollo
local y en el worktree de la rama), `port-plugin-api` **no se resuelve por tag**,
sino por la ruta local. Y la ruta local de `feature/overlay-hook` sí tiene
`OverlayHook`.

Es decir: **el registro se puede hacer y verificar antes de que exista el tag
publicado.** El tag solo hace falta para el build de release, donde
`scripts/disable-local-patch.sh` retira el parche y todo se resuelve por tag.

Eso no invalida el orden que acordamos — sigue siendo correcto para publicar —
pero cambia qué se puede verificar ahora: el registro y la verificación visual
son posibles hoy, contra el worktree, sin esperar tag ni push.

## Las dos variantes de la dependencia (elige una)

El crate no está pusheado ni publicado, así que el punto 2 tiene dos formas y
ninguna es gratis. Verificado leyendo los archivos, no supuesto.

### Variante A — `rev` fijo (requiere push de port-plugins)

```toml
port-plugin-smooth-cursor = { git = "https://github.com/loonbac/port-plugins.git", rev = "<sha>", features = ["overlay-hook"] }
```

Donde `<sha>` es `git rev-parse smooth-cursor-review` en el repo de port-plugins.
Requiere que ese commit esté **pusheado**: cargo no resuelve un sha que no está
en el remoto. Ventaja: no deja patch local que limpiar, sobrevive a un checkout
accidental, y es la forma en que PORT consumirá el plugin cuando esté publicado.

### Variante B — ruta local (no requiere push)

El `[patch."https://github.com/loonbac/port-plugins.git"]` existe **solo en el
checkout principal** (`Cargo.toml:49`, marcado "SOLO DESARROLLO LOCAL (no
commitear)") y lista herdr, selection y mobile. El worktree
`/tmp/port-overlay` **no lo tiene**, así que esta variante exige añadirlo allí:

```toml
[patch."https://github.com/loonbac/port-plugins.git"]
port-plugin-smooth-cursor = { path = "../../port-plugins/in-process/smooth-cursor" }
```

Con `path`, la dependencia del punto 2 puede quedarse en `{ workspace = true }`.

Coste: es una edición local de un archivo que hay que limpiar antes de commitear
la rama, y si se cuela en un commit el build de release falla con una ruta que no
existe en el runner. La variable de entorno `PORT_PLUGINS_PATH` no existe hoy;
el script `disable-local-patch.sh` retira el patch por nombre, así que añadir
esta entrada no rompe el script.

## Los cinco sitios

### 1. `crates/port-app/Cargo.toml`

```toml
[dependencies]
port-plugin-smooth-cursor = { workspace = true }
```

### 2. `Cargo.toml` (workspace, junto a herdr y selection)

```toml
# El cursor animado vive en el nucleo del pintado, asi que se compila dentro.
# Apunta a la rama que tiene el overlay; subir a tag cuando salga.
port-plugin-smooth-cursor = { git = "https://github.com/loonbac/port-plugins.git", rev = "<sha-del-merge>" }
```

Nota: `herdr` y `selection` usan `branch = "master"`. Para este conviene `rev`
fijo, porque el plugin todavía no está en la rama que PORT resuelve.

### 3. `crates/port-app/src/main.rs`

```rust
use port_plugin_smooth_cursor::SmoothCursorPlugin;
```

Junto a `plugin_registry.register(HerdrPlugin::new());` (línea 64):

```rust
// El cursor animado: interpola, dibuja estela y devuelve el control al
// nativo cuando la transicion termina.
plugin_registry.register(SmoothCursorPlugin::new());
```

### 4. `scripts/bump-port-plugins.sh:122`

```bash
upd=(cargo update -p port-plugin-herdr -p port-plugin-selection -p port-plugin-smooth-cursor)
```

### 5. `scripts/bump-port-plugins.sh:154`

```bash
for key in port-plugin-herdr port-plugin-selection port-plugin-smooth-cursor; do
```

`flake.nix` (`cargoLock.outputHashes`) lo reescribe el script: no editar a mano.

## Sobre el feature `overlay-hook`

El crate de port-plugins publica el adaptador detrás de:

```toml
[features]
overlay-hook = []
```

Con el `[patch]` de PORT activo la API local tiene `OverlayHook`, así que el
registro **necesita** `features = ["overlay-hook"]` hasta que el tag publicado
lo incluya:

```toml
port-plugin-smooth-cursor = { ..., features = ["overlay-hook"] }
```

Sin ese feature, `SmoothCursorPlugin` no implementa `Plugin` y
`plugin_registry.register(...)` falla en compilación (verificado: `register` es
`fn register<P: Plugin>(&mut self, plugin: P)`).

Cuando salga el tag con `OverlayHook`, en port-plugins: borrar `[features]`, el
`#[cfg(feature = "overlay-hook")]` de `pub mod host` en `lib.rs`, y la nota.
Luego en PORT: quitar `features = [...]` de la dependencia.

## Verificación después de aplicar

```bash
cd /tmp/port-overlay
cargo build -p port-app           # el registro compila
cargo test -p port-app            # sin regresiones
```

Y la visual, con configuración aislada:

```bash
PORT_CONFIG=/tmp/port-smooth-cursor-test cargo run -p port-app
```

Capturar: salto de cursor (Ctrl-A Ctrl-A en tmux o mover con flechas), presencia
de estela durante ~120 ms, y CPU en reposo una vez terminada la transición.
