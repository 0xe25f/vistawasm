import zlib from "node:zlib";
import { describe, expect, it } from "vitest";
import { VistaWasmError } from "../src/errors";
import { zip } from "../src/map-export";
import { parseManifest, readBundle } from "../src/map-import";
import { decodePng } from "../src/png-decode";
import { chunk, makePng } from "./make-png";

// Deterministic fuzzing: a small seeded generator (mulberry32) mutates
// valid files, so every run feeds the parsers the same malformed inputs.
// Each input must give a valid result or a VistaWasmError saying what is
// wrong: never another error, a hang, or a result past the decoders' caps.

const CASES = 10_000;
const MAX_SIDE = 8192;

type Random = () => number;

function generator(seed: number): Random {
  return () => {
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const int = (random: Random, below: number) => Math.floor(random() * below);
const pick = <Item>(random: Random, items: readonly Item[]): Item => items[int(random, items.length)];

/** Numbers a hostile file would try: edges, powers of two and random values. */
function nasty(random: Random): number {
  return pick(random, [0, 1, 2, 255, 256, 8191, 8192, 8193, 65535, 2 ** 31 - 1, 2 ** 31, 2 ** 32 - 1, int(random, 2 ** 32), int(random, 64)]);
}

function flip(random: Random, bytes: Uint8Array, count = 1 + int(random, 4)): Uint8Array {
  const out = bytes.slice();

  for (let index = 0; index < count && out.length; index += 1) {
    out[int(random, out.length)] = int(random, 256);
  }

  return out;
}

/** Record how each input fared, and fail on anything but a result or a VistaWasmError. */
function tally() {
  const counts = { accepted: 0, rejected: 0 };
  return {
    counts,
    async run(input: () => Promise<unknown>, check: (result: never) => void): Promise<void> {
      let result: unknown;

      try {
        result = await input();
      } catch (error) {
        if (!(error instanceof VistaWasmError) || error.code !== "INVALID_DEM" || !error.message) {
          throw error;
        }

        counts.rejected += 1;
        return;
      }

      check(result as never);
      counts.accepted += 1;
    }
  };
}

// PNG: split into chunks, change one, and write the chunks back with good
// CRCs, so mutations reach past the CRC check into the decoder proper.

type Chunk = [string, Uint8Array];

function chunks(png: Uint8Array): Chunk[] {
  const view = new DataView(png.buffer, png.byteOffset, png.byteLength);
  const out: Chunk[] = [];

  for (let at = 8; at + 12 <= png.length; at += view.getUint32(at) + 12) {
    out.push([String.fromCharCode(...png.subarray(at + 4, at + 8)), png.slice(at + 8, at + 8 + view.getUint32(at))]);
  }

  return out;
}

function build(parts: Chunk[]): Uint8Array {
  return new Uint8Array(Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), ...parts.map(([type, data]) => chunk(type, data))]));
}

const PNGS = [
  makePng({ width: 7, height: 5, colourType: 0, bitDepth: 16, samples: Array.from({ length: 35 }, (_, i) => i * 1800) }),
  makePng({ width: 9, height: 4, colourType: 3, bitDepth: 4, samples: Array.from({ length: 36 }, (_, i) => i % 5), palette: [0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255, 9, 9, 9], trns: [0, 128] }),
  makePng({ width: 6, height: 6, colourType: 6, bitDepth: 8, samples: Array.from({ length: 144 }, (_, i) => (i * 37) % 256), interlace: true }),
  makePng({ width: 13, height: 3, colourType: 0, bitDepth: 1, samples: Array.from({ length: 39 }, (_, i) => i % 2), text: { "vistawasm:range": "[0,1]" } }),
  makePng({ width: 3, height: 7, colourType: 2, bitDepth: 16, samples: Array.from({ length: 63 }, (_, i) => i * 1000), trns: [0, 1, 0, 2, 0, 3] })
].map(chunks);

