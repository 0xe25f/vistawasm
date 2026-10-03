import { PROTOTYPE_KEYS, VistaWasmError, checkOptions, invalid, malformed } from "./errors.js";
import { MAX_IMAGE_SIDE, MAX_TERRAIN_SIDE, TERRAIN_SIZE_REASON, crc32, inflate, isPng } from "./codec.js";
import { decodePng, type DecodedPng } from "./png-decode.js";
import type {
  BiomeKind,
  BiomeMap,
  BiomeMapFromImageOptions,
  DensityMask,
  DensityMaskFromImageOptions,
  ImageChannel,
  ImageSource,
  RawHeightmapOptions,
  WaterMask,
  WaterMaskFromImageOptions
} from "./types.js";

/** Every biome, in `BiomeKind` index order. */
export const BIOME_KINDS: readonly BiomeKind[] = [
  "grassyMeadows", "outerThicket", "outerForest", "innerForest", "mountainFoothills", "mountainProper",
  "outerVolcanic", "calderaVolcanic", "savannahExpanse", "coastalBeach", "coastalRocky", "outerJungle",
  "innerJungle", "swampWetlands", "ocean", "alpineTransition", "lowerSnowyPeaks", "upperSnowyPeaks", "iceArctic"
];

/** The exported biome map's legend colours as sRGB bytes, in `BiomeKind` order. */
export const BIOME_COLOURS: readonly (readonly [number, number, number])[] = [
  [140, 204, 77], [102, 158, 56], [51, 128, 46], [20, 82, 26], [153, 140, 102], [140, 140, 148], [89, 51, 46],
  [230, 64, 13], [217, 184, 89], [242, 224, 158], [128, 115, 107], [38, 179, 89], [5, 115, 51], [77, 97, 64],
  [26, 64, 140], [153, 128, 158], [158, 199, 242], [247, 252, 255], [191, 235, 255]
];

// The exported water map's river, lake, painted river, painted lake and
// ocean colours, each with the mask value it becomes (1: a river, whose
// brightness sets its strength).
const WATER_COLOURS = [[51, 140, 230, 1], [26, 89, 191, 255], [77, 204, 217, 1], [38, 153, 153, 255], [13, 38, 115, 0]];

export { MAX_TERRAIN_SIDE };

/** Limits on what a bundle may hold, so a hostile file cannot exhaust memory. */
export const BUNDLE_LIMITS = {
  /** Files in the zip. */
  entries: 64,
  /** Bytes any one file inflates to: far more than any map of a 2048 x 2048 terrain needs. */
  entryBytes: 300 * 2 ** 20,
  /** Bytes all the files the reader inflates may take together. */
  totalBytes: 2 ** 30,
  /** Bytes of `manifest.json`. */
  manifestBytes: 2 ** 20,
  /**
   * Bytes the painted maps may decode to together, from their PNG
   * headers: checked before the first is decoded, and they are decoded
   * one at a time.
   */
  decodedBytes: 2 ** 30
};

/** Most colours a `biomeMapFromImage()` legend may hold. */
export const MAX_LEGEND_COLOURS = 256;

export type Warn = (message: string) => void;
type Grid = { width: number; height: number; data: Uint8Array };

function warner(options: { onWarning?: unknown }): Warn {
  const { onWarning } = options;

  if (onWarning !== undefined && typeof onWarning !== "function") {
    throw new TypeError("onWarning must be a function.");
  }

  return (onWarning as Warn | undefined) ?? console.warn;
}

async function bytesOf(source: unknown, name: string): Promise<Uint8Array> {
  if (source instanceof Uint8Array) {
    return source;
  }

  if (source instanceof ArrayBuffer) {
    return new Uint8Array(source);
  }

  if (typeof Blob === "function" && source instanceof Blob) {
    return new Uint8Array(await source.arrayBuffer());
  }

  throw new TypeError(`${name} must be a Blob, an ArrayBuffer or a Uint8Array.`);
}

