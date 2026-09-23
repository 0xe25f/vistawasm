// Builds each minimal app with Vite (production mode: minified and
// tree-shaken, exactly what a real site ships) and reports the gzipped
// bytes a browser downloads. VistaWASM loads its WASM glue and binary at
// run time rather than through the bundle, so those two files are added
// from the built package.
import { readdir, readFile, stat } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { gzipSync } from "node:zlib";
import { build } from "vite";

const here = fileURLToPath(new URL(".", import.meta.url));
const apps = [
  { name: "VistaWASM", entry: "vistawasm.html", extra: ["../../dist/pkg/vista_wasm.js", "../../dist/pkg/vista_wasm_bg.wasm"] },
  { name: "three.js + THREE.Terrain", entry: "three-terrain-js.html", extra: [] },
  { name: "three.js + three-terrain", entry: "three-terrain.html", extra: [] }
];

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

const rows = [];

for (const app of apps) {
  const outDir = join(here, "..", "dist-size", app.entry.replace(".html", ""));
  await build({
    root: here,
    logLevel: "error",
    build: { outDir, emptyOutDir: true, rollupOptions: { input: join(here, app.entry) } }
  });
  const files = (await filesIn(outDir)).filter((path) => /\.(js|wasm)$/.test(path));
  files.push(...app.extra.map((path) => join(here, path)));
  let raw = 0;
  let gzip = 0;

  for (const path of files) {
    raw += (await stat(path)).size;
    gzip += await gzippedSize(path);
  }

  rows.push({ app: app.name, "raw KB": (raw / 1000).toFixed(0), "gzip KB": (gzip / 1000).toFixed(0) });
}

console.table(rows);
