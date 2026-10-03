import {
  Color3,
  Color4,
  DirectionalLight,
  Engine,
  FreeCamera,
  HemisphericLight,
  MeshBuilder,
  Scene,
  StandardMaterial,
  TransformNode,
  Vector3,
  VertexBuffer,
  type Mesh
} from "@babylonjs/core";
import {
  attachFlyCameraControls,
  createVistaEngine,
  readHeightmapFloats,
  type CameraOptions,
  type TerrainMetadata
} from "@vista-wasm/vista-wasm";

const vistaCanvas = document.querySelector<HTMLCanvasElement>("#vista");
const babylonCanvas = document.querySelector<HTMLCanvasElement>("#babylon");
const occlusionToggle = document.querySelector<HTMLInputElement>("#occlusion");
const status = document.querySelector<HTMLElement>("#status");

if (!vistaCanvas || !babylonCanvas || !occlusionToggle || !status) {
  throw new Error("The example page is missing an element.");
}

const SUN = { azimuthDegrees: 135, elevationDegrees: 28, intensity: 1.3 };

// --- VistaWASM: the landscape -----------------------------------------------

async function startLandscape(canvas: HTMLCanvasElement) {
  const engine = await createVistaEngine(canvas, {
    render: {
      width: canvas.clientWidth,
      height: canvas.clientHeight,
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

  return { engine, terrain };
}

// A browser without WebGPU, or a WASM file that fails to load, is shown on
// the page rather than only in the console.
const { engine, terrain } = await startLandscape(vistaCanvas).catch((error: unknown) => {
  status.textContent = `VistaWASM could not start: ${error instanceof Error ? error.message : String(error)}`;
  throw error;
});
engine.setSun(SUN);

// --- Babylon.js: objects drawn over the landscape ----------------------------

// A transparent canvas lets the landscape show through, and the last
// argument follows the device pixel ratio, as VistaWASM does.
const babylon = new Engine(babylonCanvas, true, { alpha: true }, true);
const scene = new Scene(babylon);
// VistaWASM's axes are right-handed with +Y up; Babylon.js is left-handed
// unless told otherwise.
scene.useRightHandedSystem = true;
scene.clearColor = new Color4(0, 0, 0, 0);
// The terrain occluder draws in group 0 and the objects in group 1. Keeping
// group 0's depth for group 1 is what lets hills hide objects.
scene.setRenderingAutoClearDepthStencil(1, false);
const camera = new FreeCamera("camera", Vector3.Zero(), scene);

// Light the Babylon.js objects from the same direction as VistaWASM's sun.
const sunLight = new DirectionalLight("sun", sunDirection(SUN.azimuthDegrees, SUN.elevationDegrees).scale(-1), scene);
sunLight.diffuse = new Color3(1, 0.96, 0.88);
sunLight.intensity = 2.2;
const skyLight = new HemisphericLight("sky", Vector3.Up(), scene);
skyLight.diffuse = new Color3(0.74, 0.84, 1);
skyLight.groundColor = new Color3(0.29, 0.35, 0.23);
skyLight.intensity = 0.7;

const heights = readHeightmapFloats(engine.exportHeightmap());
const metadata = terrain.metadata;
const occluder = createTerrainOccluder(heights, metadata, 256);
addBeacons(heights, metadata);
const drone = MeshBuilder.CreateTorusKnot("drone", { radius: 12, tube: 3.5, radialSegments: 128, tubularSegments: 16 }, scene);
drone.material = material("drone", new Color3(1, 0.48, 0.24));
drone.renderingGroupId = 1;

occlusionToggle.addEventListener("change", () => {
  occluder.setEnabled(occlusionToggle.checked);
});

// --- One camera, two renderers ---------------------------------------------

const controls = attachFlyCameraControls(engine, vistaCanvas, {
  initialPosition: [0, 420, 1400],
  initialYawDegrees: 180,
  initialPitchDegrees: -14,
  moveSpeedMetresPerSecond: 150,
  onCameraChange: syncCamera
});
syncCamera(controls.getCamera());

function resize(): void {
  const width = Math.max(1, vistaCanvas!.clientWidth);
  const height = Math.max(1, vistaCanvas!.clientHeight);
  engine.resize(width, height, window.devicePixelRatio);
  babylon.resize();
}

new ResizeObserver(resize).observe(vistaCanvas);
resize();
engine.start();
status.textContent = "Ready. The beacons and orange drone are Babylon.js objects.";

scene.onBeforeRenderObservable.add(() => {
  const t = performance.now() / 1000;
  const x = Math.cos(t * 0.25) * 450;
  const z = Math.sin(t * 0.25) * 450;
  drone.position.set(x, heightAt(heights, metadata, x, z) + 60, z);
  drone.rotation.set(t * 0.7, t, 0);
});
babylon.runRenderLoop(() => scene.render());

window.addEventListener("beforeunload", () => {
  babylon.stopRenderLoop();
  scene.dispose();
  babylon.dispose();
  controls.dispose();
  engine.stop();
  engine.dispose();
});

// --- Helpers ----------------------------------------------------------------

/** Copy VistaWASM's camera onto the Babylon.js camera. */
function syncCamera(view: CameraOptions): void {
  camera.fov = (view.fieldOfViewDegrees * Math.PI) / 180;
  camera.minZ = view.nearMetres ?? 0.5;
  camera.maxZ = view.farMetres ?? 120_000;
  camera.position.set(...view.position);
  camera.setTarget(new Vector3(...view.target));
}

/** The unit vector towards VistaWASM's sun (see `SunOptions`). */
function sunDirection(azimuthDegrees: number, elevationDegrees: number): Vector3 {
  const azimuth = (azimuthDegrees * Math.PI) / 180;
  const elevation = (elevationDegrees * Math.PI) / 180;
  return new Vector3(
    Math.cos(azimuth) * Math.cos(elevation),
    Math.sin(elevation),
    Math.sin(azimuth) * Math.cos(elevation)
  ).normalize();
}

/**
 * Terrain height in metres at world position (x, z), bilinearly
 * interpolated. VistaWASM centres the terrain on the origin.
 */
function heightAt(data: Float32Array, info: TerrainMetadata, x: number, z: number): number {
  const clamp = (value: number, max: number) => Math.min(Math.max(value, 0), max);
  const column = clamp(x / info.metresPerSample + (info.width - 1) / 2, info.width - 1.001);
  const row = clamp(z / info.metresPerSample + (info.height - 1) / 2, info.height - 1.001);
  const c0 = Math.floor(column);
  const r0 = Math.floor(row);
  const fx = column - c0;
  const fz = row - r0;
  const sample = (c: number, r: number) => data[r * info.width + c];
  const top = sample(c0, r0) * (1 - fx) + sample(c0 + 1, r0) * fx;
  const bottom = sample(c0, r0 + 1) * (1 - fx) + sample(c0 + 1, r0 + 1) * fx;
  return top * (1 - fz) + bottom * fz;
}

/**
 * A lit material. Logarithmic depth keeps precision across VistaWASM's
 * 0.5 m to 120 km view range.
 */
function material(name: string, colour: Color3): StandardMaterial {
  const result = new StandardMaterial(name, scene);
  result.diffuseColor = colour;
  result.specularColor = new Color3(0.2, 0.2, 0.2);
  result.useLogarithmicDepth = true;
  return result;
}

/**
 * An invisible copy of the terrain that only writes depth, so VistaWASM's
 * hills hide the Babylon.js objects behind them. `maxSegments` caps its
 * detail; it only needs to follow the terrain's shape.
 */
function createTerrainOccluder(data: Float32Array, info: TerrainMetadata, maxSegments: number): Mesh {
  const segments = Math.min(maxSegments, info.width - 1, info.height - 1);
  const ground = MeshBuilder.CreateGround(
    "terrain-occluder",
    {
      width: (info.width - 1) * info.metresPerSample,
      height: (info.height - 1) * info.metresPerSample,
      subdivisions: segments,
      updatable: true
    },
    scene
  );
  const positions = ground.getVerticesData(VertexBuffer.PositionKind);

  if (!positions) {
    throw new Error("The terrain occluder has no vertex positions.");
  }

  // Logarithmic depth is written in the shader, where a polygon offset has
  // no effect, so sink the occluder a metre instead: objects standing on
  // the ground are then not clipped.
  for (let i = 0; i < positions.length; i += 3) {
    positions[i + 1] = heightAt(data, info, positions[i], positions[i + 2]) - 1;
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

/** Tall, glowing beacons on dry land around the island. */
function addBeacons(data: Float32Array, info: TerrainMetadata): void {
  const poleMaterial = material("pole", new Color3(0.87, 0.9, 0.92));
  const lampMaterial = material("lamp", new Color3(0.4, 0.88, 1));
  lampMaterial.emissiveColor = new Color3(0.16, 0.72, 1);

  for (let i = 0; i < 24; i += 1) {
    const angle = (i / 24) * Math.PI * 2;
    const radius = 300 + (i % 3) * 350;
    const x = Math.cos(angle) * radius;
    const z = Math.sin(angle) * radius;
    const ground = heightAt(data, info, x, z);

    if (ground <= info.seaLevelMetres + 2) {
      continue;
    }

    const beacon = new TransformNode(`beacon-${i}`, scene);
    beacon.position.set(x, ground, z);
    const pole = MeshBuilder.CreateCylinder(`pole-${i}`, { diameter: 6, height: 80, tessellation: 12 }, scene);
    pole.position.y = 40;
    const lamp = MeshBuilder.CreateSphere(`lamp-${i}`, { diameter: 18, segments: 16 }, scene);
    lamp.position.y = 88;

    for (const [mesh, look] of [[pole, poleMaterial], [lamp, lampMaterial]] as const) {
      mesh.material = look;
      mesh.parent = beacon;
      mesh.renderingGroupId = 1;
    }
  }
}
