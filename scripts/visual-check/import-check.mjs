// Check map import in headless Chromium: an exact bundle round trip, the
// PNG decoder's speed, and shots of each kind of import.
//
//   python3 -m http.server 8124        (from the repository root)
//   node scripts/visual-check/import-check.mjs out-dir
//
// The script writes to `out-dir`:
//
// - `roundtrip.json`: a bundle exported from the default island with a
//   painted biome map (the north-east quarter savannah), a water mask
//   and tree and grass masks is loaded into a fresh page. Every map, the
//   trees, and a second bundle's files must be byte-identical; the
//   rendered views (`roundtrip-*.png`) are compared pixel by pixel;
// - `decode.json`: how long `decodePng()` takes for a 4096 x 4096 16-bit
//   grey PNG (the median of five runs);
// - `heightmap-top.png` and `heightmap-low.png`: a 1024 x 1024 16-bit PNG
//   heightmap made in the page, loaded with `loadHeightmapImage()`;
// - `biomes-top.png` and `biomes-debug.png`: three painted regions, with
//   their warped borders;
// - `water.png`: a painted lake and river;
// - `trees-*.png` and `grass-*.png`: a clearing and a thicket painted with
//   the tree mask, and grass thinned and doubled by the grass mask.
//
// `--only=roundtrip`, `decode`, `heightmap` or `scene` runs one part.
//
// It exits non-zero on any page error, console error, WebGPU validation
// warning or mismatch.
import fs from "node:fs";

const playwright = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const [out = "import-check"] = process.argv.slice(2).filter((arg) => !arg.startsWith("--"));
const port = process.env.PORT ?? "8124";
const size = [640, 400];
const only = process.argv.find((arg) => arg.startsWith("--only="))?.slice(7);
const part = (name) => !only || only === name;
fs.mkdirSync(out, { recursive: true });

const browser = await playwright.chromium.launch({
  headless: true,
  executablePath: process.env.CHROMIUM_PATH,
  args: [
    "--enable-unsafe-webgpu",
    "--enable-features=Vulkan,WebGPU",
    "--use-angle=swiftshader",
    "--use-webgpu-adapter=swiftshader",
    "--ignore-gpu-blocklist",
    "--enable-unsafe-swiftshader"
  ]
});
let failed = false;

async function open(config = {}) {
  const page = await browser.newPage({ viewport: { width: size[0], height: size[1] } });
  // WebGPU reports invalid pipelines and commands as warnings.
  page.on("console", (message) => {
    if (message.type() === "error" || message.type() === "warning") {
      console.log(`console ${message.type()}: ${message.text().slice(0, 1500)}`);
      failed ||= message.type() === "error" ? !/Failed to load resource/.test(message.text()) : /Invalid|WebGPU|GPU/.test(message.text());
    }
  });
  page.on("pageerror", (error) => {
    console.log(`page error: ${error.message}`);
    failed = true;
  });
  await page.goto(
    `http://127.0.0.1:${port}/scripts/visual-check/index.html?config=${encodeURIComponent(JSON.stringify({ size, ...config }))}`
  );
  await page.waitForFunction(() => window.ready === true, null, { timeout: 600000 });
  const log = await page.evaluate(() => window.log.splice(0));
  console.log("log:", JSON.stringify(log));

  if (log.some((line) => line.startsWith("error"))) {
    failed = true;
  }

  // Helpers every step uses: SHA-256 of bytes, the maps compared, and a
  // shot from a camera.
  await page.evaluate(() => {
    window.digest = async (bytes) =>
      Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)), (b) => b.toString(16).padStart(2, "0")).join("");
    window.mapDigests = async () => {
      const digests = {};

      for (const kind of ["height", "sourceHeight", "biome", "water", "materials", "treeDensity", "grassDensity", "moisture", "temperature"]) {
        const map = window.engine.exportMap(kind);
        digests[kind] = await window.digest(new Uint8Array(map.data.buffer, map.data.byteOffset, map.data.byteLength));
      }

      digests.trees = await window.digest(new TextEncoder().encode(JSON.stringify(window.engine.exportTrees())));
      return digests;
    };
    window.shoot = async (camera, debugView = "none", frames = 10) => {
      window.engine.setDebugView(debugView);
      window.engine.setCamera({ fieldOfViewDegrees: 55, nearMetres: 0.5, farMetres: 120000, ...camera });
      return (await window.capture(frames)).url;
    };
    window.above = (x, z, height) => {
      const ground = window.heightAt(x, z);
      return { position: [x, ground + height, z + 0.01], target: [x, ground, z] };
    };
  });
  return page;
}

