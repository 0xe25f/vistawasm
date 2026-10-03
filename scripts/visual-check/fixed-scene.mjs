// Like-for-like performance gate: render one fixed scene and print the GPU
// time of every pass, so a change can be compared with the code before it.
//
//   python3 -m http.server 8124        (from the repository root)
//   node scripts/visual-check/fixed-scene.mjs [out.png]
//
// The terrain is a fixed heightmap (`fixed-512.f32.gz`, 512 x 512 float32
// samples at 12 m), loaded with `loadRawHeightmap`, so generator changes
// do not change the scene. The scene is drawn clear, then in rain with
// lens drops, then at -18 °C looking out over pack ice. Each prints `gpuPassTimesMs` averaged over 6 frames, after 40
// warm-up frames. Software rendering makes absolute times meaningless;
// compare passes between runs on the same machine.
//
// `--grass` switches grass on (at its default density), to time the grass
// pass as well; grass is on by default, so this only pins its settings.
// `--no-grass` switches it off, to time the scene without it.
//
// `--jungle` renders a dense forest instead: the same heightmap, warm and
// wet (27 °C, moisture bias 0.7), with trees at density 4 and grass on,
// from three fixed cameras: a close-up 6 m up inside the stand, a
// clearing edge from 20 m, and a hillside from 300 m. `--split` also
// times the trees pass as three passes (canopy meshes, understorey meshes
// and impostors); the extra passes cost a little, so use it only here.
//
// `--sky` renders the sky views instead, on the same heightmap from
// over the sea at (0, 40, -4000): cumulus from below, a low sun, a
// mackerel sky, a veiled sun and the rain deck. Their names hold spaces,
// so each line prints the name with hyphens (`cumulus-from-below {...}`),
// and `--scene=` takes that form too.
//
// `--meadow=<density>` renders the grass meadow on generated rolling
// hills at that grass density (0.5 and 4 are the budgeted ones), with the
// camera 2, 4, 10, 25, 30 and 60 m above the ground. Each line is named
// by the height (`4m {...}`).
//
// `--scene=<name>` measures one scene only.
//
// `--write-heightmap` regenerates the heightmap file from the current
// generator (continental, seed 2, land to the edges). It was run once;
// rerunning it changes the scene, so do it only when the gate is reset.
//
// Every scene is defined in `scenes.mjs`, which the profile page in
// `bench/gpu/` shares, so the two never drift.
import fs from "node:fs";
import zlib from "node:zlib";
import * as scenes from "./scenes.mjs";

const playwright = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const port = process.env.PORT ?? "8124";
const here = new URL(".", import.meta.url);
const heightmapFile = new URL(scenes.heightmapFile, here);
const size = [960, 600];
const jungle = process.argv.includes("--jungle");
const sky = process.argv.includes("--sky");
const meadowText = process.argv.find((argument) => argument.startsWith("--meadow="))?.slice(9);
const meadow = meadowText === undefined ? null : Number(meadowText);

if (meadow !== null && !(meadow > 0 && Number.isFinite(meadow))) {
  console.error(`--meadow needs a grass density above 0, such as --meadow=0.5, not "${meadowText}".`);
  process.exit(2);
}

const engine = meadow === null
  ? scenes.fixedEngine({
    grass: process.argv.includes("--grass"),
    noGrass: process.argv.includes("--no-grass"),
    jungle,
    split: process.argv.includes("--split")
  })
  : scenes.meadowEngine(meadow);
// Streamed trees and grass fill their tiles over the first frames, eight
// a frame, so the warm-up lets them settle.
const warmFrames = 40;
const measuredFrames = 6;
const writeHeightmap = process.argv.includes("--write-heightmap");
const out = process.argv.slice(2).find((argument) => !argument.startsWith("--"));

// `[name as printed, calls]` for the chosen set of scenes.
function chosenScenes() {
  if (meadow !== null) {
    return scenes.meadowHeights.map((height) => [`${height}m`, scenes.meadowCamera(height)]);
  }

  if (sky) {
    return Object.entries(scenes.skyScenes).map(([name, calls]) => [scenes.slug(name), calls]);
  }

  return Object.entries(jungle ? scenes.jungleScenes : scenes.fixedScenes);
}

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

