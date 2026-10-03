// Brushes: pure functions that apply one dab to one layer of a paint
// document and return the rectangle they may have changed. Nothing
// outside that rectangle is read for writing or written, so history can
// copy exactly the tiles a dab touches before it runs.

export const TOOL_LAYERS = {
  raise: "height",
  lower: "height",
  smooth: "height",
  flatten: "height",
  noise: "height",
  erode: "height",
  biome: "biome",
  lake: "water",
  river: "water",
  eraseWater: "water",
  treesMore: "trees",
  treesLess: "trees",
  treesReset: "trees",
  grassMore: "grass",
  grassLess: "grass",
  grassReset: "grass"
};

/** Raise and lower move this many metres a dab at full strength, for every 100 m of the display range. */
export const RAISE_METRES = 2;

export const NOISE_METRES = 10;

/** Categories are painted where the falloff is at least this. */
export const CATEGORY_THRESHOLD = 0.5;

export const LAKE = 200;

/** The steepest slope the erode brush leaves standing, in degrees. */
export const TALUS_DEGREES = 35;

export const THERMAL_ITERATIONS = 3;
export const DROPLETS_PER_DAB = 30;

/**
 * The parts a dab of `tool` can be applied in, one call each, so a slow
 * device can spread a dear dab over frames: erode's thermal passes and
 * its droplets. Applied in order, the parts give exactly a whole dab.
 */
export function dabParts(tool) {
  return tool === "erode" ? THERMAL_ITERATIONS + 1 : 1;
}

const DENSITY_TARGETS = { More: 255, Less: 0, Reset: 128 };

export function toolLayer(tool) {
  const layer = TOOL_LAYERS[tool];

  if (!layer) {
    throw new RangeError(`There is no ${String(tool)} brush. Choose one of ${Object.keys(TOOL_LAYERS).join(", ")}.`);
  }

  return layer;
}

/** The brush weight at `t`, the distance from the centre over the radius. */
export function falloffWeight(falloff, t) {
  if (t >= 1) {
    return 0;
  }

  if (falloff === "linear") {
    return 1 - t;
  }

  if (falloff === "constant") {
    return 1;
  }

  return 0.5 + 0.5 * Math.cos(Math.PI * t);
}

export function brushRadius(size) {
  // Sizes run from 1 to 256 samples across.
  return Math.min(Math.max(size, 1), 256) / 2;
}

/** The distance between dabs along a stroke: a quarter of the radius. */
export function dabSpacing(radius) {
  return Math.max(0.5, radius * 0.25);
}

/**
 * The samples a dab at `(x, y)` with `radius` can reach, clipped to the
 * document: `x0 <= x < x1`, `y0 <= y < y1`, or `null` if none.
 */
export function dabRect(size, x, y, radius) {
  const x0 = Math.max(0, Math.ceil(x - radius));
  const y0 = Math.max(0, Math.ceil(y - radius));
  const x1 = Math.min(size, Math.floor(x + radius) + 1);
  const y1 = Math.min(size, Math.floor(y + radius) + 1);
  return x0 < x1 && y0 < y1 ? { x0, y0, x1, y1 } : null;
}

export function unionRect(a, b) {
  if (!a || !b) {
    return a ?? b ?? null;
  }

  return {
    x0: Math.min(a.x0, b.x0),
    y0: Math.min(a.y0, b.y0),
    x1: Math.max(a.x1, b.x1),
    y1: Math.max(a.y1, b.y1)
  };
}

/**
 * A new stroke. `seed` makes the erode brush's droplets repeat exactly;
 * `startHeight` is the height the flatten brush levels to.
 */
export function createStroke(doc, tool, x, y, seed = 1) {
  toolLayer(tool);
  return { tool, seed, dabs: 0, last: null, travelled: 0, startHeight: heightAt(doc, x, y) };
}

/**
 * The dab centres from the stroke's last point to `(x, y)`, one every
 * `spacing` samples, carrying the distance left over to the next call.
 * The first point of a stroke is always a dab.
 */
