import { describe, expect, it } from "vitest";
import { createDocument } from "../src/paint/document.js";
import {
  CATEGORY_THRESHOLD,
  RAISE_METRES,
  TOOL_LAYERS,
  applyDab,
  createStroke,
  dabParts,
  dabRect,
  dabsAlong,
  falloffWeight,
  fbm,
  NOISE_METRES,
  noiseGrid
} from "../src/paint/brushes.js";

function dab(doc, tool, options = {}) {
  const { x = 128, y = 128 } = options;
  const stroke = options.stroke ?? createStroke(doc, tool, x, y, 7);
  const rect = applyDab(doc, stroke, { x, y, radius: 5, strength: 1, falloff: "smooth", biome: 3, ...options });
  return { stroke, rect };
}

function variance(values) {
  const mean = values.reduce((sum, value) => sum + value, 0) / values.length;
  return values.reduce((sum, value) => sum + (value - mean) ** 2, 0) / values.length;
}

function patch(doc, x0, y0, side) {
  const values = [];

  for (let y = y0; y < y0 + side; y += 1) {
    for (let x = x0; x < x0 + side; x += 1) {
      values.push(doc.layers.height[y * doc.size + x]);
    }
  }

  return values;
}

describe("falloff", () => {
  it("is 1 at the centre and 0 at the radius for every shape but constant", () => {
    for (const falloff of ["smooth", "linear"]) {
      expect(falloffWeight(falloff, 0)).toBe(1);
      expect(falloffWeight(falloff, 1)).toBe(0);
    }

    expect(falloffWeight("constant", 0.99)).toBe(1);
    expect(falloffWeight("constant", 1)).toBe(0);
    expect(falloffWeight("smooth", 0.5)).toBeCloseTo(0.5);
  });
});

describe("raise", () => {
  it("raises the centre by 2 m for every 100 m of the display range, falling to 0 at the radius", () => {
    const doc = createDocument({ size: 256, heightMetres: 100 });
    doc.displayRange = [0, 1000];
    const { size } = doc;
    // Size 10: a radius of 5 samples.
    dab(doc, "raise", { radius: 5 });
    const height = (x, y) => doc.layers.height[y * size + x];

    expect(height(128, 128) - 100).toBeCloseTo(RAISE_METRES * 10, 4);
    expect(height(133, 128)).toBe(100);
    expect(height(128, 123)).toBe(100);
    // Half way out, the cosine falloff gives half.
    expect(height(130, 128) - 100).toBeGreaterThan(height(131, 128) - 100);
    expect(height(131, 128)).toBeGreaterThan(100);
  });

  it("lowers by the same amount", () => {
    const doc = createDocument({ size: 256, heightMetres: 100 });
    doc.displayRange = [0, 500];
    dab(doc, "lower", { radius: 5, strength: 0.5 });

    expect(doc.layers.height[128 * 256 + 128]).toBeCloseTo(100 - RAISE_METRES * 5 * 0.5, 4);
  });
});

describe("smooth", () => {
  it("reduces the variance of a noisy patch", () => {
    const doc = createDocument({ size: 256 });
    let seed = 1;

    for (let index = 0; index < doc.layers.height.length; index += 1) {
      seed = (seed * 16807) % 2147483647;
      doc.layers.height[index] = (seed / 2147483647) * 50;
    }

    const before = variance(patch(doc, 124, 124, 9));
    dab(doc, "smooth", { radius: 8 });
    const after = variance(patch(doc, 124, 124, 9));

    expect(after).toBeLessThan(before * 0.5);
  });
});

describe("flatten", () => {
  it("converges to the height at the stroke's start", () => {
    const doc = createDocument({ size: 256 });

    for (let y = 0; y < 256; y += 1) {
      for (let x = 0; x < 256; x += 1) {
        doc.layers.height[y * 256 + x] = x * 2;
      }
    }

    const stroke = createStroke(doc, "flatten", 128, 128);

    for (let step = 0; step < 40; step += 1) {
      dab(doc, "flatten", { stroke, radius: 10, strength: 0.5, falloff: "constant" });
    }

    expect(stroke.startHeight).toBe(256);

    for (const x of [120, 125, 131, 136]) {
      expect(doc.layers.height[128 * 256 + x]).toBeCloseTo(256, 3);
    }
  });
});