async function open(config) {
  const page = await browser.newPage({ viewport: { width: size[0], height: size[1] } });
  // WebGPU reports invalid shaders and pipelines as warnings.
  page.on("console", (message) => {
    const webgpu = message.type() === "warning" && /Invalid|Error while parsing/.test(message.text());

    if (message.type() === "error" || webgpu) {
      console.log(`console ${message.type()}: ${message.text().slice(0, 1500)}`);
      failed ||= !/Failed to load resource/.test(message.text());
    }
  });
  page.on("pageerror", (error) => {
    console.log(`page error: ${error.message}`);
    failed = true;
  });
  const text = JSON.stringify(config);
  await page.goto(
    `http://127.0.0.1:${port}/scripts/visual-check/index.html?config=${encodeURIComponent(text)}`
  );
  await page.waitForFunction(() => window.ready === true, null, { timeout: 600000 });
  const log = await page.evaluate(() => window.log.splice(0));

  if (log.some((line) => line.startsWith("error"))) {
    throw new Error(`The scene did not load: ${log.join("; ")}`);
  }

  return page;
}

if (writeHeightmap) {
  const page = await open({
    size,
    terrain: { landform: "continental", seed: 2, shape: null, edges: "open" }
  });
  const base64 = await page.evaluate(() => {
    const bytes = window.engine.exportHeightmap();
    let binary = "";

    for (let index = 0; index < bytes.length; index += 0x8000) {
      binary += String.fromCharCode(...bytes.subarray(index, index + 0x8000));
    }

    return btoa(binary);
  });
  const heights = new Float32Array(new Uint8Array(Buffer.from(base64, "base64")).buffer);

  // Heights rounded to 1/64 m leave the low mantissa bits zero, which
  // gzip packs far better, and 16 mm is far below what a frame can show.
  for (let index = 0; index < heights.length; index += 1) {
    heights[index] = Math.round(heights[index] * 64) / 64;
  }

  fs.writeFileSync(heightmapFile, zlib.gzipSync(Buffer.from(heights.buffer), { level: 9 }));
  console.log(`wrote ${heightmapFile.pathname} (${fs.statSync(heightmapFile).size} bytes)`);
} else {
  const page = await open(meadow === null
    ? {
      size,
      engine,
      heightmap: { url: scenes.heightmapFile, options: scenes.heightmapOptions },
      camera: scenes.camera
    }
    : { size, engine, terrain: scenes.meadowTerrain, camera: scenes.camera });
  const results = {};

  // `--scene=<name>` measures only that scene (with those before it
  // skipped, so a camera keeps its climate from the first).
  const only = process.argv.find((argument) => argument.startsWith("--scene="))?.slice(8);
  const chosen = chosenScenes();

  for (const [index, [name, calls]] of chosen.entries()) {
    if (only && name !== only) {
      if (index === 0) {
        await page.evaluate((calls) => {
          for (const [method, value] of Object.entries(calls)) {
            if (method !== "ground" && method !== "setCamera") {
              window.engine[method](value);
            }
          }
        }, calls);
      }

      continue;
    }

    await page.evaluate((calls) => window.applyCalls(calls), calls);
    const stats = await page.evaluate(
      ([warm, frames]) => window.measure(warm, frames),
      [warmFrames, measuredFrames]
    );
    const passes = {};

    for (const frame of stats) {
      for (const [pass, ms] of Object.entries(frame.gpuPassTimesMs ?? {})) {
        passes[pass] = (passes[pass] ?? 0) + ms / stats.length;
      }
    }

    for (const pass of Object.keys(passes)) {
      passes[pass] = Math.round(passes[pass] * 10) / 10;
    }

    results[name] = passes;
    // Tree triangles drawn (read back), where the build reports them.
    const triangles = stats.reduce((sum, frame) => sum + (frame.treeTriangles ?? 0), 0) / stats.length;
    console.log(`${name} ${JSON.stringify(passes)}${triangles ? ` treeTriangles ${Math.round(triangles)}` : ""}`);

    if (out) {
      const result = await page.evaluate(() => window.capture(1));
      fs.writeFileSync(
        out.replace(/\.png$/, `-${name}.png`),
        Buffer.from(result.url.split(",")[1], "base64")
      );
    }
  }

  if (only && !chosen.some(([name]) => name === only)) {
    console.log(`No scene is named "${only}". Choose one of: ${chosen.map(([name]) => name).join(", ")}.`);
    failed = true;
  }
}

await browser.close();
process.exit(failed ? 1 : 0);