function save(name, url) {
  fs.writeFileSync(`${out}/${name}`, Buffer.from(url.split(",")[1], "base64"));
}

/** Mean absolute difference per channel between two PNG data URLs, 0 to 255. */
async function difference(page, a, b) {
  return page.evaluate(async ([a, b]) => {
    const pixels = async (url) => {
      const image = new Image();
      image.src = url;
      await image.decode();
      const canvas = document.createElement("canvas");
      [canvas.width, canvas.height] = [image.width, image.height];
      const context = canvas.getContext("2d");
      context.drawImage(image, 0, 0);
      return context.getImageData(0, 0, image.width, image.height).data;
    };
    const [x, y] = [await pixels(a), await pixels(b)];
    let sum = 0;

    for (let index = 0; index < x.length; index += 4) {
      sum += Math.abs(x[index] - y[index]) + Math.abs(x[index + 1] - y[index + 1]) + Math.abs(x[index + 2] - y[index + 2]);
    }

    return sum / (x.length * 0.75);
  }, [a, b]);
}

// 1. The round trip.
if (part("roundtrip")) {
  const painted = await open();
  const views = {
    top: await painted.evaluate(() => {
      const half = ((window.metadata.width - 1) * window.metadata.metresPerSample) / 2;
      return window.above(0, 0, half / Math.tan((27.5 * Math.PI) / 180) + 200);
    }),
    low: { position: [0, 420, 900], target: [0, 300, 0] }
  };
  const first = await painted.evaluate(async (views) => {
    const vista = await import("../../dist/index.js");
    const engine = window.engine;
    const { width, height } = window.metadata;
    const grid = (fill) => ({ width, height, data: Uint8Array.from({ length: width * height }, (_, index) => fill(index % width, Math.floor(index / width))) });
    // The north-east quarter savannah, the rest the engine's own.
    engine.setBiomeMap(grid((x, y) => (x >= width / 2 && y < height / 2 ? 8 : 255)));
    engine.setWaterMask(grid((x, y) => (Math.hypot(x - width * 0.35, y - height * 0.6) < 18 ? 200 : Math.abs(y - height * 0.45) < 1 && x > width * 0.2 && x < width * 0.6 ? 60 : 0)));
    engine.setVegetationMasks({
      trees: grid((x, y) => (Math.hypot(x - width * 0.3, y - height * 0.3) < 40 ? 0 : 128 + (x % 64))),
      grass: { width: 64, height: 64, data: Uint8Array.from({ length: 4096 }, (_, index) => (index % 64 < 32 ? 40 : 255)) }
    });
    const bundle = new Uint8Array(await (await vista.exportBundle(engine)).arrayBuffer());
    const shots = {};

    for (const [name, camera] of Object.entries(views)) {
      shots[name] = await window.shoot(camera);
    }

    shots.biomes = await window.shoot(views.top, "biomes");
    return {
      digests: await window.mapDigests(),
      bundle: Array.from(bundle),
      shots
    };
  }, views);
  const bundleBytes = Uint8Array.from(first.bundle);
  fs.writeFileSync(`${out}/bundle.zip`, bundleBytes);
  await painted.close();

  const fresh = await open();
  const second = await fresh.evaluate(async ({ bytes, views }) => {
    const vista = await import("../../dist/index.js");
    const engine = window.engine;
    const phases = [];
    engine.on("progress", ({ phase, progress }) => phase === "bundle" && phases.push(progress));
    const started = performance.now();
    await vista.loadBundle(engine, Uint8Array.from(bytes));
    const loadMs = Math.round(performance.now() - started);
    // The heights changed under the page's own helpers.
    const heights = new Float32Array(engine.exportHeightmap().buffer.slice(0));
    const metadata = window.metadata;
    window.heightAt = (x, z) => heights[
      Math.round(z / metadata.metresPerSample + (metadata.height - 1) / 2) * metadata.width +
      Math.round(x / metadata.metresPerSample + (metadata.width - 1) / 2)
    ] ?? 0;
    const again = new Uint8Array(await (await vista.exportBundle(engine)).arrayBuffer());
    const shots = {};

    for (const [name, camera] of Object.entries(views)) {
      shots[name] = await window.shoot(camera);
    }

    shots.biomes = await window.shoot(views.top, "biomes");
    return { digests: await window.mapDigests(), again: Array.from(again), shots, loadMs, phases };
  }, { bytes: first.bundle, views });

  const entries = async (page, bytes) => page.evaluate(async (bytes) => {
    // The zip's files, by name, with a digest of each inflated file.
    const data = Uint8Array.from(bytes);
    const view = new DataView(data.buffer);
    const end = data.length - 22;
    const files = {};
    let at = view.getUint32(end + 16, true);

    for (let index = 0; index < view.getUint16(end + 10, true); index += 1) {
      const nameLength = view.getUint16(at + 28, true);
      const name = new TextDecoder().decode(data.subarray(at + 46, at + 46 + nameLength));
      const local = view.getUint32(at + 42, true);
      const start = local + 30 + view.getUint16(local + 26, true);
      let body = data.subarray(start, start + view.getUint32(at + 20, true));

      if (view.getUint16(at + 10, true) === 8) {
        body = new Uint8Array(await new Response(new Blob([body]).stream().pipeThrough(new DecompressionStream("deflate-raw"))).arrayBuffer());
      }

      files[name] = name === "manifest.json" ? JSON.parse(new TextDecoder().decode(body)) : await window.digest(body);
      at += 46 + nameLength;
    }

    return files;
  }, bytes);
  const [before, after] = [await entries(fresh, first.bundle), await entries(fresh, second.again)];
  const mismatches = [];

  for (const [kind, digest] of Object.entries(first.digests)) {
    if (second.digests[kind] !== digest) {
      mismatches.push(`map ${kind}`);
    }
  }

  for (const [name, digest] of Object.entries(before)) {
    if (name !== "manifest.json" && after[name] !== digest) {
      mismatches.push(`bundle file ${name}`);
    }
  }

  // The copy was loaded from a raw heightmap, which its generator names;
  // everything else in the manifest must match.
  const files = (manifest) => JSON.stringify(manifest.files, (key, value) => (key === "generator" ? undefined : value));

  if (files(before["manifest.json"]) !== files(after["manifest.json"])) {
    mismatches.push("manifest files");
  }

  const differences = {};

  for (const name of Object.keys(first.shots)) {
    save(`roundtrip-${name}-exported.png`, first.shots[name]);
    save(`roundtrip-${name}-loaded.png`, second.shots[name]);
    differences[name] = await difference(fresh, first.shots[name], second.shots[name]);
  }

  const roundtrip = {
    mismatches,
    maps: Object.keys(first.digests),
    bundleFiles: Object.keys(before),
    version: before["manifest.json"].version,
    loadMs: second.loadMs,
    progress: second.phases,
    meanPixelDifference: differences
  };
  console.log("roundtrip", JSON.stringify(roundtrip));
  fs.writeFileSync(`${out}/roundtrip.json`, JSON.stringify(roundtrip, null, 2));
  failed ||= mismatches.length > 0;
  await fresh.close();
}