/**
 * The width and height a PNG, JPEG, WebP, GIF or BMP header gives, read
 * from its first bytes without decoding it, or `null` when the bytes
 * hold none of them. A JPEG's size follows its metadata segments, so pass
 * enough of the file to reach the frame header (a megabyte is plenty).
 */
export function imageSize(bytes: Uint8Array): [number, number] | null {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const text = (at: number, length: number) => String.fromCharCode(...bytes.subarray(at, at + length));

  if (bytes.length >= 24 && isPng(bytes) && text(12, 4) === "IHDR") {
    return [view.getUint32(16), view.getUint32(20)];
  }

  if (bytes.length >= 30 && text(0, 4) === "RIFF" && text(8, 4) === "WEBP") {
    const chunk = text(12, 4);
    const side = (at: number) => 1 + (bytes[at] | (bytes[at + 1] << 8) | (bytes[at + 2] << 16));

    return chunk === "VP8X" ? [side(24), side(27)]
      : chunk === "VP8L" ? [1 + (view.getUint32(21, true) & 0x3fff), 1 + ((view.getUint32(21, true) >>> 14) & 0x3fff)]
        : chunk === "VP8 " ? [view.getUint16(26, true) & 0x3fff, view.getUint16(28, true) & 0x3fff]
          : null;
  }

  if (bytes.length >= 10 && (text(0, 6) === "GIF87a" || text(0, 6) === "GIF89a")) {
    return [view.getUint16(6, true), view.getUint16(8, true)];
  }

  if (bytes.length >= 26 && text(0, 2) === "BM") {
    return [Math.abs(view.getInt32(18, true)), Math.abs(view.getInt32(22, true))];
  }

  if (bytes.length >= 4 && bytes[0] === 0xff && bytes[1] === 0xd8) {
    let at = 2;

    while (at + 9 < bytes.length && bytes[at] === 0xff) {
      const marker = bytes[at + 1];

      // Start-of-frame markers, apart from the ones that are not frames.
      if (marker >= 0xc0 && marker <= 0xcf && marker !== 0xc4 && marker !== 0xc8 && marker !== 0xcc) {
        return [view.getUint16(at + 7), view.getUint16(at + 5)];
      }

      at += 2 + view.getUint16(at + 2);
    }
  }

  return null;
}

/** How errors name painted maps, which are held to the terrain's size. */
const PAINTED = `painted maps (${TERRAIN_SIZE_REASON})`;

/**
 * Decode an image 2 to `maxSide` pixels a side (8192 by default): PNGs in
 * full, others through the browser at 8 bits, with a warning. `limit`
 * says why the size is limited, for the error.
 */
export async function decodeImage(
  source: unknown,
  name: string,
  warn: Warn,
  maxSide = MAX_IMAGE_SIDE,
  limit = "maps"
): Promise<DecodedPng> {
  const bytes = await bytesOf(source, name);
  // The size comes from the header first, so an image too large is
  // refused before anything decodes it.
  const size = imageSize(bytes);

  if (size?.some((side) => side < 2 || side > maxSide)) {
    throw malformed(`${name} is ${size[0]} x ${size[1]}, but ${limit} must be from 2 to ${maxSide} pixels a side.`);
  }

  let image: DecodedPng;

  if (isPng(bytes)) {
    image = await decodePng(bytes);
  } else {
    if (!size) {
      throw malformed(`${name} is not a PNG, JPEG, WebP, GIF or BMP image, or its header is damaged, so its size cannot be checked before decoding it.`);
    }

    let bitmap: ImageBitmap;

    try {
      bitmap = await createImageBitmap(new Blob([bytes as Uint8Array<ArrayBuffer>]), {
        premultiplyAlpha: "none",
        colorSpaceConversion: "none"
      });
    } catch (error) {
      throw malformed(`${name} is not a PNG, and could not be decoded as another image: ${(error as Error).message}`);
    }

    const { width, height } = bitmap;
    const context = width <= MAX_IMAGE_SIDE && height <= MAX_IMAGE_SIDE ? new OffscreenCanvas(width, height).getContext("2d") : null;
    context?.drawImage(bitmap, 0, 0);
    bitmap.close();
    warn(`${name} is not a PNG, so it was read at 8 bits a channel. Use a 16-bit PNG for full precision.`);
    image = {
      width,
      height,
      channels: 4,
      bitDepth: 8,
      data: new Uint8Array(context?.getImageData(0, 0, width, height).data.buffer ?? new ArrayBuffer(0)),
      text: {}
    };
  }

  if (image.width < 2 || image.height < 2 || image.data.length === 0) {
    throw malformed(`${name} is ${image.width} x ${image.height}, but maps must be from 2 to ${MAX_IMAGE_SIDE} pixels a side.`);
  }

  return image;
}

