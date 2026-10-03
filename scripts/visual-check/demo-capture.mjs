// Drive the demo in headless Chromium and check its overlays and Paint
// tab: clicks, drags and touch strokes on the real page, screenshots of
// the 2D UI, and read-backs of the 3D view.
//
//   npm run build:demo
//   python3 -m http.server 8124        (from the repository root)
//   node scripts/visual-check/demo-capture.mjs out-dir [--only=part]
//
// Parts, each run in turn, writing to `out-dir`:
//
// - `toggles`: the full UI at 1440 x 900 and 390 x 844, then keys 1, 2,
//   3, 4 and 0 in turn, each screenshotted; the restore button must show
//   while the panel is hidden, and there must be no horizontal scroll;
// - `paint`: a blank 512 x 512 painting 6 km wide with a raised ridge,
//   an inner forest, a river from the ridge, a tree patch and a lake,
//   screenshotted in 2D, rendered and read back from the 3D view; then
//   the lake is undone and the painting rendered again, saved, the page
//   reloaded, the painting opened and rendered once more. The last two
//   read-backs must match within a mean absolute difference of 1.0;
// - `sources`: start from the current map with its biomes, paint water
//   and trees, export every layer as a PNG, start again from those PNGs
//   and export again: the biome, water, tree and grass files must be
//   byte-identical; then start from the bundle the `paint` part saved,
//   if there is one;
// - `phone`: a short touch stroke at 390 x 844 with touch emulation;
// - `timing`: strokes with a 128-sample brush on a 1024 x 1024 painting
//   at 390 x 844 with touch emulation and the CPU slowed 4 times,
//   reporting the brush work per dab and per frame, and the frame gaps.
//
// Like `capture.mjs`, it uses the software WebGPU shim in
// `offscreen-canvas.js`, and exits non-zero on any page error, console
// error or failed check. Set PLAYWRIGHT_MODULE and CHROMIUM_PATH as
// there.
import fs from "node:fs";

const playwright = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const [out = "demo-capture"] = process.argv.slice(2).filter((arg) => !arg.startsWith("--"));
const only = process.argv.find((arg) => arg.startsWith("--only="))?.slice(7);
const port = process.env.PORT ?? "8124";
const url = `http://127.0.0.1:${port}/demo/index.html`;
const shim = fs.readFileSync(new URL("offscreen-canvas.js", import.meta.url), "utf8");
// Counts the frames the engine draws, to wait for new ones.
const frameCounter = `(() => {
  const next = GPUCanvasContext.prototype.getCurrentTexture;
  window.framesDrawn = 0;
  GPUCanvasContext.prototype.getCurrentTexture = function () {
    window.framesDrawn += 1;
    return next.call(this);
  };
})();`;
const results = {};
let failed = false;
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

function check(name, ok, detail = "") {
  console.log(`${ok ? "ok  " : "FAIL"} ${name}${detail ? `: ${detail}` : ""}`);
  failed ||= !ok;
}

async function open(width, height, touch = false) {
  const context = await browser.newContext({
    viewport: { width, height },
    hasTouch: touch,
    isMobile: touch,
    deviceScaleFactor: 1,
    acceptDownloads: true
  });
  const page = await context.newPage();
  // Software WebGPU makes loads slow, and slower still on a busy machine.
  page.setDefaultNavigationTimeout(300000);
  page.on("console", (message) => {
    if (message.type() === "error") {
      console.log(`console error: ${message.text().slice(0, 800)}`);
      failed ||= !/Failed to load resource/.test(message.text());
    }
  });
  page.on("pageerror", (error) => {
    console.log(`page error: ${error.message}`);
    failed = true;
  });
  await page.addInitScript(shim);
  await page.addInitScript(frameCounter);
  await page.goto(url);
  await ready(page);
  return { context, page };
}

/** Wait for the first terrain, which the demo generates on load. */
async function ready(page) {
  await page.waitForFunction(() => /ready|failed|lost/i.test(document.querySelector("#status")?.textContent ?? ""), null, { timeout: 900000 });
  const status = await page.textContent("#status");
  check("demo started", /ready/.test(status), status);
}