export function dabsAlong(stroke, x, y, spacing) {
  if (!stroke.last) {
    stroke.last = [x, y];
    return [[x, y]];
  }

  const [lastX, lastY] = stroke.last;
  const length = Math.hypot(x - lastX, y - lastY);
  const dabs = [];
  let at = spacing - stroke.travelled;

  while (at <= length) {
    const t = at / length;
    dabs.push([lastX + (x - lastX) * t, lastY + (y - lastY) * t]);
    at += spacing;
  }

  stroke.travelled = length - (at - spacing);
  stroke.last = [x, y];
  return dabs;
}

export function heightAt(doc, x, y) {
  const { size, layers } = doc;
  const cx = Math.min(Math.max(x, 0), size - 1);
  const cy = Math.min(Math.max(y, 0), size - 1);
  const x0 = Math.min(Math.floor(cx), size - 2);
  const y0 = Math.min(Math.floor(cy), size - 2);
  const tx = cx - x0;
  const ty = cy - y0;
  const h = layers.height;
  const at = y0 * size + x0;
  return (h[at] * (1 - tx) + h[at + 1] * tx) * (1 - ty) + (h[at + size] * (1 - tx) + h[at + size + 1] * tx) * ty;
}

/**
 * Apply one dab. `dab` holds `x`, `y`, `radius`, `strength` (0 to 1),
 * `falloff`, and for the biome brush `biome` (a biome index, or 255 to
 * erase). `part`, from 0 to `dabParts(tool) - 1`, applies one part
 * of the dab; by default it applies all of it. Returns the rectangle it
 * may have changed, or `null`.
 */
export function applyDab(doc, stroke, dab, part = null) {
  const rect = dabRect(doc.size, dab.x, dab.y, dab.radius);

  if (!part) {
    stroke.dabs += 1;
  }

  if (!rect || !(dab.strength > 0)) {
    return null;
  }

  const { tool } = stroke;

  if (tool === "noise") {
    addNoise(doc, rect, dab, NOISE_METRES * dab.strength, Math.max(4, dab.radius));
    return rect;
  }

  const weights = weightsFor(rect, dab);
  const { size } = doc;
  const width = rect.x1 - rect.x0;
  const strength = dab.strength;

  if (tool === "smooth") {
    smooth(doc, rect, weights, strength);
  } else if (tool === "erode") {
    erode(doc, rect, weights, strength, stroke.seed * 7919 + stroke.dabs, dab, part);
  } else if (TOOL_LAYERS[tool] === "height") {
    const height = doc.layers.height;
    const span = doc.displayRange[1] - doc.displayRange[0];
    const raise = RAISE_METRES * (span / 100) * strength * (tool === "lower" ? -1 : 1);
    const target = stroke.startHeight;

    for (let y = rect.y0; y < rect.y1; y += 1) {
      for (let x = rect.x0, w = (y - rect.y0) * width; x < rect.x1; x += 1, w += 1) {
        const weight = weights[w];

        if (weight > 0) {
          const at = y * size + x;
          height[at] += tool === "flatten" ? (target - height[at]) * strength * weight : raise * weight;
        }
      }
    }
  } else {
    const layer = doc.layers[TOOL_LAYERS[tool]];
    const category = tool === "biome" || TOOL_LAYERS[tool] === "water";
    const value = tool === "biome"
      ? dab.biome
      : tool === "lake" ? LAKE : tool === "river" ? Math.round(1 + strength * 126) : 0;
    const target = DENSITY_TARGETS[tool.replace(/^(trees|grass)/, "")];

    for (let y = rect.y0; y < rect.y1; y += 1) {
      for (let x = rect.x0, w = (y - rect.y0) * width; x < rect.x1; x += 1, w += 1) {
        const weight = weights[w];
        const at = y * size + x;

        if (category) {
          if (weight >= CATEGORY_THRESHOLD) {
            layer[at] = value;
          }
        } else if (weight > 0) {
          const old = layer[at];
          const next = old + (target - old) * strength * weight;
          // Rounding towards the target keeps weak dabs moving.
          layer[at] = target > old ? Math.ceil(next) : Math.floor(next);
        }
      }
    }
  }

  return rect;
}

