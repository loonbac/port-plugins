# smooth-cursor — animated terminal cursor for PORT

A PORT in-process plugin that reproduces Ghostty's `smooth_cursor.glsl`: the cursor
rectangle interpolates between its previous and current cell instead of jumping, with
an easing curve and a fading trail. PORT has no cursor render hook today, so the
feature is split: **PORT core grows a `CursorHook`, this repo grows the plugin that
consumes it.**

## Specs

**S1.** The user asked for a PORT plugin "que agregue esta clase de animacion", and
explicitly did **not** want a literal port of the shader mechanics:

> pero mejor, al estilo de los plugins de port, que este bien integrado, quisa no se
> pueda hacer como lo hace ghostty pero hay que hacerlo de otra manera que port si
> soporte ya que usa gpui y todo eso

**S2.** The animation parameters and behaviour come from the Ghostty shader:

```
#define ANIMATION_DURATION 0.12  // más alto = más lento y más visible
#define TRAIL_STRENGTH 0.30      // 0.0 = sin estela
#define CORNER_RADIUS 2.0        // redondeo en píxeles
```

> 2. Interpolación con easing, no lineal. progress va de 0 a 1 y se le pasa por
> easeOutCubic (arranca rápido, frena suave). Después mix() interpola centro y tamaño
> entre iPreviousCursor e iCurrentCursor. Ese mix de tamaño además es lo que hace que
> el cursor parezca estirarse cuando salta entre una línea de texto y un bloque más alto.

**S3.** The trail is the segment already travelled, faded by remaining progress:

```
if (h <= t) {
    float fadeFactor = (1.0 - progress) * TRAIL_STRENGTH;  // 0.30
    trailMask = lineAlpha * fadeFactor;
}
```

> Con TRAIL_STRENGTH = 0.30 y el fade proporcional a (1 - progress), la estela se apaga
> a medida que el cursor llega. Es deliberadamente imperceptible: sugiere movimiento sin
> dejar un rastro visible.

**S4.** Passthrough is the default: no work unless the cursor is visible, moved, and the
transition is still running.

```
float timeSinceMove = iTime - iTimeCursorChange;
if (timeSinceMove >= ANIMATION_DURATION) { fragColor = terminalColor; return; }
```

**S5.** PORT must not pay Ghostty's `custom-shader-animation = always` cost, which the
user described as the price of smoothness:

> Es también el coste: redibuja el viewport completo a tasa de refresco aunque no pase
> nada. En una laptop eso se nota en batería.

**S6.** The three tunables are user-editable configuration, like every other PORT plugin:

> Las tres constantes al inicio del shader son todo

**S7.** Core changes live in the PORT repo and are requested from the PORT session, not
made here:

> dale la a, pero los cambios que pides de port pideselos a la sesion de pi que abri del
> repo de port

## The contract PORT provides

Final shape, agreed with the PORT session. It is a **generic per-frame overlay
primitive**, deliberately not named after this use case.

```rust
pub struct CellRect {
    pub column: f32, pub row: f32, pub width: f32, pub height: f32,
}
pub struct CursorState { pub rect: CellRect, pub shape: CursorShape, pub color: Rgb }
pub struct FrameView {
    pub cursor: Option<CursorState>,
    pub now_ms: u64,
    pub focused: bool,
}
pub struct OverlayQuad {
    pub rect: CellRect, pub color: Rgb, pub opacity: f32, pub corner_radius_px: f32,
}
pub struct Overlay { pub quads: Vec<OverlayQuad>, pub suppress_native_cursor: bool }
pub trait OverlayHook {
    fn overlay(&self, view: &FrameView) -> Option<Overlay> { None }
    fn needs_animation(&self) -> bool { false }
}
```

Plus `Plugin::overlay_hook()` with a `None` default.

Agreed semantics:

- **Positions** in fractional cell space relative to the viewport.
- **Lengths the user configures** (corner radius, future thicknesses) in pixels.
  `corner_radius_px` goes straight to `PaintQuad::corner_radii`; the host converts
  nothing and 2.0 means 2 px at any font size.
- `CursorState.rect` is the native footprint (the whole cell PORT would invert);
  `color` is the fill that inversion would produce. `shape` mirrors `term-core`
  (Block/Beam/Underline/Hidden) so the plugin never assumes a block.
- `FrameView.cursor = None` still invokes the hook: reset state and return `None`.
  This is what prevents a stale rect from producing a trail across the screen.