describe("noise", () => {
  it("adds the fbm interpolated from its grid, times strength, 10 m and the weight", () => {
    const doc = createDocument({ size: 512 });
    const before = doc.layers.height.slice();
    const [x, y, radius, strength] = [200.3, 190.7, 64, 0.6];
    const { rect } = dab(doc, "noise", { x, y, radius, strength });
    const grid = noiseGrid(rect, radius);
    // The grid interpolated bilinearly, as the brush did before it was sped up.
    const interpolated = (sx, sy) => {
      const fx = sx / grid.step - grid.gx0;
      const fy = sy / grid.step - grid.gy0;
      const [column, row] = [Math.floor(fx), Math.floor(fy)];
      const at = row * grid.columns + column;
      const v = grid.values;
      const top = v[at] + (v[at + 1] - v[at]) * (fx - column);
      const bottom = v[at + grid.columns] + (v[at + grid.columns + 1] - v[at + grid.columns]) * (fx - column);
      return top + (bottom - top) * (fy - row);
    };
    let largest = 0;
    let moved = 0;

    for (let sy = rect.y0; sy < rect.y1; sy += 1) {
      for (let sx = rect.x0; sx < rect.x1; sx += 1) {
        const at = sy * doc.size + sx;
        const weight = falloffWeight("smooth", Math.hypot(sx - x, sy - y) / radius);
        const expected = before[at] + NOISE_METRES * strength * weight * interpolated(sx, sy);
        largest = Math.max(largest, Math.abs(doc.layers.height[at] - expected));
        moved = Math.max(moved, Math.abs(doc.layers.height[at] - before[at]));
      }
    }

    // The weights come from a table of squared distances, so allow for its steps.
    expect(largest).toBeLessThan(0.01);
    expect(moved).toBeGreaterThan(1);
  });

  it("samples its grid from the fbm on the map's own lattice", () => {
    const grid = noiseGrid({ x0: 40, y0: 24, x1: 169, y1: 153 }, 64);
    expect(grid.step).toBe(5);
    expect(grid.values[grid.columns + 2]).toBeCloseTo(fbm(((grid.gx0 + 2) * 5) / 64, ((grid.gy0 + 1) * 5) / 64), 6);
  });

  it("reuses the last dab's grid points exactly where the next dab overlaps it", () => {
    noiseGrid({ x0: 40, y0: 24, x1: 169, y1: 153 }, 64);
    noiseGrid({ x0: 10, y0: 90, x1: 60, y1: 140 }, 32);
    const grid = noiseGrid({ x0: 56, y0: 30, x1: 185, y1: 159 }, 32);
    const { rows } = grid;

    for (let row = 0; row < rows; row += 1) {
      for (let column = 0; column < grid.columns; column += 1) {
        const expected = fbm(((grid.gx0 + column) * grid.step) / 32, ((grid.gy0 + row) * grid.step) / 32);
        expect(grid.values[row * grid.columns + column]).toBe(Math.fround(expected));
      }
    }
  });
});

describe("erode", () => {
  it("lowers a cone's peak and conserves its mass within 2 %", () => {
    const doc = createDocument({ size: 256 });
    doc.metresPerSample = 10;

    // A cone 400 m high and 40 samples in radius: 45 degrees, steeper than the talus.
    for (let y = 0; y < 256; y += 1) {
      for (let x = 0; x < 256; x += 1) {
        doc.layers.height[y * 256 + x] = Math.max(0, 400 - Math.hypot(x - 128, y - 128) * 10);
      }
    }

    const mass = () => doc.layers.height.reduce((sum, value) => sum + value, 0);
    const before = mass();
    const peak = doc.layers.height[128 * 256 + 128];
    const stroke = createStroke(doc, "erode", 128, 128, 3);

    for (let step = 0; step < 10; step += 1) {
      dab(doc, "erode", { stroke, radius: 48, strength: 1 });
    }

    expect(doc.layers.height[128 * 256 + 128]).toBeLessThan(peak - 1);
    expect(Math.abs(mass() - before) / before).toBeLessThan(0.02);
  });

  it("gives the same result applied in parts as whole", () => {
    const cone = () => {
      const doc = createDocument({ size: 256 });
      doc.layers.height.forEach((_, index) => {
        doc.layers.height[index] = Math.max(0, 300 - Math.hypot((index % 256) - 128, Math.floor(index / 256) - 128) * 9);
      });
      return doc;
    };
    const whole = cone();
    const split = cone();
    const wholeStroke = createStroke(whole, "erode", 128, 128, 4);
    const splitStroke = createStroke(split, "erode", 128, 128, 4);

    for (const x of [120, 126, 132]) {
      const options = { x, y: 128, radius: 30, strength: 1, falloff: "smooth" };
      applyDab(whole, wholeStroke, options);

      for (let part = 0; part < dabParts("erode"); part += 1) {
        applyDab(split, splitStroke, options, part);
      }
    }

    expect(dabParts("erode")).toBe(4);
    expect(dabParts("raise")).toBe(1);
    expect(split.layers.height).toEqual(whole.layers.height);
  });

  it("repeats exactly for the same seed", () => {
    const run = () => {
      const doc = createDocument({ size: 256 });
      doc.layers.height.forEach((_, index) => {
        doc.layers.height[index] = Math.max(0, 300 - Math.hypot((index % 256) - 128, Math.floor(index / 256) - 128) * 9);
      });
      dab(doc, "erode", { radius: 30, stroke: createStroke(doc, "erode", 128, 128, 11) });
      return doc.layers.height;
    };

    expect(run()).toEqual(run());
  });
});