// Working arrays, kept between dabs so painting makes no garbage.
const scratchArrays = [];

function scratch(slot, length) {
  if (!scratchArrays[slot] || scratchArrays[slot].length < length) {
    scratchArrays[slot] = new Float32Array(length);
  }

  return scratchArrays[slot];
}

let scratchColumns = new Int32Array(0);

function scratchInts(length) {
  if (scratchColumns.length < length) {
    scratchColumns = new Int32Array(length);
  }

  return scratchColumns;
}

// Brush weights by squared distance over the squared radius, so a dab
// needs no square roots or cosines per sample.
const WEIGHT_STEPS = 4096;
const weightTables = new Map();

function weightTable(falloff) {
  let table = weightTables.get(falloff);

  if (!table) {
    table = new Float32Array(WEIGHT_STEPS + 1).map((_, step) => falloffWeight(falloff, Math.sqrt(step / WEIGHT_STEPS)));
    weightTables.set(falloff, table);
  }

  return table;
}

function weightsFor(rect, dab) {
  const width = rect.x1 - rect.x0;
  const weights = scratch(0, width * (rect.y1 - rect.y0));
  const table = weightTable(dab.falloff);
  const scale = WEIGHT_STEPS / (dab.radius * dab.radius);

  for (let y = rect.y0; y < rect.y1; y += 1) {
    const dy = y - dab.y;
    const row = (y - rect.y0) * width - rect.x0;

    for (let x = rect.x0; x < rect.x1; x += 1) {
      const dx = x - dab.x;
      const step = (dx * dx + dy * dy) * scale;
      weights[row + x] = step < WEIGHT_STEPS ? table[step | 0] : 0;
    }
  }

  return weights;
}

/** Blend towards a 5 x 5 Gaussian of the neighbourhood, which is read up to 2 samples outside the rectangle. */
function smooth(doc, rect, weights, strength) {
  const { size } = doc;
  const height = doc.layers.height;
  const width = rect.x1 - rect.x0;
  const rows = rect.y1 - rect.y0 + 4;
  const across = scratch(1, width * rows);
  // Columns of each tap, clamped to the document once rather than per sample.
  const columns = new Int32Array(width + 4).map((_, k) => Math.min(Math.max(rect.x0 - 2 + k, 0), size - 1));

  for (let row = 0; row < rows; row += 1) {
    const start = Math.min(Math.max(rect.y0 - 2 + row, 0), size - 1) * size;

    for (let x = 0; x < width; x += 1) {
      across[row * width + x] = (height[start + columns[x]] + height[start + columns[x + 4]] +
        4 * (height[start + columns[x + 1]] + height[start + columns[x + 3]]) + 6 * height[start + columns[x + 2]]) / 16;
    }
  }

  for (let y = 0; y < rows - 4; y += 1) {
    for (let x = 0, at = (rect.y0 + y) * size + rect.x0, w = y * width; x < width; x += 1, at += 1, w += 1) {
      const weight = weights[w];

      if (weight > 0) {
        const blurred = (across[w] + across[w + 4 * width] + 4 * (across[w + width] + across[w + 3 * width]) + 6 * across[w + 2 * width]) / 16;
        height[at] += (blurred - height[at]) * strength * weight;
      }
    }
  }
}

function hash(x, y) {
  let h = Math.imul(x, 374761393) + Math.imul(y, 668265263);
  h = Math.imul(h ^ (h >>> 13), 1274126177);
  return ((h ^ (h >>> 16)) >>> 0) / 4294967296;
}

// Unit gradients the noise picks from by hash.
const GRADIENTS_X = new Float32Array(256).map((_, index) => Math.cos((index / 256) * Math.PI * 2));
const GRADIENTS_Y = new Float32Array(256).map((_, index) => Math.sin((index / 256) * Math.PI * 2));

