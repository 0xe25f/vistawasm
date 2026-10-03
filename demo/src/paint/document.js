// The paint document: five square layers the brushes write to, and the
// settings that turn them into a terrain. Every layer is N x N samples,
// row-major from the map's -x, -z corner, as `exportMap()` returns maps
// and as the painted-map setters read them.

export const DOCUMENT_SIZES = [256, 512, 1024];

/** A biome sample the engine classifies itself. */
export const NOT_PAINTED = 255;

/** A density sample that leaves the engine's own density alone. */
export const UNCHANGED = 128;

/** The narrowest and widest map, in kilometres. */
export const WIDTH_LIMITS_KM = [0.5, 100];

function checkSize(size) {
  if (!DOCUMENT_SIZES.includes(size)) {
    throw new RangeError(`The painting size must be one of ${DOCUMENT_SIZES.join(", ")} samples, but it is ${String(size)}.`);
  }
}

function checkNumber(name, value, [min, max]) {
  if (!Number.isFinite(value) || value < min || value > max) {
    throw new RangeError(`${name} must be from ${min} to ${max}, but it is ${String(value)}.`);
  }
}

/**
 * A new document of `size` samples a side, `widthKm` across, flat at
 * `heightMetres`, with nothing painted.
 */
export function createDocument({ size = 512, widthKm = 6, seaLevelMetres = 0, heightMetres = 0 } = {}) {
  checkSize(size);
  checkNumber("The map width in km", widthKm, WIDTH_LIMITS_KM);
  checkNumber("The sea level in metres", seaLevelMetres, [-10000, 10000]);
  checkNumber("The starting height in metres", heightMetres, [-10000, 10000]);
  const count = size * size;
  const layers = {
    height: new Float32Array(count).fill(heightMetres),
    biome: new Uint8Array(count).fill(NOT_PAINTED),
    water: new Uint8Array(count),
    trees: new Uint8Array(count).fill(UNCHANGED),
    grass: new Uint8Array(count).fill(UNCHANGED)
  };
  const doc = { size, widthKm, metresPerSample: 0, seaLevelMetres, displayRange: [0, 1], layers };
  setMapWidth(doc, widthKm);
  doc.displayRange = defaultDisplayRange(layers.height, seaLevelMetres);
  return doc;
}

/** Set the map width, which sets the distance between samples. */
export function setMapWidth(doc, widthKm) {
  checkNumber("The map width in km", widthKm, WIDTH_LIMITS_KM);
  doc.widthKm = widthKm;
  // The map spans (N - 1) gaps between its edge samples.
  doc.metresPerSample = (widthKm * 1000) / (doc.size - 1);
}

export function minMax(values) {
  let low = Infinity;
  let high = -Infinity;

  for (let index = 0; index < values.length; index += 1) {
    const value = values[index];
    low = value < low ? value : low;
    high = value > high ? value : high;
  }

  return [low, high];
}

/**
 * The heights the view colours over: the data and the sea, with room to
 * paint at least 1000 m of relief, in whole tens of metres.
 */
export function defaultDisplayRange(heights, seaLevelMetres) {
  const [low, high] = minMax(heights);
  const bottom = Math.floor((Math.min(low, seaLevelMetres) - 50) / 10) * 10;
  const top = Math.ceil(Math.max(high + 100, bottom + 1000) / 10) * 10;
  return [bottom, top];
}

/**
 * Resample a `width x height` grid to `size x size` bilinearly, with the
 * corner samples on the corners, since the map spans its edge samples.
 */
export function resampleBilinear(source, width, height, size) {
  const out = new Float32Array(size * size);
  const scaleX = (width - 1) / (size - 1);
  const scaleY = (height - 1) / (size - 1);

  for (let y = 0; y < size; y += 1) {
    const fy = y * scaleY;
    const y0 = Math.min(Math.floor(fy), height - 1);
    const y1 = Math.min(y0 + 1, height - 1);
    const ty = fy - y0;

    for (let x = 0; x < size; x += 1) {
      const fx = x * scaleX;
      const x0 = Math.min(Math.floor(fx), width - 1);
      const x1 = Math.min(x0 + 1, width - 1);
      const tx = fx - x0;
      const top = source[y0 * width + x0] * (1 - tx) + source[y0 * width + x1] * tx;
      const bottom = source[y1 * width + x0] * (1 - tx) + source[y1 * width + x1] * tx;
      out[y * size + x] = top * (1 - ty) + bottom * ty;
    }
  }

  return out;
}

