# Contributing to VistaWASM

Thank you for helping. Bug reports, fixes, new features, documentation,
and benchmarks are all welcome. This page gets you from a fresh clone to a
running demo in a few minutes; the detailed reference is
[`docs/testing-and-contributing.md`](docs/testing-and-contributing.md).

Please read [`AGENTS.md`](AGENTS.md) too. Despite the name, it is the
project's style guide for everyone: 2-space indentation, British English,
validated inputs at the JavaScript boundary, and documented public
behaviour.

## What you need

| Tool | Version |
| --- | --- |
| [Rust](https://rustup.rs), via rustup | 1.87 or newer |
| Node.js (with npm) | 20.19+ or 22.12+ |
| [wasm-pack](https://rustwasm.github.io/wasm-pack/installer/) | 0.13 or newer |
| A browser with WebGPU | Current Chrome or Edge, for trying your changes |

## Set up

```bash
git clone https://github.com/0xe25f/vistawasm.git
cd vistawasm
rustup target add wasm32-unknown-unknown
npm ci
npm run build
npm run dev          # the demo, at http://127.0.0.1:5173/
```

`npm ci` installs the exact versions in `package-lock.json`; prefer it to
`npm install`. The first build downloads the `wasm-bindgen` CLI and
Binaryen's `wasm-opt`; for offline machines, see
[Prerequisites](docs/testing-and-contributing.md#prerequisites).

## Everyday commands

```bash
npm run build            # rebuild after changing Rust or js/src
cargo test --workspace   # Rust tests, including validation of every shader
npm test                 # JavaScript and TypeScript tests
npm run lint:indent      # 2-space indentation, no tabs
cargo fmt --check        # Rust formatting
```

All of these must pass before a pull request is ready.

## Running the examples

Every example under `examples/` is a real Vite project that shares this
repository's `npm install` and root `vite.config.ts`. Build once, then
start the one you want:

```bash
npm run dev:vanilla
npm run dev:react
npm run dev:vue
npm run dev:svelte
npm run dev:threejs
```

Each starts a dev server on `http://127.0.0.1:5173`. The demo and
examples load the built `dist/` output, exactly as an app that installed
the package from npm would, so run `npm run build` again after changing
Rust or `js/src`.

## Where things live

| Path | What it is |
| --- | --- |
| `crates/vista_types` | Public option and result types, shared by Rust and JavaScript |
| `crates/vista_wasm` | The engine: terrain, rendering, weather, and the WASM API |
| `crates/vista_wasm/src/shaders` | WGSL shaders; `common.wgsl` is prepended to every render shader |
| `js/src` | The TypeScript wrapper published to npm |
| `demo` | The demo site (plain JavaScript, no build step) |
| `examples` | Framework examples |
| `bench` | Size and speed comparisons, with their own `package.json` |
| `docs` | Guides and reference ([index](docs/README.md)) |

[`docs/architecture.md`](docs/architecture.md) explains how the engine
fits together, frame by frame.

## Changing rendering or shaders

Tests catch shader syntax, type, and binding errors, but not a shader that
compiles and draws the wrong thing. For any change to `render/gpu.rs` or a
`.wgsl` file, look at the result in a browser: run the demo, move the
affected controls through their full range, and check the picture.

## Adding a public option

Follow
[Adding a new public option end-to-end](docs/testing-and-contributing.md#adding-a-new-public-option-end-to-end):
the Rust type, validation, the engine, the TypeScript types, the demo, the
options reference, and the changelog all move together.

## Before you open a pull request

- The commands above pass.
- New behaviour has tests, and public changes are documented in `docs/`
  and [`CHANGELOG.md`](CHANGELOG.md) under "Unreleased".
- The demo still runs, and you have looked at any visual change.
- Commits are focused, and unrelated files are left alone.

## Deploying the demo

`.github/workflows/deploy-demo.yml` builds the package, copies it into
`demo/dist/` (`npm run build:demo`), and publishes `demo/` to GitHub Pages
on every push to `main`. To host the demo elsewhere, run
`npm run build:demo` and serve the `demo/` folder from any static file
server.
