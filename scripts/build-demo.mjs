import { cpSync, existsSync, rmSync } from "node:fs";

const sourceDist = "dist";
const targetDist = "demo/dist";

if (!existsSync(sourceDist)) {
  console.error("dist/ does not exist yet. Run `npm run build` first.");
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