/** Resample a byte grid to `size x size` by the nearest sample, for categories. */
export function resampleNearest(source, width, height, size) {
  const out = new Uint8Array(size * size);

  for (let y = 0; y < size; y += 1) {
    const row = Math.round((y * (height - 1)) / (size - 1)) * width;

    for (let x = 0; x < size; x += 1) {
      out[y * size + x] = source[row + Math.round((x * (width - 1)) / (size - 1))];
    }
  }

  return out;
}

function resampleBytes(mask, size, nearest) {
  if (mask.width === size && mask.height === size) {
    return Uint8Array.from(mask.data);
  }

  if (nearest) {
    return resampleNearest(mask.data, mask.width, mask.height, size);
  }

  return Uint8Array.from(resampleBilinear(mask.data, mask.width, mask.height, size), Math.round);
}

/**
 * A document from the engine's terrain: its heights before any carving,
 * resampled to `size`, and the painted maps it has set. With
 * `copyBiomes`, the biome layer takes the biomes the engine classified.
 */
export function documentFromEngine(engine, { size = 512, copyBiomes = false } = {}) {
  checkSize(size);
  const source = engine.exportMap("sourceHeight");
  const metresPerPixel = source.encoding?.metresPerPixel?.[0];

  if (!(metresPerPixel > 0)) {
    throw new RangeError("The terrain's heights have no sample spacing, so its width is unknown.");
  }

  const widthKm = Math.min(Math.max(((source.width - 1) * metresPerPixel) / 1000, WIDTH_LIMITS_KM[0]), WIDTH_LIMITS_KM[1]);
  const doc = createDocument({ size, widthKm, seaLevelMetres: source.encoding.seaLevelMetres ?? 0 });
  const { layers } = doc;
  layers.height = source.width === size && source.height === size
    ? Float32Array.from(source.data)
    : resampleBilinear(source.data, source.width, source.height, size);
  const painted = engine.getPaintedMaps();

  if (copyBiomes) {
    const biome = engine.exportMap("biome");
    layers.biome = resampleBytes({ width: biome.width, height: biome.height, data: biome.data }, size, true);
  } else if (painted.biome) {
    layers.biome = resampleBytes(painted.biome, size, true);
  }

  if (painted.water) {
    layers.water = resampleBytes(painted.water, size, true);
  }

  for (const kind of ["trees", "grass"]) {
    if (painted[kind]) {
      layers[kind] = resampleBytes(painted[kind], size, false);
    }
  }

  doc.displayRange = defaultDisplayRange(layers.height, doc.seaLevelMetres);
  return doc;
}

function every(values, value) {
  for (let index = 0; index < values.length; index += 1) {
    if (values[index] !== value) {
      return false;
    }
  }

  return true;
}

/**
 * The painted maps for the engine's setters: each layer as a mask, or
 * `null` when nothing is painted on it, which clears the engine's mask.
 */
export function paintedInputs(doc) {
  const { size, layers } = doc;
  // Copies, so later strokes never reach a mask the engine holds.
  const mask = (data, empty) => (every(data, empty) ? null : { width: size, height: size, data: data.slice() });
  return {
    biome: mask(layers.biome, NOT_PAINTED),
    water: mask(layers.water, 0),
    trees: mask(layers.trees, UNCHANGED),
    grass: mask(layers.grass, UNCHANGED)
  };
}

/** The raw heightmap options that load the height layer. */
export function heightmapOptions(doc) {
  return {
    width: doc.size,
    height: doc.size,
    sampleFormat: "float32",
    byteOrder: "little-endian",
    metresPerSample: doc.metresPerSample,
    heightScaleMetres: 1,
    seaLevelMetres: doc.seaLevelMetres
  };
}
