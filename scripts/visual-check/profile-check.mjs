// Drive the GPU profile page (`bench/gpu/`) in headless Chromium with
// software WebGPU, then save its JSON and a screenshot of the finished page.
//
//   cd bench && npx vite --host 127.0.0.1 --port 5174      (after a root build)
//   node scripts/visual-check/profile-check.mjs <out-dir> [query]
//
// `query` holds the page's URL parameters; software rendering takes
// seconds per frame, so keep it small, for example
// `size=480x270&warm=10&frames=20`. `PORT` names the bench server's port
// (5174 by default). The script exits non-zero on any console error, page
// error or scene that could not be measured.
//
// `--no-timestamps` hides WebGPU timestamp queries from the page, as a
// browser without them would, to check its whole-frame fallback.
//
// Software WebGPU loses its device when it presents to a real canvas, so
// the page draws into a texture instead (`offscreen-canvas.js`), as the
// other visual checks do. Its timings say nothing about real GPUs.
import fs from "node:fs";
import path from "node:path";

const playwright = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const noTimestamps = process.argv.includes("--no-timestamps");
const [outDir, query = "size=480x270&warm=10&frames=20"] = process.argv
  .slice(2)
  .filter((argument) => !argument.startsWith("--"));

if (!outDir) {
  console.error("Usage: node scripts/visual-check/profile-check.mjs <out-dir> [query]");
  process.exit(2);
}

const port = process.env.PORT ?? "5174";
const here = new URL(".", import.meta.url);
fs.mkdirSync(outDir, { recursive: true });

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
const page = await browser.newPage({ viewport: { width: 1200, height: 900 } });
const errors = [];

await page.addInitScript({ path: new URL("offscreen-canvas.js", here).pathname });

if (noTimestamps) {
  await page.addInitScript(() => {
    const features = Object.getOwnPropertyDescriptor(GPUAdapter.prototype, "features").get;
    Object.defineProperty(GPUAdapter.prototype, "features", {
      get() {
        return new Set([...features.call(this)].filter((name) => name !== "timestamp-query"));
      }
    });
    const requestDevice = GPUAdapter.prototype.requestDevice;
    GPUAdapter.prototype.requestDevice = function (descriptor = {}) {
      const requiredFeatures = [...(descriptor.requiredFeatures ?? [])].filter((name) => name !== "timestamp-query");
      return requestDevice.call(this, { ...descriptor, requiredFeatures });
    };
  });
}
page.on("console", (message) => {
  const webgpu = message.type() === "warning" && /Invalid|Error while parsing/.test(message.text());

  if (message.type() === "error" || webgpu) {
    console.log(`console ${message.type()}: ${message.text().slice(0, 1500)}`);
    errors.push(message.text());
  }
});
page.on("pageerror", (error) => {
  console.log(`page error: ${error.message}`);
  errors.push(error.message);
});

const url = `http://127.0.0.1:${port}/gpu/?${query}`;
console.log(`opening ${url}`);
await page.goto(url);
await page.getByRole("button", { name: "Run the profile" }).click();
await page.waitForFunction(() => window.profile?.done === true, null, { timeout: 4 * 60 * 60 * 1000 });

const { report, error } = await page.evaluate(() => window.profile);
const rows = await page.locator("#results tbody tr").count();
await page.screenshot({ path: path.join(outDir, "profile-page.png"), fullPage: true });

if (report) {
  fs.writeFileSync(path.join(outDir, "profile.json"), `${JSON.stringify(report, null, 2)}\n`);
}

await browser.close();

const failedScenes = report ? report.scenes.filter((scene) => scene.error) : [];

for (const scene of report?.scenes ?? []) {
  const passes = Object.entries(scene.passes)
    .filter(([, summary]) => summary.p90 > 0)
    .map(([pass, summary]) => `${pass} ${summary.median}/${summary.p90}`)
    .join(", ");
  const frame = scene.frame ? `${scene.frame.median}/${scene.frame.p90} (${scene.frameSource})` : scene.error;
  console.log(`${scene.id}: frame ${frame}${passes ? `; ${passes}` : ""}`);
}

console.log(`table rows ${rows}, scenes ${report?.scenes.length ?? 0}, saved to ${outDir}`);

if (error || !report || rows === 0 || failedScenes.length > 0 || errors.length > 0) {
  console.log(`failed: ${error ?? ""} ${failedScenes.map((scene) => `${scene.id}: ${scene.error}`).join("; ")}`);
  process.exit(1);
}
