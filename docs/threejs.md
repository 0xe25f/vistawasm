# Using VistaWASM With three.js

VistaWASM and [three.js](https://threejs.org) work well together. There
are three ways to combine them:

| Approach | What draws the landscape | Browsers | Best for |
| --- | --- | --- | --- |
| [1. Overlay](#1-overlay-vistawasm-draws-the-world-threejs-adds-objects) | VistaWASM, with three.js objects on top | WebGPU | Using VistaWASM's full look (water, trees, sky, weather) with your own three.js characters, vehicles, and effects |
| [2. Heightmap mesh](#2-heightmap-mesh-threejs-draws-the-terrain) | three.js, from VistaWASM's terrain data | WebGL 2 at run time | Putting VistaWASM terrain inside an existing three.js scene, or reaching browsers without WebGPU |
| [3. OBJ model](#3-obj-model) | three.js, from an exported model | WebGL 2 at run time | A quick, static terrain model |

A complete, runnable overlay example lives in `examples/threejs/`:

```bash
npm run build
npm run dev:threejs   # http://127.0.0.1:5173/
```

## Shared coordinates

Both libraries use the same conventions, so values carry over directly:

- **Units:** metres.
- **Axes:** +Y is up. The terrain is centred on the origin.
- **Heightmap layout:** `engine.exportHeightmap()` returns little-endian
  `float32` heights, row by row. Row `r`, column `c` sits at
  `x = (c - (width - 1) / 2) * metresPerSample` and
  `z = (r - (height - 1) / 2) * metresPerSample`, using the
  `TerrainMetadata` returned when the terrain was made. Read it with
  `readHeightmapFloats()`.
- **Camera:** `fieldOfViewDegrees` is the vertical field of view, like
  `PerspectiveCamera.fov`. Near and far default to `0.5` and `120000`
  metres. VistaWASM's camera never rolls: it always uses world +Y as up,
  as `Object3D.lookAt()` does, and `rollDegrees` is not applied yet.
- **Sun:** VistaWASM's sun direction, for a three.js `DirectionalLight`, is:

```ts
function sunDirection(azimuthDegrees: number, elevationDegrees: number): THREE.Vector3 {
  const azimuth = THREE.MathUtils.degToRad(azimuthDegrees);
  const elevation = THREE.MathUtils.degToRad(elevationDegrees);
  return new THREE.Vector3(
    Math.cos(azimuth) * Math.cos(elevation),
    Math.sin(elevation),
    Math.sin(azimuth) * Math.cos(elevation)
  ).normalize();
}
```

### Terrain height at any point

Use this to stand objects on the ground, keep a camera above it, or drive
gameplay:

```ts
import type { TerrainMetadata } from "@vista-wasm/vista-wasm";

function heightAt(heights: Float32Array, info: TerrainMetadata, x: number, z: number): number {
  const column = THREE.MathUtils.clamp(x / info.metresPerSample + (info.width - 1) / 2, 0, info.width - 1.001);
  const row = THREE.MathUtils.clamp(z / info.metresPerSample + (info.height - 1) / 2, 0, info.height - 1.001);
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

## 1. Overlay: VistaWASM draws the world, three.js adds objects

VistaWASM draws into its own canvas with WebGPU, and three.js draws into a
transparent canvas stacked on top. You keep one camera and give it to both.

### Stack two canvases

```html
<main style="position: relative; width: 100vw; height: 100vh;">
  <canvas id="vista" style="position: absolute; inset: 0; width: 100%; height: 100%;"></canvas>
  <canvas id="three" style="position: absolute; inset: 0; width: 100%; height: 100%; pointer-events: none;"></canvas>
</main>
```

`pointer-events: none` lets mouse and keyboard input reach the VistaWASM
canvas, which owns the camera controls.

### Create both renderers

```ts
import * as THREE from "three";
import {
  attachFlyCameraControls,
  createVistaEngine,
  readHeightmapFloats,
  type CameraOptions
} from "@vista-wasm/vista-wasm";

const vistaCanvas = document.querySelector<HTMLCanvasElement>("#vista")!;
const threeCanvas = document.querySelector<HTMLCanvasElement>("#three")!;

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
const sun = { azimuthDegrees: 135, elevationDegrees: 28, intensity: 1.3 };
engine.setSun(sun);

// alpha lets the landscape show through; logarithmic depth keeps precision
// across VistaWASM's 0.5 m to 120 km view range.
const renderer = new THREE.WebGLRenderer({
  canvas: threeCanvas,
  alpha: true,
  antialias: true,
  logarithmicDepthBuffer: true
});
renderer.setClearColor(0x000000, 0);
const scene = new THREE.Scene();
const camera = new THREE.PerspectiveCamera();

const light = new THREE.DirectionalLight(0xfff4e0, 2.6);
light.position.copy(sunDirection(sun.azimuthDegrees, sun.elevationDegrees)).multiplyScalar(1000);
scene.add(light, new THREE.HemisphereLight(0xbcd6ff, 0x4a5a3a, 0.9));
```

### Share one camera

The ready-made fly camera reports every change, so copy it across:

```ts
function syncCamera(view: CameraOptions): void {
  camera.fov = view.fieldOfViewDegrees;
  camera.near = view.nearMetres ?? 0.5;
  camera.far = view.farMetres ?? 120_000;
  camera.position.set(...view.position);
  camera.lookAt(...view.target);
  camera.updateProjectionMatrix();
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

### Keep both the same size

```ts
function resize(): void {
  const width = Math.max(1, vistaCanvas.clientWidth);
  const height = Math.max(1, vistaCanvas.clientHeight);
  engine.resize(width, height, window.devicePixelRatio);
  renderer.setPixelRatio(window.devicePixelRatio);
  renderer.setSize(width, height, false);
  camera.aspect = width / height;
  camera.updateProjectionMatrix();
}

new ResizeObserver(resize).observe(vistaCanvas);
resize();
```

### Render

VistaWASM runs its own loop; three.js runs its own. Both draw once per
display frame with the same camera.

```ts
engine.start();
renderer.setAnimationLoop(() => renderer.render(scene, camera));
```

### Place objects on the terrain

```ts
const heights = readHeightmapFloats(engine.exportHeightmap());
const info = terrain.metadata;

const marker = new THREE.Mesh(
  new THREE.SphereGeometry(10),
  new THREE.MeshStandardMaterial({ color: 0x66e0ff })
);
marker.position.set(200, heightAt(heights, info, 200, -150) + 10, -150);
scene.add(marker);
```

Read the heights again whenever you generate or load new terrain.

### Let hills hide your objects

The two canvases have separate depth buffers, so without help a three.js
object behind a mountain still draws in front of it. Fix this by giving
three.js an invisible copy of the terrain that writes depth only:

```ts
function createTerrainOccluder(heights: Float32Array, info: TerrainMetadata, maxSegments = 256): THREE.Mesh {
  const segments = Math.min(maxSegments, info.width - 1, info.height - 1);
  const geometry = new THREE.PlaneGeometry(
    (info.width - 1) * info.metresPerSample,
    (info.height - 1) * info.metresPerSample,
    segments,
    segments
  );
  geometry.rotateX(-Math.PI / 2);
  const position = geometry.attributes.position;

  for (let i = 0; i < position.count; i += 1) {
    position.setY(i, heightAt(heights, info, position.getX(i), position.getZ(i)));
  }

  // Depth only, drawn first, and nudged slightly back so objects standing
  // on the ground are not clipped.
  const mesh = new THREE.Mesh(
    geometry,
    new THREE.MeshBasicMaterial({
      colorWrite: false,
      polygonOffset: true,
      polygonOffsetFactor: 2,
      polygonOffsetUnits: 2
    })
  );
  mesh.renderOrder = -1;
  return mesh;
}

scene.add(createTerrainOccluder(heights, info));
```

The occluder only needs the terrain's shape, so 256 segments is plenty
for most terrain; raise it if small ridges fail to hide objects.

### What the overlay cannot do

- Only the terrain hides three.js objects. VistaWASM's trees, grass, and
  water surface do not, so an object below sea level still shows through
  the water.
- three.js objects do not receive VistaWASM's shadows, fog, haze, or
  weather, and VistaWASM does not see three.js objects. Match the look
  with three.js lights (use the sun direction above) and, for distant
  objects, three.js fog in a colour close to the horizon.
- Each library keeps its own GPU context. That costs some memory, but no
  extra drawing.
- The overlay needs WebGPU for VistaWASM. For WebGL-only browsers, use
  approach 2.

## 2. Heightmap mesh: three.js draws the terrain

Here VistaWASM only makes the terrain (generation, shaping, erosion, or a
GeoTIFF import) and three.js draws it like any other mesh. It fits
anywhere in an existing three.js scene, with your own materials, shadows,
and fog.

```ts
function createTerrainMesh(heights: Float32Array, info: TerrainMetadata, maxSegments = 512): THREE.Mesh {
  const segmentsX = Math.min(maxSegments, info.width - 1);
  const segmentsZ = Math.min(maxSegments, info.height - 1);
  const geometry = new THREE.PlaneGeometry(
    (info.width - 1) * info.metresPerSample,
    (info.height - 1) * info.metresPerSample,
    segmentsX,
    segmentsZ
  );
  geometry.rotateX(-Math.PI / 2);
  const position = geometry.attributes.position;

  for (let i = 0; i < position.count; i += 1) {
    position.setY(i, heightAt(heights, info, position.getX(i), position.getZ(i)));
  }

  geometry.computeVertexNormals();

  // Simple colours: sand by the sea, grass on gentle slopes, rock on
  // steep ones, and snow on high ground.
  const normal = geometry.attributes.normal;
  const colours = new Float32Array(position.count * 3);
  const colour = new THREE.Color();
  const snowLine = info.minHeightMetres + (info.maxHeightMetres - info.minHeightMetres) * 0.8;

  for (let i = 0; i < position.count; i += 1) {
    const y = position.getY(i);

    if (y < info.seaLevelMetres + 4) {
      colour.set(0xd8c89a);
    } else if (y > snowLine) {
      colour.set(0xf2f4f7);
    } else if (normal.getY(i) < 0.8) {
      colour.set(0x7d7a74);
    } else {
      colour.set(0x5f8a45);
    }

    colours.set([colour.r, colour.g, colour.b], i * 3);
  }

  geometry.setAttribute("color", new THREE.BufferAttribute(colours, 3));
  return new THREE.Mesh(geometry, new THREE.MeshStandardMaterial({ vertexColors: true }));
}

const heights = readHeightmapFloats(engine.exportHeightmap());
scene.add(createTerrainMesh(heights, terrain.metadata));
```

Use one segment per sample up to about 512 × 512; beyond that, lower
`maxSegments` or split the terrain into tiles. For collision, feed the same
`heights` to a physics engine's heightfield (see
[`docs/game-development.md`](game-development.md#3-collision-and-physics)).

### Ship the terrain without WebGPU

VistaWASM needs WebGPU to make terrain, but a three.js app can use terrain
made earlier. Save it once, in any WebGPU browser, for example from a
small design page of your own:

```ts
import { downloadRawHeightmap, downloadText } from "@vista-wasm/vista-wasm";

downloadRawHeightmap(terrain.metadata, engine.exportHeightmap(), "island");
downloadText(JSON.stringify(terrain.metadata), "island.json", "application/json");
```

Then load it in your three.js app, which needs neither VistaWASM nor
WebGPU:

```ts
import { readHeightmapFloats, type TerrainMetadata } from "@vista-wasm/vista-wasm";

const info: TerrainMetadata = await (await fetch("/terrain/island.json")).json();
const bytes = new Uint8Array(await (await fetch("/terrain/island-512x512.f32le.bin")).arrayBuffer());
scene.add(createTerrainMesh(readHeightmapFloats(bytes), info));
```

`readHeightmapFloats()` and the other export helpers are plain
JavaScript; importing them does not load VistaWASM's WebAssembly.

## 3. OBJ model

`exportTerrainObj()` turns the heightmap into a Wavefront OBJ mesh in the
same coordinates, which three.js's `OBJLoader` reads directly:

```ts
import { OBJLoader } from "three/examples/jsm/loaders/OBJLoader.js";
import { exportTerrainObj } from "@vista-wasm/vista-wasm";

const obj = exportTerrainObj(terrain.metadata, engine.exportHeightmap(), { maxSamplesPerSide: 256 });
const model = new OBJLoader().parse(obj);
model.traverse((child) => {
  if (child instanceof THREE.Mesh) {
    child.material = new THREE.MeshStandardMaterial({ color: 0x8aa06a });
  }
});
scene.add(model);
```

OBJ files are text and grow quickly; prefer approach 2 for large terrain
or anything you update at run time.

## See also

- [`examples/threejs/`](../examples/threejs/): the runnable overlay example.
- [Game engine integration](engine-integration.md): the same patterns
  for Babylon.js and PlayCanvas.
- [Export and snapshots](export-and-snapshots.md): every export helper.
- [Camera and controls](camera-and-controls.md): the camera model and the
  fly camera.
