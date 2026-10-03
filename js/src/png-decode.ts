import { MAX_IMAGE_SIDE, crc32, isPng, inflate } from "./codec.js";
import { malformed } from "./errors.js";

/**
 * A decoded PNG. Samples are row-major, north row first, channels
 * interleaved, at the image's own bit depth: 1, 2 and 4-bit samples take
 * a byte each, 16-bit samples a `Uint16Array` entry. Palette images keep
 * their indices, with the palette alongside.
 */
export interface DecodedPng {
  width: number;
  height: number;
  /** 1 grey or palette index, 2 grey and alpha, 3 RGB, 4 RGBA. */
  channels: 1 | 2 | 3 | 4;
  bitDepth: 1 | 2 | 4 | 8 | 16;
  data: Uint8Array | Uint16Array;
  /** The `tEXt` chunks, keyword to text. */
  text: Record<string, string>;
  /** Palette images: each entry as RGBA bytes, alpha from `tRNS`. */
  palette?: Uint8Array;
  /** Grey or RGB images with a `tRNS` chunk: the sample values that are transparent. */
  transparent?: number[];
}

function paeth(a: number, b: number, c: number): number {
  const p = a + b - c;
  const [pa, pb, pc] = [Math.abs(p - a), Math.abs(p - b), Math.abs(p - c)];
  return pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
}

/** Undo one pass's filters in place, row by row: `1 + stride` bytes a row. */
function unfilter(raw: Uint8Array, at: number, rows: number, stride: number, step: number): void {
  for (let y = 0; y < rows; y += 1, at += stride + 1) {
    const filter = raw[at];
    const row = at + 1;
    const up = y ? row - stride - 1 : -1;

    if (filter > 4) {
      throw malformed(`PNG row ${y} has filter type ${filter}, but only 0 to 4 exist.`);
    }

    // One loop a filter, as this runs for every byte of the image. A
    // pass's first row has zeros above it.
    for (let x = 0; filter && x < stride; x += 1) {
      const left = x >= step ? raw[row + x - step] : 0;
      const above = up < 0 ? 0 : raw[up + x];
      raw[row + x] += filter === 1 ? left
        : filter === 2 ? above
          : filter === 3 ? (left + above) >> 1
            : paeth(left, above, up < 0 || x < step ? 0 : raw[up + x - step]);
    }
  }
}

/**
 * Decode a PNG: every colour type at every bit depth, all five filters
 * and Adam7 interlacing, with every chunk CRC checked. Images over 8192
 * pixels a side and unknown critical chunks are rejected. Malformed
 * input throws a `VistaWasmError` with code `INVALID_DEM` saying what is
 * wrong.
 */
