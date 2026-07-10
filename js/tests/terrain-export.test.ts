import { describe, expect, it } from "vitest";
import {
  computeHeightmapPixels,
  exportTerrainObj,
  readHeightmapFloats
} from "../src/terrain-export";
import type { TerrainMetadata } from "../src/types";

function encodeFloats(values: number[]): Uint8Array {
  const buffer = new ArrayBuffer(values.length * 4);
  const view = new DataView(buffer);

  values.forEach((value, index) => {
    view.setFloat32(index * 4, value, true);
  });

  return new Uint8Array(buffer);
}

function makeMetadata(overrides: Partial<TerrainMetadata> = {}): TerrainMetadata {
  return {
    width: 2,
    height: 2,
    metresPerSample: 10,
    verticalScale: 1,
    seaLevelMetres: 0,
    minHeightMetres: 0,
    maxHeightMetres: 100,
    meanHeightMetres: 50,
    source: "fractal",
    generatorVersion: "test",
    warnings: [],
    ...overrides
  };
}

describe("readHeightmapFloats", () => {
  it("round-trips little-endian float32 values", () => {
    const bytes = encodeFloats([1.5, -2.25, 100, 0]);

    expect(Array.from(readHeightmapFloats(bytes))).toEqual([1.5, -2.25, 100, 0]);
  });
});

describe("computeHeightmapPixels", () => {
  it("maps the minimum height to black and the maximum to white in grayscale mode", () => {
    const metadata = makeMetadata();
    const bytes = encodeFloats([0, 100, 100, 0]);
    const pixels = computeHeightmapPixels(metadata, bytes, { colourMode: "grayscale" });

    expect(pixels.length).toBe(4 * 4);
    expect([pixels[0], pixels[1], pixels[2]]).toEqual([0, 0, 0]);
    expect([pixels[4], pixels[5], pixels[6]]).toEqual([255, 255, 255]);
  });

  it("throws when the byte length does not match width times height", () => {
    const metadata = makeMetadata();
    const bytes = encodeFloats([0, 100]);

    expect(() => computeHeightmapPixels(metadata, bytes)).toThrow(TypeError);
  });
});

describe("exportTerrainObj", () => {
  it("emits one vertex line per sample and two faces per quad", () => {
    const metadata = makeMetadata({ width: 3, height: 3, metresPerSample: 5 });
    const bytes = encodeFloats([0, 10, 20, 30, 40, 50, 60, 70, 80]);
    const obj = exportTerrainObj(metadata, bytes, { maxSamplesPerSide: 3 });
    const vertexLines = obj.split("\n").filter((line) => line.startsWith("v "));
    const faceLines = obj.split("\n").filter((line) => line.startsWith("f "));

    expect(vertexLines).toHaveLength(9);
    expect(faceLines).toHaveLength(8);
  });

  it("downsamples large terrain to stay within the requested vertex cap", () => {
    const width = 17;
    const height = 17;
    const metadata = makeMetadata({ width, height, metresPerSample: 2 });
    const bytes = encodeFloats(new Array(width * height).fill(0));
    const obj = exportTerrainObj(metadata, bytes, { maxSamplesPerSide: 5 });
    const vertexLines = obj.split("\n").filter((line) => line.startsWith("v "));

    expect(vertexLines.length).toBeLessThanOrEqual(25);
  });
});
