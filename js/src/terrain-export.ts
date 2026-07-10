import type { TerrainMetadata } from "./types.js";

/**
 * Colour mode used when rendering a heightmap to pixels.
 */
export type HeightmapColourMode = "grayscale" | "hypsometric";

/**
 * Options shared by the heightmap pixel and image helpers.
 */
export interface HeightmapImageOptions {
  /**
   * Colour ramp used to shade each sample. Defaults to `"hypsometric"`.
   */
  colourMode?: HeightmapColourMode;
}

/**
 * Read the little-endian `f32` heights returned by `VistaEngine.exportHeightmap()`
 * into a plain `Float32Array`, regardless of the source buffer's alignment.
 */
export function readHeightmapFloats(heightBytes: Uint8Array): Float32Array {
  const view = new DataView(
    heightBytes.buffer,
    heightBytes.byteOffset,
    heightBytes.byteLength
  );
  const count = Math.floor(heightBytes.byteLength / 4);
  const floats = new Float32Array(count);

  for (let index = 0; index < count; index += 1) {
    floats[index] = view.getFloat32(index * 4, true);
  }

  return floats;
}

/**
 * Compute an RGBA pixel buffer for a top-down view of a heightmap.
 *
 * This is pure data transformation with no DOM dependency, so it can be
 * unit tested and reused by both the minimap overlay and PNG export.
 */
export function computeHeightmapPixels(
  metadata: TerrainMetadata,
  heightBytes: Uint8Array,
  options: HeightmapImageOptions = {}
): Uint8ClampedArray<ArrayBuffer> {
  const { width, height } = metadata;

  if (width <= 0 || height <= 0) {
    throw new TypeError("metadata.width and metadata.height must be greater than 0.");
  }

  const heights = readHeightmapFloats(heightBytes);

  if (heights.length !== width * height) {
    throw new TypeError(
      "heightBytes does not contain width * height float32 samples."
    );
  }

  const colourMode = options.colourMode ?? "hypsometric";
  const min = metadata.minHeightMetres;
  const max = metadata.maxHeightMetres;
  const range = max - min > 0.0001 ? max - min : 1;
  const seaLevel = metadata.seaLevelMetres;
  const pixels: Uint8ClampedArray<ArrayBuffer> = new Uint8ClampedArray(width * height * 4);

  for (let index = 0; index < heights.length; index += 1) {
    const elevation = heights[index];
    const normalised = clamp01((elevation - min) / range);
    const [r, g, b] =
      colourMode === "grayscale"
        ? grayscaleColour(normalised)
        : hypsometricColour(elevation, seaLevel, min, max);
    const offset = index * 4;

    pixels[offset] = r;
    pixels[offset + 1] = g;
    pixels[offset + 2] = b;
    pixels[offset + 3] = 255;
  }

  return pixels;
}

/**
 * Draw a top-down heightmap view onto an existing canvas element, resizing
 * the canvas to match the heightmap dimensions.
 *
 * Use this for both a live minimap overlay and as the first step of PNG
 * export.
 */
export function renderHeightmapToCanvas(
  canvas: HTMLCanvasElement,
  metadata: TerrainMetadata,
  heightBytes: Uint8Array,
  options: HeightmapImageOptions = {}
): void {
  const pixels = computeHeightmapPixels(metadata, heightBytes, options);
  canvas.width = metadata.width;
  canvas.height = metadata.height;

  const context = canvas.getContext("2d");

  if (!context) {
    throw new Error("Could not acquire a 2D canvas context to draw the heightmap.");
  }

  const imageData = new ImageData(pixels, metadata.width, metadata.height);
  context.putImageData(imageData, 0, 0);
}

/**
 * Render a heightmap to an off-screen canvas and export it as a PNG (or
 * another canvas-supported format) `Blob`.
 */
export async function exportHeightmapImage(
  metadata: TerrainMetadata,
  heightBytes: Uint8Array,
  options: HeightmapImageOptions & { mimeType?: string; quality?: number } = {}
): Promise<Blob> {
  const canvas = document.createElement("canvas");
  renderHeightmapToCanvas(canvas, metadata, heightBytes, options);

  return new Promise((resolve, reject) => {
    canvas.toBlob(
      (blob) => {
        if (!blob) {
          reject(new Error("Could not encode the heightmap image."));
          return;
        }

        resolve(blob);
      },
      options.mimeType ?? "image/png",
      options.quality
    );
  });
}

/**
 * Options for {@link exportTerrainObj}.
 */
export interface TerrainObjExportOptions {
  /**
   * Maximum vertices per mesh axis. Large terrain is downsampled to keep
   * the exported file a reasonable size. Defaults to `256`.
   */
  maxSamplesPerSide?: number;
}

/**
 * Generate a Wavefront OBJ mesh from an exported heightmap.
 *
 * The mesh is centred on the terrain origin and uses the same metres-based
 * coordinate space as the engine. Large terrain is downsampled so the file
 * stays a practical size for a "download 3D model" button; pass
 * `maxSamplesPerSide` to change the cap.
 */
