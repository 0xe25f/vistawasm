// Peak WASM memory for the largest terrain jobs, at 2048 x 2048, the
// largest terrain: loading a float32 raw heightmap at 12 m and at 60 m,
// generating a terrain at 12 m and at 30 m, and loading one, then setting
// a full-size painted water mask (a world rebuild) and exporting every map
// at full size. Each runs in its own page, as WASM memory only grows, and
// prints the memory's size before and after
// (`WebAssembly.Memory.buffer.byteLength`, which is its high-water mark).
//
//   python3 -m http.server 8124        (from the repository root)
//   node scripts/visual-check/memory-check.mjs [raw|raw60|maps|generate|generate30]
//
// Playwright is not a dependency: set PLAYWRIGHT_MODULE to its path if it
// is not resolvable, and CHROMIUM_PATH to a Chromium build if needed.

const playwright = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const port = process.env.PORT ?? "8124";
const only = process.argv[2];
const jobs = {
  // The largest raw map, on a gentle slope so the hydrology has
  // somewhere to drain, at 12 m: 25 km across, within the vegetation
  // cap, so trees, grass and boulders are placed too.
  raw: async () => {
    const side = 2048;
    const heights = new Float32Array(side * side);

    for (let z = 0; z < side; z += 1) {
      for (let x = 0; x < side; x += 1) {
        heights[z * side + x] = 40 + 0.01 * x + 8 * Math.sin(z / 300);
      }
    }

    await window.engine.loadRawHeightmap(heights.buffer, {
      width: side,
      height: side,
      sampleFormat: "float32",
      byteOrder: "little-endian",
      metresPerSample: 12,
      heightScaleMetres: 1,
      seaLevelMetres: 0
    });
  },
  // The same at 60 m: 123 km across, past the vegetation cap.
  raw60: async () => {
    const side = 2048;
    const heights = new Float32Array(side * side);

    for (let z = 0; z < side; z += 1) {
      for (let x = 0; x < side; x += 1) {
        heights[z * side + x] = 40 + 0.01 * x + 8 * Math.sin(z / 300);
      }
    }

    await window.engine.loadRawHeightmap(heights.buffer, {
      width: side,
      height: side,
      sampleFormat: "float32",
      byteOrder: "little-endian",
      metresPerSample: 60,
      heightScaleMetres: 1,
      seaLevelMetres: 0
    });
  },
  // A 2048 x 2048 map at 12 m, then a full-size painted mask (a world
  // rebuild) and every map exported at full size.
  maps: async () => {
    const side = 2048;
    const heights = new Float32Array(side * side);

    for (let z = 0; z < side; z += 1) {
      for (let x = 0; x < side; x += 1) {
        heights[z * side + x] = 40 + 0.01 * x + 8 * Math.sin(z / 300);
      }
    }

    await window.engine.loadRawHeightmap(heights.buffer, {
      width: side,
      height: side,
      sampleFormat: "float32",
      byteOrder: "little-endian",
      metresPerSample: 12,
      heightScaleMetres: 1,
      seaLevelMetres: 0
    });
    // A painted water mask at the terrain's full size, which rebuilds the
    // world, then every map exported at full size: the largest painted
    // map and exports beside the largest terrain.
    const mask = new Uint8Array(side * side);

    // A small painted lake near the middle.
    for (let z = side / 2 - 32; z < side / 2 + 32; z += 1) {
      mask.fill(200, z * side + side / 2 - 32, z * side + side / 2 + 32);
    }

    window.engine.setWaterMask({ width: side, height: side, data: mask });

    for (const kind of (await import("../../dist/index.js")).MAP_KINDS) {
      window.engine.exportMap(kind, { size: [side, side] });
    }
  },
  generate: async () => {
    // The visual checks' default terrain (`fractalDefaults` in
    // `scenes.mjs`) at 2048 samples.
    await window.engine.generateFractal({
      seed: 12345,
      size: 2048,
      horizontalScaleMetres: 12,
      verticalScale: 1,
      seaLevelMetres: 0,
      noise: { kind: "ridged", octaves: 7, gain: 0.52, lacunarity: 2.05, warp: 0.15 },
      shape: { island: 0.35 }
    });
  },
  generate30: async () => {
    // The same at 30 m with open edges: 61 km of rugged land, the most
    // rivers a map this size makes.
    await window.engine.generateFractal({
      seed: 12345,
      size: 2048,
      horizontalScaleMetres: 30,
      verticalScale: 1,
      seaLevelMetres: 0,
      noise: { kind: "ridged", octaves: 7, gain: 0.52, lacunarity: 2.05, warp: 0.15 },
      shape: { island: 0 },
      edges: "open"
    });
  }
};

for (const [name, job] of Object.entries(jobs)) {
  if (only && only !== name) {
    continue;
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
      "--enable-unsafe-swiftshader",
      "--js-flags=--max-old-space-size=8192"
    ]
  });
  const page = await browser.newPage({ viewport: { width: 64, height: 64 } });
  page.on("pageerror", (error) => console.log(`page error: ${error.message}`));
  // The smallest terrain, so the job's own memory stands out.
  const config = { size: [64, 64], terrain: { size: 16 } };
  await page.goto(`http://127.0.0.1:${port}/scripts/visual-check/index.html?config=${encodeURIComponent(JSON.stringify(config))}`);
  await page.waitForFunction(() => window.ready === true, null, { timeout: 600000 });
  // A WASM trap (running out of memory, say) can leave the job's promise
  // unsettled, so the page error ends the wait too.
  const trapped = new Promise((resolve) => page.once("pageerror", (error) => resolve({ error: error.message })));
  const result = await Promise.race([
    page.evaluate(async (source) => {
      const wasm = await (await import("../../dist/pkg/vista_wasm.js")).default();
      window.memory = wasm.memory;
      window.before = wasm.memory.buffer.byteLength;
      const started = performance.now();

      try {
        await (0, eval)(`(${source})`)();
      } catch (error) {
        return { error: `${error.code ?? ""} ${error.message}` };
      }

      return { ms: performance.now() - started };
    }, job.toString()),
    trapped
  ]);
  const { before, after } = await page.evaluate(() => ({ before: window.before, after: window.memory.buffer.byteLength }));
  const mib = (bytes) => `${(bytes / 2 ** 20).toFixed(0)} MiB`;
  const outcome = result.error ? `failed: ${result.error}` : `${Math.round(result.ms)} ms`;
  console.log(`${name}: WASM memory ${mib(before)} before, ${mib(after)} after (${after} bytes), ${outcome}`);
  await browser.close();
}
