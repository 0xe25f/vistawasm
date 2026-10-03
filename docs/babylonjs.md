# Using VistaWASM With Babylon.js

VistaWASM and [Babylon.js](https://www.babylonjs.com) work well together.
There are two ways to combine them:

| Approach | What draws the landscape | Browsers | Best for |
| --- | --- | --- | --- |
| [1. Overlay](#1-overlay-vistawasm-draws-the-world-babylonjs-adds-objects) | VistaWASM, with Babylon.js objects on top | WebGPU | Using VistaWASM's full look (water, trees, sky, weather) with your own Babylon.js characters, vehicles, and effects |
| [2. Heightmap mesh](#2-heightmap-mesh-babylonjs-draws-the-terrain) | Babylon.js, from VistaWASM's terrain data | WebGL 2 or WebGPU at run time | Putting VistaWASM terrain inside an existing Babylon.js scene, or reaching browsers without WebGPU |

A complete, runnable overlay example lives in `examples/babylonjs/`:

```bash
npm run build
npm run dev:babylonjs   # http://127.0.0.1:5173/
```

## Shared coordinates

- **Units:** metres.
- **Axes:** VistaWASM is right-handed with +Y up, and the terrain is
    centred on the origin. Babylon.js is left-handed unless you set
    `scene.useRightHandedSystem = true`. Set it, and positions, targets
    and heights carry over unchanged.
- **Heightmap layout:** `engine.exportHeightmap()` returns little-endian
    `float32` heights, row by row. Row `r`, column `c` sits at
    `x = (c - (width - 1) / 2) * metresPerSample` and
    `z = (r - (height - 1) / 2) * metresPerSample`, using the
    `TerrainMetadata` returned when the terrain was made. Read it with
    `readHeightmapFloats()`.
- **Camera:** `fieldOfViewDegrees` is the vertical field of view in
    degrees. Babylon.js's `camera.fov` is vertical too, in radians. Near
    and far default to `0.5` and `120000` metres, Babylon.js's `minZ`
    and `maxZ`. VistaWASM's camera never rolls.
- **Sun:** a Babylon.js `DirectionalLight` shines along its direction,
    so give it the reverse of the vector towards VistaWASM's sun:

```ts
import { Vector3 } from "@babylonjs/core";

function sunDirection(azimuthDegrees: number, elevationDegrees: number): Vector3 {
  const azimuth = (azimuthDegrees * Math.PI) / 180;
  const elevation = (elevationDegrees * Math.PI) / 180;
  return new Vector3(
    Math.cos(azimuth) * Math.cos(elevation),
    Math.sin(elevation),
    Math.sin(azimuth) * Math.cos(elevation)
  ).normalize();
}

const sunLight = new DirectionalLight("sun", sunDirection(135, 28).scale(-1), scene);
```

### Terrain height at any point

Use this to stand meshes on the ground, keep a camera above it, or drive
gameplay:

```ts
import type { TerrainMetadata } from "@vista-wasm/vista-wasm";

function heightAt(heights: Float32Array, info: TerrainMetadata, x: number, z: number): number {
  const clamp = (value: number, max: number) => Math.min(Math.max(value, 0), max);
  const column = clamp(x / info.metresPerSample + (info.width - 1) / 2, info.width - 1.001);
  const row = clamp(z / info.metresPerSample + (info.height - 1) / 2, info.height - 1.001);
  const c0 = Math.floor(column);
  const r0 = Math.floor(row);
  const fx = column - c0;
  const fz = row - r0;
  const sample = (c: number, r: number) => heights[r * info.width + c];
  const top = sample(c0, r0) * (1 - fx) + sample(c0 + 1, r0) * fx;
  const bottom = sample(c0, r0 + 1) * (1 - fx) + sample(c0 + 1, r0 + 1) * fx;
  return top * (1 - fz) + bottom * fz;
}
```

## 1. Overlay: VistaWASM draws the world, Babylon.js adds objects

VistaWASM draws into its own canvas with WebGPU, and Babylon.js draws
into a transparent canvas stacked on top. You keep one camera and give it
to both.

### Stack two canvases

```html
<main style="position: relative; width: 100vw; height: 100vh;">
  <canvas id="vista" style="position: absolute; inset: 0; width: 100%; height: 100%;"></canvas>
  <canvas id="babylon" style="position: absolute; inset: 0; width: 100%; height: 100%; pointer-events: none;"></canvas>
</main>
```

`pointer-events: none` lets mouse and keyboard input reach the VistaWASM
canvas, which owns the camera controls.

### Create both engines

```ts
import { Color4, Engine, FreeCamera, HemisphericLight, Scene, Vector3 } from "@babylonjs/core";
import {
  attachFlyCameraControls,
  createVistaEngine,
  readHeightmapFloats,
  type CameraOptions
} from "@vista-wasm/vista-wasm";

const vistaCanvas = document.querySelector<HTMLCanvasElement>("#vista")!;
const babylonCanvas = document.querySelector<HTMLCanvasElement>("#babylon")!;

const engine = await createVistaEngine(vistaCanvas, {
  render: {
    width: vistaCanvas.clientWidth,
    height: vistaCanvas.clientHeight,
    devicePixelRatio: window.devicePixelRatio
  }
});
const terrain = await engine.generateFractal({
  seed: 7,
  size: 512,
  horizontalScaleMetres: 12,
  verticalScale: 0.6,
  noise: { kind: "ridged", octaves: 6, gain: 0.5, lacunarity: 2 },
  shape: { island: 0.3 }
});
engine.setSun({ azimuthDegrees: 135, elevationDegrees: 28, intensity: 1.3 });

// A transparent canvas lets the landscape show through, and the last
// argument follows the device pixel ratio, as VistaWASM does.
const babylon = new Engine(babylonCanvas, true, { alpha: true }, true);
const scene = new Scene(babylon);
scene.useRightHandedSystem = true;
scene.clearColor = new Color4(0, 0, 0, 0);
const camera = new FreeCamera("camera", Vector3.Zero(), scene);
new HemisphericLight("sky", Vector3.Up(), scene);
```

Name the two engines apart: here `engine` is VistaWASM's and `babylon` is
Babylon.js's.

### Share one camera

The ready-made fly camera reports every change, so copy it across. Do not
call `camera.attachControl()`: VistaWASM's controls move the camera.

```ts
function syncCamera(view: CameraOptions): void {
  camera.fov = (view.fieldOfViewDegrees * Math.PI) / 180;
  camera.minZ = view.nearMetres ?? 0.5;
  camera.maxZ = view.farMetres ?? 120_000;
  camera.position.set(...view.position);
  camera.setTarget(new Vector3(...view.target));
}

const controls = attachFlyCameraControls(engine, vistaCanvas, {
  initialPosition: [0, 420, 1400],
  initialYawDegrees: 180,
  initialPitchDegrees: -14,
  onCameraChange: syncCamera
});
syncCamera(controls.getCamera());
```

With your own camera or a physics-driven one, build a `CameraOptions`
each frame and pass it to both `engine.setCamera()` and `syncCamera()`.

### Keep depth precise

VistaWASM draws from 0.5 m to 120 km. A normal depth buffer cannot cover
that range, so distant Babylon.js objects flicker. Turn on logarithmic
depth in every material you use over the landscape:

```ts
const material = new StandardMaterial("drone", scene);
material.useLogarithmicDepth = true;
```

### Keep both the same size

```ts
function resize(): void {
  const width = Math.max(1, vistaCanvas.clientWidth);
  const height = Math.max(1, vistaCanvas.clientHeight);
  engine.resize(width, height, window.devicePixelRatio);
  babylon.resize();
}

new ResizeObserver(resize).observe(vistaCanvas);
resize();
```

### Render

VistaWASM runs its own loop; Babylon.js runs its own. Both draw once per
display frame with the same camera.

```ts
engine.start();
babylon.runRenderLoop(() => scene.render());
```

### Place meshes on the terrain

```ts
const heights = readHeightmapFloats(engine.exportHeightmap());
const info = terrain.metadata;

const marker = MeshBuilder.CreateSphere("marker", { diameter: 20 }, scene);
marker.position.set(200, heightAt(heights, info, 200, -150) + 10, -150);
marker.renderingGroupId = 1;
```

Read the heights again whenever you generate or load new terrain.

### Let hills hide your meshes

The two canvases have separate depth buffers, so without help a
Babylon.js mesh behind a mountain still draws in front of it. Give
Babylon.js an invisible copy of the terrain that writes depth only, and
draw it before everything else:

- The occluder goes in rendering group 0 and your meshes in group 1
    (`mesh.renderingGroupId = 1`).
- Babylon.js clears depth between groups by default, so tell it to keep
    group 0's depth for group 1.

```ts
function createTerrainOccluder(heights: Float32Array, info: TerrainMetadata, maxSegments = 256): Mesh {
  const ground = MeshBuilder.CreateGround(
    "terrain-occluder",
    {
      width: (info.width - 1) * info.metresPerSample,
      height: (info.height - 1) * info.metresPerSample,
      subdivisions: Math.min(maxSegments, info.width - 1, info.height - 1),
      updatable: true
    },
    scene
  );
  const positions = ground.getVerticesData(VertexBuffer.PositionKind)!;

  // Logarithmic depth is written in the shader, where a polygon offset has
  // no effect, so sink the occluder a metre instead: meshes standing on
  // the ground are then not clipped.
  for (let i = 0; i < positions.length; i += 3) {
    positions[i + 1] = heightAt(heights, info, positions[i], positions[i + 2]) - 1;
  }

  ground.updateVerticesData(VertexBuffer.PositionKind, positions);
  ground.refreshBoundingInfo();
  const depthOnly = new StandardMaterial("terrain-occluder", scene);
  depthOnly.disableColorWrite = true;
  depthOnly.backFaceCulling = false;
  depthOnly.useLogarithmicDepth = true;
  ground.material = depthOnly;
  ground.renderingGroupId = 0;
  return ground;
}

createTerrainOccluder(heights, info);
scene.setRenderingAutoClearDepthStencil(1, false);
```

The occluder only needs the terrain's shape, so 256 segments is plenty
for most terrain; raise it if small ridges fail to hide meshes.

### What the overlay cannot do

- Only the terrain hides Babylon.js meshes. VistaWASM's trees, grass, and
    water surface do not, so a mesh below sea level still shows through
    the water.
- Babylon.js meshes do not receive VistaWASM's shadows, fog, haze, or
    weather, and VistaWASM does not see Babylon.js meshes. Match the look
    with Babylon.js lights (use the sun direction above) and, for distant
    meshes, `scene.fogMode` in a colour close to the horizon.
- Each library keeps its own GPU context. Babylon.js's `WebGPUEngine`
    works for the overlay too, but it cannot share VistaWASM's device, so
    it gains nothing here over `Engine`.
- The overlay needs WebGPU for VistaWASM. For WebGL-only browsers, use
    approach 2.

## 2. Heightmap mesh: Babylon.js draws the terrain

Here VistaWASM only makes the terrain (generation, shaping, erosion, or a
GeoTIFF import) and Babylon.js draws it like any other mesh, with your own
materials, shadows, and fog.

```ts
import { Color3, MeshBuilder, StandardMaterial, VertexBuffer, VertexData, type Mesh } from "@babylonjs/core";

function createTerrainMesh(heights: Float32Array, info: TerrainMetadata, maxSegments = 512): Mesh {
  const ground = MeshBuilder.CreateGround(
    "terrain",
    {
      width: (info.width - 1) * info.metresPerSample,
      height: (info.height - 1) * info.metresPerSample,
      subdivisions: Math.min(maxSegments, info.width - 1, info.height - 1),
      updatable: true
    },
    scene
  );
  const positions = ground.getVerticesData(VertexBuffer.PositionKind)!;

  for (let i = 0; i < positions.length; i += 3) {
    positions[i + 1] = heightAt(heights, info, positions[i], positions[i + 2]);
  }

  const indices = ground.getIndices()!;
  const normals: number[] = [];
  VertexData.ComputeNormals(positions, indices, normals, { useRightHandedSystem: scene.useRightHandedSystem });

  // Simple colours: sand by the sea, grass on gentle slopes, rock on
  // steep ones, and snow on high ground.
  const colours: number[] = [];
  const snowLine = info.minHeightMetres + (info.maxHeightMetres - info.minHeightMetres) * 0.8;

  for (let i = 0; i < positions.length; i += 3) {
    const y = positions[i + 1];
    let colour = Color3.FromHexString("#5f8a45");

    if (y < info.seaLevelMetres + 4) {
      colour = Color3.FromHexString("#d8c89a");
    } else if (y > snowLine) {
      colour = Color3.FromHexString("#f2f4f7");
    } else if (normals[i + 1] < 0.8) {
      colour = Color3.FromHexString("#7d7a74");
    }

    colours.push(colour.r, colour.g, colour.b, 1);
  }

  ground.updateVerticesData(VertexBuffer.PositionKind, positions);
  ground.setVerticesData(VertexBuffer.NormalKind, normals);
  ground.setVerticesData(VertexBuffer.ColorKind, colours);
  ground.refreshBoundingInfo();
  ground.material = new StandardMaterial("terrain", scene);
  return ground;
}

const heights = readHeightmapFloats(engine.exportHeightmap());
createTerrainMesh(heights, terrain.metadata);
```

Use one segment per sample up to about 512 × 512; beyond that, lower
`maxSegments` or split the terrain into tiles. For collision, feed the
same `heights` to a physics heightfield (see
[`docs/game-development.md`](game-development.md#3-collision-and-physics)).

`MeshBuilder.CreateGroundFromHeightMap` also works, from a greyscale
`exportHeightmapImage()`, but an 8-bit image keeps only 256 height
steps; the raw heights above keep every metre.

### Ship the terrain without WebGPU

VistaWASM needs WebGPU to make terrain, but a Babylon.js app can use
terrain made earlier. Save it once, in any WebGPU browser:

```ts
import { downloadRawHeightmap, downloadText } from "@vista-wasm/vista-wasm";

downloadRawHeightmap(terrain.metadata, engine.exportHeightmap(), "island");
downloadText(JSON.stringify(terrain.metadata), "island.json", "application/json");
```

Then load it in your Babylon.js app, which needs neither VistaWASM's
WebAssembly nor WebGPU:

```ts
import { readHeightmapFloats, type TerrainMetadata } from "@vista-wasm/vista-wasm";

const info: TerrainMetadata = await (await fetch("/terrain/island.json")).json();
const bytes = new Uint8Array(await (await fetch("/terrain/island-512x512.f32le.bin")).arrayBuffer());
createTerrainMesh(readHeightmapFloats(bytes), info);
```

## Smaller bundles

`import { … } from "@babylonjs/core"` pulls in all of Babylon.js, about
1.1 MB gzipped. For production, import each class from its own module,
for example `@babylonjs/core/Engines/engine` and
`@babylonjs/core/Meshes/Builders/groundBuilder`, as Babylon.js's own
documentation on ES6 packages describes.

## See also

- [`examples/babylonjs/`](../examples/babylonjs/): the runnable overlay
    example.
- [Using VistaWASM with three.js](threejs.md): the same patterns for
    three.js, and an OBJ export route.
- [Game engine integration](engine-integration.md): which pattern to
    choose.
- [Export and snapshots](export-and-snapshots.md): every export helper.
- [Camera and controls](camera-and-controls.md): the camera model and the
    fly camera.
