import { compress, crc32, MAX_TERRAIN_SIDE, PNG_SIGNATURE, TERRAIN_SIZE_REASON } from "./codec.js";
import { checkOptions, invalid, type VistaWasmError } from "./errors.js";
import { computeHeightmapPixels, downloadBlob } from "./terrain-export.js";
import type {
  ExportedMap,
  MapKind,
  TerrainMetadata,
  TreeRecord,
  TreeSpecies,
  VistaEngine
} from "./types.js";

/** Every tree species, in the engine's index order. */
export const TREE_SPECIES: readonly TreeSpecies[] = ["oak", "pine", "spruce", "palm", "jungle", "cypress", "acacia", "shrub"];

/** Every map `exportMap()` reads, in the order the docs list them. */
export const MAP_KINDS: readonly MapKind[] = [
  "height",
  "biome",
  "water",
  "waterDepth",
  "flow",
  "discharge",
  "materials",
  "slope",
  "normals",
  "occlusion",
  "temperature",
  "moisture",
  "treeDensity",
  "grassDensity",
  "sourceHeight"
];

/**
 * `encodePng()` options.
 */
export interface PngOptions {
  /** 8 (the default) or 16. 16-bit is for single-channel maps. */
  bitDepth?: 8 | 16;
  /**
   * Values mapped to black and white. Defaults to the map's lowest and
   * highest value, except for byte maps written at 8 bits, which are
   * written as they are.
   */
  range?: [number, number];
  /** Filter each row against the one above: smaller files, slower. */
  smaller?: boolean;
}

/** A PNG, with the value range its grey levels span when it was scaled. */
export type PngBlob = Blob & { range?: [number, number] };

/**
 * `encodeRaw()` options.
 */
export interface RawOptions {
  /** `"float32"` (the default) or `"uint16"`, scaled over `range`. */
  type?: "float32" | "uint16";
  /** Values mapped to 0 and 65535. Defaults to the map's lowest and highest. */
  range?: [number, number];
}

/**
 * `exportBundle()` options.
 */
export interface BundleOptions {
  /** Resample every map to `[width, height]`; `height.f32` stays exact at the native size. */
  size?: [number, number];
  /**
   * Include `trees.csv`. By default it is included when the map holds at
   * most `maxTrees` trees; `true` throws when it holds more.
   */
  trees?: boolean;
  /** Add `preview-height.png` and `preview-biome.png`. */
  previews?: boolean;
  /** Most trees to export, 1 to 10,000,000. Defaults to 2,000,000. */
  maxTrees?: number;
}

type Bytes = Uint8Array<ArrayBuffer>;

export { crc32 };

/** Check an export size: two whole numbers from 2 to `MAX_TERRAIN_SIDE`. */
export function checkSize(name: string, size: unknown): [number, number] {
  if (!Array.isArray(size) || size.length !== 2) {
    throw new TypeError(`${name} size must be [width, height].`);
  }

  if (!size.every((side) => Number.isInteger(side) && side >= 2 && side <= MAX_TERRAIN_SIDE)) {
    throw invalid(`${name} size must be whole numbers from 2 to ${MAX_TERRAIN_SIDE} (${TERRAIN_SIZE_REASON}), but it is [${size.join(", ")}].`);
  }

  return size as [number, number];
}

function checkRange(name: string, range: unknown): [number, number] | undefined {
  if (range === undefined) {
    return undefined;
  }

  if (
    !Array.isArray(range) ||
    range.length !== 2 ||
    !range.every((value) => typeof value === "number" && Number.isFinite(value)) ||
    range[0] > range[1]
  ) {
    throw invalid(`${name} range must be two finite numbers [low, high] with low <= high.`);
  }

  return range as [number, number];
}

function checkMap(name: string, map: ExportedMap): void {
  const { width, height, channels, data } = Object(map) as ExportedMap;
  const whole = (value: number, top: number) => Number.isInteger(value) && value >= 1 && value <= top;

  // Whole numbers this small multiply exactly, so the length check is sound.
  if (
    !(data instanceof Float32Array || data instanceof Uint8Array || data instanceof Uint16Array) ||
    !whole(width, MAX_TERRAIN_SIDE) || !whole(height, MAX_TERRAIN_SIDE) || !whole(channels, 12) ||
    data.length !== width * height * channels
  ) {
    throw new TypeError(
      `${name} expects a map from exportMap(): width and height whole numbers from 1 to ${MAX_TERRAIN_SIDE}, 1 to 12 channels, and width x height x channels values in a Float32Array, Uint8Array or Uint16Array.`
    );
  }
}