function corner(cx, cy, fx, fy) {
  const g = (hash(cx, cy) * 256) | 0;
  return GRADIENTS_X[g] * fx + GRADIENTS_Y[g] * fy;
}

function gradientNoise(x, y) {
  const x0 = Math.floor(x);
  const y0 = Math.floor(y);
  const fx = x - x0;
  const fy = y - y0;
  const a = corner(x0, y0, fx, fy);
  const b = corner(x0 + 1, y0, fx - 1, fy);
  const c = corner(x0, y0 + 1, fx, fy - 1);
  const d = corner(x0 + 1, y0 + 1, fx - 1, fy - 1);
  const sx = fx * fx * (3 - 2 * fx);
  const sy = fy * fy * (3 - 2 * fy);
  const top = a + (b - a) * sx;
  return (top + (c + (d - c) * sx - top) * sy) * 1.4;
}

/** Three octaves of gradient noise, from about -1 to 1. */
export function fbm(x, y) {
  return (gradientNoise(x, y) + 0.5 * gradientNoise(x * 2.03 + 17.1, y * 2.03 - 5.3) +
    0.25 * gradientNoise(x * 4.1 - 31.7, y * 4.1 + 9.2)) / 1.75;
}

/**
 * Add the fbm at `wavelength`, times `amount` and the brush weight, over
 * `rect`. The fbm is evaluated on a grid a twelfth of the wavelength
 * apart, anchored to the map so overlapping dabs agree, and interpolated
 * between, which resolves its finest octave at a fraction of the cost.
 */
function addNoise(doc, rect, dab, amount, wavelength) {
  const { size } = doc;
  const height = doc.layers.height;
  const { step, gx0, gy0, columns, values } = noiseGrid(rect, wavelength);
  const width = rect.x1 - rect.x0;
  const table = weightTable(dab.falloff);
  const scale = WEIGHT_STEPS / (dab.radius * dab.radius);
  // Each sample's grid column and its fraction, the same on every row;
  // then one row of the grid interpolated down to the sample's row, so
  // the inner loop is a single lerp with no calls or allocations. The
  // brush weight comes from its table here too, and only across the
  // brush's circle, rather than in a pass of its own over the rectangle.
  const column = scratchInts(width);
  const along = scratch(3, width);
  const line = scratch(4, columns);

  for (let x = 0; x < width; x += 1) {
    const fx = (rect.x0 + x) / step - gx0;
    column[x] = Math.floor(fx);
    along[x] = fx - column[x];
  }

  for (let y = rect.y0; y < rect.y1; y += 1) {
    const fy = y / step - gy0;
    const row = Math.floor(fy);
    const ty = fy - row;
    const top = row * columns;
    const dy = y - dab.y;
    // The circle's span on this row, a sample wider each side for rounding.
    const reach = Math.sqrt(Math.max(0, dab.radius * dab.radius - dy * dy)) + 1;
    const start = Math.max(rect.x0, Math.floor(dab.x - reach));
    const end = Math.min(rect.x1, Math.ceil(dab.x + reach) + 1);

    for (let c = 0; c < columns; c += 1) {
      const upper = values[top + c];
      line[c] = upper + (values[top + columns + c] - upper) * ty;
    }

    for (let x = start, at = y * size + start; x < end; x += 1, at += 1) {
      const dx = x - dab.x;
      const weightStep = (dx * dx + dy * dy) * scale;

      if (weightStep < WEIGHT_STEPS) {
        const local = x - rect.x0;
        const c = column[local];
        const left = line[c];
        height[at] += amount * table[weightStep | 0] * (left + (line[c + 1] - left) * along[local]);
      }
    }
  }
}

// The last grid, whose values the next dab reuses where they overlap:
// dabs a quarter of the radius apart share most of their grid points,
// and the fbm is most of what a noise dab costs.
let lastGrid = null;

/**
 * The fbm at `wavelength` sampled for `rect` on a grid `step` samples
 * apart, from grid point `(gx0, gy0)`: `rows` rows of `columns` values.
 */