async function noHorizontalScroll(page, name) {
  const [scroll, width] = await page.evaluate(() => [document.documentElement.scrollWidth, window.innerWidth]);
  check(`${name}: no horizontal scroll`, scroll <= width, `${scroll} px of content in ${width} px`);
}

async function setValue(page, selector, value) {
  await page.$eval(selector, (element, value) => {
    element.value = String(value);
    element.dispatchEvent(new Event("input", { bubbles: true }));
    element.dispatchEvent(new Event("change", { bubbles: true }));
  }, value);
}

/** Wait for the engine to draw `frames` more frames, then read the 3D view back as RGBA. */
async function readBack(page, name, frames = 6) {
  const start = await page.evaluate(() => window.framesDrawn);
  await page.waitForFunction((target) => window.framesDrawn >= target, start + frames, { timeout: 900000 });
  const { width, height, rgba, png } = await page.evaluate(async () => {
    const device = window.gpuDevice;
    await device.queue.onSubmittedWorkDone();
    const texture = window.gpuTexture;
    const { width: w, height: h } = texture;
    const bytesPerRow = Math.ceil((w * 4) / 256) * 256;
    const buffer = device.createBuffer({ size: bytesPerRow * h, usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ });
    const encoder = device.createCommandEncoder();
    encoder.copyTextureToBuffer({ texture }, { buffer, bytesPerRow }, [w, h]);
    device.queue.submit([encoder.finish()]);
    await buffer.mapAsync(GPUMapMode.READ);
    const source = new Uint8Array(buffer.getMappedRange());
    const pixels = new Uint8ClampedArray(w * h * 4);
    const bgra = String(window.gpuFormat).startsWith("bgra");

    for (let y = 0; y < h; y += 1) {
      for (let x = 0; x < w; x += 1) {
        const from = y * bytesPerRow + x * 4;
        const to = (y * w + x) * 4;
        pixels[to] = source[from + (bgra ? 2 : 0)];
        pixels[to + 1] = source[from + 1];
        pixels[to + 2] = source[from + (bgra ? 0 : 2)];
        pixels[to + 3] = 255;
      }
    }

    buffer.unmap();
    const image = document.createElement("canvas");
    image.width = w;
    image.height = h;
    image.getContext("2d").putImageData(new ImageData(pixels, w, h), 0, 0);
    let binary = "";

    for (let index = 0; index < pixels.length; index += 32768) {
      binary += String.fromCharCode(...pixels.subarray(index, index + 32768));
    }

    return { width: w, height: h, rgba: btoa(binary), png: image.toDataURL("image/png") };
  });
  fs.writeFileSync(`${out}/${name}.png`, Buffer.from(png.split(",")[1], "base64"));
  return { width, height, rgba: Buffer.from(rgba, "base64") };
}

/** The mean absolute difference of the RGB values, over the part of the view `region` gives as fractions. */
function meanAbsoluteDifference(a, b, region = [0, 0, 1, 1]) {
  if (a.width !== b.width || a.height !== b.height) {
    return Infinity;
  }

  const [x0, y0, x1, y1] = [region[0] * a.width, region[1] * a.height, region[2] * a.width, region[3] * a.height].map(Math.round);
  let sum = 0;

  for (let y = y0; y < y1; y += 1) {
    for (let x = x0; x < x1; x += 1) {
      const index = (y * a.width + x) * 4;
      sum += Math.abs(a.rgba[index] - b.rgba[index]) + Math.abs(a.rgba[index + 1] - b.rgba[index + 1]) + Math.abs(a.rgba[index + 2] - b.rgba[index + 2]);
    }
  }

  return sum / ((x1 - x0) * (y1 - y0) * 3);
}

/** Screen position of painting sample `(x, y)`, given as fractions of the painting. */
async function toScreen(page, fx, fy) {
  const box = await page.$eval("#paintCanvas", (canvas) => {
    const rect = canvas.getBoundingClientRect();
    return { left: rect.left, top: rect.top, width: rect.width, height: rect.height };
  });
  return [box.left + fx * box.width, box.top + fy * box.height];
}