function minMax(data: ArrayLike<number>): [number, number] {
  let [low, high] = [Infinity, -Infinity];

  for (let index = 0; index < data.length; index += 1) {
    low = Math.min(low, data[index]);
    high = Math.max(high, data[index]);
  }

  return low <= high ? [low, high] : [0, 0];
}

/** `value` over `range`, as a whole number from 0 to `top`. */
function quantise(value: number, [low, high]: [number, number], top: number): number {
  return high > low ? Math.round(Math.min(Math.max((value - low) / (high - low), 0), 1) * top) : 0;
}

function chunk(type: string, data: Uint8Array): Bytes {
  const out = new Uint8Array(data.length + 12);
  const view = new DataView(out.buffer);
  view.setUint32(0, data.length);

  for (let index = 0; index < 4; index += 1) {
    out[4 + index] = type.charCodeAt(index);
  }

  out.set(data, 8);
  view.setUint32(data.length + 8, crc32(out.subarray(4, data.length + 8)));
  return out;
}

/**
 * Write a PNG from its rows: `row(y)` returns row `y`'s bytes, unfiltered.
 * Colour types: 0 grey, 2 RGB, 3 palette, 6 RGBA.
 */
async function png(
  width: number,
  height: number,
  colourType: number,
  bitDepth: number,
  row: (y: number) => Uint8Array,
  extra: Bytes[],
  smaller = false
): Promise<Blob> {
  const header = new Uint8Array(13);
  const view = new DataView(header.buffer);
  view.setUint32(0, width);
  view.setUint32(4, height);
  header.set([bitDepth, colourType], 8);
  const step = ([1, 0, 3, 1, 2, 0, 4][colourType] * bitDepth) / 8;

  function* scanlines(): Generator<Bytes> {
    let above: Uint8Array = new Uint8Array(width * step);

    for (let y = 0; y < height; y += 1) {
      const bytes = row(y);
      const line = new Uint8Array(bytes.length + 1);
      line[0] = smaller ? 2 : 0;

      for (let index = 0; index < bytes.length; index += 1) {
        line[index + 1] = smaller ? bytes[index] - above[index] : bytes[index];
      }

      above = bytes;
      yield line;
    }
  }

  const data = await compress("deflate", scanlines());
  return new Blob([
    new Uint8Array(PNG_SIGNATURE),
    chunk("IHDR", header),
    ...extra,
    ...data.map((piece) => chunk("IDAT", piece)),
    chunk("IEND", new Uint8Array(0))
  ], { type: "image/png" });
}

function text(key: string, value: string): Bytes {
  return chunk("tEXt", new TextEncoder().encode(`${key}\0${value}`));
}

/**
 * Encode a map as a PNG, writing the file directly:
 *
 * - `biome` and `water` as palette images in their legend's colours;
 * - `normals` as RGB, each component `(n + 1) / 2 x 255`;
 * - `materials` as three RGBA files (lush grass, dry grass, forest floor,
 *   sand; rock, snow, mud, volcanic; ice, tundra, gravel, scree), so the
 *   promise resolves to an array of three;
 * - every other map as grey, 8 or 16-bit. Byte maps at 8 bits are written
 *   as they are; otherwise values are scaled over `options.range` (by
 *   default the map's lowest and highest), which is stored in a
 *   `vistawasm:range` text chunk and returned as the Blob's `range`.
 */
