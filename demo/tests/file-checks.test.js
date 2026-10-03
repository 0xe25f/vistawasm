import { describe, expect, it } from "vitest";
import { MAX_BUNDLE_BYTES, MAX_IMAGE_BYTES, MAX_TERRAIN_SIDE, checkFile, checkImage, imageSize } from "../src/file-checks.js";

function png(width, height) {
  const bytes = new Uint8Array(33);
  bytes.set([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, 0x49, 0x48, 0x44, 0x52]);
  const view = new DataView(bytes.buffer);
  view.setUint32(16, width);
  view.setUint32(20, height);
  return bytes;
}

function jpeg(width, height) {
  // SOI, an APP0 segment to skip, then a baseline frame header.
  const bytes = new Uint8Array([0xff, 0xd8, 0xff, 0xe0, 0, 6, 1, 2, 3, 4, 0xff, 0xc0, 0, 11, 8, 0, 0, 0, 0, 1, 1, 0x11, 0, 0, 0]);
  const view = new DataView(bytes.buffer);
  view.setUint16(15, height);
  view.setUint16(17, width);
  return bytes;
}

function webp(chunk, width, height) {
  const bytes = new Uint8Array(40);
  bytes.set([..."RIFF"].map((c) => c.charCodeAt(0)), 0);
  bytes.set([..."WEBP"].map((c) => c.charCodeAt(0)), 8);
  bytes.set([...chunk].map((c) => c.charCodeAt(0)), 12);
  const view = new DataView(bytes.buffer);

  if (chunk === "VP8X") {
    view.setUint16(24, (width - 1) & 0xffff, true);
    bytes[26] = (width - 1) >> 16;
    view.setUint16(27, (height - 1) & 0xffff, true);
    bytes[29] = (height - 1) >> 16;
  } else if (chunk === "VP8L") {
    bytes[20] = 0x2f;
    view.setUint32(21, (width - 1) | ((height - 1) << 14), true);
  } else {
    bytes.set([0x9d, 0x01, 0x2a], 23);
    view.setUint16(26, width, true);
    view.setUint16(28, height, true);
  }

  return bytes;
}

function file(bytes, name, type) {
  return new File([bytes], name, { type });
}

describe("imageSize", () => {
  it("reads PNG, JPEG and every kind of WebP header", () => {
    expect(imageSize(png(1024, 512))).toEqual([1024, 512]);
    expect(imageSize(jpeg(640, 480))).toEqual([640, 480]);
    expect(imageSize(webp("VP8 ", 300, 200))).toEqual([300, 200]);
    expect(imageSize(webp("VP8L", 4096, 2048))).toEqual([4096, 2048]);
    expect(imageSize(webp("VP8X", 20000, 3))).toEqual([20000, 3]);
  });

  it("finds no size in other bytes", () => {
    expect(imageSize(new TextEncoder().encode("not an image at all, just some text"))).toBeNull();
    expect(imageSize(new Uint8Array(0))).toBeNull();
  });
});

describe("checkFile", () => {
  it("accepts images and bundles of the right type and size", () => {
    const image = file(png(8, 8), "height.png", "image/png");
    expect(checkFile(image, "image", "a heightmap image")).toBe(image);
    const bundle = file(new Uint8Array(10), "painting.zip", "application/zip");
    expect(checkFile(bundle, "zip", "a bundle")).toBe(bundle);
  });

  it("refuses a missing file, the wrong type and an empty file, saying what to choose", () => {
    expect(() => checkFile(undefined, "image", "a heightmap image")).toThrow("Choose a heightmap image first.");
    expect(() => checkFile(file(new Uint8Array(4), "notes.txt", "text/plain"), "image", "a water mask")).toThrow(TypeError);
    expect(() => checkFile(file(new Uint8Array(4), "height.png", "image/png"), "zip", "a bundle")).toThrow("is not a .zip bundle");
    expect(() => checkFile(file(new Uint8Array(0), "height.png", "image/png"), "image", "a heightmap image")).toThrow(RangeError);
  });

  it("refuses files over the caps", () => {
    const big = (size, name, type) => ({ name, type, size });
    expect(() => checkFile(big(MAX_IMAGE_BYTES + 1, "huge.png", "image/png"), "image", "a heightmap image")).toThrow("at most 160.0 MB");
    expect(() => checkFile(big(MAX_BUNDLE_BYTES + 1, "huge.zip", "application/zip"), "zip", "a bundle")).toThrow("at most 512.0 MB");
  });
});

describe("checkImage", () => {
  it("accepts images from 2 to 8192 pixels a side", async () => {
    const image = file(png(8192, 2), "biome.png", "image/png");
    await expect(checkImage(image, "a biome map")).resolves.toBe(image);
  });

  it("holds heightmap images to the 2048-sample terrain limit", async () => {
    const image = file(png(2048, 2048), "height.png", "image/png");
    await expect(checkImage(image, "a heightmap image", MAX_TERRAIN_SIDE)).resolves.toBe(image);
    await expect(checkImage(file(png(4096, 2), "height.png", "image/png"), "a heightmap image", MAX_TERRAIN_SIDE)).rejects.toThrow(
      "from 2 to 2048 pixels a side (WebAssembly's limit on terrain size)"
    );
  });

  it("refuses images over 8192 or under 2 pixels a side before decoding them", async () => {
    await expect(checkImage(file(png(8193, 16), "wide.png", "image/png"), "a heightmap image")).rejects.toThrow("8193 x 16 pixels");
    await expect(checkImage(file(jpeg(30000, 30000), "photo.jpg", "image/jpeg"), "a texture image")).rejects.toThrow(RangeError);
    await expect(checkImage(file(webp("VP8X", 1, 64), "tiny.webp", "image/webp"), "a biome map")).rejects.toThrow("from 2 to 8192");
  });

  it("refuses an image whose header gives no size", async () => {
    await expect(checkImage(file(new Uint8Array(64), "broken.png", "image/png"), "a water mask")).rejects.toThrow("may be damaged");
  });
});