async function strokes(page) {
  return Number((await page.textContent("#paintHistoryReadout")).match(/^(\d+)/)?.[1] ?? 0);
}

/** Drag the mouse through `points` (fractions of the painting) and wait for the stroke to be recorded. */
async function drag(page, points, steps = 12) {
  const before = await strokes(page);
  const [first, ...rest] = await Promise.all(points.map(([fx, fy]) => toScreen(page, fx, fy)));
  await page.mouse.move(...first);
  await page.mouse.down();

  for (const point of rest) {
    await page.mouse.move(...point, { steps });
  }

  await page.mouse.up();
  await page.waitForFunction((count) => Number(document.querySelector("#paintHistoryReadout").textContent.match(/^(\d+)/)?.[1] ?? 0) > count, before);
}

async function brush(page, tool, size, strength = 1) {
  await page.selectOption("#paintTool", tool);
  await setValue(page, "#paintBrushSize", size);
  await setValue(page, "#paintStrength", strength);
}

async function startBlank(page, size) {
  await page.click("#tabPaint");
  await page.selectOption("#paintSource", "blank");
  await page.selectOption("#paintSize", String(size));
  await setValue(page, "#paintWidthKm", 6);
  await setValue(page, "#paintSea", 0);
  await page.click("#paintStart");
  await page.waitForFunction(() => /samples/.test(document.querySelector("#paintDocReadout").textContent));
}