export async function encodePng(
  map: ExportedMap,
  options: PngOptions = {}
): Promise<PngBlob | PngBlob[]> {
  checkMap("encodePng()", map);
  checkOptions("encodePng()", options);
  const { width, height, data, channels } = map;
  const bitDepth = options.bitDepth ?? 8;
  let range = checkRange("encodePng()", options.range);

  if (bitDepth !== 8 && bitDepth !== 16) {
    throw invalid(`encodePng() bitDepth must be 8 or 16, but it is ${String(bitDepth)}.`);
  }

  if (bitDepth === 16 && channels !== 1) {
    throw invalid(`encodePng() writes 16-bit PNGs for single-channel maps only, and ${map.kind} has ${channels} channels.`);
  }

  const rows = (bytes: number, pixel: (index: number, out: Uint8Array, at: number) => void) => (y: number) => {
    const out = new Uint8Array(width * bytes);

    for (let x = 0; x < width; x += 1) {
      pixel(y * width + x, out, x * bytes);
    }

    return out;
  };
  const byte = (value: number) => Math.round(Math.min(Math.max(value, 0), 255));

  if (map.kind === "materials") {
    return Promise.all([0, 4, 8].map((first) =>
      png(width, height, 6, 8, rows(4, (index, out, at) => {
        for (let k = 0; k < 4; k += 1) {
          out[at + k] = byte(data[index * 12 + first + k]);
        }
      }), [], options.smaller)
    ));
  }

  if (channels === 3) {
    return png(width, height, 2, 8, rows(3, (index, out, at) => {
      for (let k = 0; k < 3; k += 1) {
        out[at + k] = byte((data[index * 3 + k] + 1) * 127.5);
      }
    }), [], options.smaller);
  }

  const legend = map.encoding?.legend;

  if ((map.kind === "biome" || map.kind === "water") && legend) {
    // A palette holds at most 256 colours, one for each byte value.
    if (!Array.isArray(legend) || legend.length > 256 || !legend.every((entry) => Number.isInteger(entry?.index) && entry.index >= 0 &&
      entry.index < 256 && Array.isArray(entry.colour) && entry.colour.length === 3 && entry.colour.every(Number.isFinite))) {
      throw invalid("encodePng() map legend must hold at most 256 entries, each with an index from 0 to 255 and three finite colour components.");
    }

    const palette = new Uint8Array(3 * (Math.max(...legend.map((entry) => entry.index)) + 1));

    for (const entry of legend) {
      palette.set(entry.colour.map((component) => byte(component * 255)), entry.index * 3);
    }

    return png(width, height, 3, 8, rows(1, (index, out, at) => {
      out[at] = data[index];
    }), [chunk("PLTE", palette)], options.smaller);
  }

  if (bitDepth === 8 && !range && !(data instanceof Float32Array)) {
    return png(width, height, 0, 8, rows(1, (index, out, at) => {
      out[at] = byte(data[index]);
    }), [], options.smaller);
  }

  range ??= minMax(data);
  const scale = range;
  const blob: PngBlob = await png(width, height, 0, bitDepth, rows(bitDepth / 8, (index, out, at) => {
    const value = quantise(data[index], scale, bitDepth === 16 ? 65535 : 255);

    if (bitDepth === 16) {
      // PNG samples are big-endian.
      out[at] = value >> 8;
      out[at + 1] = value & 255;
    } else {
      out[at] = value;
    }
  }), [text("vistawasm:range", JSON.stringify(range))], options.smaller);
  blob.range = range;
  return blob;
}

/**
 * Encode a map as raw little-endian samples, row-major with channels
 * interleaved: Float32 as they are, or Uint16 scaled over
 * `options.range` (by default the map's lowest and highest value), which
 * is returned.
 */
export function encodeRaw(
  map: ExportedMap,
  options: RawOptions = {}
): { bytes: Bytes; range?: [number, number] } {
  checkMap("encodeRaw()", map);
  checkOptions("encodeRaw()", options);
  const { data } = map;
  const type = options.type ?? "float32";

  if (type !== "float32" && type !== "uint16") {
    throw invalid(`encodeRaw() type must be "float32" or "uint16", but it is ${String(type)}.`);
  }

  const size = type === "float32" ? 4 : 2;

  if (data.length * size > 2 ** 30) {
    throw invalid(
      `encodeRaw() would need ${data.length * size} bytes for this ${map.kind} map, over the 1 GiB limit. Export it at a smaller size.`
    );
  }

  const bytes = new Uint8Array(data.length * size);
  const view = new DataView(bytes.buffer);

  if (type === "float32") {
    for (let index = 0; index < data.length; index += 1) {
      view.setFloat32(index * 4, data[index], true);
    }

    return { bytes };
  }

  const range = checkRange("encodeRaw()", options.range) ?? minMax(data);

  for (let index = 0; index < data.length; index += 1) {
    view.setUint16(index * 2, quantise(data[index], range, 65535), true);
  }

  return { bytes, range };
}

