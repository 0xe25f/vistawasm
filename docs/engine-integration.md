# Integrating VistaWASM With Web Game Engines

VistaWASM is a **self-contained WebGPU renderer**: it creates its own
`GPUDevice`, adapter, and surface from a `<canvas>` you give it, and it
draws terrain, flora, grass, water, and sky into that canvas every frame. It is not
a "terrain data" library that some other engine's renderer consumes
directly — this matters a lot for how you integrate it, so read this before
picking an approach.

There are two fundamentally different integration patterns. Pick based on
whether you want VistaWASM to be the thing rendering pixels at runtime, or
just the thing that *generated* your terrain.

## Pattern A — VistaWASM renders, your engine handles everything else

Use this when you want VistaWASM's actual renderer (its lighting,
atmosphere, water, flora, grass, and LOD) visible in the final frame, and your
"game engine" usage is really about UI, audio, input, entities, and
non-terrain 2D/3D overlays.

- Give VistaWASM its own `<canvas>`, sized and positioned exactly where you
  want the world to appear (typically `position: absolute; inset: 0` behind
  everything else).
- Put your other engine's canvas (Three.js, Babylon.js, PlayCanvas, Phaser,
  a 2D UI canvas, whatever) in a sibling element on top, either fully
  transparent (`alpha: true` on that engine's renderer) so VistaWASM shows
  through, or only covering part of the viewport (e.g. a HUD/minimap panel
  drawn with a 2D canvas, or a foreground 3D object layer drawn with a
  transparent-background Three.js scene).
- Synchronise cameras yourself: read your gameplay camera state once per
  frame and call both `engine.setCamera(...)` (VistaWASM) and your other
  engine's camera update with equivalent position/orientation/FOV, so the
  background terrain and foreground objects (e.g. characters, vehicles,
  particle effects rendered by the other engine) line up. There is no
  built-in camera sync between VistaWASM and another engine — you own this.
- This works well for vehicle/flight/exploration games where the terrain
  *is* the environment and other engines only render characters, effects,
  or UI on top. It does not give you shared depth-testing between
  VistaWASM's terrain and the other engine's objects (each renders to a
  fully separate WebGPU/WebGL context with no shared depth buffer) — so
  objects from the other engine will always draw fully in front of or
  fully behind VistaWASM's terrain unless you fake it (e.g. hide a
  character behind a hill by not rendering it, based on your own
  height-query against the exported heightmap — see
  [`docs/game-development.md`](game-development.md#2-player-movement-and-camera-control)).

```html
<div style="position: relative; width: 100vw; height: 100vh;">
  <canvas id="vista" style="position: absolute; inset: 0;"></canvas>
  <canvas id="overlay" style="position: absolute; inset: 0; pointer-events: none;"></canvas>
</div>
```

```ts
const vista = await createVistaEngine(document.querySelector("#vista")!);
const overlayRenderer = new THREE.WebGLRenderer({
  canvas: document.querySelector("#overlay")!,
  alpha: true
});

function tick() {
  const camera = computeSharedCamera();
  vista.setCamera(toVistaCamera(camera));
  syncThreeCamera(threeCamera, camera);
  overlayRenderer.render(overlayScene, threeCamera);
  requestAnimationFrame(tick);
}
```

### A note on WebGPU device sharing

VistaWASM creates its own `GPUDevice` internally and does not currently
expose it. If your other engine also targets WebGPU (Babylon.js's WebGPU
engine, PlayCanvas's WebGPU support, or a raw `wgpu`/WebGPU renderer of your
own) you *cannot* currently share a single device between VistaWASM and that
engine, or render both into one depth-tested pass — they will each request
their own adapter/device. Two independent WebGPU contexts on one page work
fine (this is Pattern A above), but true single-pass compositing (shared
depth buffer, objects properly occluded by terrain) would require VistaWASM
to expose its `GPUDevice`/`GPUQueue`, which it does not do today. This is a
known limitation, not an oversight — track it if you need deeper engine
interop than Pattern A provides.

## Pattern B — VistaWASM generates, your engine renders

Use this when you want to design/generate terrain with VistaWASM's noise,
shape, and erosion controls (and optionally its DEM import), but render the
final world entirely inside another engine — including on WebGL2, mobile
browsers without WebGPU, or inside an engine that already has its own
terrain/heightfield renderer you'd rather use for consistency with the rest
of your scene.

The integration surface is exactly the two export functions:

- `engine.exportHeightmap()` → raw little-endian `Float32Array` heights,
  row-major, paired with `TerrainHandle.metadata` (`width`, `height`,
  `metresPerSample`, `minHeightMetres`, `maxHeightMetres`, ...).
- `exportTerrainObj(...)` (exported from the package root, alongside
  `engine`) → a downsampled Wavefront OBJ triangle mesh string, for engines
  that want a ready-made mesh rather than a heightfield.

A typical flow: run VistaWASM headless-ish (a hidden or tiny offscreen
canvas is still required today since `createVistaEngine()` needs one, even
if you never call `engine.start()`), generate/tune terrain interactively in
a "world designer" screen, then export once and hand the data to your
runtime engine.

### Three.js: heightfield-displaced plane

```ts
import * as THREE from "three";
import { readHeightmapFloats } from "@vista-wasm/vista-wasm";

const handle = await engine.generateFractal(options);
const heights = readHeightmapFloats(engine.exportHeightmap());
const { width, height, metresPerSample } = handle.metadata;

const geometry = new THREE.PlaneGeometry(
  (width - 1) * metresPerSample,
  (height - 1) * metresPerSample,
  width - 1,
  height - 1
);
geometry.rotateX(-Math.PI / 2);

const position = geometry.attributes.position;
for (let z = 0; z < height; z += 1) {
  for (let x = 0; x < width; x += 1) {
    const vertexIndex = z * width + x;
    position.setY(vertexIndex, heights[z * width + x]);
  }
}
position.needsUpdate = true;
geometry.computeVertexNormals();

const terrainMesh = new THREE.Mesh(
  geometry,
  new THREE.MeshStandardMaterial({ color: 0x6b8f5a })
);
scene.add(terrainMesh);
```

For very large heightmaps, downsample before building the `PlaneGeometry`
(step through `heights` with a stride) rather than building one vertex per
sample — the same reasoning VistaWASM's own renderer uses internally (see
[`docs/architecture.md`](architecture.md#terrain-rendering-and-level-of-detail)).

### Babylon.js: built-in heightmap terrain

Babylon has first-class heightmap terrain support
(`MeshBuilder.CreateGroundFromHeightMap`), but it expects an *image*
(a grayscale heightmap texture), not a raw float buffer. Render VistaWASM's
own PNG minimap/heightmap export to a canvas and use that:

```ts
import { exportHeightmapImage } from "@vista-wasm/vista-wasm";

const blob = await exportHeightmapImage(handle.metadata, engine.exportHeightmap(), {
  colourMode: "grayscale" // Babylon reads pixel luminance as height, not colour
});
const url = URL.createObjectURL(blob);

const ground = BABYLON.MeshBuilder.CreateGroundFromHeightMap(
  "terrain",
  url,
  {
    width: (handle.metadata.width - 1) * handle.metadata.metresPerSample,
    height: (handle.metadata.height - 1) * handle.metadata.metresPerSample,
    subdivisions: 256,
    minHeight: handle.metadata.minHeightMetres,
    maxHeight: handle.metadata.maxHeightMetres
  },
  scene
);
```

A grayscale image loses precision compared to the raw `Float32Array` (8 bits
per channel unless you encode 16-bit across two channels yourself) — prefer
the raw-buffer approach (as in the Three.js example) if Babylon's heightmap
image path isn't precise enough for your terrain's height range.

### PlayCanvas

PlayCanvas doesn't ship a built-in heightfield terrain component; build a
plane/grid mesh procedurally (`pc.createPlane` plus manual vertex
displacement, mirroring the Three.js example above) using
`pc.Mesh`/`pc.MeshInstance` with the raw exported heights.

### Physics engines (any of the above)

Physics collision is engine-agnostic — see
[`docs/game-development.md`](game-development.md#3-collision-and-physics)
for feeding the same exported heightmap into a heightfield collider
(Rapier, Ammo.js, Cannon-es) regardless of which rendering engine you chose
in this section.

## Choosing between Pattern A and Pattern B

| | Pattern A (VistaWASM renders) | Pattern B (export, re-render elsewhere) |
| --- | --- | --- |
| Visuals | VistaWASM's real atmosphere/clouds/mist/water/flora/grass/lighting | Whatever your engine's materials/lighting produce |
| Browser support | Requires WebGPU | Works anywhere your chosen engine works (including WebGL2/mobile) |
| Runtime cost | One extra WebGPU context/canvas | None beyond your engine's own terrain rendering |
| Terrain updates | Live — regenerate any time, renders immediately | Re-export and re-build your engine's mesh/collider on every change |
| Depth-correct compositing with other 3D objects | No (separate canvases, see note above) | Yes — it's all one scene in your engine |
| Best for | Exploration/flight/environment-forward games where VistaWASM's own look is the point | Games that need a specific existing rendering pipeline, WebGL fallback, or want terrain as just one asset among many in an existing engine |

Most projects that reach for "integrate with a web game engine" want
Pattern B — treat VistaWASM as your terrain *design tool and exporter*,
not your runtime renderer.