/** Pixel `index`'s RGBA into `out`, each 0 to 1, at the image's full precision. */
function pixel(image: DecodedPng, index: number, out: number[]): number[] {
  const { data, channels, palette, transparent } = image;
  const at = index * channels;
  const top = 2 ** image.bitDepth - 1;

  for (let c = 0; c < 4; c += 1) {
    out[c] = palette
      ? (palette[data[at] * 4 + c] ?? 0) / 255
      : c < 3
        ? data[at + (channels < 3 ? 0 : c)] / top
        : transparent?.every((value, k) => value === data[at + k]) ? 0 : channels % 2 ? 1 : data[at + channels - 1] / top;
  }

  return out;
}

const CHANNELS: readonly ImageChannel[] = ["luminance", "r", "g", "b", "a"];

/** One channel of every pixel, 0 to 1. Luminance weighs RGB by Rec. 709. */
export function channelOf(image: DecodedPng, channel: unknown, call: string): Float32Array {
  channel ??= "luminance";

  if (!CHANNELS.includes(channel as ImageChannel)) {
    throw invalid(`${call} channel must be one of ${CHANNELS.join(", ")}, but it is ${String(channel)}.`);
  }

  const grey = image.channels < 3 && !image.palette;
  const out = new Float32Array(image.width * image.height);
  const px = [0, 0, 0, 0];

  for (let index = 0; index < out.length; index += 1) {
    const [r, g, b] = pixel(image, index, px);
    out[index] = channel === "luminance" ? (grey ? r : 0.2126 * r + 0.7152 * g + 0.0722 * b) : px[CHANNELS.indexOf(channel as ImageChannel) - 1];
  }

  return out;
}

/**
 * The bytes `decodePng()` would give a PNG, from its IHDR alone: a byte a
 * sample (two at 16 bits), as many samples a pixel as its colour type has.
 */
function decodedSize(png: Uint8Array, what: string): number {
  const view = new DataView(png.buffer, png.byteOffset, png.byteLength);
  const channels = png.length >= 26 ? [1, 0, 3, 1, 2, 0, 4][png[25]] : 0;

  if (!isPng(png) || png.length < 29 || !channels) {
    throw malformed(`${what} is not a PNG with a valid IHDR chunk.`);
  }

  const [width, height] = [view.getUint32(16), view.getUint32(20)];

  if ([width, height].some((side) => side < 2 || side > MAX_TERRAIN_SIDE)) {
    throw malformed(`${what} is ${width} x ${height}, but ${PAINTED} must be from 2 to ${MAX_TERRAIN_SIDE} pixels a side.`);
  }

  return view.getUint32(16) * view.getUint32(20) * channels * (png[24] === 16 ? 2 : 1);
}

function toBytes(image: DecodedPng, values: Float32Array): Grid {
  return { width: image.width, height: image.height, data: Uint8Array.from(values, (value) => Math.round(value * 255)) };
}

