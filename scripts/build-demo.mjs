import { createHash } from "node:crypto";
import { cpSync, existsSync, readFileSync, rmSync } from "node:fs";

const sourceDist = "dist";
const targetDist = "demo/dist";

if (!existsSync(sourceDist)) {
  console.error("dist/ does not exist yet. Run `npm run build` first.");
  process.exit(1);
}

// The demo's Content Security Policy admits its inline import map by hash,
// so a changed import map would be blocked on the published page. Catch
// that here, before it is deployed, and name the hash to use instead.
const page = readFileSync("demo/index.html", "utf8");
const importMap = page.match(/<script type="importmap">([\s\S]*?)<\/script>/)?.[1];
const policy = page.match(/<meta http-equiv="Content-Security-Policy" content="([^"]*)">/)?.[1];

if (importMap === undefined || policy === undefined) {
  console.error("demo/index.html needs its import map and its Content-Security-Policy meta tag.");
  process.exit(1);
}

const hash = `'sha256-${createHash("sha256").update(importMap).digest("base64")}'`;

if (!policy.split(/[\s;]+/).includes(hash)) {
  console.error(`demo/index.html's Content-Security-Policy does not admit its import map. Put ${hash} in its script-src.`);
  process.exit(1);
}

rmSync(targetDist, {
  force: true,
  recursive: true
});

cpSync(sourceDist, targetDist, {
  recursive: true
});

console.log(`Copied ${sourceDist}/ -> ${targetDist}/`);
