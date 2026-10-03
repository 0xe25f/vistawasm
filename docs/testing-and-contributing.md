# Testing and Contributing

This is the practical reference for building, testing, and contributing to
VistaWASM itself — not for consuming the published package (see
[`docs/getting-started.md`](getting-started.md) for that). Read
[`AGENTS.md`](../AGENTS.md) first for the project's style and process rules;
this document is the concrete commands and workflow that satisfy them.

## Prerequisites

| Tool | Version | Notes |
| --- | --- | --- |
| Rust | 1.87 or newer, via [rustup](https://rustup.rs) | wgpu 30 needs 1.87. Add the WebAssembly target with `rustup target add wasm32-unknown-unknown`. |
| Node.js | 22.12+ (22.x), 24 (which CI uses) or 26+ | Vitest 5 needs one of these. npm comes with Node. |
| wasm-pack | 0.15.0, as CI pins it | `cargo install wasm-pack --version 0.15.0 --locked`, or the installer at [rustwasm.github.io/wasm-pack](https://rustwasm.github.io/wasm-pack/installer/). |

On its first run, `npm run build` fetches the prebuilt `wasm-bindgen`
CLI that matches `Cargo.lock` (0.2.129) into `node_modules/.cache/`,
and release builds download Binaryen's
`wasm-opt` (version 117, the one wasm-pack 0.15.0 uses) unless one is
already on your `PATH`. Release sizes are measured with that `wasm-opt`;
another version gives a slightly different binary. Both need network access once. On
an offline machine, install Binaryen yourself (`npm install -g binaryen`
provides `wasm-opt`), or build without optimisation:
`node scripts/build-wasm.mjs --no-opt`.

`scripts/build-wasm.mjs` fetches `wasm-bindgen` itself because
wasm-pack has no prebuilt copy for Apple Silicon Macs. There, wasm-pack
would compile `wasm-bindgen-cli` from source, which takes minutes and
prints future-incompatibility warnings about that tool's own
dependencies (`buf_redux`, `multipart`). A matching `wasm-bindgen` already
on your `PATH` is used as it is. If the fetch fails, the script says why
and wasm-pack falls back to compiling it.

The downloaded archive must match the SHA-256 recorded for its version
and platform in `scripts/wasm-bindgen-sha256.json`, or the script refuses
it and wasm-pack compiles `wasm-bindgen` from source instead. The
checksums were taken from the official release archives when the version
was locked. When Dependabot or anyone moves `wasm-bindgen` to a new
version, add that release's five checksums in the same change, after
downloading them from the release page; until then the prebuilt binary
is refused.

### Release toolchain

Release artefacts, and the sizes in `scripts/size-budget.json`, come from
these versions, which CI pins:

| Tool | Version |
| --- | --- |
| Rust | 1.97.0 |
| wasm-bindgen | 0.2.129 (from `Cargo.lock`) |
| wasm-pack | 0.15.0 |
| wasm-opt (Binaryen) | 117 |
| Node.js | 24 |

Other versions build and pass the tests, but may give a WASM a few bytes
different in size.

### Fresh setup

From a new clone, these commands install everything, build, and run every
check:

```bash
rustup target add wasm32-unknown-unknown
npm ci                   # exact dependency versions from package-lock.json
npm run build            # WASM package and TypeScript wrapper into dist/
cargo test --workspace   # Rust tests, including shader validation
npm test                 # JavaScript and TypeScript tests
npm run lint:indent      # 2-space indentation and no tabs
npm run dev              # the demo at http://127.0.0.1:5173/
```

Use `npm ci` rather than `npm install` for a fresh setup: it installs
exactly the versions in `package-lock.json`. Development dependencies are
declared as caret ranges, so `npm install` can move them to newer minor
releases; Dependabot proposes those updates weekly.

### A macOS/Homebrew toolchain quirk

If your `cargo`/`rustc` resolve to a Homebrew-installed Rust (check with
`which cargo`), it will have **no `wasm32-unknown-unknown` standard
library**, even after `rustup target add wasm32-unknown-unknown` reports
success — that command installs the target into the *rustup* toolchain, not
the Homebrew one. Native builds/tests work fine with Homebrew's `cargo`
regardless (they only need the host target); only wasm32 builds need the
fix below.

To build/check the wasm32 target in that situation, put the rustup
toolchain's `bin` directory first on `PATH` for that one command:

```bash
PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH" \
  cargo check --target wasm32-unknown-unknown -p vista_wasm
```

(Substitute your own rustup toolchain directory name if it differs.)

## Building

```bash
npm run clean       # remove dist/ and any stale build output
npm run build:wasm  # scripts/build-wasm.mjs: wasm-pack build, with a prebuilt wasm-bindgen
npm run build:ts    # tsc: tsconfig.json, tsconfig.strip.json, tsconfig.glue.json
npm run build       # clean + build:wasm + build:ts, in order
npm run build:demo  # build + copy dist/ into demo/dist/ (see below)
```

`npm run build` is what the published package's `dist/` is generated from,
and is also what the demo/example dev servers expect to already exist —
run it once before `npm run dev*` (see
[`docs/architecture.md`](architecture.md) and the root
`vite.config.ts` aliasing note below).

`js/src/*.ts`'s relative imports/exports always include an explicit `.js`
extension (e.g. `from "./errors.js"`, even though the source file is
`errors.ts`) — this is deliberate, not a typo. TypeScript's `Bundler`
module resolution accepts this and still resolves it to the sibling
`.ts` file for type-checking, but the *compiled* `dist/*.js` output then
also has real `.js` extensions on its own internal imports, which is what
lets a plain browser load `dist/index.js` directly via native ES modules
(no bundler, no Node resolution) — see `demo/`, below. Omitting the
extension compiles fine and works under Vite/Node, but silently breaks
for any consumer loading the compiled output directly in a browser (a real
regression this repository has hit once already).

### `demo/`: a plain static site, not a Vite app

Unlike `examples/`, `demo/` is intentionally
**not** a bundled app — `demo/src/main.js` is plain JavaScript, and
`demo/index.html` resolves `@vista-wasm/vista-wasm` via a native browser
[import map](https://developer.mozilla.org/en-US/docs/Web/HTML/Reference/Elements/script/type/importmap)
pointing at `./dist/index.js`, not a bundler alias. `npm run build:demo`
builds the package and copies the real output into `demo/dist/` (see
`scripts/build-demo.mjs`), after which `demo/` is fully self-contained and
can be served by *any* static file server (`python3 -m http.server` run
from inside `demo/`, `npx serve demo`, GitHub Pages, and so on) with zero
Node/build step at request time. `npm run dev` (Vite) still works against
`demo/` too, for convenient local hot-reload during development — Vite's
own alias resolves the bare specifier during dev, and the import map is
simply unused in that mode (the two do not conflict). See
`.github/workflows/deploy-demo.yml` for the GitHub Pages deployment this
enables (the demo is published at `demo/` on the project site), and [`CONTRIBUTING.md`](../CONTRIBUTING.md#deploying-the-demo)
for the deployment-facing summary.

If you change anything under `demo/` (new controls, wiring), mirror the
same change into `demo/src/main.js` directly (there is no `.ts` source to
compile from any more) and keep it in sync with `examples/vanilla/src/main.ts`
where the two overlap, per this repo's near-duplicate convention for those
two.

## Testing

```bash
cargo test --workspace   # native Rust tests (both crates), no wasm32 needed
npm test                 # vitest, the JS/TS test suite
npm run lint:indent      # repo-wide 2-space indentation / no-tabs check
```

All three must pass before considering a change complete. Also run
`cargo fmt --check`.

### Continuous integration

`.github/workflows/checks.yml` runs on every push and pull request, in
parallel jobs, and every job must pass. The deploy workflow calls it and
publishes the site only when it is green. It runs exactly what you can
run locally:

| Job | Commands |
| --- | --- |
| Rust | `cargo fmt --all --check`; `cargo clippy --workspace --lib -- -D warnings -D clippy::unwrap_used -D clippy::expect_used -D clippy::panic`, natively and with `--target wasm32-unknown-unknown`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace` |
| JavaScript | `npm ci`, `npm run build`, `npx tsc -p tsconfig.json --noEmit`, `npx vitest run` (type tests included), `npm run lint:indent` |
| Size | a clean `npm run build`, then `node scripts/check-size.mjs` |
| Audits | `npm audit --audit-level=low` (root and `bench/`), `cargo audit --deny warnings` |
| Fuzzers | the Rust and JavaScript fuzzers at their full case counts |
| Browser | `node scripts/visual-check/smoke.mjs`: three scenes, the render loop and a forced GPU error in headless Chromium with software WebGPU |

Clippy's warning baseline is zero: any warning fails the job. Library
code may not `unwrap`, `expect` or `panic!`; tests opt out with their own
`allow`.

`scripts/check-size.mjs` compares the gzipped size (`gzip -9 -c`) of the
WASM and every `dist/*.js` with its budget in `scripts/size-budget.json`,
prints the change for each, and fails when a file is over. Raising a
budget is a reviewed change to that file; lower it when a change makes a
file smaller, so the saving stays. `scripts/build-wasm.mjs` remaps the
Cargo home in embedded source paths to `/cargo`, so the WASM is the same
byte for byte wherever it is built.

The browser smoke test renders the default scene, the fixed raw
heightmap and an alpine river scene, runs the render loop for 20
seconds, and checks that a forced GPU validation error arrives as a
`gpuError` event. It fails on any page or console
error, any console warning (a WGSL error is only a warning and a black
frame), any error the page logged, or a black frame. It is the only CI
coverage of the WebGPU paths. Run it locally from a static server at the
repository root:

```bash
python3 -m http.server 8124 &
node scripts/visual-check/smoke.mjs /tmp/smoke
```

`cargo test --workspace` parses and validates every `.wgsl` shader with
naga (see `render/shaders.rs`), with `common.wgsl` prepended exactly as at
run time, so syntax, type, and binding errors fail the tests. It cannot
run the GPU, though: a shader with a logic bug still compiles cleanly and
only shows up on screen. For any change that touches `render/gpu.rs` or a
`.wgsl` file, also do the in-browser verification below. It is the only
way to catch rendering bugs.

`build.rs` strips comments and indentation from each shader into
`OUT_DIR` at build time, so shader comments cost nothing in the shipped
binary. Write them freely. It also renames what the shaders declare
(functions, structs and members, constants, variables and parameters) to
names of one to three characters, from one map for every shader, so long,
clear names cost nothing either. Entry points and `override` constants
keep their names, as the engine asks for them by name. A WGSL error in
the browser therefore names the short names: look a name up in
`OUT_DIR/shader_names.rs`, the map the build writes beside the shaders
(`target/<target>/<profile>/build/vista_wasm-*/out/`), or reproduce the
error with `cargo test`, whose naga messages quote the same source.

### Dependency audits

The dependencies are kept at their latest releases (Dependabot proposes
updates weekly) and must carry no known advisories. Check both
ecosystems, and `bench/` on its own:

```bash
npm audit                     # root: must report 0 vulnerabilities
(cd bench && npm audit)       # bench/: likewise
npm outdated; (cd bench && npm outdated)
cargo install --locked cargo-audit cargo-outdated   # once; local tools, not dependencies
cargo audit                   # must report no vulnerabilities
cargo outdated -R             # direct dependencies behind their latest release
```

An advisory with no released fix is recorded in
[`docs/security.md`](security.md), with the reason and the scope it
affects. The `wasm-bindgen` crates move together, and with the
`wasm-pack` in CI; `wgpu` and `naga` move together.

### In-browser verification

```bash
npm run dev          # demo, at http://127.0.0.1:5173/
npm run dev:vanilla  # vanilla example
npm run dev:react
npm run dev:vue
npm run dev:svelte
npm run dev:threejs   # three.js overlay example
npm run dev:babylonjs # Babylon.js overlay example
```

Each of these needs a built `dist/`. If it is missing, the matching
`predev*` script (`scripts/ensure-built.mjs`) runs `npm run build` first,
and it warns when `crates/` or `js/src/` changed after the last build. The
examples/demo alias
`@vista-wasm/vista-wasm` to the *built* `dist/index.js`, not the raw
TypeScript source, since the WASM loader in `js/src/index.ts` resolves its
`.wasm`/glue files relative to its own module URL, which only exist next to
the built output.

For a shader or rendering change specifically, load the affected
control(s) in a real browser, toggle them through their full range, and
visually confirm the result — a screenshot is the actual test here, not a
green terminal.

### Headless visual checks

`scripts/visual-check/` renders the engine in headless Chromium with
software WebGPU and saves PNGs, so rendering changes can be checked
without a GPU. Build first, then serve the repository root:

```bash
npm run build
python3 -m http.server 8124 &
node scripts/visual-check/capture.mjs shot.png '{"size":[960,600]}' 3
```

The config JSON can set the engine options, the fractal terrain, setter
calls (for example `{"set":{"setWeather":{"enabled":true,"state":"rain"}}}`),
the camera, and a list of `shots`; see the header of `capture.mjs`. The
script exits non-zero on any page or console error, which is how shaders
rejected by Chrome's WGSL compiler show up. Playwright is not a
dependency: set `PLAYWRIGHT_MODULE` to its `index.mjs` if it cannot be
resolved, and `CHROMIUM_PATH` to a Chromium binary. Software rendering
takes seconds per frame, so compare relative pass times, not absolute
ones.

### Performance gate

`scripts/visual-check/fixed-scene.mjs` renders one fixed scene clear,
then in rain with lens drops, then at -18 °C looking out over pack ice,
and prints the GPU time of every pass, averaged over 6 frames after 40
warm-up frames:

```bash
node scripts/visual-check/fixed-scene.mjs          # add out.png to save the frames
```

The terrain is a committed heightmap (`fixed-512.f32.gz`), loaded with
`loadRawHeightmap`, so generator changes leave the scene alone. Dynamic
resolution is off. Run it before and after a rendering change, on the
same machine, and compare pass by pass: a pass more than 5 % slower needs
a reason and a budget.

These modes cover vegetation:

```bash
node scripts/visual-check/fixed-scene.mjs --grass           # grass on, at its default density
node scripts/visual-check/fixed-scene.mjs --no-grass        # grass off
node scripts/visual-check/fixed-scene.mjs --jungle --split  # dense jungle, trees pass split
node scripts/visual-check/fixed-scene.mjs --jungle --scene=closeUp
node scripts/visual-check/fixed-scene.mjs --sky             # the sky views over the sea
node scripts/visual-check/fixed-scene.mjs --meadow=0.5      # the meadow from 2 to 60 m up
```

`--jungle` renders the same heightmap warm and wet (27 °C, moisture
bias 0.7), with trees at density 4 and grass on, from three cameras: a
close-up 6 m up inside the stand, a clearing edge from 20 m and a
hillside from 300 m. `--split` times the trees pass as canopy meshes,
understorey meshes and impostors, and each scene also prints the tree
triangles drawn. `--scene=<name>` measures one scene only. To estimate a
frame at 1080p on a mid-range GPU, scale every pass by 1.6 ms over the
terrain pass's time: the terrain pass costs about 1.6 ms there.

`--sky` renders the same heightmap from over the sea at (0, 40, -4000):
cumulus from below, a low sun, a mackerel sky, a veiled sun and the rain
deck. Their names hold spaces, so the lines and `--scene=` use hyphens
(`--sky --scene=rain-deck`). `--meadow=<density>` renders the grass
meadow on generated rolling hills at that density, with the camera 2, 4,
10, 25, 30 and 60 m up; its lines are named by height
(`--meadow=4 --scene=25m`). Every scene is defined in
`scripts/visual-check/scenes.mjs`.

### Profile page

`bench/gpu/` measures the same scenes on a real GPU: every pass's median
and 90th percentile over 120 frames at 1920 x 1080, as a table and as
JSON. Run it with `npm run gpu` from `bench/` after a build.
[`performance.md`](performance.md) explains how to run it on other
devices, lists every budget beside its scene, and holds the results.
`scripts/visual-check/profile-check.mjs` runs the page headless under
software WebGPU and fails on any console error; the pure helpers it
reports with (`bench/gpu/stats.mjs`) are tested in
`js/tests/profile-stats.test.ts`.

## Project structure

```text
crates/
  vista_types/   # shared serialisable types, no WebGPU dependency
  vista_wasm/    # the engine: terrain, camera, and all render/ pipelines
js/
  src/           # the published TypeScript wrapper (index.ts, types.ts, ...)
  tests/         # vitest suite for js/src
demo/            # the full-featured demo app
examples/        # vanilla, react, vue, svelte (same feature set), threejs and babylonjs
bench/           # size and speed comparisons, with their own package.json
docs/            # this documentation
```

See [`docs/architecture.md`](architecture.md) for how the two Rust crates
and the JS wrapper fit together, and [`docs/options-reference.md`](options-reference.md)
for the full current public option surface if you are adding to it.

## Code style

- **2-space indentation everywhere** (Rust, TypeScript, WGSL, JSON, YAML,
  Markdown code blocks) — no tabs, ever. `npm run lint:indent` enforces
  this repo-wide, including inside Markdown files; numbered-list
  continuation lines in Markdown must use 4-space indents (not the
  CommonMark-conventional 3), since the linter only recognises even-numbered
  indent steps.
- **British English** in every comment, doc string, and user-facing message
  (`optimise`, `behaviour`, `colour`, `initialise`, and so on) — see
  [`AGENTS.md`](../AGENTS.md) for the full spelling list.
- No image files ship with the engine. Every texture (terrain materials,
  bark and leaves, water ripples, noise) is generated on the GPU at
  start-up, and tree models are built from code. Hosts may replace them
  through the hooks (see [`docs/hooks.md`](hooks.md)), but keep new
  rendering features procedural rather than adding the first bundled
  asset.
- Validate every new public numeric/enum option in `config.rs`, the same
  way existing ones are (`validate_finite`, `validate_positive`,
  `validate_non_negative`, `validate_range`) — JavaScript input is
  untrusted, and any value that bounds a shader loop or allocation size
  must be hard-clamped server-side, not just documented as "please don't
  set this too high".

## Adding a new public option end-to-end

If you are adding a new engine option (as opposed to fixing a bug), the
current codebase touches roughly this many places, in this order:

1. `crates/vista_types/src/lib.rs` — the new field/struct/enum, with a
    `Default` impl. Use `#[serde(default)]` on new fields added to an
    *existing* struct so old serialised options without the new key still
    deserialise correctly.
2. `crates/vista_wasm/src/config.rs` — validation.
3. `crates/vista_wasm/src/engine.rs` — a field on `EngineCore`, a
    `set_*`/getter as needed, and wiring into `render_once()` if it affects
    per-frame GPU state.
4. `crates/vista_wasm/src/api.rs` — the `#[wasm_bindgen]` method, if it is
    a new top-level setter.
5. `crates/vista_wasm/src/render/gpu.rs` and the relevant `shaders/*.wgsl`
    files, if it affects rendering — remember `FrameUniforms` fields are
    append-only across every shader that shares the bind group (see
    [`docs/architecture.md`](architecture.md#rendering)).
6. `js/src/types.ts` and `js/src/index.ts` — the TypeScript type and any
    wrapper method, including the `pendingCall` reentrancy guard on any new
    synchronous setter (see
    [`docs/events-errors-and-lifecycle.md`](events-errors-and-lifecycle.md#reentrancy)).
7. `demo/index.html` + `demo/src/main.js`, and the matching controls in
    each of `examples/{vanilla,react,vue,svelte}` — the demo and vanilla
    example are near-duplicates by design (same DOM element IDs), so those
    two are usually a near-identical pair of edits; the framework examples
    each have their own idiomatic wiring around the same underlying calls.
8. This documentation — at minimum
    [`docs/options-reference.md`](options-reference.md), plus whichever
    topic guide covers that system.

Rust tests live in `crates/vista_wasm/tests/` (integration) and
`#[cfg(test)] mod tests` blocks next to the code they cover (unit). JS
tests live in `js/tests/`, mirroring `js/src/`.

## Pull request checklist

Before considering a change complete:

- [ ] `cargo test --workspace` passes.
- [ ] `cargo check --target wasm32-unknown-unknown -p vista_wasm` passes
      (using the `PATH` fix above if needed).
- [ ] `npm run build` succeeds end-to-end.
- [ ] `npm test` passes.
- [ ] `npm run lint:indent` passes.
- [ ] Any shader/rendering change has been visually verified in a real
      browser, not just compiled.
- [ ] Public API changes are additive (new optional fields/methods) unless
      a breaking change was explicitly requested and discussed.
- [ ] Documentation affected by the change has been updated — see
      [`docs/options-reference.md`](options-reference.md) in particular,
      since it is the one place every public field is enumerated.