/** sRGB bytes to CIELAB (D65). */
function lab(colour: readonly number[]): number[] {
  const linear = colour.map((value) => (value /= 255) <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4);
  const [x, y, z] = [
    [0.4124, 0.3576, 0.1805, 0.95047],
    [0.2126, 0.7152, 0.0722, 1],
    [0.0193, 0.1192, 0.9505, 1.08883]
  ].map(([r, g, b, white]) => {
    const value = (r * linear[0] + g * linear[1] + b * linear[2]) / white;
    return value > 0.008856 ? Math.cbrt(value) : 7.787 * value + 16 / 116;
  });
  return [116 * y - 16, 500 * (x - y), 200 * (y - z)];
}

/**
 * Read a painted biome map from an image. A palette PNG in the exported
 * biome colours maps its indices directly; any other image maps each
 * pixel to the nearest legend colour in CIELAB, and pixels more than
 * ΔE 25 from all of them count as unmatched (they still take the
 * nearest). Fully transparent pixels are not painted (255).
 */
export async function biomeMapFromImage(
  image: ImageSource,
  options: BiomeMapFromImageOptions = {}
): Promise<{ map: BiomeMap; unmatchedFraction: number }> {
  const { legend, borderSamples = 3 } = checkOptions("biomeMapFromImage()", options);

  if (!Number.isInteger(borderSamples) || (borderSamples as number) < 0 || (borderSamples as number) > 8) {
    throw invalid(`biomeMapFromImage() borderSamples must be a whole number from 0 to 8, but it is ${String(borderSamples)}.`);
  }

  if (legend !== undefined && (!Array.isArray(legend) || legend.length === 0)) {
    throw new TypeError("biomeMapFromImage() legend must be a non-empty array of { colour, biome }.");
  }

  // Every distinct pixel colour is compared with every legend colour.
  if (Array.isArray(legend) && legend.length > MAX_LEGEND_COLOURS) {
    throw invalid(`biomeMapFromImage() legend may hold at most ${MAX_LEGEND_COLOURS} colours, but it holds ${legend.length}.`);
  }

  const targets = ((legend as unknown[] | undefined) ?? BIOME_COLOURS.map((colour, index) => ({ colour, biome: BIOME_KINDS[index] })))
    .map((entry) => {
      const { colour, biome } = Object(entry) as Record<string, unknown>;
      const index = BIOME_KINDS.indexOf(biome as BiomeKind);

      if (!Array.isArray(colour) || colour.length !== 3 || !colour.every((c) => Number.isInteger(c) && c >= 0 && c <= 255) || index < 0) {
        throw invalid("biomeMapFromImage() legend entries need a colour of three whole numbers from 0 to 255 and a biome that is a BiomeKind.");
      }

      return [index, ...lab(colour)];
    });
  const warn = warner(options);
  const decoded = await decodeImage(image, "The biome map image", warn, MAX_TERRAIN_SIDE, PAINTED);
  const { palette } = decoded;
  const data = new Uint8Array(decoded.width * decoded.height);
  const cache = new Map<number, number>();
  const px = [0, 0, 0, 0];
  let unmatched = 0;
  // The exported palette maps its indices directly.
  const direct = !legend && palette && BIOME_COLOURS.every((colour, index) => colour.every((c, k) => (palette[index * 4 + k] ?? c) === c));

  for (let index = 0; index < data.length; index += 1) {
    const [r, g, b, a] = pixel(decoded, index, px).map((value) => Math.round(value * 255));
    const key = direct ? decoded.data[index] : a ? (r << 16) | (g << 8) | b : -1;
    let found = cache.get(key);

    if (found === undefined) {
      const here = lab([r, g, b]);
      let [best, nearest] = [Infinity, 0];

      for (const [biome, l, u, v] of targets) {
        const distance = Math.hypot(here[0] - l, here[1] - u, here[2] - v);
        [best, nearest] = distance < best ? [distance, biome] : [best, nearest];
      }

      // Bit 8 marks a colour further than ΔE 25 from every legend colour.
      found = direct ? (key < 19 && a ? key : 255) : a ? nearest | (best > 25 ? 256 : 0) : 255;
      cache.set(key, found);
    }

    data[index] = found;
    unmatched += found >> 8;
  }

  const unmatchedFraction = unmatched / data.length;

  if (unmatchedFraction > 0.01) {
    warn(`${(unmatchedFraction * 100).toFixed(1)} % of the biome map's pixels are more than ΔE 25 from every legend colour, so they took the nearest biome. Paint in the legend colours, or pass your own legend.`);
  }

  return { map: { width: decoded.width, height: decoded.height, data, borderSamples: borderSamples as number }, unmatchedFraction };
}