const TREE_FIELDS = ["x", "y", "z", "species", "variant", "scale", "rotation", "tint", "dryness"] as const;

/**
 * Check trees before they are written out. Only numbers and known species
 * names reach a CSV cell, so no cell can hold a comma, a line break or a
 * formula.
 */
function checkTrees(call: string, trees: TreeRecord[]): void {
  if (!Array.isArray(trees)) {
    throw new TypeError(`${call} expects an array of trees from exportTrees().`);
  }

  trees.forEach((tree, index) => {
    if (!TREE_SPECIES.includes(tree?.species) || !TREE_FIELDS.every((field) => field === "species" || typeof tree[field] === "number")) {
      throw new TypeError(`${call} tree ${index} needs numbers for ${TREE_FIELDS.join(", ")} (except species), and a species from: ${TREE_SPECIES.join(", ")}.`);
    }
  });
}

/** Trees as CSV, one tree a row after a header row. Numbers keep full precision. */
export function treesToCsv(trees: TreeRecord[]): string {
  checkTrees("treesToCsv()", trees);
  const lines = [[...TREE_FIELDS, "handPlaced"].join(",")];

  for (const tree of trees) {
    lines.push([...TREE_FIELDS.map((field) => tree[field]), tree.handPlaced === true].join(","));
  }

  return `${lines.join("\n")}\n`;
}

/** Trees as a JSON array of `TreeRecord`s. */
export function treesToJson(trees: TreeRecord[]): string {
  checkTrees("treesToJson()", trees);
  return JSON.stringify(trees);
}

/** Throw when a zip field would pass the 4 GiB a zip without ZIP64 can record. */
export function checkZip32(bytes: number): void {
  if (bytes > 0xffffffff) {
    throw invalid(
      "The bundle would be over 4 GiB, which a zip without ZIP64 cannot hold. Export at a smaller size, or without trees."
    );
  }
}

/**
 * Write a zip: each entry deflated (or stored, when `store` is set, for
 * data that is compressed already), with UTF-8 names and a fixed date so
 * the same maps always give the same file.
 */
export async function zip(entries: { name: string; data: Bytes; store?: boolean }[]): Promise<Blob> {
  const parts: BlobPart[] = [];
  const central: Bytes[] = [];
  let offset = 0;

  for (const { name, data, store } of entries) {
    const path = new TextEncoder().encode(name);
    const body = store ? [data] : await compress("deflate-raw", [data]);
    const size = body.reduce((sum, piece) => sum + piece.length, 0);
    const crc = crc32(data);
    // Version needed, UTF-8 flag, method, time, date (1 January 1980),
    // CRC, sizes and the name's length.
    const fields = (header: DataView, at: number) => {
      header.setUint16(at, 20, true);
      header.setUint16(at + 2, 0x800, true);
      header.setUint16(at + 4, store ? 0 : 8, true);
      header.setUint16(at + 8, 33, true);
      header.setUint32(at + 10, crc, true);
      header.setUint32(at + 14, size, true);
      header.setUint32(at + 18, data.length, true);
      header.setUint16(at + 22, path.length, true);
    };
    checkZip32(offset + 30 + path.length + size);
    const local = new Uint8Array(30 + path.length);
    const localView = new DataView(local.buffer);
    localView.setUint32(0, 0x04034b50, true);
    fields(localView, 4);
    local.set(path, 30);
    const record = new Uint8Array(46 + path.length);
    const recordView = new DataView(record.buffer);
    recordView.setUint32(0, 0x02014b50, true);
    recordView.setUint16(4, 20, true);
    fields(recordView, 6);
    recordView.setUint32(42, offset, true);
    record.set(path, 46);
    parts.push(local, ...body);
    central.push(record);
    offset += local.length + size;
  }

  const directory = central.reduce((sum, record) => sum + record.length, 0);
  checkZip32(offset + directory);
  const end = new Uint8Array(22);
  const endView = new DataView(end.buffer);
  endView.setUint32(0, 0x06054b50, true);
  endView.setUint16(8, entries.length, true);
  endView.setUint16(10, entries.length, true);
  endView.setUint32(12, directory, true);
  endView.setUint32(16, offset, true);
  return new Blob([...parts, ...central, end], { type: "application/zip" });
}

