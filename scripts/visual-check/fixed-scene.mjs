// Like-for-like performance gate: render one fixed scene and print the GPU
// time of every pass, so a change can be compared with the code before it.
//
//   python3 -m http.server 8124        (from the repository root)
//   node scripts/visual-check/fixed-scene.mjs [out.png]
//
// The terrain is a fixed heightmap (`fixed-512.f32.gz`, 512 x 512 float32
// samples at 12 m), loaded with `loadRawHeightmap`, so generator changes
// do not change the scene. The scene is drawn clear and then in rain with
// lens drops. Each prints `gpuPassTimesMs` averaged over 6 frames, after 3
// warm-up frames. Software rendering makes absolute times meaningless;
// compare passes between runs on the same machine.
//
// `--write-heightmap` regenerates the heightmap file from the current
// generator (continental, seed 2, land to the edges). It was run once;
// rerunning it changes the scene, so do it only when the gate is reset.
import fs from "node:fs";
import zlib from "node:zlib";

const playwright = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const port = process.env.PORT ?? "8124";
const here = new URL(".", import.meta.url);
const heightmapFile = new URL("fixed-512.f32.gz", here);
const size = [960, 600];
// Dynamic resolution would change the work between runs.
const engine = { quality: { preset: "balanced", dynamicResolution: false, renderScale: 1 } };
const warmFrames = 3;
const measuredFrames = 6;
const camera = {
  position: [-1500, 1500, 2600],
  target: [0, 300, 0],
  fieldOfViewDegrees: 55,
  nearMetres: 0.5,
  farMetres: 120000
};
const heightmapOptions = {
  width: 512,
  height: 512,
  sampleFormat: "float32",
  byteOrder: "little-endian",
  metresPerSample: 12,
  heightScaleMetres: 1,
  seaLevelMetres: 0
};
const scenes = {
  clear: {},
  rain: {
    setWeather: {
      enabled: true,
      state: "rain",
      autoCycle: false,
      transitionSeconds: 0.1,
      lensDrops: true
    }
  }
};
const writeHeightmap = process.argv.includes("--write-heightmap");
const out = process.argv.slice(2).find((argument) => !argument.startsWith("--"));

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
  const page = await open({
    size,
    engine,
    heightmap: { url: "fixed-512.f32.gz", options: heightmapOptions },
    camera
  });
  const results = {};

  for (const [name, calls] of Object.entries(scenes)) {
    await page.evaluate((calls) => {
      for (const [method, value] of Object.entries(calls)) {
        window.engine[method](value);
      }
    }, calls);
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
    console.log(`${name} ${JSON.stringify(passes)}`);

    if (out) {
      const result = await page.evaluate(() => window.capture(1));
      fs.writeFileSync(
        out.replace(/\.png$/, `-${name}.png`),
        Buffer.from(result.url.split(",")[1], "base64")
      );
    }
  }
}

await browser.close();
process.exit(failed ? 1 : 0);
