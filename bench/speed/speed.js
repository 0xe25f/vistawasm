// Times how long each library takes to turn a 512 x 512 terrain request
// into a mesh ready to draw, in the same browser on the same machine.
// Each case runs once to warm up, then RUNS times; the median is reported.
import * as THREE from "three";
import Terrain, { TerrainNS, createSeededRandom } from "three.terrain.js";
import VoxelTerrain from "three-terrain";
import { CesiumWidget } from "cesium";
import { VIEW, fieldProvider } from "../size/cesium-field.js";
import { createVistaEngine } from "@vista-wasm/vista-wasm";

const RUNS = 9;
const results = [];

// Table rows are built with textContent, so no label or error message is
// ever parsed as HTML.
function addRow(cells) {
  const row = document.createElement("tr");

  for (const { text, colSpan } of cells) {
    const cell = document.createElement("td");
    cell.textContent = text;

    if (colSpan) {
      cell.colSpan = colSpan;
    }

    row.append(cell);
  }

  document.querySelector("#results").append(row);
}

async function time(label, detail, run) {
  await run(0);
  const samples = [];

  for (let i = 1; i <= RUNS; i += 1) {
    const start = performance.now();
    await run(i);
    samples.push(performance.now() - start);
  }

  samples.sort((a, b) => a - b);
  const row = { label, detail, median: samples[Math.floor(RUNS / 2)], fastest: samples[0] };
  results.push(row);
  addRow([
    { text: label },
    { text: detail },
    { text: row.median.toFixed(0) },
    { text: row.fastest.toFixed(0) }
  ]);
}

const fractal = (seed) => ({
  seed,
  size: 512,
  horizontalScaleMetres: 12,
  verticalScale: 1,
  noise: { kind: "simplex", octaves: 5, gain: 0.5, lacunarity: 2 }
});

try {
  const canvas = document.querySelector("#view");
  const engine = await createVistaEngine(canvas, { render: { width: 320, height: 180 } });

  // Height field, normals, surface materials, and GPU mesh; no biomes,
  // water, or vegetation. The closest match to the other two libraries.
  engine.setBiomes({ enabled: false });
  engine.setWater({ enabled: false, seaLevelMetres: 0, waveScale: 0.8, reflectivity: 0.35, shorelineSoftnessMetres: 6 });
  engine.setFlora({ enabled: false, density: 0.35, treeLineMetres: 1800, seedOffset: 3001, maxInstances: 500000 });
  await time(
    "VistaWASM",
    "5-octave simplex fractal, normals, materials, mesh (no biomes, water, or trees)",
    (i) => engine.generateFractal(fractal(i + 1))
  );

  // The full default world: biomes, rivers and lakes, and tree placement.
  engine.setBiomes({ enabled: true });
  engine.setWater({ enabled: true, seaLevelMetres: 0, waveScale: 0.8, reflectivity: 0.35, shorelineSoftnessMetres: 6 });
  engine.setFlora({ enabled: true, density: 0.35, treeLineMetres: 1800, seedOffset: 3001, maxInstances: 500000 });
  await time(
    "VistaWASM",
    "Same, plus biomes, rivers and lakes, and tree placement",
    (i) => engine.generateFractal(fractal(i + 1))
  );
  engine.dispose();
} catch (error) {
  addRow([{ text: "VistaWASM" }, { text: `Skipped: ${error.message}`, colSpan: 3 }]);
}

await time(
  "THREE.Terrain 3.1.1",
  "5-octave simplex (SimplexLayers), mesh with normals",
  (i) => {
    const mesh = Terrain({
      heightmap: TerrainNS.SimplexLayers,
      material: new THREE.MeshBasicMaterial(),
      maxHeight: 300,
      minHeight: -100,
      random: createSeededRandom(i + 1),
      xSegments: 511,
      xSize: 6132,
      ySegments: 511,
      ySize: 6132
    });
    mesh.traverse((child) => child.geometry?.dispose());
  }
);

await time(
  "three-terrain 0.0.10",
  "Voxel mesh of a 512 x 512 heightmap (filled by a JavaScript sine field)",
  () => new Promise((resolve) => {
    const terrain = new VoxelTerrain({
      width: 512,
      height: 255,
      depth: 512,
      generator: (target) => {
        const heights = target.mesher.memory.heightmap.view;

        for (let z = 0; z < 512; z += 1) {
          for (let x = 0; x < 512; x += 1) {
            heights[z * 512 + x] = 0.5 + 0.25 * Math.sin(x * 0.02) * Math.cos(z * 0.017);
          }
        }
      },
      onLoad: () => {
        terrain.dispose();
        resolve();
      }
    });
  })
);

// Cesium streams terrain as tiles, meshed on web workers, so a run is the
// time from handing it a new terrain provider to every tile in view being
// loaded. The widget, its WebGL context and its workers are made once.
const cesium = new CesiumWidget(document.querySelector("#globe"), {
  baseLayer: false,
  terrainProvider: fieldProvider()
});
cesium.camera.setView(VIEW);
// Resolves on the first frame with every tile in view loaded, once a
// frame has been drawn with some still loading: just after the provider
// changes, the globe still reports its old tiles as loaded.
const tilesLoaded = (waitForLoading) => new Promise((resolve) => {
  let loading = !waitForLoading;
  const remove = cesium.scene.postRender.addEventListener(() => {
    const loaded = cesium.scene.globe.tilesLoaded;
    loading ||= !loaded;

    if (loading && loaded) {
      remove();
      resolve();
    }
  });
});
await tilesLoaded(false);

await time(
  "CesiumJS 1.146",
  "Terrain tiles in view of a 512 x 512 height field (filled by a JavaScript sine field), meshed on workers",
  async () => {
    const done = tilesLoaded(true);
    cesium.scene.terrainProvider = fieldProvider();
    await done;
  }
);
cesium.destroy();

document.querySelector("#status").textContent = "Done.";
document.querySelector("#environment").textContent =
  `${navigator.userAgent} · ${navigator.hardwareConcurrency} logical cores`;
window.benchmarkResults = results;