/**
 * Read a water mask from an image. By default (`mode: "legend"`) pixels
 * in the exported water map's lake blues become lakes, and pixels in its
 * river blues and cyans rivers, their brightness setting the strength (1
 * to 127); anything else is dry. `mode: "grey"` reads luminance as
 * `WaterMask` values.
 */
export async function waterMaskFromImage(image: ImageSource, options: WaterMaskFromImageOptions = {}): Promise<WaterMask> {
  const { mode = "legend" } = checkOptions("waterMaskFromImage()", options);

  if (mode !== "legend" && mode !== "grey") {
    throw invalid(`waterMaskFromImage() mode must be "legend" or "grey", but it is ${String(mode)}.`);
  }

  const decoded = await decodeImage(image, "The water mask image", warner(options), MAX_TERRAIN_SIDE, PAINTED);

  if (mode === "grey") {
    return toBytes(decoded, channelOf(decoded, "luminance", ""));
  }

  const px = [0, 0, 0, 0];
  const data = new Uint8Array(decoded.width * decoded.height).map((_, index) => {
    const [r, g, b, a] = pixel(decoded, index, px);
    const bright = Math.max(r, g, b);
    // The hue picks the kind of water, within 0.3 of a water colour.
    let [best, value, top] = [0.3, 0, 1];

    for (const [wr, wg, wb, kind] of WATER_COLOURS) {
      const peak = Math.max(wr, wg, wb);
      const distance = Math.hypot(r / bright - wr / peak, g / bright - wg / peak, b / bright - wb / peak);
      [best, value, top] = distance < best ? [distance, kind, peak / 255] : [best, value, top];
    }

    return a && bright >= 16 / 255 ? value === 1 ? Math.max(1, Math.min(127, Math.round((127 * bright) / top))) : value : 0;
  });
  return { width: decoded.width, height: decoded.height, data };
}

/**
 * Read a tree or grass density mask from an image: luminance, or
 * `options.channel`, as bytes (0 none, 128 no change, 255 twice as dense).
 */
export async function densityMaskFromImage(image: ImageSource, options: DensityMaskFromImageOptions = {}): Promise<DensityMask> {
  const { channel } = checkOptions("densityMaskFromImage()", options);
  const decoded = await decodeImage(image, "The density mask image", warner(options), MAX_TERRAIN_SIDE, PAINTED);
  return toBytes(decoded, channelOf(decoded, channel, "densityMaskFromImage()"));
}

/** Parse a bundle's manifest, rejecting oversized files and prototype keys. */
export function parseManifest(bytes: Uint8Array): Record<string, unknown> {
  if (bytes.length > BUNDLE_LIMITS.manifestBytes) {
    throw malformed(`The bundle's manifest.json is ${bytes.length} bytes, over the limit of ${BUNDLE_LIMITS.manifestBytes}.`);
  }

  let manifest: Record<string, unknown>;

  try {
    manifest = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes), (key, value: unknown) => {
      if (PROTOTYPE_KEYS.includes(key)) {
        throw malformed(`The bundle's manifest.json holds the key ${JSON.stringify(key)}, which is not allowed.`);
      }

      return value;
    });
  } catch (error) {
    throw error instanceof VistaWasmError ? error : malformed(`The bundle's manifest.json is not valid JSON: ${(error as Error).message}`);
  }

  const { format, version, terrain, files } = Object(manifest) as Record<string, unknown>;

  if (format !== "vistawasm-bundle" || (version !== 1 && version !== 2)) {
    throw malformed(`The bundle is not a VistaWASM bundle of version 1 or 2 (it says ${JSON.stringify(format)}, version ${String(version)}).`);
  }

  const { width, height, metresPerSample, seaLevelMetres = 0 } = Object(terrain) as Record<string, number>;
  const side = (value: number) => Number.isInteger(value) && value >= 2 && value <= MAX_TERRAIN_SIDE;

  if (!side(width) || !side(height) || !(metresPerSample > 0 && metresPerSample < Infinity) || !Number.isFinite(seaLevelMetres) ||
    !Array.isArray(files) || !files.every((file) => typeof Object(file).path === "string")) {
    throw malformed(`The bundle's manifest.json needs terrain { width and height from 2 to ${MAX_TERRAIN_SIDE}, metresPerSample over 0, a finite seaLevelMetres } and a list of files with paths.`);
  }

  return manifest;
}