const fresh = await open();

// 2. Decoding a 4096 x 4096 16-bit PNG.
if (part("decode")) {
  const decode = await fresh.evaluate(async () => {
    const vista = await import("../../dist/index.js");
    const side = 4096;
    const data = new Float32Array(side * side);

    for (let y = 0; y < side; y += 1) {
      for (let x = 0; x < side; x += 1) {
        data[y * side + x] = Math.sin(x * 0.01) * Math.cos(y * 0.013) * 800 + Math.sin((x + y) * 0.07) * 20;
      }
    }

    const map = { kind: "height", width: side, height: side, channels: 1, type: "float32", data, encoding: { metresPerPixel: [1, 1], seaLevelMetres: 0, generator: "test" } };
    const png = new Uint8Array(await (await vista.encodePng(map, { bitDepth: 16, smaller: true })).arrayBuffer());
    const times = [];

    for (let run = 0; run < 5; run += 1) {
      const started = performance.now();
      const decoded = await vista.decodePng(png);
      times.push(Math.round(performance.now() - started));

      if (decoded.width !== side || decoded.bitDepth !== 16) {
        throw new Error("The 4096 x 4096 PNG decoded wrongly.");
      }
    }

    return { bytes: png.length, times, medianMs: times.sort((a, b) => a - b)[2] };
  });
  console.log("decode", JSON.stringify(decode));
  fs.writeFileSync(`${out}/decode.json`, JSON.stringify(decode, null, 2));
  failed ||= decode.medianMs >= 1500;
}