export async function decodePng(bytes: Uint8Array): Promise<DecodedPng> {
  if (!(bytes instanceof Uint8Array)) {
    throw new TypeError("decodePng() expects the PNG file as a Uint8Array.");
  }

  if (!isPng(bytes)) {
    throw malformed("This is not a PNG file: it does not start with the PNG signature.");
  }

  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const chunks: Record<string, Uint8Array[]> = {};
  let at = 8;

  for (let type = ""; type !== "IEND"; at += view.getUint32(at) + 12) {
    const length = at + 12 <= bytes.length ? view.getUint32(at) : -1;
    type = String.fromCharCode(...bytes.subarray(at + 4, at + 8));

    if (length < 0 || length > bytes.length - at - 12) {
      throw malformed("The PNG ends before its IEND chunk, or a chunk claims more bytes than the file holds.");
    }

    if (crc32(bytes.subarray(at + 4, at + 8 + length)) !== view.getUint32(at + 8 + length)) {
      throw malformed(`PNG chunk CRC mismatch in ${JSON.stringify(type)}.`);
    }

    if (!/^[A-Za-z]{4}$/.test(type) || (type.charCodeAt(0) & 32) === 0 && !["IHDR", "PLTE", "IDAT", "IEND"].includes(type)) {
      throw malformed(`The PNG holds critical chunk ${JSON.stringify(type)}, which this decoder does not know.`);
    }

    (chunks[type] ??= []).push(bytes.subarray(at + 8, at + 8 + length));
  }

  const [header, palette, alpha] = [chunks.IHDR?.[0], chunks.PLTE?.[0], chunks.tRNS?.[0]];

  if (header?.length !== 13 || chunks.IHDR.length > 1 || (palette && (chunks.PLTE.length > 1 || palette.length % 3 || palette.length > 768))) {
    throw malformed("The PNG needs one 13-byte IHDR chunk, and at most one PLTE chunk of 1 to 256 RGB entries.");
  }

  const [width, height] = [view.getUint32(header.byteOffset - bytes.byteOffset), view.getUint32(header.byteOffset - bytes.byteOffset + 4)];
  const [bitDepth, colourType, compression, filtering, interlace] = header.subarray(8);
  const channels = ([1, 0, 3, 1, 2, 0, 4] as const)[colourType] || 0;
  const valid = [[1, 2, 4, 8, 16], 0, [8, 16], [1, 2, 4, 8], [8, 16], 0, [8, 16]][colourType];

  if (width < 1 || height < 1 || width > MAX_IMAGE_SIDE || height > MAX_IMAGE_SIDE) {
    throw malformed(`The PNG is ${width} x ${height}, but images must be from 1 to ${MAX_IMAGE_SIDE} pixels a side.`);
  }

  if (!channels || !(valid as number[]).includes(bitDepth) || compression || filtering || interlace > 1) {
    throw malformed(`The PNG's colour type ${colourType} at ${bitDepth} bits, or its compression, filter or interlace method, is not one PNG defines.`);
  }

  if (colourType === 3 && !(palette && palette.length / 3 <= 2 ** bitDepth)) {
    throw malformed("The palette PNG has no PLTE chunk, or more entries than its bit depth can index.");
  }

  if (!chunks.IDAT) {
    throw malformed("The PNG has no IDAT chunk, so it holds no pixels.");
  }

  // At most 8192 x 8192 x 4 two-byte samples: 512 MiB, checked before
  // any allocation, in plain numbers, which cannot overflow here.
  const samples = width * height * channels;
  const bits = channels * bitDepth;
  const step = Math.max(1, bits >> 3);
  // Adam7 passes: first column and row, and the steps between them.
  const passes = (interlace ? [0x0088, 0x4088, 0x0448, 0x2044, 0x0224, 0x1022, 0x0112] : [0x0011]).map((code) => {
    const [x0, y0, dx, dy] = [code >> 12, (code >> 8) & 15, (code >> 4) & 15, code & 15];
    const columns = Math.max(0, Math.ceil((width - x0) / dx));
    const rows = columns && Math.max(0, Math.ceil((height - y0) / dy));
    return { x0, y0, dx, dy, columns, rows, stride: Math.ceil((columns * bits) / 8) };
  });
  const raw = await inflate(chunks.IDAT, "deflate", passes.reduce((sum, pass) => sum + pass.rows * (pass.stride + 1), 0), "The PNG's IDAT data");
  const data = bitDepth === 16 ? new Uint16Array(samples) : new Uint8Array(samples);
  const mask = (1 << bitDepth) - 1;
  at = 0;

  for (const { x0, y0, dx, dy, columns, rows, stride } of passes) {
    unfilter(raw, at, rows, stride, step);

    for (let y = 0; y < rows; y += 1) {
      const row = at + y * (stride + 1) + 1;

      for (let x = 0, out = ((y0 + y * dy) * width + x0) * channels; x < columns; x += 1, out += (dx - 1) * channels) {
        for (let c = 0; c < channels; c += 1, out += 1) {
          const index = x * channels + c;
          data[out] = bitDepth === 16
            ? (raw[row + index * 2] << 8) | raw[row + index * 2 + 1]
            : (raw[row + ((index * bitDepth) >> 3)] >> (8 - bitDepth - ((index * bitDepth) & 7))) & mask;
        }
      }
    }

    at += rows * (stride + 1);
  }

  const latin1 = new TextDecoder("latin1");
  const text = (chunks.tEXt ?? []).map((chunk) => latin1.decode(chunk).split("\0")).filter((pair) => pair.length === 2 && pair[0]);
  const result: DecodedPng = { width, height, channels, bitDepth: bitDepth as DecodedPng["bitDepth"], data, text: Object.fromEntries(text) };

  if (palette) {
    result.palette = Uint8Array.from({ length: (palette.length / 3) * 4 }, (_, index) =>
      index % 4 < 3 ? palette[(index >> 2) * 3 + (index % 4)] : colourType === 3 ? alpha?.[index >> 2] ?? 255 : 255);
  }

  if (alpha && colourType !== 3 && alpha.length === channels * 2) {
    result.transparent = Array.from({ length: channels }, (_, c) => (alpha[c * 2] << 8) | alpha[c * 2 + 1]);
  }

  return result;
}
