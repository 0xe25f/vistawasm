import { VistaWasmError, malformed } from "./errors.js";

// The pieces the PNG and zip writers and readers share.

type Bytes = Uint8Array<ArrayBuffer>;

let crcTable: Uint32Array | undefined;

/** CRC-32 (as PNG and zip use it) of `bytes`, continuing from `crc`. */
export function crc32(bytes: Uint8Array, crc = 0): number {
  crcTable ??= Uint32Array.from({ length: 256 }, (_, n) => {
    for (let k = 0; k < 8; k += 1) {
      n = n & 1 ? 0xedb88320 ^ (n >>> 1) : n >>> 1;
    }

    return n;
  });
  let c = ~crc;

  for (let index = 0; index < bytes.length; index += 1) {
    c = crcTable[(c ^ bytes[index]) & 255] ^ (c >>> 8);
  }

  return ~c >>> 0;
}

/** Compress chunks with the platform's `CompressionStream`. */
export async function compress(format: CompressionFormat, chunks: Iterable<Bytes>): Promise<Bytes[]> {
  const stream = new CompressionStream(format);
  const writer = stream.writable.getWriter();
  const reader = stream.readable.getReader();
  const output: Bytes[] = [];
  const write = async () => {
    for (const chunk of chunks) {
      await writer.write(chunk);
    }

    await writer.close();
  };
  const read = async () => {
    for (let next = await reader.read(); !next.done; next = await reader.read()) {
      output.push(next.value as Bytes);
    }
  };
  await Promise.all([write(), read()]);
  return output;
}

/** The eight bytes every PNG file starts with. */
export const PNG_SIGNATURE = [137, 80, 78, 71, 13, 10, 26, 10];

/**
 * The longest side, in pixels, of an image the decoders accept. At four
 * 16-bit channels an image this size decodes to 512 MiB.
 */
export const MAX_IMAGE_SIDE = 8192;

/**
 * Most samples per side of a terrain, as the engine allows: 2048. It is
 * set by WebAssembly's memory: a WASM module can address at most 4 GiB,
 * which never shrinks, and a 4096 x 4096 terrain peaks at up to 3.7 GiB,
 * too close to the limit to rebuild and export it safely.
 * Painted maps and exports are held to it too: they are resampled to or
 * from the terrain, so detail past it would be lost, and they are held
 * in WASM memory beside it.
 */
export const MAX_TERRAIN_SIDE = 2048;

/** Why terrains, painted maps and exports stop at `MAX_TERRAIN_SIDE`, for errors. */
export const TERRAIN_SIZE_REASON = "WebAssembly's limit on terrain size";

/** Whether `bytes` start with the PNG signature. */
export function isPng(bytes: Uint8Array): boolean {
  return PNG_SIGNATURE.every((value, index) => bytes[index] === value);
}

/**
 * Inflate `parts`, one compressed stream split over several arrays, with
 * the platform's `DecompressionStream`, expecting exactly `size` bytes.
 * Reading stops as soon as the output passes `size`, so a small file
 * cannot expand into a huge one, and the output is allocated only once
 * the data has proved to be as long as its header says, so a header alone
 * cannot claim any memory.
 */
export async function inflate(parts: Uint8Array[], format: CompressionFormat, size: number, what: string): Promise<Bytes> {
  const stream = new DecompressionStream(format);
  const writer = stream.writable.getWriter();
  const reader = stream.readable.getReader();
  const pieces: Uint8Array[] = [];
  let at = 0;
  // Errors in the data surface through the reader.
  (async () => {
    for (const part of parts) {
      await writer.write(part as Bytes);
    }

    await writer.close();
  })().catch(() => undefined);

  try {
    for (let next = await reader.read(); !next.done; next = await reader.read()) {
      if ((at += next.value.length) > size) {
        throw malformed(`${what} inflates to more than the ${size} bytes its header allows.`);
      }

      pieces.push(next.value);
    }
  } catch (error) {
    reader.cancel().catch(() => undefined);
    throw error instanceof VistaWasmError ? error : malformed(`${what} is not valid compressed data: ${(error as Error).message}`);
  }

  if (at !== size) {
    throw malformed(`${what} inflates to ${at} bytes, but its header needs ${size}.`);
  }

  const out = new Uint8Array(size);
  at = 0;

  for (const piece of pieces) {
    out.set(piece, at);
    at += piece.length;
  }

  return out;
}