/** Clouds, waves and weather off, so two renders of one painting can be compared. */
async function stillScene(page) {
  await setValue(page, "#cloudStyle", "off");
  await page.$eval("#wavesEnabled", (box) => {
    box.checked = false;
    box.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await page.$eval("#weatherEnabled", (box) => {
    box.checked = false;
    box.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

async function renderAndReadBack(page, name) {
  await page.click("#paintRender");
  await page.waitForFunction(() => document.querySelector("#tabExplore").getAttribute("aria-selected") === "true", null, { timeout: 900000 });
  console.log(`  ${await page.textContent("#paintRenderReadout")}`);
  return settledReadBack(page, name);
}

/** Read back every few frames until streamed terrain and vegetation stop changing the view. */
async function settledReadBack(page, name) {
  let previous = await readBack(page, name, 8);

  for (let attempt = 0; attempt < 10; attempt += 1) {
    const next = await readBack(page, name, 4);
    const change = meanAbsoluteDifference(previous, next);
    previous = next;

    if (change < 0.2) {
      console.log(`  ${name} settled after ${attempt + 1} more read-backs`);
      break;
    }
  }

  return previous;
}

async function toggles() {
  for (const [width, height] of [[1440, 900], [390, 844]]) {
    const { context, page } = await open(width, height, width < 700);
    await page.screenshot({ path: `${out}/ui-${width}.png` });
    await noHorizontalScroll(page, `${width} px`);

    if (width < 700) {
      check("phone starts with the panel collapsed", await page.$eval("#controls", (panel) => panel.hidden));
      check("restore button shows on the phone", await page.isVisible("#showControls"));
      await page.click("#showControls");
      check("restore button shows the panel", !(await page.$eval("#controls", (panel) => panel.hidden)));
      await page.screenshot({ path: `${out}/ui-${width}-panel.png`, fullPage: true });
      await noHorizontalScroll(page, `${width} px with the panel`);
      await context.close();
      continue;
    }

    await page.focus("body");

    for (const [key, selector] of [["1", "#stats"], ["2", "#minimap"], ["3", "#hint"], ["4", "#status"], ["0", "#controls"]]) {
      await page.keyboard.press(key);
      check(`key ${key} hides ${selector}`, await page.$eval(selector, (element) => element.hidden));
      await page.screenshot({ path: `${out}/toggle-${key}.png` });
    }

    check("restore button shows with the panel hidden", await page.isVisible("#showControls"));
    await page.keyboard.press("0");
    check("key 0 shows the panel again", await page.isVisible("#controls"));
    await page.focus("#seed");
    await page.keyboard.press("1");
    check("keys are ignored while typing", await page.$eval("#stats", (element) => element.hidden));
    await page.reload();
    await ready(page);
    check("toggles are remembered", await page.$eval("#minimap", (element) => element.hidden));
    await page.evaluate(() => localStorage.clear());
    await context.close();
  }
}

async function paint() {
  const { context, page } = await open(1440, 900);
  await stillScene(page);
  await startBlank(page, 512);

  // A ridge across the north of the map.
  await brush(page, "raise", 60, 1);

  for (let pass = 0; pass < 4; pass += 1) {
    await drag(page, [[0.15, 0.35], [0.5, 0.3], [0.85, 0.36]]);
  }

  await brush(page, "biome", 110, 1);
  await page.click('#paintPalette button[data-biome="3"]');

  for (const y of [0.55, 0.65, 0.72]) {
    await drag(page, [[0.2, y], [0.5, y]]);
  }

  // A river from the ridge's southern foot to where the lake will be.
  await brush(page, "river", 8, 1);
  await drag(page, [[0.62, 0.4], [0.66, 0.55], [0.72, 0.68], [0.76, 0.76]], 20);
  await brush(page, "treesMore", 80, 1);
  await drag(page, [[0.2, 0.12], [0.35, 0.15]]);
  await drag(page, [[0.2, 0.18], [0.35, 0.2]]);
  // The lake is painted last, so undoing it leaves the rest.
  await brush(page, "lake", 70, 1);
  await drag(page, [[0.74, 0.78], [0.82, 0.8], [0.78, 0.86]]);
  await page.screenshot({ path: `${out}/paint-2d.png` });
  console.log(`  ${await page.textContent("#paintTimingReadout")}`);

  const painted = await renderAndReadBack(page, "render-painted");
  await page.screenshot({ path: `${out}/paint-rendered-ui.png` });

  await page.click("#tabPaint");
  await page.click("#paintUndo");
  const lakeGone = await page.evaluate(() => /Undid/.test(document.querySelector("#paintStatus").textContent));
  check("undo takes the lake off", lakeGone);
  await page.screenshot({ path: `${out}/paint-2d-undo.png` });
  const undone = await renderAndReadBack(page, "render-undo");
  // Where the lake lies in the framed view of the painting. Hazy with
  // distance, lake and grass differ by about 10; two renders of one
  // painting differ by under 0.01.
  const lakeDifference = meanAbsoluteDifference(painted, undone, [0.76, 0.78, 0.86, 0.9]);
  check("the lake's removal shows in 3D", lakeDifference > 5, `mean absolute difference ${lakeDifference.toFixed(2)} where the lake was`);

  await page.click("#tabPaint");
  const [download] = await Promise.all([page.waitForEvent("download"), page.click("#paintSave")]);
  await download.saveAs(`${out}/painting.zip`);
  check("saved the painting", fs.statSync(`${out}/painting.zip`).size > 1000, `${fs.statSync(`${out}/painting.zip`).size} bytes`);

  await page.reload();
  await ready(page);
  await stillScene(page);
  await page.click("#tabPaint");
  await page.setInputFiles("#paintOpen", `${out}/painting.zip`);
  await page.waitForFunction(() => /Loaded/.test(document.querySelector("#paintStatus").textContent), null, { timeout: 900000 });
  await page.screenshot({ path: `${out}/paint-2d-opened.png` });
  const reopened = await renderAndReadBack(page, "render-opened");
  const difference = meanAbsoluteDifference(undone, reopened);
  results.saveAndOpen = { meanAbsoluteDifference: difference, lakeDifference };
  check("the opened painting renders as saved", difference < 1, `mean absolute difference ${difference.toFixed(3)}`);
  await noHorizontalScroll(page, "paint at 1440 px");
  await context.close();
}

async function exportLayers(page, prefix) {
  const files = {};
  await page.$eval('details[data-section="paint-layers"]', (section) => {
    section.open = true;
  });

  for (const layer of ["height", "biome", "water", "trees", "grass"]) {
    await page.selectOption("#paintExportLayer", layer);
    const [download] = await Promise.all([page.waitForEvent("download"), page.click("#paintExportLayerButton")]);
    files[layer] = `${out}/${prefix}-${layer}.png`;
    await download.saveAs(files[layer]);
  }

  return files;
}

async function sources() {
  const { context, page } = await open(1440, 900);
  await page.click("#tabPaint");
  await page.selectOption("#paintSource", "current");
  await page.selectOption("#paintSize", "256");
  await page.check("#paintCopyBiomes");
  await page.click("#paintStart");
  await page.waitForFunction(() => /256 x 256/.test(document.querySelector("#paintDocReadout").textContent));
  check("starts from the current map", /Started from the current map/.test(await page.textContent("#paintStatus")));
  await page.screenshot({ path: `${out}/sources-current.png` });
  await brush(page, "lake", 30, 1);
  await drag(page, [[0.4, 0.4], [0.5, 0.45]]);
  await brush(page, "treesLess", 60, 0.7);
  await drag(page, [[0.2, 0.7], [0.6, 0.75]]);
  await brush(page, "grassMore", 60, 0.7);
  await drag(page, [[0.7, 0.2], [0.8, 0.5]]);
  const first = await exportLayers(page, "layer");

  await page.selectOption("#paintSource", "import");
  await page.uncheck("#paintCopyBiomes");
  await page.setInputFiles("#paintImportHeight", first.height);
  await page.waitForFunction(() => /from the PNG/.test(document.querySelector("#paintDocReadout").textContent));
  await page.setInputFiles("#paintImportBiome", first.biome);
  await page.setInputFiles("#paintImportWater", first.water);
  await page.setInputFiles("#paintImportTrees", first.trees);
  await page.setInputFiles("#paintImportGrass", first.grass);
  await page.click("#paintStart");
  await page.waitForFunction(() => /Loaded|failed|must|not/.test(document.querySelector("#paintStatus").textContent), null, { timeout: 900000 });
  check("starts from imported images", /Loaded the images/.test(await page.textContent("#paintStatus")), await page.textContent("#paintStatus"));
  await page.screenshot({ path: `${out}/sources-imported.png` });
  const second = await exportLayers(page, "reimported");

  for (const layer of ["biome", "water", "trees", "grass"]) {
    check(`the ${layer} layer survives export and import`, fs.readFileSync(first[layer]).equals(fs.readFileSync(second[layer])));
  }

  await page.setInputFiles("#paintImportHeight", []);
  await page.setInputFiles("#paintImportBiome", `${out}/painting.zip`).catch(() => undefined);
  const refused = await page.evaluate(() => {
    document.querySelector("#paintStart").click();
    return new Promise((resolve) => setTimeout(() => resolve(document.querySelector("#paintStatus").textContent), 500));
  });
  check("a missing heightmap is reported", /Choose a heightmap image/.test(refused), refused);

  if (fs.existsSync(`${out}/painting.zip`)) {
    await page.setInputFiles("#paintImportBiome", []);
    await page.setInputFiles("#paintImportBundle", `${out}/painting.zip`);
    await page.click("#paintStart");
    await page.waitForFunction(() => /Loaded|failed/.test(document.querySelector("#paintStatus").textContent), null, { timeout: 900000 });
    check("starts from a bundle", /Loaded the bundle/.test(await page.textContent("#paintStatus")), await page.textContent("#paintStatus"));
    await page.screenshot({ path: `${out}/sources-bundle.png` });
  }

  await context.close();
}

/** A touch stroke through `points` (fractions of the painting), `stepMs` apart. */
async function touchStroke(page, session, points, stepMs = 16) {
  const screen = await Promise.all(points.map(([fx, fy]) => toScreen(page, fx, fy)));
  const touch = ([x, y]) => [{ x, y, id: 1, radiusX: 4, radiusY: 4, force: 1 }];
  const before = await strokes(page);
  await session.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: touch(screen[0]) });

  for (let index = 1; index < screen.length; index += 1) {
    const [x0, y0] = screen[index - 1];
    const [x1, y1] = screen[index];

    for (let step = 1; step <= 10; step += 1) {
      await session.send("Input.dispatchTouchEvent", {
        type: "touchMove",
        touchPoints: touch([x0 + ((x1 - x0) * step) / 10, y0 + ((y1 - y0) * step) / 10])
      });
      await page.waitForTimeout(stepMs);
    }
  }

  await session.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
  await page.waitForFunction((count) => Number(document.querySelector("#paintHistoryReadout").textContent.match(/^(\d+)/)?.[1] ?? 0) > count, before, { timeout: 120000 });
}

async function phone() {
  const { context, page } = await open(390, 844, true);
  const session = await context.newCDPSession(page);
  await page.click("#showControls");
  await startBlank(page, 256);
  await page.$eval("#paintWorkspace", (workspace) => workspace.scrollIntoView());
  await brush(page, "raise", 40, 1);
  await touchStroke(page, session, [[0.2, 0.5], [0.5, 0.45], [0.8, 0.5]]);
  const timing = await page.textContent("#paintTimingReadout");
  check("a touch stroke paints", /Last stroke: [1-9]/.test(timing), timing);
  await page.screenshot({ path: `${out}/phone-stroke.png` });
  await page.screenshot({ path: `${out}/phone-stroke-full.png`, fullPage: true });
  await noHorizontalScroll(page, "paint at 390 px");
  await context.close();
}

async function timing() {
  const { context, page } = await open(390, 844, true);
  const session = await context.newCDPSession(page);
  await page.click("#showControls");
  await startBlank(page, 1024);
  await page.$eval("#paintWorkspace", (workspace) => workspace.scrollIntoView());
  await session.send("Emulation.setCPUThrottlingRate", { rate: 4 });
  results.timing = {};

  for (const tool of ["raise", "smooth", "noise", "erode", "biome", "treesMore"]) {
    await brush(page, tool, 128, 0.5);
    // Frame gaps while painting, from requestAnimationFrame.
    await page.evaluate(() => {
      window.frameGaps = [];
      let last = performance.now();
      const tick = (now) => {
        window.frameGaps.push(now - last);
        last = now;

        if (window.frameGaps.length < 100000 && !window.stopGaps) {
          requestAnimationFrame(tick);
        }
      };
      window.stopGaps = false;
      requestAnimationFrame(tick);
    });
    await touchStroke(page, session, [[0.15, 0.3], [0.5, 0.5], [0.85, 0.35], [0.6, 0.8]]);
    const gaps = await page.evaluate(() => {
      window.stopGaps = true;
      return window.frameGaps.slice(1).sort((a, b) => a - b);
    });
    const text = await page.textContent("#paintTimingReadout");
    const [, dabs, frames, perDab, brushP95, brushMax, totalP95, totalMax] = text
      .match(/(\d+) dabs over (\d+) frames, ([\d.]+) ms a dab\. Brush work a frame: ([\d.]+) ms at the 95th percentile, ([\d.]+) ms at most; with the redraw: ([\d.]+) ms and ([\d.]+) ms/)
      .slice(0, 8)
      .map(Number);
    const median = gaps[Math.floor(gaps.length / 2)];
    const p95 = gaps[Math.floor(gaps.length * 0.95)];
    results.timing[tool] = {
      dabs,
      frames,
      msPerDab: perDab,
      brushMsPerFrame: { p95: brushP95, max: brushMax },
      withRedrawMsPerFrame: { p95: totalP95, max: totalMax },
      frameGapMs: { median: Number(median.toFixed(1)), p95: Number(p95.toFixed(1)) }
    };
    // The longest gaps are idle time while the touch events are sent, so they are not reported.
    console.log(`  ${tool}: ${text} Frame gaps: median ${median.toFixed(1)} ms, 95th percentile ${p95.toFixed(1)} ms.`);
    check(`${tool}: brush work within 8 ms a frame`, brushP95 <= 8, `${brushP95} ms at the 95th percentile`);
  }

  await session.send("Emulation.setCPUThrottlingRate", { rate: 1 });
  await context.close();
}

const parts = { toggles, paint, sources, phone, timing };

for (const [name, run] of Object.entries(parts)) {
  if (!only || only === name) {
    console.log(`# ${name}`);
    await run();
  }
}

fs.writeFileSync(`${out}/results.json`, JSON.stringify(results, null, 2));
await browser.close();
process.exit(failed ? 1 : 0);