function mutatePng(random: Random): Uint8Array {
  const parts = pick(random, PNGS).map(([type, data]): Chunk => [type, data.slice()]);
  const at = int(random, parts.length);
  const header = parts[0][1];
  const view = new DataView(header.buffer);

  switch (int(random, 10)) {
    case 0:
      return flip(random, build(parts));
    case 1:
      return build(parts).subarray(0, int(random, build(parts).length));
    case 2:
      // Any size, up to far past the cap, with the pixels left as they were.
      view.setUint32(int(random, 2) * 4, nasty(random));
      break;
    case 3:
      header[8 + int(random, 5)] = pick(random, [0, 1, 2, 3, 4, 5, 6, 7, 8, 16, 255]);
      break;
    case 4:
      parts[at][1] = flip(random, parts[at][1]);
      break;
    case 5: {
      // Pixels that inflate to more, or less, than the header needs: a
      // bomb of zeros, or a short row.
      const size = pick(random, [0, 1, int(random, 400), 2 ** 16 + int(random, 2 ** 20)]);
      const idat = parts.findIndex(([type]) => type === "IDAT");
      parts.splice(idat, parts.filter(([type]) => type === "IDAT").length, ["IDAT", zlib.deflateSync(new Uint8Array(size))]);
      break;
    }
    case 6:
      parts.splice(at, 0, [pick(random, ["IHDR", "PLTE", "tRNS", "IDAT", "IEND", "tEXt", "ABCD", "abcd", "\0\0\0\0"]), flip(random, new Uint8Array(int(random, 40)))]);
      break;
    case 7:
      parts.splice(at, 1);
      break;
    case 8:
      parts.splice(at, 0, parts[int(random, parts.length)]);
      break;
    default:
      // Random bytes after a valid header.
      return build([parts[0], ["IDAT", Uint8Array.from({ length: int(random, 300) }, () => int(random, 256))], ["IEND", new Uint8Array(0)]]);
  }

  return build(parts);
}

// Bundles: a small valid one, changed at the zip, directory, manifest or
// file level.

const heights = new Uint8Array(new Float32Array([1, 2, 3, 4]).buffer);
const mask = makePng({ width: 2, height: 2, colourType: 0, bitDepth: 8, samples: [0, 64, 128, 255] });
const biome = makePng({ width: 2, height: 2, colourType: 3, bitDepth: 8, samples: [0, 1, 2, 255], palette: Array.from({ length: 768 }, (_, i) => i % 256) });
const MANIFEST = {
  format: "vistawasm-bundle",
  version: 2,
  terrain: { width: 2, height: 2, metresPerSample: 10, seaLevelMetres: 0, landform: "fjords" },
  options: { flora: { density: 1 }, terrain: { landform: "fjords" } },
  files: [
    { path: "source-height.f32", kind: "sourceHeight" },
    { path: "water-mask.png", kind: "waterMask" },
    { path: "tree-mask.png", kind: "treeMask" },
    { path: "painted-biome.png", kind: "paintedBiome", borderSamples: 3 }
  ]
};

type Entry = { name: string; data: Uint8Array<ArrayBuffer>; store?: boolean };

function entries(manifest: unknown = MANIFEST): Entry[] {
  return [
    { name: "manifest.json", data: new TextEncoder().encode(JSON.stringify(manifest)), store: true },
    { name: "source-height.f32", data: heights.slice(), store: true },
    { name: "water-mask.png", data: mask.slice() as Uint8Array<ArrayBuffer>, store: true },
    { name: "tree-mask.png", data: mask.slice() as Uint8Array<ArrayBuffer>, store: true },
    { name: "painted-biome.png", data: biome.slice() as Uint8Array<ArrayBuffer>, store: true }
  ];
}

async function bytes(blob: Blob): Promise<Uint8Array> {
  return new Uint8Array(await blob.arrayBuffer());
}

/** A JSON value a hostile manifest might hold. */
function value(random: Random, depth = 0): unknown {
  switch (int(random, depth > 2 ? 5 : 8)) {
    case 0: return nasty(random) * pick(random, [1, -1, 0.5, 1e300]);
    case 1: return pick(random, ["", "x".repeat(int(random, 300)), "../../etc/passwd", "manifest.json", "height.f32", "__proto__"]);
    case 2: return pick(random, [null, true, false]);
    case 3: return int(random, 3);
    case 4: return 2;
    case 5: return Array.from({ length: int(random, 4) }, () => value(random, depth + 1));
    case 6: return { path: value(random, depth + 1), kind: pick(random, ["sourceHeight", "height", "waterMask", "treeMask", "grassMask", "paintedBiome", "x"]) };
    default: return Object.fromEntries(Array.from({ length: int(random, 3) }, () => [pick(random, ["width", "height", "files", "terrain", "__proto__", "constructor", "prototype", "path", "kind"]), value(random, depth + 1)]));
  }
}

/** Set a random field of a copy of the manifest, at any depth. */
function mutateManifest(random: Random): unknown {
  const copy = JSON.parse(JSON.stringify(MANIFEST));
  let target = copy;

  for (let depth = 0; depth < 3 && typeof target === "object" && target && random() < 0.6; depth += 1) {
    const keys = Object.keys(target);
    const next = target[pick(random, keys)];

    if (typeof next !== "object" || next === null) {
      break;
    }

    target = next;
  }

  const keys = Object.keys(target);
  const key = random() < 0.8 && keys.length ? pick(random, keys) : pick(random, ["__proto__", "constructor", "prototype", "extra"]);
  // Defined as an own property, as JSON.parse would, not through the setter.
  Object.defineProperty(target, key, { value: value(random), enumerable: true, configurable: true, writable: true });
  return copy;
}

let deflated: Uint8Array;

