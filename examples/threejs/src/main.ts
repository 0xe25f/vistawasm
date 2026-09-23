import * as THREE from "three";
import {
  attachFlyCameraControls,
  createVistaEngine,
  readHeightmapFloats,
  type CameraOptions,
  type TerrainMetadata
} from "@vista-wasm/vista-wasm";

const vistaCanvas = document.querySelector<HTMLCanvasElement>("#vista");
const threeCanvas = document.querySelector<HTMLCanvasElement>("#three");
const occlusionToggle = document.querySelector<HTMLInputElement>("#occlusion");
const status = document.querySelector<HTMLElement>("#status");

if (!vistaCanvas || !threeCanvas || !occlusionToggle || !status) {
  throw new Error("The example page is missing an element.");
}

const SUN = { azimuthDegrees: 135, elevationDegrees: 28, intensity: 1.3 };

// --- VistaWASM: the landscape -----------------------------------------------

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
engine.setSun(SUN);

// --- three.js: objects drawn over the landscape ------------------------------

// Logarithmic depth keeps precision across VistaWASM's 0.5 m to 120 km view
// range, and alpha lets the landscape show through.
const renderer = new THREE.WebGLRenderer({
  canvas: threeCanvas,
  alpha: true,
  antialias: true,
  logarithmicDepthBuffer: true
});
renderer.setClearColor(0x000000, 0);
const scene = new THREE.Scene();
const camera = new THREE.PerspectiveCamera();

// Light the three.js objects from the same direction as VistaWASM's sun.
const sunLight = new THREE.DirectionalLight(0xfff4e0, 2.6);
sunLight.position.copy(sunDirection(SUN.azimuthDegrees, SUN.elevationDegrees)).multiplyScalar(1000);
scene.add(sunLight, new THREE.HemisphereLight(0xbcd6ff, 0x4a5a3a, 0.9));

const heights = readHeightmapFloats(engine.exportHeightmap());
const metadata = terrain.metadata;
const occluder = createTerrainOccluder(heights, metadata, 256);
scene.add(occluder);
addBeacons(scene, heights, metadata);
const drone = new THREE.Mesh(
  new THREE.TorusKnotGeometry(12, 3.5, 128, 16),
  new THREE.MeshStandardMaterial({ color: 0xff7a3d, metalness: 0.3, roughness: 0.4 })
);
scene.add(drone);

occlusionToggle.addEventListener("change", () => {
  occluder.visible = occlusionToggle.checked;
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
  renderer.setPixelRatio(window.devicePixelRatio);
  renderer.setSize(width, height, false);
  camera.aspect = width / height;
  camera.updateProjectionMatrix();
}

new ResizeObserver(resize).observe(vistaCanvas);
resize();
engine.start();
status.textContent = "Ready. The beacons and orange drone are three.js objects.";

renderer.setAnimationLoop((timeMs) => {
  const t = timeMs / 1000;
  const x = Math.cos(t * 0.25) * 450;
  const z = Math.sin(t * 0.25) * 450;
  drone.position.set(x, heightAt(heights, metadata, x, z) + 60, z);
  drone.rotation.set(t * 0.7, t, 0);
  renderer.render(scene, camera);
});

window.addEventListener("beforeunload", () => {
  renderer.setAnimationLoop(null);
  controls.dispose();
  engine.stop();
  engine.dispose();
});

// --- Helpers ----------------------------------------------------------------

/** Copy VistaWASM's camera onto the three.js camera. */
function syncCamera(view: CameraOptions): void {
  camera.fov = view.fieldOfViewDegrees;
  camera.near = view.nearMetres ?? 0.5;
  camera.far = view.farMetres ?? 120_000;
  camera.position.set(...view.position);
  camera.lookAt(...view.target);
  camera.updateProjectionMatrix();
}

/** The unit vector towards VistaWASM's sun (see `SunOptions`). */
function sunDirection(azimuthDegrees: number, elevationDegrees: number): THREE.Vector3 {
  const azimuth = THREE.MathUtils.degToRad(azimuthDegrees);
  const elevation = THREE.MathUtils.degToRad(elevationDegrees);
  return new THREE.Vector3(
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
  const column = THREE.MathUtils.clamp(x / info.metresPerSample + (info.width - 1) / 2, 0, info.width - 1.001);
  const row = THREE.MathUtils.clamp(z / info.metresPerSample + (info.height - 1) / 2, 0, info.height - 1.001);
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
 * An invisible copy of the terrain that only writes depth, so VistaWASM's
 * hills hide the three.js objects behind them. `maxSegments` caps its
 * detail; it only needs to follow the terrain's shape.
 */
function createTerrainOccluder(data: Float32Array, info: TerrainMetadata, maxSegments: number): THREE.Mesh {
  const segments = Math.min(maxSegments, info.width - 1, info.height - 1);
  const sizeX = (info.width - 1) * info.metresPerSample;
  const sizeZ = (info.height - 1) * info.metresPerSample;
  const geometry = new THREE.PlaneGeometry(sizeX, sizeZ, segments, segments);
  geometry.rotateX(-Math.PI / 2);
  const position = geometry.attributes.position;

  for (let i = 0; i < position.count; i += 1) {
    position.setY(i, heightAt(data, info, position.getX(i), position.getZ(i)));
  }

  // Draw first, write depth only, and sit a little behind the real surface
  // so objects standing on the ground are not clipped.
  const material = new THREE.MeshBasicMaterial({
    colorWrite: false,
    polygonOffset: true,
    polygonOffsetFactor: 2,
    polygonOffsetUnits: 2
  });
  const mesh = new THREE.Mesh(geometry, material);
  mesh.renderOrder = -1;
  return mesh;
}

/** Tall, glowing beacons on dry land around the island. */
function addBeacons(target: THREE.Scene, data: Float32Array, info: TerrainMetadata): void {
  const pole = new THREE.CylinderGeometry(3, 3, 80, 12);
  pole.translate(0, 40, 0);
  const lamp = new THREE.SphereGeometry(9, 24, 16);
  const poleMaterial = new THREE.MeshStandardMaterial({ color: 0xdfe6ea, roughness: 0.5 });
  const lampMaterial = new THREE.MeshStandardMaterial({ color: 0x66e0ff, emissive: 0x2ab8ff, emissiveIntensity: 2 });
  const seaLevel = info.seaLevelMetres;

  for (let i = 0; i < 24; i += 1) {
    const angle = (i / 24) * Math.PI * 2;
    const radius = 300 + (i % 3) * 350;
    const x = Math.cos(angle) * radius;
    const z = Math.sin(angle) * radius;
    const ground = heightAt(data, info, x, z);

    if (ground <= seaLevel + 2) {
      continue;
    }

    const beacon = new THREE.Group();
    beacon.add(new THREE.Mesh(pole, poleMaterial));
    const light = new THREE.Mesh(lamp, lampMaterial);
    light.position.y = 88;
    beacon.add(light);
    beacon.position.set(x, ground, z);
    target.add(beacon);
  }
}
