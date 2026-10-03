import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { delimiter, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

// Builds the WASM package with wasm-pack.
//
// wasm-pack downloads a prebuilt wasm-bindgen for most platforms, but not
// for Apple Silicon Macs. There it compiles wasm-bindgen-cli from source:
// several minutes, and warnings from that tool's own dependencies that say
// nothing about this project. wasm-pack uses any wasm-bindgen on PATH whose
// version matches Cargo.lock, so this script fetches the official prebuilt
// binary for the locked version, from the same GitHub releases wasm-pack
// uses on other platforms, and puts it on PATH first. The archive must
// match the SHA-256 recorded for its version and platform in
// `wasm-bindgen-sha256.json`; any other is refused, and so is a version
// with no recorded checksums, until they are added as a reviewed change.
//
// Panic locations in dependencies embed their source paths, which start
// with the Cargo home (`/home/<name>/.cargo/registry/src/...`). Those are
// remapped to a fixed prefix, so the WASM is byte for byte the same
// wherever it is built (the size budget in CI depends on it) and holds no
// one's home directory.
//
// `--no-pack` skips the package.json, README and licence copy wasm-pack
// would write for a separate npm package. The published package is the
// repository root, which has its own.

const TARGETS = {
  "darwin-arm64": "aarch64-apple-darwin",
  "darwin-x64": "x86_64-apple-darwin",
  "linux-arm64": "aarch64-unknown-linux-gnu",
  "linux-x64": "x86_64-unknown-linux-musl",
  "win32-x64": "x86_64-pc-windows-msvc"
};

const executable = process.platform === "win32" ? "wasm-bindgen.exe" : "wasm-bindgen";

function lockedVersion() {
  const lock = readFileSync("Cargo.lock", "utf8");
  const version = lock.match(/\[\[package\]\]\r?\nname = "wasm-bindgen"\r?\nversion = "([^"]+)"/)?.[1];

  if (!version) {
    throw new Error("Cargo.lock does not lock a wasm-bindgen version. Run `cargo generate-lockfile`, then build again.");
  }

  return version;
}

function reportedVersion(command) {
  const result = spawnSync(command, ["--version"], { encoding: "utf8" });
  return result.status === 0 ? result.stdout.trim().split(/\s+/)[1] : null;
}

/** The SHA-256 recorded for wasm-bindgen `version` on `target`. */
function expectedChecksum(version, target) {
  const checksums = JSON.parse(readFileSync(new URL("./wasm-bindgen-sha256.json", import.meta.url), "utf8"));

  if (checksums.version !== version) {
    throw new Error(
      `scripts/wasm-bindgen-sha256.json holds checksums for wasm-bindgen ${checksums.version}, not ${version}; ` +
        "add the new release's checksums to use its prebuilt binary"
    );
  }

  const checksum = checksums.sha256?.[target];

  if (!checksum) {
    throw new Error(`scripts/wasm-bindgen-sha256.json has no checksum for ${target}`);
  }

  return checksum;
}

async function download(url, path) {
  const response = await fetch(url);

  if (!response.ok) {
    throw new Error(`${url} answered ${response.status} ${response.statusText}`);
  }

  writeFileSync(path, Buffer.from(await response.arrayBuffer()));
}

/** The directory holding a prebuilt wasm-bindgen of `version`, fetching it when needed. */
async function prebuiltWasmBindgen(version) {
  const target = TARGETS[`${process.platform}-${process.arch}`];

  if (!target) {
    throw new Error(`there is no prebuilt wasm-bindgen for ${process.platform} on ${process.arch}`);
  }

  const checksum = expectedChecksum(version, target);
  const cache = resolve("node_modules", ".cache", "vistawasm");
  const name = `wasm-bindgen-${version}-${target}`;
  const directory = join(cache, name);

  if (reportedVersion(join(directory, executable)) === version) {
    return directory;
  }

  console.log(`Fetching the prebuilt wasm-bindgen ${version} for ${target}...`);
  const unpacked = join(cache, `${name}.partial`);
  const archive = join(cache, `${name}.tar.gz`);
  rmSync(unpacked, { force: true, recursive: true });
  rmSync(directory, { force: true, recursive: true });
  mkdirSync(unpacked, { recursive: true });

  try {
    await download(`https://github.com/wasm-bindgen/wasm-bindgen/releases/download/${version}/${name}.tar.gz`, archive);
    const actual = createHash("sha256").update(readFileSync(archive)).digest("hex");

    if (actual !== checksum) {
      throw new Error(`the downloaded ${name}.tar.gz has SHA-256 ${actual}, not the recorded ${checksum}, so it was refused`);
    }

    const tar = spawnSync("tar", ["-xzf", archive, "-C", unpacked], { encoding: "utf8" });

    if (tar.status !== 0) {
      throw new Error(`tar could not unpack ${archive}: ${tar.stderr || tar.error?.message}`);
    }

    // The archive holds one folder with the same name as the archive.
    renameSync(join(unpacked, name), directory);
  } finally {
    rmSync(unpacked, { force: true, recursive: true });
    rmSync(archive, { force: true });
  }

  const fetched = reportedVersion(join(directory, executable));

  if (fetched !== version) {
    throw new Error(`the fetched wasm-bindgen reports version ${fetched ?? "nothing"}, not ${version}`);
  }

  return directory;
}

const version = lockedVersion();
const path = [process.env.PATH ?? ""];

if (reportedVersion("wasm-bindgen") !== version) {
  try {
    path.unshift(await prebuiltWasmBindgen(version));
  } catch (error) {
    console.warn(
      `Could not use a prebuilt wasm-bindgen ${version}: ${error.message}. ` +
        "wasm-pack will build it from source instead, which takes a few minutes and prints warnings from its dependencies."
    );
  }
}

const cargoHome = resolve(process.env.CARGO_HOME ?? join(homedir(), ".cargo"));
const rustflags = [process.env.RUSTFLAGS, `--remap-path-prefix=${cargoHome}=/cargo`].filter(Boolean).join(" ");
const build = spawnSync(
  "wasm-pack",
  ["build", "crates/vista_wasm", "--target", "web", "--out-dir", "../../dist/pkg", "--no-pack", ...process.argv.slice(2)],
  { env: { ...process.env, PATH: path.join(delimiter), RUSTFLAGS: rustflags }, stdio: "inherit" }
);

if (build.error?.code === "ENOENT") {
  console.error("wasm-pack is not installed. Install it with `cargo install wasm-pack`, or see https://rustwasm.github.io/wasm-pack/installer/.");
  process.exit(1);
}

if (build.error) {
  throw build.error;
}

if (build.status !== 0) {
  process.exit(build.status ?? 1);
}

// wasm-bindgen's import and closure names carry hashes and type paths that compress poorly.
const shorten = spawnSync(process.execPath, [fileURLToPath(new URL("./shorten-names.mjs", import.meta.url)), "dist/pkg"], {
  stdio: "inherit"
});
process.exit(shorten.status ?? 1);
