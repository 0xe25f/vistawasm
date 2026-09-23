# Getting Started With VistaWASM

This is the practical, ground-up walkthrough: install the package, create an
engine, generate terrain, and render your first frame. For *what each
control does creatively*, see [`docs/world-design-guide.md`](world-design-guide.md).
For *the exact shape of every option*, see
[`docs/options-reference.md`](options-reference.md).

## 1. Requirements

VistaWASM targets modern 2025-and-newer browsers with WebGPU. It does not
fall back to WebGL2 — if WebGPU is unavailable, engine creation fails with a
clear `WEBGPU_UNAVAILABLE` error rather than silently degrading.

You need:

- A secure context: HTTPS, or `http://localhost`/`http://127.0.0.1` for
  local development.
- A browser with WebGPU enabled (current Chrome/Edge by default; other
  browsers vary — check before shipping).
- Native ES module support (no legacy bundler targets required).
- `.wasm` served with the `application/wasm` MIME type. Vite, and most
  modern dev servers/CDNs, do this automatically; if you host the package's
  `dist/pkg/vista_wasm_bg.wasm` yourself behind a custom server, confirm the
  MIME type explicitly.

Check support *before* you show any VistaWASM UI, so you can offer a
sensible fallback instead of a blank canvas:

```ts
import { detectVistaWasmSupport } from "@vista-wasm/vista-wasm";

const support = detectVistaWasmSupport();

if (!support.webGpu) {
  showUnsupportedBrowserMessage();
}
```

## 2. Install

```bash
npm install @vista-wasm/vista-wasm
```

The package ships pre-built: JavaScript/TypeScript glue in `dist/`, and the
compiled WebAssembly binary in `dist/pkg/`. You do not need Rust, `wasm-pack`,
or any build step to *consume* the package — those are only needed if you
are building VistaWASM itself from source (see
[`docs/testing-and-contributing.md`](testing-and-contributing.md)).

## 3. Create an engine

Every VistaWASM engine owns exactly one `<canvas>`. Give it one sized to
where you want the world to render:

```html
<canvas id="vista" style="width: 100%; height: 100%;"></canvas>
```

```ts
import { createVistaEngine } from "@vista-wasm/vista-wasm";

const canvas = document.querySelector<HTMLCanvasElement>("#vista");

if (!canvas) {
  throw new Error("Missing canvas element.");
}

const engine = await createVistaEngine(canvas, {
  render: {
    width: canvas.clientWidth,
    height: canvas.clientHeight,
    devicePixelRatio: window.devicePixelRatio
  }
});
```

`createVistaEngine()` loads the WASM module (once per page, cached after
the first call), requests a WebGPU adapter/device, and configures the
canvas's WebGPU surface. It rejects with a `VistaWasmError` if any of that
fails — wrap it in a `try`/`catch` in real code (see
[`docs/events-errors-and-lifecycle.md`](events-errors-and-lifecycle.md) for
the full error code reference).

You can pass initial `camera`, `sun`, `atmosphere`, `water`, `flora`,
`grass`, `clouds`, `mist`, `quality`, `biomes`, `weather`, `shadows`, and
`surface` options here too, instead of
calling the matching `set*()` method immediately afterwards — see
[`docs/options-reference.md`](options-reference.md).

## 4. Generate terrain

```ts
const handle = await engine.generateFractal({
  seed: 12345,
  size: 1024,
  horizontalScaleMetres: 12,
  verticalScale: 1,
  noise: {
    kind: "ridged",
    octaves: 7,
    gain: 0.5,
    lacunarity: 2
  }
});

console.log(handle.metadata.minHeightMetres, handle.metadata.maxHeightMetres);
```

