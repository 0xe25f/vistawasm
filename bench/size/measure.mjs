// Builds each minimal app with Vite (production mode: minified and
// tree-shaken, exactly what a real site ships) and reports the gzipped
// bytes a browser downloads. VistaWASM loads its WASM glue and binary at
// run time rather than through the bundle, so those two files are added
// from the built package.
//
// CesiumJS fetches its web workers at run time from files copied beside
// the bundle, and only those the scene needs. Its app is therefore opened
// in Chromium, and the JavaScript and WebAssembly it fetches from that
// copy are counted. Playwright is not a dependency: set PLAYWRIGHT_MODULE
// to its path if it is not resolvable, and CHROMIUM_PATH if needed.
import { createServer } from "node:http";
import { cp, readdir, readFile, stat } from "node:fs/promises";
import { extname, join, normalize } from "node:path";
import { fileURLToPath } from "node:url";
import { gzipSync } from "node:zlib";
import { build } from "vite";

const here = fileURLToPath(new URL(".", import.meta.url));
const apps = [
  { name: "VistaWASM", entry: "vistawasm.html", extra: ["../../dist/pkg/vista_wasm.js", "../../dist/pkg/vista_wasm_bg.wasm"] },
  { name: "three.js + THREE.Terrain", entry: "three-terrain-js.html", extra: [] },
  { name: "three.js + three-terrain", entry: "three-terrain.html", extra: [] },
  { name: "CesiumJS", entry: "cesium.html", extra: [], runtime: "cesium" }
];
const cesiumBuild = join(here, "..", "node_modules", "cesium", "Build", "Cesium");

async function filesIn(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  const files = await Promise.all(
    entries.map((entry) => {
      const path = join(directory, entry.name);
      return entry.isDirectory() ? filesIn(path) : [path];
    })
  );
  return files.flat();
}

async function gzippedSize(path) {
  return gzipSync(await readFile(path), { level: 9 }).length;
}

// Serves `directory` and returns the paths of the files Chromium fetched
// from it below `prefix` while the page settled.
async function fetchedFiles(directory, entry, prefix) {
  const types = { ".html": "text/html", ".js": "text/javascript", ".wasm": "application/wasm", ".json": "application/json" };
  const server = createServer(async (request, response) => {
    const path = join(directory, normalize(decodeURIComponent(new URL(request.url, "http://x").pathname)));

    try {
      const body = await readFile(path);
      response.writeHead(200, { "content-type": types[extname(path)] ?? "application/octet-stream" });
      response.end(body);
    } catch {
      response.writeHead(404);
      response.end();
    }
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const { port } = server.address();
  const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
  const browser = await chromium.launch({
    executablePath: process.env.CHROMIUM_PATH,
    args: ["--enable-unsafe-swiftshader", "--use-angle=swiftshader"]
  });
  const page = await browser.newPage();
  const fetched = new Set();
  page.on("requestfinished", (request) => {
    const path = new URL(request.url()).pathname;

    if (path.startsWith(prefix) && /\.(js|wasm)$/.test(path)) {
      fetched.add(join(directory, path));
    }
  });
  await page.goto(`http://127.0.0.1:${port}/${entry}`);
  // Long enough for every tile in view to be meshed by the workers.
  await page.waitForTimeout(15000);
  await browser.close();
  server.close();
  return [...fetched];
}

const rows = [];

for (const app of apps) {
  const outDir = join(here, "..", "dist-size", app.entry.replace(".html", ""));
  await build({
    root: here,
    logLevel: "error",
    build: { outDir, emptyOutDir: true, rollupOptions: { input: join(here, app.entry) } }
  });
  const files = (await filesIn(outDir)).filter((path) => /\.(js|wasm)$/.test(path));
  // Vite bundles the WASM glue when it can see it, so an extra file is
  // added only when no hashed copy of it was emitted.
  const emitted = files.map((path) => path.split("/").pop());
  const extra = app.extra.filter((path) => {
    const stem = path.split("/").pop().replace(/\.[^.]+$/, "");
    return !emitted.some((name) => name.startsWith(`${stem}-`));
  });
  files.push(...extra.map((path) => join(here, path)));

  if (app.runtime === "cesium") {
    for (const folder of ["Workers", "ThirdParty", "Assets", "Widgets"]) {
      await cp(join(cesiumBuild, folder), join(outDir, "cesium", folder), { recursive: true });
    }

    files.push(...(await fetchedFiles(outDir, app.entry, "/cesium/")));
  }
  let raw = 0;
  let gzip = 0;

  for (const path of files) {
    raw += (await stat(path)).size;
    gzip += await gzippedSize(path);
  }

  rows.push({ app: app.name, "raw KB": (raw / 1000).toFixed(0), "gzip KB": (gzip / 1000).toFixed(0) });
}

console.table(rows);
