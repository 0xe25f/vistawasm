// Runs before every `npm run dev*` script. The demo and examples load the
// built package from `dist/` (see vite.config.ts), so a fresh clone has
// nothing to load until `npm run build` has run. Build it automatically
// rather than letting Vite fail with an unresolved import.
import { spawnSync } from "node:child_process";
import { existsSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

const builtFiles = ["dist/index.js", "dist/pkg/vista_wasm.js", "dist/pkg/vista_wasm_bg.wasm"];
const sourceDirectories = ["crates", "js/src"];

if (builtFiles.every((path) => existsSync(path))) {
  warnIfStale();
  process.exit(0);
}

console.log("VistaWASM has not been built yet, so running `npm run build` first.");
console.log("This takes a few minutes the first time.\n");

const npm = process.platform === "win32" ? "npm.cmd" : "npm";
const result = spawnSync(npm, ["run", "build"], { stdio: "inherit", shell: process.platform === "win32" });

if (result.status !== 0) {
  console.error(`
The build failed, so the dev server cannot start. The build needs:

  - Rust 1.87 or newer, via rustup (https://rustup.rs)
  - The WebAssembly target: rustup target add wasm32-unknown-unknown
  - wasm-pack 0.13 or newer: cargo install wasm-pack

See CONTRIBUTING.md for the full setup, then run \`npm run build\` again.
`);
  process.exit(result.status ?? 1);
}

function warnIfStale() {
  const builtAt = statSync("dist/index.js").mtimeMs;
  const changed = sourceDirectories.find((directory) => newestModification(directory) > builtAt);

  if (changed) {
    console.warn(
      `Warning: files in ${changed}/ changed after the last build. ` +
        "Run `npm run build` to see those changes in the demo and examples.\n"
    );
  }
}

function newestModification(directory) {
  let newest = 0;

  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    if (entry.name === "target" || entry.name === "node_modules") {
      continue;
    }

    const path = join(directory, entry.name);
    const modified = entry.isDirectory() ? newestModification(path) : statSync(path).mtimeMs;
    newest = Math.max(newest, modified);
  }

  return newest;
}