// 3. A 1024 x 1024 16-bit heightmap made in the page.
if (part("heightmap")) {
  const heightmap = await fresh.evaluate(async () => {
    const vista = await import("../../dist/index.js");
    const side = 1024;
    const data = new Float32Array(side * side);

    for (let y = 0; y < side; y += 1) {
      for (let x = 0; x < side; x += 1) {
        const [u, v] = [x / side - 0.5, y / side - 0.5];
        // A ridged range across an island that falls to the sea at its edges.
        const island = Math.max(0, 1 - Math.hypot(u, v) * 2.2);
        const ridges = 1 - Math.abs(Math.sin(u * 19 + Math.sin(v * 7) * 1.5));
        data[y * side + x] = island * (600 + 700 * ridges * island) + Math.sin(u * 83) * Math.cos(v * 71) * 25 - 60;
      }
    }

    const map = { kind: "height", width: side, height: side, channels: 1, type: "float32", data, encoding: { metresPerPixel: [8, 8], seaLevelMetres: 0, generator: "test" } };
    const png = await vista.encodePng(map, { bitDepth: 16 });
    const handle = await window.engine.loadHeightmapImage(png, { metresPerSample: 8 });
    const heights = new Float32Array(window.engine.exportMap("sourceHeight").data);
    // Within one 16-bit step of the heights the PNG was made from.
    const step = (png.range[1] - png.range[0]) / 65535;
    const worst = heights.reduce((most, value, index) => Math.max(most, Math.abs(value - data[index])), 0);
    const metadata = handle.metadata;
    window.metadata = metadata;
    window.heightAt = (x, z) => heights[
      Math.round(z / metadata.metresPerSample + (metadata.height - 1) / 2) * metadata.width +
      Math.round(x / metadata.metresPerSample + (metadata.width - 1) / 2)
    ] ?? 0;
    const half = (side - 1) * 8 / 2;
    return {
      size: [metadata.width, metadata.height],
      range: png.range,
      worstErrorMetres: worst,
      stepMetres: step,
      top: await window.shoot(window.above(0, 0, half / Math.tan((27.5 * Math.PI) / 180) + 200)),
      low: await window.shoot({ position: [0, 700, 3400], target: [0, 350, 0] })
    };
  });
  save("heightmap-top.png", heightmap.top);
  save("heightmap-low.png", heightmap.low);
  console.log("heightmap", JSON.stringify({ ...heightmap, top: undefined, low: undefined }));
  failed ||= heightmap.worstErrorMetres > heightmap.stepMetres;
}

await fresh.close();

