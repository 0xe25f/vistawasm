// Print a profile page's JSON as Markdown rows for docs/performance.md:
//
//   node bench/gpu/markdown.mjs vistawasm-gpu-profile-2026-09-30.json
import fs from "node:fs";
import { reportFormat, reportToMarkdown } from "./stats.mjs";

const file = process.argv[2];

if (!file) {
  console.error("Usage: node bench/gpu/markdown.mjs <profile JSON from the GPU profile page>");
  process.exit(2);
}

const report = JSON.parse(fs.readFileSync(file, "utf8"));

if (report.format !== reportFormat) {
  console.error(`${file} is not a GPU profile report (its "format" is not "${reportFormat}").`);
  process.exit(1);
}

process.stdout.write(reportToMarkdown(report));
