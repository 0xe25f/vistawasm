# Examples

Small browser apps that use VistaWASM. Each folder is a Vite project that
shares this repository's install and its root `vite.config.ts`.

| Example | What it shows |
| --- | --- |
| [`vanilla/`](vanilla/) | Plain TypeScript: generate terrain with live controls, then export a screenshot, a heightmap image, an OBJ or raw heights |
| [`react/`](react/) | A React component that owns an engine |
| [`vue/`](vue/) | The same as a Vue component |
| [`svelte/`](svelte/) | The same as a Svelte component |
| [`threejs/`](threejs/) | three.js objects drawn over a VistaWASM landscape |
| [`babylonjs/`](babylonjs/) | Babylon.js objects drawn over a VistaWASM landscape |

## Running one

From the repository root:

```sh
npm ci
npm run build
npm run dev:vanilla   # or dev:react, dev:vue, dev:svelte, dev:threejs, dev:babylonjs
```

Then open `http://127.0.0.1:5173/` in a browser with WebGPU, such as
current Chrome or Edge. The examples load the built `dist/`, exactly as an
app that installed the package would, so run `npm run build` again after
changing Rust or `js/src`.

A production build works too, and writes the `.wasm` file beside the
JavaScript:

```sh
npx vite build --config vite.config.ts examples/vanilla
```

Serve the output over HTTP, with `.wasm` as `application/wasm`. Opening the
files from `file://` does not work.
