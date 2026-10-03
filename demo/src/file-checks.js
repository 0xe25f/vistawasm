// Checks on the files people choose, made before anything decodes them, so
// a wrong or oversized file fails at once with a clear message instead of
// exhausting memory in a decoder.
//
// The caps follow the library's own limits:
//
// - Heightmap images and painted maps are at most 2048 pixels a side
//   (`MAX_TERRAIN_SIDE`): they become, or are resampled to, the terrain,
//   and 2048 is set by WebAssembly's 4 GiB of memory.
//   Texture images may be up to 8192, the largest `decodePng()` reads.
//   An image's header is read for its size, so a huge JPEG or WebP is
//   refused before the browser decodes it.
// - Image files are at most 160 MiB: a 16-bit grey PNG 8192 pixels a side
//   holds 128 MiB of samples, and even stored uncompressed stays below it.
// - Bundles are at most 512 MiB. `loadBundle()` inflates at most 1 GiB in
//   all, and a painting's files compress to well under half that.

import { MAX_TERRAIN_SIDE, imageSize } from "@vista-wasm/vista-wasm";

// The header reader is the library's, which `loadHeightmapImage()` and
// the mask helpers use too.
export { imageSize };

export const MAX_IMAGE_SIDE = 8192;
export { MAX_TERRAIN_SIDE };
export const MAX_IMAGE_BYTES = 160 * 2 ** 20;
export const MAX_BUNDLE_BYTES = 512 * 2 ** 20;

const IMAGE_TYPES = ["image/png", "image/jpeg", "image/webp"];
// Some systems give zip files no type, or an older one.
const ZIP_TYPES = ["application/zip", "application/x-zip-compressed", "application/x-zip", ""];

// A JPEG's size sits in its frame header, after any metadata segments,
// which together stay well within this.
const HEADER_BYTES = 2 ** 20;

function megabytes(bytes) {
  return `${(bytes / 2 ** 20).toFixed(1)} MB`;
}

/**
 * Check a chosen file's type and size: `kind` is "image" (PNG, JPEG or
 * WebP) or "zip". `label` names what the file is for, as in "a heightmap
 * image". Returns the file, or throws a `TypeError` or `RangeError` that
 * says what to choose instead.
 */
export function checkFile(file, kind, label) {
  if (!file) {
    throw new TypeError(`Choose ${label} first.`);
  }

  const image = kind === "image";
  const limit = image ? MAX_IMAGE_BYTES : MAX_BUNDLE_BYTES;
  const typed = image
    ? IMAGE_TYPES.includes(file.type) || (!file.type && /\.(png|jpe?g|webp)$/i.test(file.name))
    : ZIP_TYPES.includes(file.type) && /\.zip$/i.test(file.name);

  if (!typed) {
    throw new TypeError(`${file.name} is not ${image ? "a PNG, JPEG or WebP image" : "a .zip bundle"}. Choose ${label} of that type.`);
  }

  if (file.size === 0 || file.size > limit) {
    throw new RangeError(`${file.name} is ${megabytes(file.size)}; ${label} must hold something and be at most ${megabytes(limit)}.`);
  }

  return file;
}

/**
 * `checkFile()` for an image, then its header: it must give a size from 2
 * to `maxSide` pixels a side (`MAX_IMAGE_SIDE` by default). Returns the
 * file.
 */
export async function checkImage(file, label, maxSide = MAX_IMAGE_SIDE) {
  checkFile(file, "image", label);
  const size = imageSize(new Uint8Array(await file.slice(0, HEADER_BYTES).arrayBuffer()));

  if (!size) {
    throw new TypeError(`${file.name} has no PNG, JPEG or WebP header giving its size, so it may be damaged. Choose ${label} saved as a PNG.`);
  }

  if (size.some((side) => side < 2 || side > maxSide)) {
    const why = maxSide === MAX_TERRAIN_SIDE ? " (WebAssembly's limit on terrain size)" : "";
    throw new RangeError(`${file.name} is ${size[0]} x ${size[1]} pixels; ${label} must be from 2 to ${maxSide} pixels a side${why}. Scale it down first.`);
  }

  return file;
}
