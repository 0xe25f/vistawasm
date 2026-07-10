import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

const extensions = new Set([
  ".css",
  ".html",
  ".js",
  ".json",
  ".md",
  ".rs",
  ".svelte",
  ".toml",
  ".ts",
  ".tsx",
  ".vue",
  ".wgsl"
]);

const ignoredParts = new Set([
  ".git",
  "dist",
  "js/pkg",
  "node_modules",
  "target"
]);

function shouldIgnore(path) {
  return path.split("/").some((part) => ignoredParts.has(part));
}

function collectFiles(dir, result = []) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);

    if (shouldIgnore(path)) {
      continue;
    }

    if (entry.isDirectory()) {
      collectFiles(path, result);
      continue;
    }

    const dot = entry.name.lastIndexOf(".");
    const extension = dot === -1 ? "" : entry.name.slice(dot);

    if (extensions.has(extension)) {
      result.push(path);
    }
  }

  return result;
}

let failed = false;

for (const file of collectFiles(".")) {
  const text = readFileSync(file, "utf8");
  const lines = text.split("\n");

  lines.forEach((line, index) => {
    if (line.includes("\t")) {
      console.error(`${file}:${index + 1}: contains a tab.`);
      failed = true;
    }

    const match = line.match(/^( +)\S/);

    if (/^ +\*/.test(line)) {
      return;
    }

    if (match && match[1].length % 2 !== 0) {
      console.error(`${file}:${index + 1}: indentation must use 2-space steps.`);
      failed = true;
    }
  });
}

if (failed) {
  process.exit(1);
}
