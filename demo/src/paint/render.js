// Sending a paint document to the 3D engine, and saving, opening and
// exporting it, through the library's public API only.
import { BIOME_COLOURS, encodePng, exportBundle, loadBundle } from "@vista-wasm/vista-wasm";
import { DOCUMENT_SIZES, documentFromEngine, heightmapOptions, paintedInputs } from "./document.js";
import { heightAt } from "./brushes.js";

/**
 * Load `doc` into `engine`: the height layer as a raw float32 heightmap,
 * then the painted biome, water and density maps, or `null` for a layer
 * with nothing painted. Returns the terrain handle and any warnings the
 * engine raised.
 */
export async function renderDocument(engine, doc) {
  const warnings = [];
  const stop = engine.on("warning", ({ message }) => warnings.push(message));

  try {
    // Copies, taken together, so painting while the engine loads never changes what it reads.
    const inputs = paintedInputs(doc);
    const handle = await engine.loadRawHeightmap(doc.layers.height.slice().buffer, heightmapOptions(doc));
    engine.setBiomeMap(inputs.biome);
    engine.setWaterMask(inputs.water);
    engine.setVegetationMasks({ trees: inputs.trees, grass: inputs.grass });
    return { handle, warnings };
  } finally {
    stop();
  }
}

/** A camera 45 degrees above the map's centre, 0.8 map widths away, looking at it. */
export function framingCamera(doc) {
  const centre = (doc.size - 1) / 2;
  const ground = Math.max(heightAt(doc, centre, centre), doc.seaLevelMetres);
  const distance = 0.8 * doc.widthKm * 1000;
  const along = distance * Math.SQRT1_2;
  return { position: [0, ground + along, along], target: [0, ground, 0] };
}

/**
 * Run `render` for the latest request only: a request made while one
 * runs waits for it, and replaces any request already waiting.
 * `later()` requests after `delayMs`, restarting the delay each time.
 */
export function createRenderQueue(render, delayMs = 600) {
  let running = null;
  let waiting = null;
  let timer = 0;

  async function drain() {
    while (waiting) {
      const next = waiting;
      waiting = null;

      try {
        next.resolve(await render(next.options));
      } catch (error) {
        next.reject(error);
      }
    }

    running = null;
  }

  function now(options = {}) {
    clearTimeout(timer);

    if (waiting) {
      // The request it replaces settles with this one.
      const replaced = waiting;
      waiting = { ...replaced, options };
      return replaced.promise;
    }

    let resolve;
    let reject;
    const promise = new Promise((done, fail) => {
      resolve = done;
      reject = fail;
    });
    waiting = { options, promise, resolve, reject };
    running ??= drain();
    return promise;
  }

  return {
    now,
    later(options, onError) {
      clearTimeout(timer);
      timer = setTimeout(() => now(options).catch(onError), delayMs);
    },
    cancel() {
      clearTimeout(timer);
    },
    get busy() {
      return running !== null;
    }
  };
}

/** Save what the engine shows as a version 2 bundle: source height, painted maps and options. */
export function savePainting(engine) {
  // Trees are recomputed from the rest on open, so the file leaves them out.
  return exportBundle(engine, { trees: false });
}

/**
 * Open a bundle into the engine, then read its heights and painted maps
 * back as a document. A square bundle of a paint size keeps its size;
 * any other is resampled to `size`. The scene options stay as they are.
 */
export async function openPainting(engine, file, size) {
  const handle = await loadBundle(engine, file, { applySettings: false });
  const { width, height } = handle.metadata;
  const native = width === height && DOCUMENT_SIZES.includes(width);
  const doc = documentFromEngine(engine, { size: native ? width : size });
  return { doc, handle };
}

/** Every biome's legend colour, and black for not painted, which `biomeMapFromImage()` reads back as 255. */
function biomeLegend() {
  const legend = BIOME_COLOURS.map((colour, index) => ({
    index,
    name: String(index),
    colour: colour.map((component) => component / 255)
  }));
  return [...legend, { index: 255, name: "not painted", colour: [0, 0, 0] }];
}

/**
 * Encode one layer as a PNG: height as 16-bit grey over its own range
 * (kept in the file), biomes in the exported legend colours, and water,
 * tree and grass values as 8-bit grey.
 */
export async function exportLayerPng(doc, name) {
  const data = doc.layers[name];
  const map = {
    kind: name === "height" ? "sourceHeight" : name === "biome" ? "biome" : name === "water" ? "water" : name === "trees" ? "treeDensity" : "grassDensity",
    width: doc.size,
    height: doc.size,
    channels: 1,
    type: data instanceof Float32Array ? "float32" : "uint8",
    data,
    encoding: name === "biome" ? { legend: biomeLegend() } : {}
  };
  return encodePng(map, { bitDepth: name === "height" ? 16 : 8 });
}