// 4. Painted biomes, water and vegetation on the default island.
if (part("scene")) {
  const scene = await open();
  const shots = await scene.evaluate(async () => {
    const engine = window.engine;
    const { width, height, metresPerSample } = window.metadata;
    const grid = (fill) => ({ width, height, data: Uint8Array.from({ length: width * height }, (_, index) => fill(index % width, Math.floor(index / width))) });
    const half = ((width - 1) * metresPerSample) / 2;
    const top = window.above(0, 0, half / Math.tan((27.5 * Math.PI) / 180) + 200);
    const result = {};
    const warnings = [];
    engine.on("warning", ({ message }) => warnings.push(message));
    const toWorld = (x, y) => [(x - (width - 1) / 2) * metresPerSample, (y - (height - 1) / 2) * metresPerSample];
    // Look from `from` to `to` (world x and z), each `lift` metres above
    // the ground under it, so the camera never sits inside a hill.
    const look = (from, to, lift = [150, 0]) => ({
      position: [from[0], window.heightAt(...from) + lift[0], from[1]],
      target: [to[0], window.heightAt(...to) + lift[1], to[1]]
    });
    // Three large regions meeting at the island's centre: forest, savannah
    // and ice, as wedges.
    const land = window.engine.exportMap("sourceHeight").data;
    let [cx, cy, count] = [0, 0, 0];
    land.forEach((h, index) => h > 5 && ([cx, cy, count] = [cx + (index % width), cy + Math.floor(index / width), count + 1]));
    [cx, cy] = [cx / count, cy / count];
    const wedge = (x, y) => Math.floor(((Math.atan2(y - cy, x - cx) + Math.PI) / (2 * Math.PI)) * 3) % 3;
    engine.setBiomeMap({ ...grid((x, y) => [3, 8, 18][wedge(x, y)]), borderSamples: 6 });
    result.biomesDebug = await window.shoot(top, "biomes");
    result.biomesTop = await window.shoot(top);
    // The forest and ice border west of the centre, close up from above
    // and from low down: its warp and the ground's blend.
    const border = toWorld(cx - 40, cy);
    result.biomesBorderDebug = await window.shoot(window.above(...border, 700), "biomes");
    result.biomesBorderTop = await window.shoot(window.above(...border, 700));
    result.biomesBorder = await window.shoot(look([border[0] - 350, border[1] + 350], border));
    engine.setBiomeMap(null);

    // A painted lake and river.
    const lake = [Math.round(width * 0.4), Math.round(height * 0.55)];
    engine.setWaterMask(grid((x, y) => (Math.hypot(x - lake[0], y - lake[1]) < 14 ? 220 : Math.abs(y - lake[1] - (x - lake[0]) * 0.3) < 1.2 && x > lake[0] && x < lake[0] + 90 ? 70 : 0)));
    const [lx, lz] = [(lake[0] - width / 2) * metresPerSample, (lake[1] - height / 2) * metresPerSample];
    result.water = await window.shoot({ position: [lx - 300, window.heightAt(lx, lz) + 220, lz + 380], target: [lx + 200, window.heightAt(lx, lz), lz] });
    engine.setWaterMask(null);

    // Trees: a clearing on the west half, a thicket on the east.
    const forest = window.findBiome("innerForest") ?? window.findBiome("outerForest") ?? [0, 0];
    const [fx, fz] = forest.map((v) => Math.round(v / metresPerSample + width / 2));
    const view = { position: [forest[0], window.heightAt(...forest) + 140, forest[1] + 180], target: [forest[0], window.heightAt(...forest), forest[1]] };
    result.treesBefore = await window.shoot(view);
    engine.setVegetationMasks({ trees: grid((x, y) => (Math.hypot(x - fx + 6, y - fz) < 8 ? 0 : x > fx + 4 ? 255 : 128)) });
    result.treesMasked = await window.shoot(view);
    result.treesTop = await window.shoot(window.above(forest[0], forest[1], 260));
    engine.setVegetationMasks({ trees: null });

    // Grass: none west of a meadow's centre, doubled east of it. The
    // meadow is the grassiest gentle ground away from the coast.
    const grassMap = engine.exportMap("grassDensity").data;
    const slope = engine.exportMap("slope").data;
    let [gx, gz, best] = [cx, cy, -1];

    for (let y = 16; y < height - 16; y += 4) {
      for (let x = 16; x < width - 16; x += 4) {
        let score = 0;

        for (let dy = -6; dy <= 6; dy += 2) {
          for (let dx = -6; dx <= 6; dx += 2) {
            const index = (y + dy) * width + x + dx;
            score += slope[index] < 12 && land[index] > 8 ? grassMap[index] : -255;
          }
        }

        [gx, gz, best] = score > best ? [x, y, score] : [gx, gz, best];
      }
    }

    const meadow = toWorld(gx, gz);
    const low = look([meadow[0], meadow[1] + 40], [meadow[0], meadow[1] - 25], [9, 0]);
    const far = look([meadow[0], meadow[1] + 320], meadow, [70, 0]);
    result.grassBefore = await window.shoot(low, "none", 14);
    engine.setVegetationMasks({ grass: grid((x) => (x < gx ? 0 : 255)) });
    result.grassMasked = await window.shoot(low, "none", 14);
    result.grassFar = await window.shoot(far, "none", 14);
    engine.setVegetationMasks({ grass: null });
    result.grassFarBefore = await window.shoot(far, "none", 14);
    result.warnings = warnings;
    result.places = { forest, meadow };
    return result;
  });

  for (const [name, url] of Object.entries(shots)) {
    if (typeof url === "string") {
      save(`${name.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`)}.png`, url);
    }
  }

  console.log("scene", JSON.stringify({ warnings: shots.warnings, places: shots.places }));
}

await browser.close();
process.exit(failed ? 1 : 0);