async function blobBytes(blob: Blob): Promise<Bytes> {
  return new Uint8Array(await blob.arrayBuffer());
}

/** A painted mask as an 8-bit grey PNG, its bytes as they are. */
function maskPng(mask: { width: number; height: number; data: Uint8Array }): Promise<Blob> {
  return png(mask.width, mask.height, 0, 8, (y) => mask.data.subarray(y * mask.width, (y + 1) * mask.width), []);
}

/**
 * Export every map, the trees and the options into one zip with a
 * `manifest.json` describing each file, which `loadBundle()` reads back:
 * the heights before any carving, the options and the painted maps
 * recreate the scene exactly. See `docs/export-and-snapshots.md` for the
 * layout.
 */
export async function exportBundle(engine: VistaEngine, options: BundleOptions = {}): Promise<Blob> {
  const { maxTrees = 2_000_000 } = checkOptions<BundleOptions>("exportBundle()", options);
  const size = options.size === undefined ? undefined : checkSize("exportBundle()", options.size);

  if (!Number.isInteger(maxTrees) || maxTrees < 1 || maxTrees > 10_000_000) {
    throw invalid(`exportBundle() maxTrees must be a whole number from 1 to 10000000, but it is ${String(maxTrees)}.`);
  }

  const height = engine.exportMap("height");
  const [min, max] = height.encoding.range ?? [0, 0];
  const sea = height.encoding.seaLevelMetres ?? 0;
  const entries: { name: string; data: Bytes; store?: boolean }[] = [];
  const files: Record<string, unknown>[] = [];
  const describe = (path: string, map: ExportedMap, extra: Record<string, unknown> = {}) => {
    files.push({
      path,
      kind: map.kind,
      type: map.type,
      width: map.width,
      height: map.height,
      channels: map.channels,
      ...extra,
      encoding: map.encoding
    });
  };
  const addRaw = (map: ExportedMap) => {
    entries.push({ name: `${map.kind}.f32`, data: encodeRaw(map).bytes });
    describe(`${map.kind}.f32`, map);
  };
  const addPng = async (map: ExportedMap, png: PngOptions = {}) => {
    const blobs = await encodePng(map, png);

    for (const [index, blob] of (Array.isArray(blobs) ? blobs : [blobs]).entries()) {
      const name = Array.isArray(blobs) ? `splat${index}.png` : `${map.kind}.png`;
      entries.push({ name, data: await blobBytes(blob), store: true });
      describe(name, map, Array.isArray(blobs)
        ? { channels: 4, firstChannel: index * 4 }
        : { bitDepth: 8, ...(blob.range ? { range: blob.range } : {}) });
    }
  };

  // The heights stay exact at the terrain's own size; the rest follow
  // `size`.
  entries.push({ name: "height.f32", data: encodeRaw(height).bytes });
  describe("height.f32", height);
  const source = engine.exportMap("sourceHeight");
  entries.push({ name: "source-height.f32", data: encodeRaw(source).bytes });
  describe("source-height.f32", source);
  const map = (kind: MapKind) => engine.exportMap(kind, size ? { size } : {});

  for (const kind of ["biome", "water"] as const) {
    await addPng(map(kind));
  }

  addRaw(map("flow"));
  addRaw(map("discharge"));
  await addPng(map("materials"));
  await addPng(map("slope"), { range: [0, 90] });

  for (const kind of ["normals", "occlusion"] as const) {
    await addPng(map(kind));
  }

  addRaw(map("temperature"));

  for (const kind of ["moisture", "treeDensity", "grassDensity"] as const) {
    await addPng(map(kind));
  }

  let trees: TreeRecord[] | undefined;

  if (options.trees !== false) {
    try {
      trees = engine.exportTrees({ maxCount: maxTrees });
    } catch (error) {
      // By default a forest over the cap is left out, not an error.
      if (options.trees === true || (error as VistaWasmError).code !== "OPTIONS_INVALID") {
        throw error;
      }
    }
  }

  if (trees) {
    entries.push({ name: "trees.csv", data: new TextEncoder().encode(treesToCsv(trees)) });
    files.push({ path: "trees.csv", kind: "trees", type: "csv", count: trees.length });
  }

  if (options.previews) {
    const metadata = {
      width: height.width,
      height: height.height,
      minHeightMetres: min,
      maxHeightMetres: max,
      seaLevelMetres: sea
    } as TerrainMetadata;
    const pixels = computeHeightmapPixels(metadata, encodeRaw(height).bytes);
    const biome = engine.exportMap("biome");
    const colours = (biome.encoding.legend ?? []).map((entry) => entry.colour.map((c) => Math.round(c * 255)));
    const biomeRow = (y: number) => {
      const row = new Uint8Array(biome.width * 3);

      for (let x = 0; x < biome.width; x += 1) {
        row.set(colours[biome.data[y * biome.width + x]] ?? [0, 0, 0], x * 3);
      }

      return row;
    };
    const previews: [string, Blob][] = [
      ["preview-height.png", await png(height.width, height.height, 6, 8, (y) =>
        new Uint8Array(pixels.buffer, y * height.width * 4, height.width * 4), [])],
      ["preview-biome.png", await png(biome.width, biome.height, 2, 8, biomeRow, [])]
    ];

    for (const [name, blob] of previews) {
      entries.push({ name, data: await blobBytes(blob), store: true });
      files.push({ path: name, kind: "preview", type: "uint8" });
    }
  }

  // The painted inputs, at the sizes they were painted, which the scene
  // is recomputed from on import.
  const painted = typeof engine.getPaintedMaps === "function" ? engine.getPaintedMaps() : {};

  if (painted.biome) {
    const { width, data, borderSamples = 3 } = painted.biome;
    // Every biome's legend colour, and 255 (not painted) transparent.
    const palette = new Uint8Array(768);
    const alpha = new Uint8Array(256).fill(255);
    alpha[255] = 0;
    (engine.exportMap("biome", { size: [2, 2] }).encoding.legend ?? []).forEach((entry) =>
      palette.set(entry.colour.map((c) => Math.round(c * 255)), entry.index * 3));
    const blob = await png(width, painted.biome.height, 3, 8, (y) => data.subarray(y * width, (y + 1) * width), [
      chunk("PLTE", palette),
      chunk("tRNS", alpha)
    ]);
    entries.push({ name: "painted-biome.png", data: await blobBytes(blob), store: true });
    files.push({ path: "painted-biome.png", kind: "paintedBiome", type: "uint8", width, height: painted.biome.height, channels: 1, borderSamples });
  }

  for (const [path, kind, mask] of [
    ["water-mask.png", "waterMask", painted.water],
    ["tree-mask.png", "treeMask", painted.trees],
    ["grass-mask.png", "grassMask", painted.grass]
  ] as const) {
    if (mask) {
      entries.push({ name: path, data: await blobBytes(await maskPng(mask)), store: true });
      files.push({ path, kind, type: "uint8", width: mask.width, height: mask.height, channels: 1 });
    }
  }

  const snapshot = engine.getOptionsSnapshot();
  const manifest = {
    format: "vistawasm-bundle",
    version: 2,
    generator: height.encoding.generator,
    terrain: {
      width: height.width,
      height: height.height,
      metresPerSample: height.encoding.metresPerPixel?.[0],
      seaLevelMetres: sea,
      minHeightMetres: min,
      maxHeightMetres: max,
      landform: snapshot.terrain?.landform ?? snapshot.landform
    },
    options: snapshot,
    files
  };
  entries.unshift({ name: "manifest.json", data: new TextEncoder().encode(JSON.stringify(manifest, null, 2)) });
  return zip(entries);
}

/** Export a bundle with `exportBundle()` and download it as `filename`. */
export async function downloadBundle(
  engine: VistaEngine,
  filename: string,
  options?: BundleOptions
): Promise<void> {
  downloadBlob(await exportBundle(engine, options), filename);
}
