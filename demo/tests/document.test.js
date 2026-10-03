import { describe, expect, it } from "vitest";
import {
  NOT_PAINTED,
  UNCHANGED,
  createDocument,
  defaultDisplayRange,
  documentFromEngine,
  heightmapOptions,
  paintedInputs,
  resampleBilinear
} from "../src/paint/document.js";

/**
 * A stand-in engine with a 101 x 101 terrain 3 km across, whose source
 * height rises 1 m a sample eastwards and 2 m a sample southwards.
 */
function mockEngine(painted = {}) {
  const side = 101;
  const heights = new Float32Array(side * side).map((_, index) => (index % side) + 2 * Math.floor(index / side));
  const biomes = new Uint8Array(side * side).map((_, index) => ((index % side) < 50 ? 3 : 8));
  const map = (kind, data) => ({
    kind,
    width: side,
    height: side,
    channels: 1,
    type: data instanceof Float32Array ? "float32" : "uint8",
    data,
    encoding: { metresPerPixel: [30, 30], seaLevelMetres: 12 }
  });
  return {
    exportMap: (kind) => (kind === "sourceHeight" ? map(kind, heights) : map(kind, biomes)),
    getPaintedMaps: () => painted
  };
}

describe("createDocument", () => {
  it("starts blank: flat, nothing painted and densities unchanged", () => {
    const doc = createDocument({ size: 512, widthKm: 6, heightMetres: 120 });

    expect(doc.layers.height.every((value) => value === 120)).toBe(true);
    expect(doc.layers.biome.every((value) => value === NOT_PAINTED)).toBe(true);
    expect(doc.layers.water.every((value) => value === 0)).toBe(true);
    expect(doc.layers.trees.every((value) => value === UNCHANGED)).toBe(true);
    expect(doc.metresPerSample).toBeCloseTo(6000 / 511);
    expect(paintedInputs(doc)).toEqual({ biome: null, water: null, trees: null, grass: null });
  });

  it("rejects sizes and widths it cannot load", () => {
    expect(() => createDocument({ size: 300 })).toThrow(/256, 512, 1024/);
    expect(() => createDocument({ widthKm: 0 })).toThrow(/width/);
  });

  it("gives the display at least 1000 m of relief", () => {
    expect(defaultDisplayRange(new Float32Array([20, 30]), 0)).toEqual([-50, 950]);
    expect(defaultDisplayRange(new Float32Array([0, 2400]), 0)).toEqual([-50, 2500]);
  });
});

describe("documentFromEngine", () => {
  it("resamples the current map's source height to N", () => {
    const doc = documentFromEngine(mockEngine(), { size: 256 });
    const { height } = doc.layers;
    // Sample (x, y) of 256 lands at (x, y) x 100 / 255 of the source.
    const expected = (x, y) => (x * 100) / 255 + (2 * y * 100) / 255;

    expect(doc.size).toBe(256);
    expect(doc.widthKm).toBe(3);
    expect(doc.seaLevelMetres).toBe(12);
    expect(height[0]).toBe(0);
    expect(height[255]).toBeCloseTo(100, 4);
    expect(height[255 * 256 + 255]).toBeCloseTo(300, 4);

    for (const [x, y] of [[17, 3], [128, 200], [251, 90]]) {
      expect(height[y * 256 + x]).toBeCloseTo(expected(x, y), 3);
    }

    expect(doc.layers.biome.every((value) => value === NOT_PAINTED)).toBe(true);
  });

  it("copies generated biomes on request, and painted masks when set", () => {
    const water = { width: 2, height: 2, data: new Uint8Array([0, 200, 30, 0]) };
    const trees = { width: 2, height: 2, data: new Uint8Array([0, 255, 255, 0]) };
    const doc = documentFromEngine(mockEngine({ water, trees }), { size: 256, copyBiomes: true });

    expect(doc.layers.biome[0]).toBe(3);
    expect(doc.layers.biome[255]).toBe(8);
    expect(doc.layers.water[0]).toBe(0);
    expect(doc.layers.water[255]).toBe(200);
    expect(doc.layers.water[255 * 256]).toBe(30);
    expect(doc.layers.trees[0]).toBe(0);
    expect(doc.layers.trees[255]).toBe(255);
    // Near the middle, the four corners blend to about half.
    expect(Math.abs(doc.layers.trees[128 * 256 + 128] - 128)).toBeLessThanOrEqual(1);
    expect(doc.layers.grass.every((value) => value === UNCHANGED)).toBe(true);
  });
});

describe("resampleBilinear", () => {
  it("keeps a grid of the same size", () => {
    const grid = new Float32Array([1, 2, 3, 4]);
    expect(resampleBilinear(grid, 2, 2, 2)).toEqual(grid);
  });
});

describe("paintedInputs and heightmapOptions", () => {
  it("pass painted layers as copies and describe the height layer", () => {
    const doc = createDocument({ size: 256, widthKm: 5.1, seaLevelMetres: 4 });
    doc.layers.water[10] = 200;
    const inputs = paintedInputs(doc);
    doc.layers.water[10] = 0;

    expect(inputs.water.data[10]).toBe(200);
    expect(inputs.biome).toBeNull();
    expect(heightmapOptions(doc)).toEqual({
      width: 256,
      height: 256,
      sampleFormat: "float32",
      byteOrder: "little-endian",
      metresPerSample: 20,
      heightScaleMetres: 1,
      seaLevelMetres: 4
    });
  });
});