/**
 * List a zip's files from its central directory, with every offset,
 * size and count checked against the file and `BUNDLE_LIMITS`.
 */
function readZip(bytes: Uint8Array): Map<string, { method: number; crc: number; size: number; data: Uint8Array }> {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const u32 = (at: number) => view.getUint32(at, true);
  const u16 = (at: number) => view.getUint16(at, true);
  const entries = new Map();
  let end = bytes.length - 22;

  // The end record sits before a comment of up to 65535 bytes.
  while (end >= 0 && end > bytes.length - 65580 && u32(end) !== 0x06054b50) {
    end -= 1;
  }

  const count = end < 0 ? 0 : u16(end + 10);
  const start = end < 0 ? 0 : u32(end + 16);
  let at = start;

  if (end < 0 || u32(end) !== 0x06054b50 || u16(end + 8) !== count || start + u32(end + 12) > end) {
    throw malformed("The bundle is not a zip file, or its directory is damaged.");
  }

  if (count > BUNDLE_LIMITS.entries) {
    throw malformed(`The bundle holds ${count} files, over the limit of ${BUNDLE_LIMITS.entries}.`);
  }

  for (let index = 0; index < count; index += 1) {
    const next = at + 46 + (at + 46 <= end ? u16(at + 28) + u16(at + 30) + u16(at + 32) : 0);

    if (next > end || u32(at) !== 0x02014b50) {
      throw malformed(`The bundle's zip directory entry ${index} is damaged.`);
    }

    let name: string;

    try {
      name = new TextDecoder("utf-8", { fatal: true }).decode(bytes.subarray(at + 46, at + 46 + u16(at + 28)));
    } catch {
      throw malformed(`The bundle's zip entry ${index} has a name that is not UTF-8.`);
    }

    const [method, packed, size, local] = [u16(at + 10), u32(at + 20), u32(at + 24), u32(at + 42)];
    const body = local + 30 <= start ? local + 30 + u16(local + 26) + u16(local + 28) : Infinity;

    if (u16(at + 8) & 1 || (method !== 8 && (method !== 0 || packed !== size))) {
      throw malformed(`The bundle's ${JSON.stringify(name)} is encrypted, or compressed other than by deflate.`);
    }

    if (size > BUNDLE_LIMITS.entryBytes) {
      throw malformed(`The bundle's ${JSON.stringify(name)} inflates to ${size} bytes, over the limit of ${BUNDLE_LIMITS.entryBytes}.`);
    }

    if (body + packed > start || u32(local) !== 0x04034b50 || entries.has(name)) {
      throw malformed(`The bundle's ${JSON.stringify(name)} lies outside the file, or is listed twice.`);
    }

    entries.set(name, { method, crc: u32(at + 16), size, data: bytes.subarray(body, body + packed) });
    at = next;
  }

  return entries;
}

/** A bundle's contents, decoded and checked, for `loadBundle()`. */
export interface DecodedBundle {
  version: number;
  /** The heights to load, and how. */
  heights: ArrayBuffer;
  raw: RawHeightmapOptions;
  /** The options snapshot it was exported with. */
  options: Record<string, unknown>;
  water?: WaterMask;
  biome?: BiomeMap;
  trees?: DensityMask;
  grass?: DensityMask;
}

