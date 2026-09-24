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
| Node.js | 20.19+ or 22.12+ | Vite 8 needs one of these. npm comes with Node. |
| wasm-pack | 0.13 or newer | `cargo install wasm-pack`, or the installer at [rustwasm.github.io/wasm-pack](https://rustwasm.github.io/wasm-pack/installer/). |

On its first run, `wasm-pack` downloads the `wasm-bindgen` CLI that
matches `Cargo.lock`, and release builds download Binaryen's `wasm-opt`
unless one is already on your `PATH`. Both need network access once. On
an offline machine, install Binaryen yourself (`npm install -g binaryen`
provides `wasm-opt`), or build without optimisation:
`wasm-pack build crates/vista_wasm --target web --out-dir ../../dist/pkg --no-opt`.

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

Use `npm ci` rather than `npm install` for a fresh setup. Several
development dependencies are declared as `latest`, so `npm install` can
pull in newer major versions than the ones the lock file was tested with.

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
npm run build:wasm  # wasm-pack build crates/vista_wasm --target web --out-dir ../../dist/pkg
npm run build:ts    # tsc -p tsconfig.json
npm run build       # clean + build:wasm + build:ts, in order
npm run build:demo  # build + copy dist/ into demo/dist/ (see below)
```

`npm run build` is what the published package's `dist/` is generated from,
and is also what the demo/example dev servers expect to already exist —
run it once before `npm run dev*` (see
[`docs/architecture.md`](architecture.md) and the examples' own
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
enables, and [`CONTRIBUTING.md`](../CONTRIBUTING.md#deploying-the-demo)
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

`cargo test --workspace` parses and validates every `.wgsl` shader with
naga (see `render/shaders.rs`), with `common.wgsl` prepended exactly as at
run time, so syntax, type, and binding errors fail the tests. It cannot
run the GPU, though: a shader with a logic bug still compiles cleanly and
only shows up on screen. For any change that touches `render/gpu.rs` or a
`.wgsl` file, also do the in-browser verification below. It is the only
way to catch rendering bugs.

`build.rs` strips comments and indentation from each shader into
`OUT_DIR` at build time, so shader comments cost nothing in the shipped
binary. Write them freely.

### In-browser verification

```bash
npm run dev          # demo, at http://127.0.0.1:5173/
npm run dev:vanilla  # vanilla example
npm run dev:react
npm run dev:vue
npm run dev:svelte
npm run dev:threejs   # three.js overlay example
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
and prints the GPU time of every pass, averaged over 6 frames after 3
warm-up frames:

```bash
node scripts/visual-check/fixed-scene.mjs          # add out.png to save the frames
```

The terrain is a committed heightmap (`fixed-512.f32.gz`), loaded with
`loadRawHeightmap`, so generator changes leave the scene alone. Dynamic
resolution is off. Run it before and after a rendering change, on the
same machine, and compare pass by pass: a pass more than 5 % slower needs
a reason and a budget.

## Project structure

```text
crates/
  vista_types/   # shared serialisable types, no WebGPU dependency
  vista_wasm/    # the engine: terrain, camera, and all render/ pipelines
js/
  src/           # the published TypeScript wrapper (index.ts, types.ts, ...)
  tests/         # vitest suite for js/src
demo/            # the full-featured demo app
examples/        # vanilla, react, vue, svelte (same feature set), and threejs
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
