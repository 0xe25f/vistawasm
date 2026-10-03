import { spawnSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { brotliCompressSync, constants } from "node:zlib";

// Compares the gzipped size of every release artefact with its budget in
// `scripts/size-budget.json`, prints each delta, and fails when any file
// is over budget or missing. Run it after a clean `npm run build`.
//
//   node scripts/check-size.mjs
//
// Sizes are `gzip -9 -c <file> | wc -c`, the figure the changelog and
// `docs/performance.md` quote, rather than Node's zlib, which compresses differently.
// Each line also gives the Brotli size (quality 11), for information only:
// hosts that serve Brotli send that many bytes (see `docs/getting-started.md`).
// Budgets stay on gzip, which every host supports.
// Raising a budget is a reviewed change to `size-budget.json`; lower it
// whenever a change makes a file smaller, so the saving is kept.

const budgetFile = new URL("./size-budget.json", import.meta.url);
const { files } = JSON.parse(readFileSync(budgetFile, "utf8"));

function gzippedSize(path) {
  const result = spawnSync("gzip", ["-9", "-c", path], { maxBuffer: 64 * 1024 * 1024 });

  if (result.error || result.status !== 0) {
    throw new Error(`gzip could not compress ${path}: ${result.error?.message ?? result.stderr.toString()}`);
  }

  return result.stdout.length;
}

function brotliSize(path) {
  const bytes = readFileSync(path);

  return brotliCompressSync(bytes, {
    params: {
      [constants.BROTLI_PARAM_QUALITY]: constants.BROTLI_MAX_QUALITY,
      [constants.BROTLI_PARAM_SIZE_HINT]: bytes.length
    }
  }).length;
}

let failed = false;
let totalGzip = 0;
let totalBrotli = 0;

for (const [path, budget] of Object.entries(files)) {
  if (!existsSync(path)) {
    console.log(`MISSING  ${path}: not built. Run \`npm run build\` first.`);
    failed = true;
    continue;
  }

  const size = gzippedSize(path);
  const brotli = brotliSize(path);
  const delta = size - budget;
  const sign = delta > 0 ? "+" : "";
  const over = delta > 0;
  failed ||= over;
  totalGzip += size;
  totalBrotli += brotli;
  console.log(`${over ? "OVER    " : "ok      "} ${path}: ${size} bytes gzipped, budget ${budget} (${sign}${delta}); ${brotli} with Brotli`);
}

console.log(`\nIn all: ${totalGzip} bytes gzipped, ${totalBrotli} with Brotli (${(100 * (1 - totalBrotli / totalGzip)).toFixed(1)}% smaller).`);

// A new module must be given a budget, or it would grow unchecked.
for (const name of existsSync("dist") ? readdirSync("dist") : []) {
  if (name.endsWith(".js") && !(`dist/${name}` in files)) {
    console.log(`UNBUDGETED dist/${name}: ${gzippedSize(`dist/${name}`)} bytes gzipped. Add it to scripts/size-budget.json.`);
    failed = true;
  }
}

if (failed) {
  console.log("\nA file is over its budget. Make it smaller, or raise its budget in scripts/size-budget.json as a reviewed change.");
  process.exit(1);
}
