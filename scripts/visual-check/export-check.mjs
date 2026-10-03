// Check map, tree and bundle export in headless Chromium against what the
// engine draws, and time the exports.
//
//   python3 -m http.server 8124        (from the repository root)
//   node scripts/visual-check/export-check.mjs out-dir ['<config JSON>']
//
// The config is the one `capture.mjs` takes (`size`, `engine`, `terrain`,
// `set`); `size` defaults to 600 x 600 and the terrain to the default
// island. The script writes to `out-dir`:
//
// - `bundle.zip`, from `exportBundle()` with previews;
// - `topdown.png` and `topdown-biomes.png`: the whole map from straight
//   above, as drawn and in the `biomes` debug view, to lay beside the
//   bundle's `biome.png`, `water.png`, `splat0.png`, `normals.png` and
//   `treeDensity.png` (north is up in both);
// - `trees.png`: a forest from above, with a red dot on every tree
//   `exportTrees()` lists there, and `trees.csv` for that region;
// - `timings.json`: how long each export took, in milliseconds.
//
// `--big` also times resampling the height, normals and materials maps
// to 8192 x 8192.
import fs from "node:fs";

const playwright = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const args = process.argv.slice(2).filter((arg) => !arg.startsWith("--"));
const [out = "export-check", configText = "{}"] = args;
const big = process.argv.includes("--big");
const config = { size: [600, 600], ...JSON.parse(configText) };
const port = process.env.PORT ?? "8124";
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
const page = await browser.newPage({ viewport: { width: config.size[0], height: config.size[1] } });
let failed = false;

page.on("console", (message) => {
  if (message.type() === "error") {
    console.log(`console error: ${message.text().slice(0, 1500)}`);
    failed ||= !/Failed to load resource/.test(message.text());
  }
});
page.on("pageerror", (error) => {
  console.log(`page error: ${error.message}`);
  failed = true;
});

await page.goto(
  `http://127.0.0.1:${port}/scripts/visual-check/index.html?config=${encodeURIComponent(JSON.stringify(config))}`
);
await page.waitForFunction(() => window.ready === true, null, { timeout: 600000 });
console.log("log:", JSON.stringify(await page.evaluate(() => window.log.splice(0))));

const timings = await page.evaluate(async (big) => {
  const vista = await import("../../dist/index.js");
  const engine = window.engine;
  const time = async (work) => {
    const start = performance.now();
    const result = await work();
    return [Math.round(performance.now() - start), result];
  };
  const timings = { native: {}, png: {} };

  for (const kind of vista.MAP_KINDS) {
    const [ms, map] = await time(() => engine.exportMap(kind));
    timings.native[kind] = ms;
    timings.size = [map.width, map.height];
    [timings.png[kind]] = await time(() => vista.encodePng(map));
  }

  [timings.trees, window.allTrees] = await time(() => engine.exportTrees());
  timings.treeCount = window.allTrees.length;
  [timings.bundle, window.bundle] = await time(() => vista.exportBundle(engine, { previews: true }));
  timings.bundleBytes = window.bundle.size;

  if (big) {
    timings.resampled8192 = {};

    for (const kind of ["height", "normals", "materials"]) {
      [timings.resampled8192[kind]] = await time(() => engine.exportMap(kind, { size: [8192, 8192] }).width);
    }

    [timings.resampled2048Png] = await time(async () =>
      vista.encodePng(engine.exportMap("height", { size: [2048, 2048] }), { bitDepth: 16 }));
  }

  return timings;
}, big);
console.log("timings", JSON.stringify(timings));
fs.writeFileSync(`${out}/timings.json`, JSON.stringify(timings, null, 2));

const bundle = await page.evaluate(async () => {
  const bytes = new Uint8Array(await window.bundle.arrayBuffer());
  let text = "";

  for (let at = 0; at < bytes.length; at += 32768) {
    text += String.fromCharCode(...bytes.subarray(at, at + 32768));
  }

  return btoa(text);
});
fs.writeFileSync(`${out}/bundle.zip`, Buffer.from(bundle, "base64"));

// Look straight down (north, -z, up the image) from `height` metres over
// `(x, z)`, with a 55-degree field of view.
async function topDown(name, [x, z], height, debugView = "none") {
  const shot = await page.evaluate(async ({ x, z, height, debugView }) => {
    const ground = window.heightAt(x, z);
    window.engine.setDebugView(debugView);
    window.engine.setCamera({
      position: [x, ground + height, z + 0.01],
      target: [x, ground, z],
      fieldOfViewDegrees: 55,
      nearMetres: 0.5,
      farMetres: 120000
    });
    return window.capture(8);
  }, { x, z, height, debugView });
  fs.writeFileSync(`${out}/${name}`, Buffer.from(shot.url.split(",")[1], "base64"));
  return shot;
}

const half = await page.evaluate(() => ((window.metadata.width - 1) * window.metadata.metresPerSample) / 2);
// The whole map fits the view's height from this far up.
const above = half / Math.tan((27.5 * Math.PI) / 180) + 200;
await topDown("topdown.png", [0, 0], above);
await topDown("topdown-biomes.png", [0, 0], above, "biomes");

// A forest from 90 m up, with every exported tree marked where the camera
// projects it.
const forest = await page.evaluate(() => window.findBiome("innerForest") ?? window.findBiome("outerForest") ?? [0, 0]);
const reach = 60;
const trees = await page.evaluate(({ forest, reach }) => {
  const [x, z] = forest;
  return window.engine.exportTrees({ region: { minX: x - reach, minZ: z - reach, maxX: x + reach, maxZ: z + reach } });
}, { forest, reach });
console.log(`forest at ${JSON.stringify(forest)}: ${trees.length} trees exported within ${reach} m`);
const csv = await page.evaluate(async (trees) => (await import("../../dist/index.js")).treesToCsv(trees), trees);
fs.writeFileSync(`${out}/trees.csv`, csv);
const shot = await topDown("trees-plain.png", forest, 90);
const marked = await page.evaluate(async ({ url, trees, forest, size }) => {
  const [w, h] = size;
  const [cx, cz] = forest;
  const ground = window.heightAt(cx, cz);
  const eye = [cx, ground + 90, cz + 0.01];
  const image = new Image();
  image.src = url;
  await image.decode();
  const canvas = document.createElement("canvas");
  canvas.width = w;
  canvas.height = h;
  const context = canvas.getContext("2d");
  context.drawImage(image, 0, 0);
  // Straight down, with -z up the screen: a point at (dx, dz) from the eye
  // lands at the screen centre plus (dx, dz) over the depth, scaled by the
  // focal length.
  const focal = h / 2 / Math.tan((27.5 * Math.PI) / 180);
  context.fillStyle = "red";

  for (const tree of trees) {
    const depth = eye[1] - tree.y;
    const px = w / 2 + ((tree.x - eye[0]) / depth) * focal;
    const py = h / 2 + ((tree.z - eye[2]) / depth) * focal;
    context.fillRect(px - 1.5, py - 1.5, 3, 3);
  }

  return canvas.toDataURL("image/png");
}, { url: shot.url, trees, forest, size: config.size });
fs.writeFileSync(`${out}/trees.png`, Buffer.from(marked.split(",")[1], "base64"));

await browser.close();
process.exit(failed ? 1 : 0);