describe("categories", () => {
  it("paints a biome where the falloff is at least the threshold", () => {
    const doc = createDocument({ size: 256 });
    dab(doc, "biome", { radius: 10, biome: 3, falloff: "linear" });

    for (let x = 118; x <= 138; x += 1) {
      const weight = falloffWeight("linear", Math.abs(x - 128) / 10);
      expect(doc.layers.biome[128 * 256 + x]).toBe(weight >= CATEGORY_THRESHOLD ? 3 : 255);
    }
  });

  it("paints lakes at 200, rivers by strength and erases water", () => {
    const doc = createDocument({ size: 256 });
    const at = 128 * 256 + 128;
    dab(doc, "lake");
    expect(doc.layers.water[at]).toBe(200);
    dab(doc, "river", { strength: 0.5 });
    expect(doc.layers.water[at]).toBe(64);
    dab(doc, "river", { strength: 1 });
    expect(doc.layers.water[at]).toBe(127);
    dab(doc, "eraseWater");
    expect(doc.layers.water[at]).toBe(0);
  });

  it("moves density towards more, less and unchanged", () => {
    const doc = createDocument({ size: 256 });
    const at = 128 * 256 + 128;
    dab(doc, "treesMore", { strength: 1 });
    expect(doc.layers.trees[at]).toBe(255);
    dab(doc, "treesLess", { strength: 0.5 });
    expect(doc.layers.trees[at]).toBe(127);
    dab(doc, "treesReset", { strength: 1 });
    expect(doc.layers.trees[at]).toBe(128);
    dab(doc, "grassLess", { strength: 0.01 });
    expect(doc.layers.grass[at]).toBe(126);
  });
});

describe("dirty rectangles", () => {
  it("each brush writes only inside the rectangle it returns", () => {
    for (const tool of Object.keys(TOOL_LAYERS)) {
      const doc = createDocument({ size: 256 });
      doc.layers.height.forEach((_, index) => {
        doc.layers.height[index] = Math.max(0, 300 - Math.hypot((index % 256) - 100, Math.floor(index / 256) - 90) * 12);
      });
      const layer = doc.layers[TOOL_LAYERS[tool]];
      // Values every brush changes: a river, and densities off 128.
      doc.layers.water.fill(50);
      doc.layers.trees.fill(90);
      doc.layers.grass.fill(90);
      const before = layer.slice();
      const { rect } = dab(doc, tool, { x: 100.4, y: 90.7, radius: 17 });
      let changedInside = 0;

      expect(rect).toEqual(dabRect(256, 100.4, 90.7, 17));

      for (let index = 0; index < layer.length; index += 1) {
        const x = index % 256;
        const y = Math.floor(index / 256);
        const inside = x >= rect.x0 && x < rect.x1 && y >= rect.y0 && y < rect.y1;

        if (layer[index] !== before[index]) {
          expect(inside, `${tool} changed ${x}, ${y} outside its rectangle`).toBe(true);
          changedInside += 1;
        }
      }

      expect(changedInside, `${tool} changed nothing`).toBeGreaterThan(0);
    }
  });

  it("clips to the document", () => {
    expect(dabRect(256, 1, 254, 5)).toEqual({ x0: 0, y0: 249, x1: 7, y1: 256 });
    expect(dabRect(256, -20, 10, 5)).toBeNull();
  });
});

describe("stroke spacing", () => {
  it("places a dab every quarter radius, carrying the remainder between points", () => {
    const stroke = { last: null, travelled: 0 };

    expect(dabsAlong(stroke, 0, 0, 2)).toEqual([[0, 0]]);
    expect(dabsAlong(stroke, 3, 0, 2)).toEqual([[2, 0]]);
    expect(dabsAlong(stroke, 7, 0, 2)).toEqual([[4, 0], [6, 0]]);
  });
});