- One invocation per painted frame through `invoke_hook` (panic and timeout
  isolation, 250 ms budget). Frames are only painted when something asks for one
  (PTY output, animation, gesture), not at a free 60 fps.
- `needs_animation()` is OR-ed with `LayoutHook::needs_animation()`. The obligation:
  it must fall back to `false` when the transition ends, or the plugin buys back
  the permanent repaint this feature exists to avoid.
- First `Some` wins, in plugin registration order (documented, not deterministic
  from the plugin's point of view).
- Paint order: after glyphs and images, before the chrome. `suppress_native_cursor`
  gates the native cursor in that same frame.

PORT-side files: `crates/plugin-api/src/hooks.rs`, `crates/plugin-api/src/registry/
hooks.rs`, `crates/port-app/src/view/mod.rs`, `crates/port-app/src/view/paint.rs`.

## Design note: the clock the hook does not get

`OverlayHook::needs_animation(&self)` takes no `now_ms`, so the plugin cannot compute
"am I mid-transition" while the registry asks. Inventing `true` would be a sticky
request and reintroduce permanent repaint.

The plugin answers from the last painted frame: `quads()` recomputes the transition
state for the frame and stores it; `needs_animation()` returns that stored value. The
cost is one frame of lag in switching the channel off, which coincides with
`HANDOFF_GRACE_MS` — the same frame the plugin needs to hand the native cursor back.
Tests: `cached_frame_request_stops_after_the_transition`, `idle_cursor_does_not_request_frames`.

## Design note: enabled by default follows the host

`Config::default()` ships `enabled: true`. That is not a preference: PORT's
`PluginConfig::is_enabled` returns `config.get_bool("enabled").unwrap_or(true)`, so a
plugin that omitted the key would be on anyway. Shipping it on keeps the file honest
about what is happening, and `enabled = false` in the `smooth-cursor` block is the kill
switch.

## Design note: the frame budget is a tested property

The battery cost of this feature is the number of frames a transition requests. That
is now an assertion, not a hope: `a_transition_requests_a_bounded_number_of_frames`
simulates a 60 Hz host through a full transition and requires it to finish in 5..15
frames, then verifies that 50 further frames request nothing.

It is written this way on purpose. A regression here is invisible in a screenshot and
shows up as a warm laptop, so the ceiling belongs in a test rather than in a comment.

## Design note: one entry point per frame

`SmoothCursorPlugin::frame(rect, now_ms)` is the only method the adapter calls. It
registers the cursor (or the reset) and then consumes the result, in that order, so
`needs_animation()` is refreshed even when the hook returns `None`.

This exists because of a defect found while reviewing the adapter: reading the color
with `view.cursor?.color` before calling `quads()` returns early when the cursor is
gone, leaving the cached frame request stuck on `true` — the permanent repaint the
whole design avoids. Test: `disappearing_cursor_does_not_leave_frames_requested`.

## Integration surface: more than one line

Wiring the crate into PORT touches more than `Cargo.toml` and `main.rs`. Verified:

1. `crates/port-app/Cargo.toml` — add `port-plugin-smooth-cursor` as a path dep
   (dev) / git dep (release), like `port-plugin-herdr`.
2. `crates/port-app/src/main.rs` — `plugin_registry.register(SmoothCursorPlugin::new())`
   next to `HerdrPlugin`.
3. `[patch."https://github.com/loonbac/port.git"]` in port-plugins already covers
   `port-plugin-api`; nothing to add there.
4. **`scripts/bump-port-plugins.sh` hardcodes the plugin list**: `cargo update -p
   port-plugin-herdr -p port-plugin-selection` at line 122 and the hash extraction
   loop at line 154. A new plugin that is not added to both is silently left out of
   the lock bump, and the failure surfaces much later as a Nix hash mismatch. This is
   the step easiest to forget and the most expensive to debug.
5. `flake.nix` `cargoLock.outputHashes` is rewritten by that script from the build's
   own report; do not hand-edit.

PORT's own repo owns steps 1, 2, 4 and 5. This repo owns the crate.

## Tasks

| ID | What | Links | Route | Commit |
|----|------|-------|-------|--------|
| T1 | Write the contract document and send it to the PORT session | S1,S7 | inline | done |
| T2 | Create `in-process/smooth-cursor` crate: `CursorState` with previous/current rect, movement detection, `progress()`, `ease_out_cubic`, trail segment | S2,S3,S4 | inline | pending |
| T3 | Plugin shell: id, config keys, `needs_animation()` from the last frame, `overlay()` passthrough when idle | S4,S5,S6 | inline | done (33 tests) |
| T4 | Adapter against the real API (`host.rs`): `Plugin` + `OverlayHook`, host `CellRect` conversion at the boundary | S1,S4,S5,S6 | inline | done (39 tests, commit 0570cb7 of PORT) |
| T5 | Wire the plugin into PORT: `port-app/Cargo.toml` (with `features = ["overlay-hook"]`), `main.rs` registration, and both hardcoded lists in `scripts/bump-port-plugins.sh` | S7 | mine, after PORT merges its branch | |
| T6 | Narrow the overlay rect for `Beam`/`Underline` using `CursorState.shape` | S2 | inline | done (43 tests) |
| T7 | Verify visually in a real compositor: before/after capture of a cursor jump, trail presence, idle CPU when the transition ends | S2,S5 | tbd | |

## Log

**L1** (verbatim, user):

> necesito que crees un nuevo plugin para port que agregue esta clase de animacion
> Cómo está montado
> Son dos piezas: la config que enciende la animación y un shader GLSL que la dibuja.
> Ghostty no trae animación de cursor; lo que trae es un sistema de custom shaders que se
> ejecuta en cada frame con uniformes sobre el estado del cursor.
> [config: custom-shader + custom-shader-animation = always; uniformes iChannel0,
> iCurrentCursor/iPreviousCursor, iCurrentCursorColor, iTimeCursorChange, iCursorVisible;
> las cinco ideas del shader; ANIMATION_DURATION 0.12, TRAIL_STRENGTH 0.30,
> CORNER_RADIUS 2.0; el 0.80 de mezcla; el coste de `always`]
> pero mejor, al estilo de los plugins de port, que este bien integrado, quisa no se
> pueda hacer como lo hace ghostty pero hay que hacerlo de otra manera que port si
> soporte ya que usa gpui y todo eso

**L2** (verbatim, user): "dale la a" — implement option A: full effect (interpolation,
easing, trail, rounded corners), which requires new PORT core API plus the plugin.

**L3** (verbatim, user): "pero los cambios que pides de port pideselos a la sesion de pi
que abri del repo de port" — core work is requested from PORT session
`01a18027-ad72-75f5-af04-20d8f67a8685`; this repo does not modify PORT.

**L4** (evidence, exploration): a store plugin cannot do this. `Capability` has no paint
or animation message (`crates/plugin-sdk/src/protocol.rs:44`), `LayoutHook` is excluded
from the protocol (`:30-33`), and `needs_animation` iterates in-process plugins only
(`crates/plugin-api/src/registry/hooks.rs:565-577`). An in-process plugin with today's
hooks cannot either: `LayoutHook`'s three bars are flex siblings of the canvas
(`crates/port-app/src/view/mod.rs:402-425`), and the cursor is a style inversion inside
`paint_frame` (`crates/port-app/src/view/paint.rs:589-597`). Hence the new `CursorHook`.

**L6** (decision, PORT session): `corner_radius` stays in **pixels**. PORT owns the
conversion with its `Metrics` before calling `PaintQuad::corner_radii`, so the plugin
never receives cell size and cannot hold a stale value when the font changes. Rule:
cell-space positions, pixel-space lengths the user configures. `cell_width_px` /
`cell_height_px` are not exposed at all. Accepted; `Tuning::corner_radius_px` already
reflects it.

**L7** (decision, PORT session): `CursorView.shape` mirrors `term-core`'s enum so the
plugin never assumes a block, and `CursorHook::needs_animation()` is OR-ed with
`LayoutHook`'s. Open request from L8 below: the hook must be invoked even when
`rect` is `None`, so the plugin can reset instead of holding a stale cursor.

**L8** (accepted, PORT): unconditional invocation. One call per painted frame even
with `cursor: None`, which means "reset and return `None`". This is what keeps a
stale rect from producing a trail across the screen when the cursor reappears
somewhere else.

**L9** (decision, PORT): the contract is a generic `OverlayHook`, not `CursorHook`.
Same mechanism, no use-case name in the API. `suppress_native_cursor` becomes
explicit instead of implied by `Some`.

**L11** (accepted, PORT): two guarantees. (a) Suppression is evaluated only from the
hook's result in the same frame, with no state carried between frames; the sequence is
an invariant tested at registry level (second call returns `None` ⇒ native visible).
(b) Unconditional invocation is confirmed by the host owner. Units closed on
`corner_radius_px` with nothing to convert anywhere.

**L13** (discovery, adapter): `CursorShape` and `Rgb` live in `port-term-core`, not in
`port-plugin-api`, and `Rgb` is 8-bit per channel (`u8`), not float. The crate already
depends on `port-term-core`, so nothing new was needed.

**L20** (verified): the adapter builds, tests and passes clippy against the branch tip
`3c5f922`. `cargo fmt --check` is clean for this crate. The Cargo feature
`overlay-hook` stays off by default until the API lands in the branch the `[patch]`
resolves.

**L22** (commit): branch `feat/smooth-cursor-plugin-clean`, based on `origin/master`
(f43dfc7), worktree `/tmp/sc-clean`. **The reviewable commit is the one tagged
`smooth-cursor-review`**; the sha is not written here because editing this file moves
it. The crate directory is byte-identical across every amended sha (verified with
`git diff` against each), so the tag is the only stable reference:

    git rev-parse smooth-cursor-review
The first attempt sat on the `herdr-layout-cwd` branch and dragged four unrelated
commits into the review, so it was replayed with `git am` onto master.
37 tests pass on that base; the adapter needs PORT's branch, hence the feature flag.

**L25** (decision, PORT): registration is mine, performed after the PORT branch is
merged; whether it rides the same PR is the host owner's call together with the push
authorization.

**L36** (defect found by the reported symptom, fixed plugin-side): PORT asks
`needs_animation()` **before** invoking `overlay()` (`view/mod.rs:282-287`). In the frame
where the cursor jumps after `Alt+Backspace`, the answer comes from the previous frame
(idle ⇒ `false`), so the host does **not** call `request_animation_frame`. The overlay
then paints the interpolated position — which at progress 0 is the *old* cursor — and
`suppress_native_cursor` hides the real one. No further frames are requested, so the
cursor stays painted where it no longer is until another keypress forces a frame. That
is the reported symptom exactly ("borro y se queda a la derecha, hasta que presiono una
tecla y se actualiza").

Plugin-side fix: the frame that starts a transition returns `None` instead of quads, so
the native cursor is painted at the correct position; the frame flag is still stored, so
the next frame animates. Tests:
`the_frame_that_starts_a_transition_does_not_paint_a_stale_cursor`,
`the_next_frame_paints_the_animation`. 47 tests green.

**Host-side follow-up (owned by PORT):** reordering `overlay()` before
`needs_animation()` makes the flag reflect the current frame, which also restores the
animation on that very first jump. With the current order the plugin fix is correct but
costs the first jump its animation.

**L34** (registration landed, verified by reading): PORT master `2518809` (tag `v0.1.13`)
carries all three code sites: `crates/port-app/Cargo.toml:21`, `main.rs:27` (import) and
`:68` (`register`), and the workspace dep at `Cargo.toml:39` pinned by
`rev = "25aaa7cb9f592364b86edce781483f1daeca210b"`. `Cargo.lock` records
`?rev=25aaa7cb...`.

**L35** (correction to my own "five sites" warning): the script lists are a **fixed list
for branch-tracked plugins** — `for pkg in port-plugin-herdr port-plugin-selection`, i.e.
those declared `branch = "master"`. `smooth-cursor` is pinned by immutable `rev`, so it is
correctly **absent** from both lists. Furthermore the script's verifier greps the lock for
`?branch=master#`, which a `rev`-pinned entry never matches, so adding it would have made
the script's post-condition check meaningless. My earlier claim that forgetting the two
script sites would cause a Nix hash mismatch applied to herdr/selection, not to this crate.
The registration is complete as delivered.

**L33** (pushed, verified reachable): `feat/smooth-cursor-plugin-clean` is pushed;
`git ls-remote` reports `25aaa7cb9f592364b86edce781483f1daeca210b` on the remote, and a
clone-less `git fetch --depth=1 origin <sha>` resolves it, so cargo can pin it with `rev`.
The correct full sha is `25aaa7cb9f592364b86edce781483f1daeca210b`; an earlier truncated
value quoted in conversation (`25aaa7c517c5…`) does not exist as an object — shas are
copied from `git rev-parse`, never typed.

**L32** (pin bumped and verified): `v0.1.12` resolves to `98f23a5` on PORT's master and
declares version 0.1.12 with `OverlayHook` present. The pin for all three crates
(`port-term-core`, `port-plugin-api`, `port-plugin-sdk`) now reads `tag = "v0.1.12"`.
`Cargo.lock` had to be regenerated **with the local `[patch]` removed**, because while it
is active cargo resolves the sibling path and the lock never learns the published
revision — the coupling `scripts/bump-port-plugins.sh` documents in its header. The
regenerated lock records
`git+https://github.com/loonbac/port.git?tag=v0.1.12#98f23a5`, which is what Nix can
vendorize. The `[patch]` is restored afterwards for local development.

Verified with the patch removed and `--offline` **not** passed (a fresh lock needs to
fetch): 43 tests pass against the published tag, no feature selected.

**L31** (merge, resolved): PORT merged `feature/overlay-hook` into master as `afb2566`
(PR #18). `crates/plugin-api/src/hooks.rs` on that branch has `OverlayHook` and
`Plugin::overlay_hook`. The feature flag is therefore **gone**: `[features]` removed from
`Cargo.toml`, `#[cfg(feature = "overlay-hook")]` removed from `pub mod host` in `lib.rs`.
Verified against a detached worktree at `afb2566`: 43 tests, clippy `-D warnings` and
`cargo fmt --check` clean, with no feature selected.

Registration still needs the crate reachable by PORT: either a push (variant A) or the
local patch entry in its worktree (variant B). The merge removed the API blocker, not the
distribution one.

**L30** (blocker for the path-local variant, verified): the `[patch."...port-plugins.git"]`
block exists **only in PORT's main checkout** (`Cargo.toml:49`, marked "SOLO DESARROLLO
LOCAL (no commitear)") and lists herdr, selection and mobile. The feature worktree
`/tmp/port-overlay` does **not** have it. So registering by local path in that worktree
requires adding the patch entry there first, and it is a local-only edit that must not be
committed. With `rev` pinned instead, a push is mandatory before PORT can resolve it.

Either way the registration patch cannot be verified from this repo alone; it needs one
of those two decisions from the host owner.

**L29** (correction to the order, verified): PORT's `[patch."https://github.com/loonbac/port.git"]`
redirects `port-plugin-api` to its own local `crates/plugin-api`, with an explicit comment
that without it `Plugin` resolves to two different crates. Consequence: with the patch
active — always in local dev and in the feature worktree — the tag is bypassed and the
local crate wins, which already has `OverlayHook`.

So registration **can be built and verified today**, before any tag exists. The tag is
needed for the release build, where `scripts/disable-local-patch.sh` strips the patch and
everything resolves from tags. The ordering agreed with PORT stays correct for publishing;
what changes is that it does not block verifying the feature.

**L28** (handoff state): ownership is settled and nothing is pending on this side.
PORT merges and tags; this repo bumps the pin; then the feature comes out; then the five
registration sites. `smooth-cursor-review` stays frozen at the code commit `25012eb` and
is never force-moved again: PORT verified the tag at three different shas while it was
moving, which is the defect, not the identifier. Documentation commits (50b9e65,
083878c) sit above the tag without changing the crate, verified by `git diff`.

**L27** (ordering, agreed with PORT): wait for PORT's tag, then delete the `overlay-hook`
feature and register **without** `features`. Sequence, so nobody registers a silent
no-op:

1. PORT merges the overlay branch and cuts a tag that contains `OverlayHook`
   (`origin/master` at 3ad62a5 does not have it yet; nor does `v0.1.5`).
2. port-plugins bumps its pinned `tag`/`rev` to that new PORT tag.
3. Drop `features.overlay-hook` from this crate: delete the `[features]` entry, the
   `#[cfg(feature = ...)]` on `pub mod host`, and the note. The adapter then compiles
   always.
4. Register in PORT: `port-app/Cargo.toml` (no `features`), `main.rs`, and both
   `scripts/bump-port-plugins.sh` lists.

Doing 4 before 2 leaves the plugin registered with no `overlay_hook()`, which paints
nothing and reports nothing — the failure mode this ordering exists to prevent.

**Correction (verified, not assumed):** without the `overlay-hook` feature this crate
does not implement `Plugin` **at all**, because both `impl Plugin` and `impl OverlayHook`
live in `host.rs`. PORT's `PluginRegistry::register<P: Plugin>` is generic over the
trait, so `plugin_registry.register(SmoothCursorPlugin::new())` fails to compile rather
than silently registering nothing.

That is better news than feared: the failure is loud and at build time. But it means the
feature is **mandatory** for any registration attempt, not merely recommended, and the
ordering above is really "bump the pin, then drop the feature, then register" with no
viable intermediate state.

**L26** (decision, PORT): `enabled` stays opt-out (`true` by default). An explicit
`enabled` in this plugin's block is correct, and flipping the global default would turn
existing plugins off without asking. Confirms the reasoning in the design note above.

**L24** (slice, deliberate): this feature lands as one commit (~1810 authored lines),
past the ~400-line guideline. Splitting it would produce commits that do not
compile independently: `anim` is exercised by `plugin`, `plugin` by `host`, and `config`
feeds both. The reviewable unit here is the feature, and the four modules are read in
that order. Recorded so the exception is visible rather than accidental.

**L23** (release blocker, PORT): the `[patch]` pins `v0.1.5`, which declares version
0.1.2 and has no `OverlayHook`. Registering the plugin against port-app therefore needs
a new PORT tag containing the overlay branch, or the build fails resolving the hook.
Not a defect in this crate; it is a release ordering constraint.

**L21** (defect, PORT, cosmetic): commit `3c5f922` added the requested warning but left
the doc block mis-indented — two lines start at column 0 inside a `///` block, and one
sentence got merged onto the previous line ("Cuesta un cuadro de retraso al apagar," now
trails the warning). It compiles and `cargo check` passes, so it is documentation
appearance only, not a build break. Worth one `cargo fmt` before the PR.

**L17** (decision, PORT): `needs_animation()` keeps zero arguments (`LayoutHook`
parity, no implementor breakage). The cached-flag pattern is documented in `hooks.rs`
as the contract, commit `9657906`. Nothing to delete in the adapter.

**L18** (decision, PORT): quads are free geometry, clipped only to the grid; a Beam may
emit a quad of 0.15 cells. Commit `d4688b6` also records that `CursorShape` and `Rgb`
live in `term_core` with `u8` channels. Implemented as `anim::shape_rect` with
`BEAM_WIDTH = 0.15` and `UNDERLINE_HEIGHT = 0.12`; the trail inherits the shape.

**L19** (defect, adapter, caught by a test): adding `shape` to the adapter first used
`view.cursor.map(...)?`, whose early return skipped the result consumption and left the
cached frame request stuck on `true` — the exact defect `hidden_cursor_resets_and_stops_frames`
was written for. Fixed with `unwrap_or_default()` so `frame()` always runs. The test paid
for itself immediately.

**L15** (build, adapter): the adapter compiles only against the overlay API, which
lives in PORT's `feature/overlay-hook` worktree and is not merged. The crate therefore
gates `host.rs` behind a Cargo feature, `overlay-hook`, off by default, so the
workspace keeps building. Verified with the patch pointed at `/tmp/port-overlay`:
39 tests green and clippy clean with the feature on.

**L16** (pre-existing, not ours): `cargo test --workspace` fails in `port-plugin-mobile`
(`missing fields debug and images in initializer of Frame`) because the local
`../port` branch is ahead of the `v0.1.5` API that crate pins. Its file has no local
changes; unrelated to this feature.

**L14** (discovery, adapter): `in-process/smooth-cursor` keeps its own `CellRect` so
`anim` compiles without `port-plugin-api`; `host.rs` converts at the single boundary.

**L12** (process, PORT): implementation lands in a new worktree on a feature branch,
not master. Push and PR are the host owner's decision, so visible evidence is the
commit plus `path:line` in the sibling clone, not a PR.

**L10** (plugin, handoff gap): with `suppress_native_cursor: true` the native cursor
is hidden while animating. If the transition ends in the same frame that stops
requesting animation, the last painted frame keeps the native cursor suppressed with
the animated one frozen mid-way. Fixed on the plugin side: the tick survives
`HANDOFF_GRACE_MS = 32` after the transition, and `overlay()` returns `None` once the
transition is over, so the native cursor is restored. Tests
`handoff_keeps_one_extra_frame_then_stops` and `idle_cursor_never_requests_frames`.

**L5** (discovery): PORT always paints a block cursor. `CursorShape::Beam` and
`Underline` are produced by `term-core` (`crates/term-core/src/frame/mod.rs:84`) but never
read by `port-app`, which uses only `frame.cursor_run`. There is also no cursor blink and
no inter-frame cursor state. Both are out of scope here, noted so the plugin does not
assume shape support.