async function mutateBundle(random: Random): Promise<Uint8Array> {
  switch (int(random, 7)) {
    case 0:
      return flip(random, deflated);
    case 1:
      return deflated.subarray(0, int(random, deflated.length));
    case 2: {
      // A field of the central directory or the end record.
      const out = deflated.slice();
      const view = new DataView(out.buffer);
      const directory = view.getUint32(out.length - 6, true);
      const at = directory + int(random, out.length - directory - 2);
      pick(random, [() => view.setUint16(at, nasty(random) & 0xffff, true), () => view.setUint32(Math.min(at, out.length - 4), nasty(random), true)])();
      return out;
    }
    case 3:
      return bytes(await zip(entries(mutateManifest(random))));
    case 4: {
      const files = entries();
      const file = files[1 + int(random, files.length - 1)];
      file.data = flip(random, file.data) as Uint8Array<ArrayBuffer>;
      return bytes(await zip(files));
    }
    case 5: {
      const files = entries();
      files.splice(int(random, files.length), 1);
      return bytes(await zip(random() < 0.5 ? files : [...files, ...files.slice(0, 1)]));
    }
    default: {
      const text = JSON.stringify(MANIFEST);
      const at = int(random, text.length);
      const edited = text.slice(0, at) + pick(random, ["{", "}", "[", "]", ",", "\"", ":", "0", "-", "e999", "\\u0000", "\"__proto__\":{},"]) + text.slice(at + int(random, 3));
      const files = entries();
      files[0].data = new TextEncoder().encode(edited);
      return bytes(await zip(files));
    }
  }
}

describe("fuzzing the TypeScript decoders", () => {
  it(`decodes ${CASES} malformed PNGs to an image within the caps or an INVALID_DEM error`, async () => {
    const random = generator(0x5eed);
    const { counts, run } = tally();

    for (let index = 0; index < CASES; index += 1) {
      const input = mutatePng(random);
      await run(() => decodePng(input), (image: { width: number; height: number; channels: number; data: ArrayLike<number> }) => {
        expect(image.width).toBeGreaterThanOrEqual(1);
        expect(Math.max(image.width, image.height)).toBeLessThanOrEqual(MAX_SIDE);
        expect(image.data.length).toBe(image.width * image.height * image.channels);
      });
    }

    expect(counts.accepted + counts.rejected).toBe(CASES);
    // Both paths must be exercised for the test to mean anything.
    expect(counts.accepted).toBeGreaterThan(100);
    expect(counts.rejected).toBeGreaterThan(CASES / 2);
  }, 60_000);

  it(`reads ${CASES} malformed bundles to maps within the caps or an INVALID_DEM error`, async () => {
    const random = generator(0xb0b);
    const { counts, run } = tally();
    deflated = await bytes(await zip(entries().map((entry) => ({ ...entry, store: false }))));
    expect((await readBundle(deflated)).raw.width).toBe(2);

    for (let index = 0; index < CASES; index += 1) {
      const input = await mutateBundle(random);
      await run(() => readBundle(input), (bundle: Awaited<ReturnType<typeof readBundle>>) => {
        const { width, height } = bundle.raw;
        expect(Math.max(width, height)).toBeLessThanOrEqual(MAX_SIDE);
        expect(bundle.heights.byteLength).toBe(width * height * 4);

        for (const map of [bundle.water, bundle.biome, bundle.trees, bundle.grass]) {
          expect(map === undefined || (map.width <= MAX_SIDE && map.height <= MAX_SIDE && map.data.length === map.width * map.height)).toBe(true);
        }
      });
    }

    expect(counts.accepted + counts.rejected).toBe(CASES);
    expect(counts.accepted).toBeGreaterThan(100);
    expect(counts.rejected).toBeGreaterThan(CASES / 2);
  }, 60_000);

  it(`parses ${CASES} malformed manifests to a checked manifest or an INVALID_DEM error`, async () => {
    const random = generator(0x3a4);
    const { counts, run } = tally();
    const before = Object.getOwnPropertyNames(Object.prototype).join();

    for (let index = 0; index < CASES; index += 1) {
      const text = random() < 0.5
        ? JSON.stringify(mutateManifest(random))
        : Array.from({ length: int(random, 60) }, () => pick(random, ["{", "}", "[", "]", ",", ":", "\"a\"", "1", "null", "\"__proto__\"", "￿"])).join("");
      const input = new TextEncoder().encode(text);
      await run(async () => parseManifest(input), (manifest: Record<string, unknown>) => {
        expect(manifest.format).toBe("vistawasm-bundle");
      });
    }

    expect(counts.accepted + counts.rejected).toBe(CASES);
    expect(counts.accepted).toBeGreaterThan(100);
    expect(Object.getOwnPropertyNames(Object.prototype).join()).toBe(before);
    expect(({} as Record<string, unknown>).width).toBeUndefined();
  });
});