export function noiseGrid(rect, wavelength) {
  const step = Math.max(1, Math.floor(wavelength / 12));
  const gx0 = Math.floor(rect.x0 / step);
  const gy0 = Math.floor(rect.y0 / step);
  const columns = Math.floor((rect.x1 - 1) / step) - gx0 + 2;
  const rows = Math.floor((rect.y1 - 1) / step) - gy0 + 2;
  // Two buffers in turn, so the last grid stays readable while this one fills.
  const slot = lastGrid?.slot === 2 ? 5 : 2;
  const values = scratch(slot, columns * rows);
  const last = lastGrid?.wavelength === wavelength ? lastGrid : null;

  for (let row = 0; row < rows; row += 1) {
    const lastRow = last ? gy0 + row - last.gy0 : -1;
    const reuse = last && lastRow >= 0 && lastRow < last.rows;

    for (let column = 0; column < columns; column += 1) {
      const lastColumn = gx0 + column - (last?.gx0 ?? 0);
      values[row * columns + column] = reuse && lastColumn >= 0 && lastColumn < last.columns
        ? last.values[lastRow * last.columns + lastColumn]
        : fbm(((gx0 + column) * step) / wavelength, ((gy0 + row) * step) / wavelength);
    }
  }

  lastGrid = { slot, wavelength, gx0, gy0, columns, rows, values };
  return { step, gx0, gy0, columns, rows, values };
}

