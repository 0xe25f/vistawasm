// Render VistaWASM in headless Chromium with software WebGPU and save PNGs.
//
//   python3 -m http.server 8124        (from the repository root)
//   node scripts/visual-check/capture.mjs out.png '<config JSON>' [frames]
//
// Config fields: size [w, h]; engine (createVistaEngine options); terrain
// (generateFractal options); set ({ method: argument } called once);
// camera; shots (a list of { set, eval, position, target, biome, back,
// height, targetHeight, ground }, one PNG each, named out-0.png, out-1.png).
// Playwright is not a dependency: set PLAYWRIGHT_MODULE to its path if it
// is not resolvable, and CHROMIUM_PATH to a Chromium build if needed.
import fs from "node:fs";

const playwright = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const [out = "capture.png", configText = "{}", framesText = "3"] = process.argv.slice(2);
const config = JSON.parse(configText);
const frames = Number(framesText);
const port = process.env.PORT ?? "8124";
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
const [width, height] = config.size || [960, 600];
const page = await browser.newPage({ viewport: { width, height } });
let failed = false;

page.on("console", (message) => {
  if (message.type() === "error" || message.type() === "warning") {
    console.log(`console ${message.type()}: ${message.text().slice(0, 1500)}`);
    failed ||= message.type() === "error" && !/Failed to load resource/.test(message.text());
  }
});
page.on("pageerror", (error) => {
  console.log(`page error: ${error.message}`);
  failed = true;
});

await page.goto(
  `http://127.0.0.1:${port}/scripts/visual-check/index.html?config=${encodeURIComponent(configText)}`
);
await page.waitForFunction(() => window.ready === true, null, { timeout: 600000 });
console.log("log:", JSON.stringify(await page.evaluate(() => window.log.splice(0))));

const shots = config.shots || [null];

for (const [index, shot] of shots.entries()) {
  if (shot) {
    await page.evaluate((shot) => {
      for (const [method, value] of Object.entries(shot.set || {})) {
        window.engine[method](value);
      }

      if (shot.eval) {
        (0, eval)(shot.eval);
      }

      let { position, target } = shot;

      if (shot.biome) {
        const at = window.findBiome(shot.biome);
        window.log.push(`biome ${shot.biome} at ${JSON.stringify(at)}`);

        if (at) {
          const back = shot.back || 60;
          position = [at[0] + back, 0, at[1] + back];
          target = [at[0], 0, at[1]];
        }
      }

      if (position && shot.ground !== false) {
        position = [position[0], window.heightAt(position[0], position[2]) + (shot.height || 12), position[2]];
        target = [target[0], window.heightAt(target[0], target[2]) + (shot.targetHeight || 4), target[2]];
      }

      if (position) {
        window.engine.setCamera({ fieldOfViewDegrees: 55, nearMetres: 0.5, farMetres: 120000, position, target });
      }
    }, shot);
  }

  const result = await page.evaluate((frames) => window.capture(frames), frames);
  console.log("log:", JSON.stringify(await page.evaluate(() => window.log.splice(0))));
  console.log("stats", JSON.stringify(result.stats), "gpuMs", Math.round(result.gpuMs));
  const name = shots.length > 1 ? out.replace(/\.png$/, `-${index}.png`) : out;
  fs.writeFileSync(name, Buffer.from(result.url.split(",")[1], "base64"));
}

await browser.close();
process.exit(failed ? 1 : 0);