/**
 * Read a bundle: the zip, every CRC, the manifest, the heights (the
 * source heights of version 2, the final ones of version 1) and the
 * painted maps, within `BUNDLE_LIMITS`.
 */
export async function readBundle(source: ImageSource): Promise<DecodedBundle> {
  const zip = readZip(await bytesOf(source, "The bundle"));
  let budget = BUNDLE_LIMITS.totalBytes;
  // Paths are only keys into the zip.
  const read = async (path: string, limit = BUNDLE_LIMITS.entryBytes) => {
    const entry = zip.get(path);
    const what = `The bundle's ${JSON.stringify(path)}`;

    if (!entry) {
      throw malformed(`The bundle has no ${JSON.stringify(path)}, which it needs.`);
    }

    if (entry.size > limit) {
      throw malformed(`${what} is ${entry.size} bytes, over the limit of ${limit}.`);
    }

    if ((budget -= entry.size) < 0) {
      throw malformed(`The bundle's files inflate to over ${BUNDLE_LIMITS.totalBytes} bytes together.`);
    }

    const data = entry.method ? await inflate([entry.data], "deflate-raw", entry.size, what) : entry.data.slice();

    if (crc32(data) !== entry.crc) {
      throw malformed(`${what} fails its CRC check: the file is damaged.`);
    }

    return data;
  };
  const manifest = parseManifest(await read("manifest.json", BUNDLE_LIMITS.manifestBytes));
  const { terrain, files, version } = manifest as { terrain: Record<string, number>; files: Record<string, unknown>[]; version: number };
  const file = (kind: string) => files.find((entry) => entry.kind === kind);
  const { width, height, metresPerSample, seaLevelMetres = 0 } = terrain;
  const kind = version === 2 ? "sourceHeight" : "height";
  const heights = await read(String(file(kind)?.path ?? `${kind === "height" ? "" : "source-"}height.f32`));

  if (heights.length !== width * height * 4) {
    throw malformed(`The bundle's heights hold ${heights.length} bytes, but ${width} x ${height} float32 samples need ${width * height * 4}.`);
  }

  const kinds = ["paintedBiome", "waterMask", "treeMask", "grassMask"];
  const pngs = await Promise.all(kinds.map((kind) => file(kind) && read(file(kind)!.path as string)));
  let decoded = 0;

  // Every header is checked before the first decode.
  for (const [index, png] of pngs.entries()) {
    if (png) {
      decoded += decodedSize(png, `The bundle's ${kinds[index]} map`);

      if (decoded > BUNDLE_LIMITS.decodedBytes) {
        throw malformed(`The bundle's painted maps decode to over ${BUNDLE_LIMITS.decodedBytes} bytes together.`);
      }
    }
  }

  // One at a time, so at most one decoded image is held besides the bytes.
  const maps: (Grid | undefined)[] = [];

  for (const [index, png] of pngs.entries()) {
    const image = png && await decodePng(png);
    // Biome indices are kept as they are; the masks are grey.
    maps.push(image ? toBytes(image, kinds[index] === "paintedBiome"
      ? Float32Array.from(image.data, (value) => (value < 19 ? value : 255) / 255)
      : channelOf(image, "luminance", "")) : undefined);
  }

  const [biome, water, trees, grass] = maps;
  const options = Object(manifest.options) as Record<string, unknown>;
  const landform = Object(options.terrain).landform ?? options.landform ?? terrain.landform;
  const border = file("paintedBiome")?.borderSamples;
  return {
    version,
    heights: heights.buffer,
    raw: {
      width,
      height,
      sampleFormat: "float32",
      byteOrder: "little-endian",
      metresPerSample,
      heightScaleMetres: 1,
      seaLevelMetres,
      ...(typeof landform === "string" && { landform: landform as RawHeightmapOptions["landform"] })
    },
    options,
    water,
    biome: biome && { ...biome, borderSamples: Number.isInteger(border) ? (border as number) : 3 },
    trees,
    grass
  };
}