function random(seed) {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = Math.imul(state ^ (state >>> 15), 1 | state);
    t ^= t + Math.imul(t ^ (t >>> 7), 61 | t);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/**
 * Thermal erosion down to the talus angle, then a droplet pass. Material
 * only moves between samples inside the rectangle, so the dab conserves
 * the mass of the height layer.
 */
function erode(doc, rect, weights, strength, seed, dab, part) {
  const { size, metresPerSample } = doc;
  const height = doc.layers.height;
  const width = rect.x1 - rect.x0;
  const rows = rect.y1 - rect.y0;
  const talus = Math.tan((TALUS_DEGREES * Math.PI) / 180) * metresPerSample;
  // An eighth of the excess a neighbour pair holds over the talus moves
  // down it, scaled by the brush at the higher sample; four neighbours
  // at most then move half the excess, which keeps it stable. Each move
  // takes from one sample what it gives the other, so mass is kept.
  const rate = 0.125 * strength;
  // Only pairs within a sample of the brush's circle can move anything.
  const half = (y) => {
    const dy = rect.y0 + y - dab.y;
    return Math.sqrt(Math.max(0, dab.radius * dab.radius - dy * dy));
  };

  for (let iteration = 0; iteration < THERMAL_ITERATIONS; iteration += 1) {
    if (part !== null && part !== iteration) {
      continue;
    }

    // Each pair of neighbours once: a sample and those right of and below it.
    for (let y = 0; y < rows; y += 1) {
      const reach = Math.max(half(y), y + 1 < rows ? half(y + 1) : 0);
      const start = Math.max(0, Math.floor(dab.x - reach) - rect.x0 - 1);
      const end = Math.min(width, Math.ceil(dab.x + reach) - rect.x0 + 2);

      for (let x = start, local = y * width + x, at = (rect.y0 + y) * size + rect.x0 + x; x < end; x += 1, local += 1, at += 1) {
        const weight = weights[local];

        if (x + 1 < width) {
          const drop = height[at] - height[at + 1];

          if (drop > talus && weight > 0) {
            const moved = (drop - talus) * rate * weight;
            height[at] -= moved;
            height[at + 1] += moved;
          } else if (drop < -talus && weights[local + 1] > 0) {
            const moved = (-drop - talus) * rate * weights[local + 1];
            height[at + 1] -= moved;
            height[at] += moved;
          }
        }

        if (y + 1 < rows) {
          const drop = height[at] - height[at + size];

          if (drop > talus && weight > 0) {
            const moved = (drop - talus) * rate * weight;
            height[at] -= moved;
            height[at + size] += moved;
          } else if (drop < -talus && weights[local + width] > 0) {
            const moved = (-drop - talus) * rate * weights[local + width];
            height[at + size] -= moved;
            height[at] += moved;
          }
        }
      }
    }
  }

  if (part === null || part === THERMAL_ITERATIONS) {
    droplets(doc, rect, weights, strength, seed);
  }
}

/** Droplets that pick up soil going downhill and drop it where they slow. */
function droplets(doc, rect, weights, strength, seed) {
  const { size } = doc;
  const height = doc.layers.height;
  const width = rect.x1 - rect.x0;
  const next = random(seed);
  const cx = (rect.x0 + rect.x1 - 1) / 2;
  const cy = (rect.y0 + rect.y1 - 1) / 2;
  const radius = Math.max(rect.x1 - rect.x0, rect.y1 - rect.y0) / 2;
  // A droplet's cell and the one beyond it must both be in the rectangle.
  const inside = (x, y) => x >= rect.x0 && y >= rect.y0 && x < rect.x1 - 1 && y < rect.y1 - 1;
  // Height and gradient at a point, written to `probe` so the steps allocate nothing.
  const probe = new Float64Array(3);
  const sample = (x, y) => {
    const x0 = Math.floor(x);
    const y0 = Math.floor(y);
    const tx = x - x0;
    const ty = y - y0;
    const at = y0 * size + x0;
    const a = height[at];
    const b = height[at + 1];
    const c = height[at + size];
    const d = height[at + size + 1];
    probe[0] = (a * (1 - tx) + b * tx) * (1 - ty) + (c * (1 - tx) + d * tx) * ty;
    probe[1] = (b - a) * (1 - ty) + (d - c) * ty;
    probe[2] = (c - a) * (1 - tx) + (d - b) * tx;
    return probe;
  };
  const spread = (x, y, amount) => {
    const x0 = Math.floor(x);
    const y0 = Math.floor(y);
    const tx = x - x0;
    const ty = y - y0;
    const at = y0 * size + x0;
    height[at] += amount * (1 - tx) * (1 - ty);
    height[at + 1] += amount * tx * (1 - ty);
    height[at + size] += amount * (1 - tx) * ty;
    height[at + size + 1] += amount * tx * ty;
  };

  for (let drop = 0; drop < DROPLETS_PER_DAB; drop += 1) {
    const angle = next() * Math.PI * 2;
    const distance = Math.sqrt(next()) * radius;
    let x = cx + Math.cos(angle) * distance;
    let y = cy + Math.sin(angle) * distance;
    let dirX = 0;
    let dirY = 0;
    let speed = 1;
    let water = 1;
    let sediment = 0;

    if (!inside(x, y)) {
      continue;
    }

    for (let step = 0; step < 30; step += 1) {
      const here = sample(x, y)[0];
      dirX = dirX * 0.3 - probe[1] * 0.7;
      dirY = dirY * 0.3 - probe[2] * 0.7;
      const length = Math.sqrt(dirX * dirX + dirY * dirY);

      if (length < 1e-6) {
        break;
      }

      dirX /= length;
      dirY /= length;
      const nx = x + dirX;
      const ny = y + dirY;

      if (!inside(nx, ny)) {
        break;
      }

      const fall = here - sample(nx, ny)[0];
      const capacity = Math.max(fall, 0.01) * speed * water * 4;
      const weight = weights[(Math.round(y) - rect.y0) * width + Math.round(x) - rect.x0] * strength;

      if (fall < 0 || sediment > capacity) {
        // Uphill, fill the step; otherwise drop what the water cannot carry.
        const amount = fall < 0 ? Math.min(-fall, sediment) : (sediment - capacity) * 0.3;
        spread(x, y, amount);
        sediment -= amount;
      } else {
        const amount = Math.min((capacity - sediment) * 0.3, fall) * weight;
        spread(x, y, -amount);
        sediment += amount;
      }

      speed = Math.sqrt(Math.max(speed * speed + fall * 0.5, 0));
      water *= 0.96;
      x = nx;
      y = ny;
    }

    // Whatever is still carried settles where the droplet stops.
    spread(x, y, sediment);
  }
}