export function exportTerrainObj(
  metadata: TerrainMetadata,
  heightBytes: Uint8Array,
  options: TerrainObjExportOptions = {}
): string {
  const { width, height, metresPerSample } = metadata;

  if (width <= 1 || height <= 1) {
    throw new TypeError("metadata.width and metadata.height must be greater than 1.");
  }

  const heights = readHeightmapFloats(heightBytes);

  if (heights.length !== width * height) {
    throw new TypeError(
      "heightBytes does not contain width * height float32 samples."
    );
  }

  const maxSamplesPerSide = Math.max(2, options.maxSamplesPerSide ?? 256);
  const stride = Math.max(1, Math.ceil((Math.max(width, height) - 1) / (maxSamplesPerSide - 1)));
  const samplesX = Math.floor((width - 1) / stride) + 1;
  const samplesY = Math.floor((height - 1) / stride) + 1;
  const sampleStep = Math.max(metresPerSample, 0.001);
  const halfWidth = ((width - 1) * sampleStep) / 2;
  const halfHeight = ((height - 1) * sampleStep) / 2;

  const lines: string[] = [
    "# Exported from VistaWASM.",
    `# ${samplesX * samplesY} vertices, source terrain ${width}x${height} samples.`
  ];

  for (let sy = 0; sy < samplesY; sy += 1) {
    const y = Math.min(sy * stride, height - 1);

    for (let sx = 0; sx < samplesX; sx += 1) {
      const x = Math.min(sx * stride, width - 1);
      const elevation = heights[y * width + x];
      const worldX = x * sampleStep - halfWidth;
      const worldZ = y * sampleStep - halfHeight;

      lines.push(`v ${worldX.toFixed(3)} ${elevation.toFixed(3)} ${worldZ.toFixed(3)}`);
    }
  }

  for (let sy = 0; sy < samplesY - 1; sy += 1) {
    for (let sx = 0; sx < samplesX - 1; sx += 1) {
      const topLeft = sy * samplesX + sx + 1;
      const topRight = topLeft + 1;
      const bottomLeft = topLeft + samplesX;
      const bottomRight = bottomLeft + 1;

      lines.push(`f ${topLeft} ${bottomLeft} ${topRight}`);
      lines.push(`f ${topRight} ${bottomLeft} ${bottomRight}`);
    }
  }

  return lines.join("\n");
}

/**
 * Trigger a browser download for the raw little-endian `float32` heightmap
 * bytes returned by `engine.exportHeightmap()`.
 *
 * The `.bin` payload has no header of its own — it is exactly
 * `width * height * 4` bytes, row-major — so `width`/`height` only exist in
 * the accompanying {@link TerrainMetadata}. Software that imports raw
 * heightmaps typically needs those dimensions typed in separately, and some
 * tools guess them by assuming the sample count is a perfect square, which
 * fails for non-square terrain. Encoding `width`/`height` into the filename
 * (`prefix-WIDTHxHEIGHT.f32le.bin`) gives users the numbers to enter by
 * hand, and matches a convention several raw heightmap importers already
 * recognise automatically.
 */
export function downloadRawHeightmap(
  metadata: TerrainMetadata,
  heightBytes: Uint8Array<ArrayBuffer>,
  filenamePrefix = "vistawasm-heightmap"
): void {
  const { width, height } = metadata;

  downloadBlob(
    new Blob([heightBytes], { type: "application/octet-stream" }),
    `${filenamePrefix}-${width}x${height}.f32le.bin`
  );
}

/**
 * Trigger a browser download for a `Blob`.
 */
export function downloadBlob(blob: Blob, filename: string): void {
  const url = URL.createObjectURL(blob);

  try {
    downloadUrl(url, filename);
  } finally {
    URL.revokeObjectURL(url);
  }
}

/**
 * Trigger a browser download for a plain text payload.
 */
export function downloadText(text: string, filename: string, mimeType = "text/plain"): void {
  downloadBlob(new Blob([text], { type: mimeType }), filename);
}

function downloadUrl(url: string, filename: string): void {
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = filename;
  anchor.rel = "noopener";
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
}

function clamp01(value: number): number {
  return Math.min(Math.max(value, 0), 1);
}

function grayscaleColour(normalised: number): [number, number, number] {
  const value = Math.round(normalised * 255);
  return [value, value, value];
}

const HYPSOMETRIC_LAND_STOPS: Array<{ stop: number; colour: [number, number, number] }> = [
  { stop: 0, colour: [58, 92, 56] },
  { stop: 0.25, colour: [104, 137, 68] },
  { stop: 0.5, colour: [163, 158, 91] },
  { stop: 0.72, colour: [140, 104, 78] },
  { stop: 0.88, colour: [120, 108, 104] },
  { stop: 1, colour: [246, 248, 250] }
];

const HYPSOMETRIC_WATER_STOPS: Array<{ stop: number; colour: [number, number, number] }> = [
  { stop: 0, colour: [12, 36, 66] },
  { stop: 1, colour: [64, 118, 168] }
];

/**
 * Map an elevation to a topographic-style colour: blues below sea level and
 * a green-to-brown-to-white ramp above it.
 */
function hypsometricColour(
  elevation: number,
  seaLevel: number,
  min: number,
  max: number
): [number, number, number] {
  if (elevation <= seaLevel) {
    const depthRange = seaLevel - min > 0.0001 ? seaLevel - min : 1;
    const t = clamp01((elevation - min) / depthRange);
    return sampleGradient(HYPSOMETRIC_WATER_STOPS, t);
  }

  const landRange = max - seaLevel > 0.0001 ? max - seaLevel : 1;
  const t = clamp01((elevation - seaLevel) / landRange);
  return sampleGradient(HYPSOMETRIC_LAND_STOPS, t);
}

function sampleGradient(
  stops: Array<{ stop: number; colour: [number, number, number] }>,
  t: number
): [number, number, number] {
  for (let index = 0; index < stops.length - 1; index += 1) {
    const current = stops[index];
    const next = stops[index + 1];

    if (t >= current.stop && t <= next.stop) {
      const span = next.stop - current.stop || 1;
      const localT = (t - current.stop) / span;

      return [
        lerp(current.colour[0], next.colour[0], localT),
        lerp(current.colour[1], next.colour[1], localT),
        lerp(current.colour[2], next.colour[2], localT)
      ];
    }
  }

  return stops[stops.length - 1].colour;
}

function lerp(a: number, b: number, t: number): number {
  return Math.round(a + (b - a) * t);
}
