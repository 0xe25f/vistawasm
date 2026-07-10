# VistaWASM

> VistaWASM turns a plain web canvas into a live 3D landscape — hills, trees, water, and sky. Point it at a canvas, call a few functions, and the engine does the rest.

[![GitHub Stars](https://img.shields.io/github/stars/0xe25f/vistawasm?style=social)](https://github.com/0xe25f/vistawasm/stargazers)
[![npm](https://img.shields.io/npm/v/%40vista-wasm%2Fvista-wasm)](https://www.npmjs.com/package/@vista-wasm/vista-wasm)
[![Licence: AGPL v3](https://img.shields.io/badge/licence-AGPL--3.0-blue.svg)](LICENSE)

If VistaWASM saves you time, please ⭐ **[star it on GitHub](https://github.com/0xe25f/vistawasm)** — it helps others find the project.

VistaWASM is a Rust and WebAssembly terrain engine for modern browsers. It
generates seeded fractal landscapes, loads terrain height data, and owns a
WebGPU render surface from a canvas supplied by the host frontend.

It is inspired by classic landscape projectors: choose or load a landscape,
place a camera, set the light and atmosphere, then explore the result.

## Current Status

This repository contains a working terrain engine with a browser-first API:

- Rust workspace with shared public types and a WASM engine crate.
- `wasm-bindgen` browser API wrapped by TypeScript.
- WebGPU device and surface setup through Rust `wgpu`.
- Deterministic Rust terrain generation with seeded fractal controls,
  including noise kind, island/terrace/basin/canyon/crater shaping, and
  hydraulic/thermal erosion. Erosion runs as GPU compute passes on browser
  builds (falling back to the CPU implementation automatically if the GPU
  pass fails for any reason), and as CPU passes on native/test builds.
- Raw heightmap loading.
- Uncompressed GeoTIFF header, strip, scale, tiepoint, and no-data handling.
- Camera/projector maths, plus a ready-made `attachFlyCameraControls()` fly
  camera (drag to look, WASD to move, Space/Shift or middle-mouse-drag to
  rise and fall, scroll wheel to zoom the field of view).
- A real terrain render pipeline: CPU-baked height, normal, and material
  data are uploaded as a single mesh and drawn with a lit, depth-tested
  WebGPU pipeline (`shaders/clipmap_render.wgsl`). The mesh recentres on the
  camera and uses exponentially increasing sample spacing with distance
  from that centre, giving full detail near the camera and much greater
  reach than a uniform mesh of the same vertex budget, without the seams a
  multi-tier clipmap needs skirts to hide (see
  [`docs/architecture.md`](docs/architecture.md#terrain-rendering-and-level-of-detail)).
- A real atmosphere: an analytic Rayleigh/Mie sky dome
  (`shaders/atmosphere.wgsl`) with a sun disc/glare, horizon haze blend,
  below-horizon darkening, and an optional cloud layer (`off`/`painted`/
  `volumetric`), driven by the same sun/atmosphere controls as the terrain
  fog.
- Ground mist: an independent height-based ground fog (`off`/`flat`/
  `volumetric`, distinct from the distance-only haze above) that pools in
  valleys and can rise off the water surface, applied consistently across
  terrain, flora, grass, and water.
- Real vegetation: deterministic flora placement (`render/flora.rs`) drawn
  as tree billboards (`shaders/flora_instances.wgsl`) with a `billboard`/
  `cross-quad`/`mesh` quality dial, per-tree canopy variety, and wind sway,
  respecting slope, tree line, water level, and density. An independent
  grass ground-cover layer (`render/grass.rs`, opt-in) scatters crossed
  blade tufts driven by the terrain's own material weights, with a
  configurable view-distance fade.
- Real water: an animated, fresnel-shaded water plane (`shaders/water.wgsl`)
  sized to the terrain footprint and driven by the live sea level control.
- Terrain export helpers: a top-down hypsometric minimap/PNG export
  (`renderHeightmapToCanvas`/`exportHeightmapImage`), a Wavefront OBJ 3D
  model export (`exportTerrainObj`), a raw heightmap download, and a canvas
  PNG screenshot (`exportSnapshot`).
- A fully-featured demo and four framework examples (vanilla, React, Vue,
  Svelte) that all expose the same rich control panel: terrain shape and
  erosion, sun and atmosphere, water, vegetation, grass, clouds, mist,
  render quality, debug views, a live minimap, and every export button
  above.

Unsupported DEM compression returns a clear error.

## Documentation

**Start here:**

- [`docs/getting-started.md`](docs/getting-started.md) — install, create an
  engine, generate terrain, and render your first frame.
- [`docs/options-reference.md`](docs/options-reference.md) — every public
  option, its type, default, and validation rule, in one reference.

**Guides, by system:**

- [`docs/world-design-guide.md`](docs/world-design-guide.md) — what each
  terrain/erosion/atmosphere control does, with worked recipes.
- [`docs/terrain-data.md`](docs/terrain-data.md) — fractal generation data
  model, GeoTIFF DEM import (exact supported subset), and raw heightmap
  loading.
- [`docs/vegetation.md`](docs/vegetation.md) — trees and grass: placement,
  quality tiers, and performance.
- [`docs/sky-atmosphere-and-weather.md`](docs/sky-atmosphere-and-weather.md) —
  sun, atmosphere, clouds, and mist (and the haze-vs-mist distinction).
- [`docs/water.md`](docs/water.md) — the water plane.
- [`docs/camera-and-controls.md`](docs/camera-and-controls.md) — the
  camera model and the bundled fly-camera controller.
- [`docs/render-quality-and-diagnostics.md`](docs/render-quality-and-diagnostics.md) —
  quality presets, render statistics, and debug overlays (including which
  are implemented today).
- [`docs/export-and-snapshots.md`](docs/export-and-snapshots.md) —
  heightmap, PNG, OBJ, and screenshot export helpers.
- [`docs/events-errors-and-lifecycle.md`](docs/events-errors-and-lifecycle.md) —
  the engine lifecycle, every event, and every error code.

**Application integration:**

- [`docs/game-development.md`](docs/game-development.md) — using VistaWASM
  as the terrain layer under a game: camera/game-loop integration, height
  queries, collision/physics, performance, and error handling.
- [`docs/engine-integration.md`](docs/engine-integration.md) — integrating
  with other web game engines (Three.js, Babylon.js, PlayCanvas), including
  when to let VistaWASM render versus exporting terrain data to another
  renderer.

**Contributing to VistaWASM itself:**

- [`docs/architecture.md`](docs/architecture.md) — internals: crate layout,
  rendering pipeline, terrain LOD strategy, and engine lifecycle.
- [`docs/testing-and-contributing.md`](docs/testing-and-contributing.md) —
  build/test commands, code style, and the checklist for adding a new
  public option end-to-end.

## Browser Requirements

VistaWASM targets modern 2025 and newer browsers with WebGPU support.

You need:

- HTTPS or localhost.
- WebGPU support.
- ES module support.
- WebAssembly support.

VistaWASM does not silently fall back to WebGL2. If WebGPU is missing, it
returns a clear error.

## Installation

```bash
npm install @vista-wasm/vista-wasm
```

## Minimal Usage

```ts
import {
  createVistaEngine,
  initialiseVistaWasm
} from "@vista-wasm/vista-wasm";

await initialiseVistaWasm();

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

await engine.generateFractal({
  seed: 12345,
  size: 2048,
  horizontalScaleMetres: 10,
  verticalScale: 1,
  noise: {
    kind: "ridged",
    octaves: 7,
    gain: 0.5,
    lacunarity: 2
  }
});

engine.setCamera({
  position: [800, 300, 800],
  target: [1200, 80, 1200],
  fieldOfViewDegrees: 55
});

engine.start();
```

## Loading A DEM

VistaWASM currently supports classic uncompressed GeoTIFF files with one sample
per pixel, stripped storage, 16-bit or 32-bit integer samples, and 32-bit float
samples. It reads `ModelPixelScaleTag`, `ModelTiepointTag`, `GeoKeyDirectoryTag`
presence, and GDAL no-data values when present.

```ts
const response = await fetch("/terrain/yosemite.tif");
const buffer = await response.arrayBuffer();

await engine.loadDemFromArrayBuffer(buffer, {
  verticalScale: 1,
  generateNormals: true,
  generateMaterialMasks: true
});
```

For DEM files served from another origin, configure CORS on that origin.

## Loading A Raw Heightmap

```ts
await engine.loadRawHeightmap(buffer, {
  width: 1024,
  height: 1024,
  sampleFormat: "float32",
  metresPerSample: 30,
  heightScaleMetres: 1,
  noDataValue: -9999
});
```

## React Example

```tsx
import { useEffect, useRef } from "react";
import { createVistaEngine, type VistaEngine } from "@vista-wasm/vista-wasm";

export function VistaPanel() {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const engineRef = useRef<VistaEngine | null>(null);

  useEffect(() => {
    let disposed = false;

    async function run() {
      const canvas = canvasRef.current;

      if (!canvas) {
        return;
      }

      const engine = await createVistaEngine(canvas);

      if (disposed) {
        engine.dispose();
        return;
      }

      engineRef.current = engine;

      await engine.generateFractal({
        seed: 9876,
        size: 2048,
        horizontalScaleMetres: 12,
        verticalScale: 1.1,
        noise: {
          kind: "simplex",
          octaves: 8,
          gain: 0.5,
          lacunarity: 2
        }
      });

      engine.start();
    }

    run().catch((error) => {
      console.error(error);
    });

    return () => {
      disposed = true;
      engineRef.current?.dispose();
      engineRef.current = null;
    };
  }, []);

  return <canvas ref={canvasRef} className="vista-canvas" />;
}
```

## Vue Example

```vue
<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import { createVistaEngine, type VistaEngine } from "@vista-wasm/vista-wasm";

const canvas = ref<HTMLCanvasElement | null>(null);
let engine: VistaEngine | null = null;

onMounted(async () => {
  if (!canvas.value) {
    return;
  }

  engine = await createVistaEngine(canvas.value);

  await engine.generateFractal({
    seed: 2222,
    size: 2048,
    horizontalScaleMetres: 10,
    verticalScale: 1,
    noise: {
      kind: "ridged",
      octaves: 7,
      gain: 0.5,
      lacunarity: 2
    }
  });

  engine.start();
});

onUnmounted(() => {
  engine?.dispose();
  engine = null;
});
</script>

<template>
  <canvas ref="canvas" class="vista-canvas" />
</template>
```

## Svelte Example

```svelte
<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { createVistaEngine, type VistaEngine } from "@vista-wasm/vista-wasm";

  let canvas: HTMLCanvasElement;
  let engine: VistaEngine | null = null;

  onMount(async () => {
    engine = await createVistaEngine(canvas);

    await engine.generateFractal({
      seed: 3333,
      size: 2048,
      horizontalScaleMetres: 10,
      verticalScale: 1,
      noise: {
        kind: "simplex",
        octaves: 7,
        gain: 0.5,
        lacunarity: 2
      }
    });

    engine.start();
  });

  onDestroy(() => {
    engine?.dispose();
    engine = null;
  });
</script>

<canvas bind:this={canvas} class="vista-canvas" />
```

## Resizing

```ts
const observer = new ResizeObserver(() => {
  const rect = canvas.getBoundingClientRect();

  engine.resize(
    Math.max(1, Math.floor(rect.width)),
    Math.max(1, Math.floor(rect.height)),
    window.devicePixelRatio
  );
});

observer.observe(canvas);
```

## Error Handling

```ts
import { VistaWasmError } from "@vista-wasm/vista-wasm";

try {
  const engine = await createVistaEngine(canvas);
} catch (error) {
  if (error instanceof VistaWasmError) {
    console.error(error.code, error.message, error.details);
  } else {
    console.error(error);
  }
}
```

## Deployment Notes

Serve the app over HTTPS or localhost.

Serve `.wasm` files with:

```text
Content-Type: application/wasm
```

If future threaded builds are enabled, use:

```text
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Embedder-Policy: require-corp
```

DEM files loaded from other origins need CORS headers.

### Deploying `demo/` as a static site (GitHub Pages)

`demo/` is a plain static site — plain JS (no TypeScript build step, no
bundler), loading `@vista-wasm/vista-wasm` via a native browser [import
map](https://developer.mozilla.org/en-US/docs/Web/HTML/Reference/Elements/script/type/importmap)
rather than a bundler alias. Once `demo/dist/` is populated (a copy of the
real compiled package output — see `npm run build:demo` below), the whole
`demo/` folder can be served by any static file server, including GitHub
Pages, with no Node or build step at request time.

```bash
npm run build:demo   # builds the wasm package + TS wrapper, then copies
                      # the output into demo/dist/ (see scripts/build-demo.mjs)
```

`.github/workflows/deploy-demo.yml` runs this in CI (Rust + wasm-pack to
compile the engine, Node + tsc to compile the TypeScript wrapper — both are
build-time-only dependencies) and publishes `demo/` to GitHub Pages via
`actions/upload-pages-artifact`/`actions/deploy-pages`. Enable "GitHub
Actions" as the Pages source in the repository's Settings → Pages, and the
demo deploys automatically on every push to `main`.

## Disposal

Always call `dispose()` when the canvas is removed.

```ts
engine.stop();
engine.dispose();
```

## Development

```bash
npm install
npm run build
npm test
npm run dev
```

Rust checks:

```bash
cargo fmt --check
cargo test
```

## Running The Examples

Every example under `examples/` is a real, runnable Vite project that shares
this repository's single `npm install` and root `vite.config.ts`. Build the
package once, then start whichever example you want:

```bash
npm install
npm run build
npm run dev:vanilla
npm run dev:react
npm run dev:vue
npm run dev:svelte
```

Each command starts a dev server on `http://127.0.0.1:5173`. Rebuild
(`npm run build`) after changing Rust or `js/src` sources, since the examples
load the built `dist/index.js` and `dist/pkg/vista_wasm.js`, matching exactly
how a real consumer installing `@vista-wasm/vista-wasm` from npm would load
the package.

## Licence

AGPL-3.0-or-later. See [LICENSE](LICENSE).

Any application that uses this library and is distributed or offered as a network service must make its complete source code available under the same licence.