`generateFractal()` is async. It takes from tens of milliseconds for a
small terrain to much longer for a large one with erosion. While it runs,
`set*()` calls are skipped (see
[Reentrancy](events-errors-and-lifecycle.md#reentrancy)), so await it
before configuring the scene. It returns a
`TerrainHandle` with `metadata` describing the real height range, sea level,
and sample spacing of what was generated, which you will need for camera
placement, sea level tuning, and gameplay height queries.

To load a real-world elevation model instead of generating one, see
[`docs/terrain-data.md`](terrain-data.md).

## 5. Place a camera and start rendering

```ts
// Look across the terrain from above its highest peak, near one edge.
const { maxHeightMetres, width, metresPerSample } = handle.metadata;
const start: [number, number, number] = [0, maxHeightMetres + 400, (width - 1) * metresPerSample * 0.45];

engine.setCamera({
  position: start,
  target: [0, maxHeightMetres * 0.3, 0],
  fieldOfViewDegrees: 55
});

engine.start();
```

Heights depend on the seed and options (fractal terrain spans up to about
±900 m × `verticalScale`), so place the camera from `handle.metadata`
rather than fixed numbers, which can end up inside a hill.

`engine.start()` runs VistaWASM's own `requestAnimationFrame` loop. If you'd
rather drive rendering from your own game loop, call `engine.renderOnce()`
per frame instead — see [`docs/game-development.md`](game-development.md).

For an interactive fly camera instead of a fixed one, use the bundled
controller:

```ts
import { attachFlyCameraControls } from "@vista-wasm/vista-wasm";

const controls = attachFlyCameraControls(engine, canvas, {
  initialPosition: start,
  initialYawDegrees: 180, // face −z, towards the centre
  initialPitchDegrees: -15
});
```

See [`docs/camera-and-controls.md`](camera-and-controls.md) for every
option and for writing your own player-relative camera controller.

## 6. Handle resizing and disposal

```ts
const observer = new ResizeObserver(() => {
  const rect = canvas.getBoundingClientRect();
  engine.resize(Math.round(rect.width), Math.round(rect.height), window.devicePixelRatio);
});
observer.observe(canvas);

window.addEventListener("beforeunload", () => {
  observer.disconnect();
  controls.dispose();
  engine.stop();
  engine.dispose();
});
```

`dispose()` is terminal — releases every GPU resource and cannot be undone.
Always call `stop()` (or stop your own render loop) before disposing so no
frame renders against a disposed engine.

## 7. Deploy

- Serve your site over HTTPS (or `localhost` while developing). WebGPU
  only works in a secure context.
- Serve `.wasm` files as `application/wasm`. Most hosts and CDNs do this
  already; if yours does not, VistaWASM falls back to a slower load path
  and still works, but fix the header for production.
- Elevation files (`loadDemFromUrl()`) fetched from another origin need
  CORS headers on that origin.
- VistaWASM does not use threads today, so you do not need the
  `Cross-Origin-Opener-Policy` and `Cross-Origin-Embedder-Policy` headers.
  If a future release adds threaded builds, it will need:

  ```text
  Cross-Origin-Opener-Policy: same-origin
  Cross-Origin-Embedder-Policy: require-corp
  ```

Using a framework? See [`docs/frameworks.md`](frameworks.md) for complete
React, Vue, and Svelte components.

## 8. Where to go next

- [`docs/README.md`](README.md) — the full documentation index.
- [`docs/frameworks.md`](frameworks.md) — React, Vue, and Svelte
  components.
- [`docs/threejs.md`](threejs.md) — using VistaWASM with three.js.
- [`docs/options-reference.md`](options-reference.md) — every public
  option, its type, default, and validation rule, in one table-driven
  reference.
- [`docs/world-design-guide.md`](world-design-guide.md) — what each
  terrain/erosion/sky control does creatively, with worked recipes.
- [`docs/terrain-data.md`](terrain-data.md) — fractal generation data
  model, DEM/GeoTIFF import, and raw heightmap loading.
- [`docs/vegetation.md`](vegetation.md) — trees and grass, quality tiers,
  and performance.
- [`docs/sky-atmosphere-and-weather.md`](sky-atmosphere-and-weather.md) —
  sun, atmosphere, clouds, cloud types, and mist.
- [`docs/weather.md`](weather.md) — weather states, transitions, and
  automatic cycling.
- [`docs/shadows.md`](shadows.md) — terrain, tree, and cloud shadows.
- [`docs/hooks.md`](hooks.md) — replacing tree models, placement, species
  mixes, and textures with your own.
- [`docs/biomes.md`](biomes.md) — biomes and how to shape them.
- [`docs/water.md`](water.md) — ocean waves, currents, rivers, and lakes.
- [`docs/camera-and-controls.md`](camera-and-controls.md) — camera model
  and the bundled fly-camera controller.
- [`docs/render-quality-and-diagnostics.md`](render-quality-and-diagnostics.md) —
  quality settings, render statistics, and debug overlays.
- [`docs/export-and-snapshots.md`](export-and-snapshots.md) — heightmap,
  PNG, OBJ, and screenshot export helpers.
- [`docs/events-errors-and-lifecycle.md`](events-errors-and-lifecycle.md) —
  the engine lifecycle, every event, and every error code.
- [`docs/game-development.md`](game-development.md) — using VistaWASM as
  the terrain layer under a real game (game loops, height queries,
  collision, error handling).
- [`docs/engine-integration.md`](engine-integration.md) — integrating with
  other web game engines (Three.js, Babylon.js, PlayCanvas).
- [`docs/architecture.md`](architecture.md) — internals, for anyone
  contributing to VistaWASM itself.
