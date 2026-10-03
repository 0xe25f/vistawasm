import zlib from "node:zlib";
import { beforeAll, describe, expect, it, vi } from "vitest";
import { VistaWasmError } from "../src/errors";
import { createVistaEngine, initialiseVistaWasm, loadBundle, loadTerrainFromImages } from "../src/index";
import { crc32, exportBundle, zip } from "../src/map-export";
import {
  BIOME_COLOURS,
  BIOME_KINDS,
  BUNDLE_LIMITS,
  MAX_TERRAIN_SIDE,
  biomeMapFromImage,
  decodeImage,
  densityMaskFromImage,
  parseManifest,
  waterMaskFromImage
} from "../src/map-import";
import { decodePng } from "../src/png-decode";
import type {
  BiomeMap,
  DensityMask,
  ExportedMap,
  MapKind,
  PaintedMaps,
  RawHeightmapOptions,
  VistaEngine,
  VistaWasmGeneratedModule,
  WaterMask
} from "../src/types";
import { chunk, makePng } from "./make-png";

/** A seeded generator (mulberry32), so every fuzz run is the same. */
function random(seed: number): () => number {
  return () => {
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** An RGB(A) PNG two rows tall, each row `colours`, one a pixel. */
function colourPng(colours: number[][]): Uint8Array {
  const alpha = colours[0].length === 4;
  return makePng({ width: colours.length, height: 2, colourType: alpha ? 6 : 2, bitDepth: 8, samples: [...colours, ...colours].flat() });
}

/** The first row of a two-row map, as numbers. */
function firstRow(data: Uint8Array): number[] {
  return Array.from(data.subarray(0, data.length / 2));
}

describe("biomeMapFromImage", () => {
  it("maps exact legend colours exactly, and transparent pixels to 255", async () => {
    const colours = BIOME_COLOURS.map((colour) => [...colour, 255]);
    colours.push([12, 34, 56, 0]);
    const { map, unmatchedFraction } = await biomeMapFromImage(colourPng(colours));
    expect(firstRow(map.data)).toEqual([...BIOME_KINDS.map((_, index) => index), 255]);
    expect(unmatchedFraction).toBe(0);
    expect(map.borderSamples).toBe(3);
  });

  it("maps colours within ΔE 10 to the right biome", async () => {
    // Small nudges to each channel keep every colour well within ΔE 10.
    const colours = BIOME_COLOURS.map((colour, index) => colour.map((c, k) => Math.min(255, Math.max(0, c + ((index + k) % 3) - 1))));
    const { map, unmatchedFraction } = await biomeMapFromImage(colourPng(colours));
    expect(firstRow(map.data)).toEqual(BIOME_KINDS.map((_, index) => index));
    expect(unmatchedFraction).toBe(0);
  });

  it("gives a far colour the nearest biome and counts it as unmatched, with a warning", async () => {
    const warnings: string[] = [];
    const colours = [[255, 0, 255], ...Array.from({ length: 9 }, () => [...BIOME_COLOURS[2]])];
    const { map, unmatchedFraction } = await biomeMapFromImage(colourPng(colours), { onWarning: (message) => warnings.push(message) });
    expect(unmatchedFraction).toBeCloseTo(0.1);
    expect(map.data[0]).toBeLessThan(19);
    expect(warnings[0]).toMatch(/10\.0 % of the biome map's pixels are more than ΔE 25/);
  });

  it("maps an exported palette PNG through its indices", async () => {
    const palette = BIOME_COLOURS.flat();
    const png = makePng({ width: 4, height: 2, colourType: 3, bitDepth: 8, samples: [0, 5, 18, 14, 3, 3, 8, 0], palette });
    const { map } = await biomeMapFromImage(png);
    expect(Array.from(map.data)).toEqual([0, 5, 18, 14, 3, 3, 8, 0]);
  });

  it("takes a custom legend and border", async () => {
    const { map } = await biomeMapFromImage(colourPng([[255, 0, 0], [0, 0, 255]]), {
      legend: [{ colour: [250, 0, 0], biome: "calderaVolcanic" }, { colour: [0, 0, 250], biome: "ocean" }],
      borderSamples: 0
    });
    expect(firstRow(map.data)).toEqual([7, 14]);
    expect(map.borderSamples).toBe(0);

    await expect(biomeMapFromImage(colourPng([[0, 0, 0], [0, 0, 0]]), { borderSamples: 9 })).rejects.toMatchObject({ code: "OPTIONS_INVALID" });
    await expect(biomeMapFromImage(colourPng([[0, 0, 0], [0, 0, 0]]), { legend: [{ colour: [0, 0, 256], biome: "ocean" }] })).rejects.toMatchObject({ code: "OPTIONS_INVALID" });
    await expect(biomeMapFromImage(colourPng([[0, 0, 0], [0, 0, 0]]), { legend: [{ colour: [0, 0, 0], biome: "moon" as never }] })).rejects.toMatchObject({ code: "OPTIONS_INVALID" });
    await expect(biomeMapFromImage("nope" as never)).rejects.toThrow(TypeError);
    await expect(biomeMapFromImage(makePng({ width: 1, height: 1, colourType: 0, bitDepth: 8, samples: [0] }))).rejects.toMatchObject({ code: "INVALID_DEM", message: expect.stringContaining("from 2 to 2048") });
  });
});

describe("waterMaskFromImage", () => {
  it("reads lake blues as lakes and river blues and cyans as rivers, brightness setting strength", async () => {
    const colours = [
      [0, 0, 0],
      [26, 89, 191],
      [38, 153, 153],
      [77, 204, 217],
      [39, 102, 109],
      [51, 140, 230],
      [13, 38, 115],
      [40, 200, 40],
      [255, 255, 255]
    ];
    const mask = await waterMaskFromImage(colourPng(colours));
    expect(firstRow(mask.data)).toEqual([0, 255, 255, 127, 64, 127, 0, 0, 0]);
  });

  it("reads grey values as they are in grey mode", async () => {
    const png = makePng({ width: 2, height: 2, colourType: 0, bitDepth: 8, samples: [0, 1, 127, 200] });
    const mask = await waterMaskFromImage(png, { mode: "grey" });
    expect(Array.from(mask.data)).toEqual([0, 1, 127, 200]);
    await expect(waterMaskFromImage(png, { mode: "blue" as never })).rejects.toMatchObject({ code: "OPTIONS_INVALID" });
  });
});

describe("densityMaskFromImage", () => {
  it("reads luminance by default, or one channel, as bytes", async () => {
    const png = colourPng([[255, 255, 255, 255], [128, 128, 128, 10], [255, 0, 0, 255]]);
    expect(firstRow((await densityMaskFromImage(png)).data)).toEqual([255, 128, 54]);
    expect(firstRow((await densityMaskFromImage(png, { channel: "r" })).data)).toEqual([255, 128, 255]);
    expect(firstRow((await densityMaskFromImage(png, { channel: "a" })).data)).toEqual([255, 10, 255]);
    const wide = makePng({ width: 2, height: 2, colourType: 0, bitDepth: 16, samples: [0, 65535, 0, 65535] });
    expect(firstRow((await densityMaskFromImage(wide)).data)).toEqual([0, 255]);
    await expect(densityMaskFromImage(png, { channel: "x" as never })).rejects.toMatchObject({ code: "OPTIONS_INVALID" });
  });
});

// The generated engine, recording calls, so the wrapper can be tested
// without a GPU.
const calls: { name: string; args: unknown[] }[] = [];
const raw = new Proxy({}, {
  get(_target, name: string) {
    // GPU events are polled after every async call; there are none here.
    if (name === "then" || name === "takeGpuEvents") {
      return undefined;
    }

    return (...args: unknown[]) => {
      calls.push({ name, args });

      if (name === "loadRawHeightmap") {
        return Promise.resolve({ id: 3, metadata: { warnings: [] } });
      }

      if (name === "setBiomeMap") {
        return "resampled\nocean above the sea";
      }

      return undefined;
    };
  }
});
let engine: VistaEngine;

beforeAll(async () => {
  vi.stubGlobal("navigator", { gpu: {} });
  await initialiseVistaWasm({ wasmModule: { VistaEngine: { create: async () => raw } } as unknown as VistaWasmGeneratedModule });
  engine = await createVistaEngine({} as HTMLCanvasElement, {});
});

describe("loadHeightmapImage", () => {
  it("scales 16-bit samples over the stored range and loads them as float32", async () => {
    const png = makePng({
      width: 3,
      height: 2,
      colourType: 0,
      bitDepth: 16,
      samples: [0, 65535, 32768, 1, 2, 3],
      text: { "vistawasm:range": "[-100,900]" }
    });
    await engine.loadHeightmapImage(png, { metresPerSample: 30, seaLevelMetres: 5 });
    const [buffer, options] = calls.at(-1)!.args as [ArrayBuffer, RawHeightmapOptions];
    expect(options).toEqual({
      width: 3,
      height: 2,
      sampleFormat: "float32",
      byteOrder: "little-endian",
      metresPerSample: 30,
      heightScaleMetres: 1,
      seaLevelMetres: 5
    });
    const heights = new Float32Array(buffer);
    expect(heights[0]).toBe(-100);
    expect(heights[1]).toBe(900);
    expect(heights[2]).toBeCloseTo(-100 + (32768 / 65535) * 1000, 3);
  });

  it("reads 8-bit channels over the given range", async () => {
    const png = colourPng([[255, 0, 0], [0, 0, 255]]);
    await engine.loadHeightmapImage(png, { metresPerSample: 10, minHeightMetres: 0, maxHeightMetres: 255, channel: "b" });
    expect(Array.from(new Float32Array(calls.at(-1)!.args[0] as ArrayBuffer))).toEqual([0, 255, 0, 255]);
    await engine.loadHeightmapImage(png, { metresPerSample: 10, minHeightMetres: 0, maxHeightMetres: 1000 });
    const heights = new Float32Array(calls.at(-1)!.args[0] as ArrayBuffer);
    expect(heights[0]).toBeCloseTo(212.6, 1);
  });

  it("validates every option", async () => {
    const png = colourPng([[0, 0, 0], [0, 0, 0]]);
    const bad = [
      [{ metresPerSample: 0, minHeightMetres: 0, maxHeightMetres: 1 }, /over 0/],
      [{ metresPerSample: Number.NaN, minHeightMetres: 0, maxHeightMetres: 1 }, /finite/],
      [{ metresPerSample: 1 }, /needs minHeightMetres and maxHeightMetres/],
      [{ metresPerSample: 1, minHeightMetres: 5, maxHeightMetres: 5 }, /must be below/],
      [{ metresPerSample: 1, minHeightMetres: 0, maxHeightMetres: Infinity }, /finite/],
      [{ metresPerSample: 1, minHeightMetres: 0, maxHeightMetres: 1, channel: "z" }, /channel must be/],
      [{ metresPerSample: 1, minHeightMetres: 0, maxHeightMetres: 1, seaLevelMetres: "0" }, /finite/]
    ] as const;

    for (const [options, message] of bad) {
      await expect(engine.loadHeightmapImage(png, options as never)).rejects.toMatchObject({ code: "OPTIONS_INVALID", message: expect.stringMatching(message) });
    }

    await expect(engine.loadHeightmapImage(png, undefined as never)).rejects.toThrow(TypeError);
    await expect(engine.loadHeightmapImage(new Uint8Array([1, 2, 3]), { metresPerSample: 1, minHeightMetres: 0, maxHeightMetres: 1 })).rejects.toMatchObject({ code: "INVALID_DEM" });
  });
});

describe("painted maps on the engine", () => {
  it("sends biome maps and density masks to the engine, with their warnings", () => {
    const warnings: string[] = [];
    engine.on("warning", ({ message }) => warnings.push(message));
    const data = new Uint8Array(6).fill(255);
    engine.setBiomeMap({ width: 3, height: 2, data });
    expect(calls.at(-1)).toEqual({ name: "setBiomeMap", args: [3, 2, data, 3] });
    expect(warnings.splice(0)).toEqual(["resampled", "ocean above the sea"]);

    const trees = { width: 2, height: 2, data: new Uint8Array(4) };
    engine.setVegetationMasks({ trees, grass: null });
    expect(calls.slice(-2)).toEqual([
      { name: "setVegetationMask", args: [false, 2, 2, trees.data] },
      { name: "setVegetationMask", args: [true, 0, 0, undefined] }
    ]);
    const before = calls.length;
    engine.setVegetationMasks({});
    expect(calls.length).toBe(before);

    // Copies, so later edits by the caller change nothing.
    data[0] = 1;
    const painted = engine.getPaintedMaps();
    expect(painted.biome!.data[0]).toBe(255);
    expect(painted.biome!.borderSamples).toBe(3);
    expect(painted.trees!.data).toEqual(trees.data);
    expect(painted.grass).toBeUndefined();

    engine.setBiomeMap(null);
    expect(calls.at(-1)).toEqual({ name: "setBiomeMap", args: [0, 0, undefined, 0] });
    expect(engine.getPaintedMaps().biome).toBeUndefined();
  });

  it("validates every boundary rule", () => {
    const map = (width: number, height: number, length = width * height) => ({ width, height, data: new Uint8Array(length) });

    for (const bad of [map(1, 4), map(4, 8193), map(3, 3, 8)]) {
      expect(() => engine.setBiomeMap(bad)).toThrow(expect.objectContaining({ code: "OPTIONS_INVALID" }));
      expect(() => engine.setVegetationMasks({ grass: bad })).toThrow(expect.objectContaining({ code: "OPTIONS_INVALID" }));
    }

    for (const bad of [{ width: 2.5, height: 2, data: new Uint8Array(5) }, { width: 2, height: 2, data: [0, 0, 0, 0] }, undefined]) {
      expect(() => engine.setBiomeMap(bad as never)).toThrow(TypeError);

      if (bad) {
        // Neither mask is set when one is bad.
        const before = calls.length;
        expect(() => engine.setVegetationMasks({ grass: map(2, 2), trees: bad as never })).toThrow(TypeError);
        expect(calls.length).toBe(before);
      }
    }

    for (const borderSamples of [-1, 9, 1.5]) {
      expect(() => engine.setBiomeMap({ ...map(2, 2), borderSamples })).toThrow(/borderSamples must be a whole number from 0 to 8/);
    }

    expect(() => engine.setVegetationMasks(null as never)).toThrow(TypeError);
  });

  it("clears painted maps when new terrain loads", async () => {
    engine.setVegetationMasks({ grass: { width: 2, height: 2, data: new Uint8Array(4) } });
    await engine.loadRawHeightmap(new ArrayBuffer(16), { width: 2, height: 2, sampleFormat: "float32", metresPerSample: 1, heightScaleMetres: 1, landform: "alpine" });
    expect(engine.getPaintedMaps()).toEqual({});
    expect(engine.getOptionsSnapshot().landform).toBe("alpine");
  });
});

/**
 * An engine held in memory: it keeps the heights and painted maps it is
 * given and derives every map from them, as the real engine does, so
 * the bundle layer can be tested end to end without a GPU.
 */
function memoryEngine(): VistaEngine & { events: { name: string; payload: unknown }[] } {
  let heights = new Float32Array(0);
  let size: [number, number] = [0, 0];
  let painted: PaintedMaps = {};
  const options: Record<string, unknown> = {};
  const events: { name: string; payload: unknown }[] = [];
  const map = (kind: MapKind): ExportedMap => {
    const [width, height] = size;
    const channels = kind === "materials" ? 12 : kind === "normals" ? 3 : 1;
    const float = ["height", "sourceHeight", "waterDepth", "flow", "discharge", "slope", "normals", "temperature"].includes(kind);
    const data = float ? new Float32Array(width * height * channels) : new Uint8Array(width * height * channels);
    const seed = JSON.stringify(options).length + (painted.water?.data[0] ?? 0) + (painted.trees?.data[1] ?? 0);

    data.forEach((_, index) => {
      const sample = Math.floor(index / channels);
      // The final heights are the source heights carved a little.
      const base = heights[sample] + (kind === "sourceHeight" ? 0 : 2);

      if (kind === "sourceHeight" || kind === "height") {
        data[index] = base;
        return;
      }

      const paintedBiome = painted.biome?.data[sample];
      data[index] = kind === "biome"
        ? paintedBiome !== undefined && paintedBiome < 19 ? paintedBiome : Math.abs(Math.round(base)) % 19
        : float ? Math.fround(base * (1 + (index % channels)) + seed) : (Math.round(Math.abs(base)) + index + seed) % 256;
    });

    const legend = kind === "biome"
      ? BIOME_COLOURS.map((colour, index) => ({ index, name: BIOME_KINDS[index], colour: colour.map((c) => c / 255) as [number, number, number] }))
      : undefined;
    return {
      kind,
      width,
      height,
      channels,
      type: float ? "float32" : "uint8",
      data,
      encoding: { legend, range: float ? [Math.min(...data), Math.max(...data)] : undefined, metresPerPixel: [12, 12], seaLevelMetres: 0, generator: "memory" }
    };
  };
  const store = (key: string) => (value: unknown) => {
    options[key] = value;
  };

  return {
    events,
    emit: (name: string, payload: unknown) => events.push({ name, payload }),
    exportMap: map,
    exportTrees: () => [],
    getOptionsSnapshot: () => JSON.parse(JSON.stringify(options)),
    getPaintedMaps: () => JSON.parse(JSON.stringify(painted, (_, value) => value instanceof Uint8Array ? Array.from(value) : value), (key, value) => key === "data" ? Uint8Array.from(value) : value),
    loadRawHeightmap: async (buffer: ArrayBuffer, raw: RawHeightmapOptions) => {
      heights = new Float32Array(buffer.slice(0));
      size = [raw.width, raw.height];
      painted = {};
      options.landform = raw.landform;
      return { id: 1, metadata: { warnings: [] } };
    },
    setWaterMask: (mask: WaterMask | null) => {
      painted.water = mask ?? undefined;
    },
    setBiomeMap: (biome: BiomeMap | null) => {
      painted.biome = biome ? { borderSamples: 3, ...biome } : undefined;
    },
    setVegetationMasks: (masks: { trees?: DensityMask | null; grass?: DensityMask | null }) => {
      painted = { ...painted, ...masks } as PaintedMaps;
    },
    setCamera: store("camera"),
    setSun: store("sun"),
    setFlora: store("flora"),
    setGrass: store("grass"),
    setBiomes: store("biomes"),
    setWater: store("water"),
    setWeather: store("weather"),
    setRenderQuality: store("quality")
  } as unknown as VistaEngine & { events: { name: string; payload: unknown }[] };
}

/** Every entry of a zip, by name, with its bytes. */
async function entries(bundle: Blob): Promise<Map<string, Uint8Array>> {
  const bytes = new Uint8Array(await bundle.arrayBuffer());
  const view = new DataView(bytes.buffer);
  const end = bytes.length - 22;
  const out = new Map<string, Uint8Array>();
  let at = view.getUint32(end + 16, true);

  for (let index = 0; index < view.getUint16(end + 10, true); index += 1) {
    const name = new TextDecoder().decode(bytes.subarray(at + 46, at + 46 + view.getUint16(at + 28, true)));
    const local = view.getUint32(at + 42, true);
    const start = local + 30 + view.getUint16(local + 26, true);
    const body = bytes.subarray(start, start + view.getUint32(at + 20, true));
    out.set(name, view.getUint16(at + 10, true) === 8 ? new Uint8Array(zlib.inflateRawSync(body)) : body);
    at += 46 + view.getUint16(at + 28, true);
  }

  return out;
}

/** A scene with a painted biome map, water, tree and grass masks and options. */
async function paintedScene(width = 12, height = 9): Promise<ReturnType<typeof memoryEngine>> {
  const scene = memoryEngine();
  const heights = Float32Array.from({ length: width * height }, (_, index) => Math.sin(index * 0.7) * 300 + 40);
  await scene.loadRawHeightmap(heights.buffer, { width, height, sampleFormat: "float32", metresPerSample: 12, heightScaleMetres: 1, landform: "fjords" });
  scene.setFlora({ density: 2, seedOffset: 9 } as never);
  scene.setBiomes({ meanTemperatureCelsius: 4 } as never);
  scene.setCamera({ position: [1, 2, 3], target: [0, 0, 0] } as never);
  const grid = (fill: (index: number) => number) => ({ width, height, data: Uint8Array.from({ length: width * height }, (_, index) => fill(index)) });
  scene.setBiomeMap({ ...grid((index) => (index % width > width / 2 ? 8 : 255)), borderSamples: 5 });
  scene.setWaterMask(grid((index) => (index === 20 ? 200 : index % 7 === 0 ? 40 : 0)));
  scene.setVegetationMasks({ trees: grid((index) => (index * 11) % 256), grass: { width: 5, height: 4, data: Uint8Array.from({ length: 20 }, (_, index) => index * 12) } });
  return scene;
}

describe("bundles", () => {
  it("leave the scene as it was when the bundle is damaged", async () => {
    const bundle = await exportBundle(await paintedScene(), { trees: false });
    const scene = async () => {
      const engine = memoryEngine();
      await engine.loadRawHeightmap(Float32Array.from({ length: 16 }, (_, index) => index).buffer, { width: 4, height: 4, sampleFormat: "float32", metresPerSample: 30, heightScaleMetres: 1 });
      engine.setFlora({ density: 1 } as never);
      return engine;
    };
    const state = (engine: VistaEngine) => [engine.getOptionsSnapshot(), Array.from(engine.exportMap("sourceHeight").data)];

    // A setting the engine refuses is found before anything changes.
    const checked = Object.assign(await scene(), {
      checkSettings: (options: { flora?: { density: number } }) => {
        if (options.flora?.density === 2) {
          throw new VistaWasmError("OPTIONS_INVALID", "flora.density is refused here.");
        }
      }
    });
    const before = state(checked);
    await expect(loadBundle(checked, bundle)).rejects.toThrow(/flora.density is refused/);
    expect(state(checked)).toEqual(before);

    // Without the check, a setter that fails after the heights load has
    // the settings applied before it put back: flora is set, then biomes
    // fail.
    const unchecked = await scene();
    unchecked.setBiomes = () => {
      throw new VistaWasmError("OPTIONS_INVALID", "biomes are refused here.");
    };
    await expect(loadBundle(unchecked, bundle)).rejects.toThrow(/biomes are refused/);
    expect(unchecked.getOptionsSnapshot()).toMatchObject({ flora: { density: 1 } });

    // Damaged heights fail before any setting is applied.
    const files = await entries(bundle);
    const short = await zip([...files].map(([name, data]) => ({ name, data: (name === "source-height.f32" ? data.slice(0, 8) : data.slice()) as Uint8Array<ArrayBuffer> })));
    const damaged = await scene();
    const unchanged = state(damaged);
    await expect(loadBundle(damaged, short)).rejects.toMatchObject({ code: "INVALID_DEM" });
    expect(state(damaged)).toEqual(unchanged);
  });

  it("check the painted maps' sizes from their headers before decoding any", async () => {
    const bundle = await entries(await exportBundle(await paintedScene(), { trees: false }));
    // A PNG header claiming 4096 x 4096 RGBA at 16 bits (128 MiB decoded),
    // with no pixels: past the largest terrain, so it is refused from its
    // header. Decoding it would fail on its missing IDAT chunk instead.
    const header = new Uint8Array(13);
    new DataView(header.buffer).setUint32(0, 4096);
    new DataView(header.buffer).setUint32(4, 4096);
    header.set([16, 6], 8);
    const huge = new Uint8Array(Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk("IHDR", header), chunk("IEND", new Uint8Array(0))]));
    const swapped = await zip([...bundle].map(([name, data]) => ({
      name,
      data: (name === "water-mask.png" ? huge : data.slice()) as Uint8Array<ArrayBuffer>
    })));
    await expect(loadBundle(memoryEngine(), swapped)).rejects.toThrow(
      /4096 x 4096, but painted maps \(WebAssembly's limit on terrain size\) must be from 2 to 2048/
    );
    // Four maps at 2048 decode to at most 128 MiB, so this budget is a
    // second guard only.
    expect(BUNDLE_LIMITS.decodedBytes).toBe(2 ** 30);
  });

  it("hold terrains and painted maps to the 2048-sample limit", async () => {
    // A 4096 x 4096 PNG header with no pixels: refused from its header,
    // before decoding, which would fail on the missing IDAT chunk instead.
    const header = new Uint8Array(13);
    new DataView(header.buffer).setUint32(0, 4096);
    new DataView(header.buffer).setUint32(4, 4096);
    header.set([16, 0], 8);
    const png = new Uint8Array(Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk("IHDR", header), chunk("IEND", new Uint8Array(0))]));
    await expect(decodeImage(png, "The heightmap image", () => undefined, MAX_TERRAIN_SIDE, "heightmaps")).rejects.toThrow(
      /4096 x 4096, but heightmaps must be from 2 to 2048 pixels a side/
    );

    for (const read of [() => biomeMapFromImage(png), () => waterMaskFromImage(png), () => densityMaskFromImage(png)]) {
      await expect(read()).rejects.toThrow(/4096 x 4096, but painted maps \(WebAssembly's limit on terrain size\) must be from 2 to 2048/);
    }

    // Image decoding itself takes up to 8192, for images that are not
    // terrain: it gets past the size check and fails on the data.
    await expect(decodeImage(png, "The image", () => undefined)).rejects.not.toThrow(/must be from 2 to/);

    const bundle = await entries(await exportBundle(await paintedScene(), { trees: false }));
    const manifest = parseManifest(bundle.get("manifest.json")!);
    const huge = { ...manifest, terrain: { ...(manifest.terrain as object), width: 4096, height: 4096 } };
    expect(() => parseManifest(new TextEncoder().encode(JSON.stringify(huge)))).toThrow(/width and height from 2 to 2048/);
    expect(MAX_TERRAIN_SIDE).toBe(2048);
  });

  it("round-trip exactly: export, load into a fresh engine, export again", async () => {
    const first = await exportBundle(await paintedScene(), { trees: false });
    const copy = memoryEngine();
    await loadBundle(copy, first);
    const second = await exportBundle(copy, { trees: false });
    const [a, b] = [await entries(first), await entries(second)];
    expect([...b.keys()]).toEqual([...a.keys()]);
    expect([...a.keys()]).toEqual(expect.arrayContaining(["source-height.f32", "painted-biome.png", "water-mask.png", "tree-mask.png", "grass-mask.png"]));

    for (const [name, bytes] of a) {
      if (name !== "manifest.json") {
        expect(Array.from(b.get(name)!), name).toEqual(Array.from(bytes));
      }
    }

    // The options come back in the order the setters ran, so the
    // manifest is compared by its fields.
    const [before, after] = [parseManifest(a.get("manifest.json")!), parseManifest(b.get("manifest.json")!)];
    expect(after).toEqual(before);
    expect(copy.events.filter((event) => event.name === "progress").map((event) => event.payload)).toEqual(
      [0, 0.3, 0.7, 1].map((progress) => ({ phase: "bundle", progress }))
    );
  });

  it("writes version 2 with the source heights, the landform and the painted maps", async () => {
    const files = await entries(await exportBundle(await paintedScene(), { trees: false }));
    const manifest = parseManifest(files.get("manifest.json")!);
    expect(manifest.version).toBe(2);
    expect(manifest.terrain).toMatchObject({ width: 12, height: 9, landform: "fjords" });
    const described = manifest.files as { path: string; kind: string; borderSamples?: number }[];
    expect(described.find((file) => file.path === "painted-biome.png")).toMatchObject({ kind: "paintedBiome", borderSamples: 5 });
    const biome = await decodePng(files.get("painted-biome.png")!);
    expect(biome.palette![255 * 4 + 3]).toBe(0);
    expect(Array.from(biome.palette!.subarray(8 * 4, 8 * 4 + 3))).toEqual(BIOME_COLOURS[8]);
    const grass = await decodePng(files.get("grass-mask.png")!);
    expect([grass.width, grass.height, grass.channels]).toEqual([5, 4, 1]);
  });

  it("applies the settings unless told not to", async () => {
    const bundle = await exportBundle(await paintedScene(), { trees: false });
    const plain = memoryEngine();
    await loadBundle(plain, bundle, { applySettings: false });
    expect(plain.getOptionsSnapshot()).toEqual({ landform: "fjords" });
    const full = memoryEngine();
    await loadBundle(full, bundle);
    expect(full.getOptionsSnapshot()).toMatchObject({ flora: { density: 2, seedOffset: 9 }, camera: { position: [1, 2, 3] } });
    await expect(loadBundle(full, bundle, { applySettings: "no" } as never)).rejects.toThrow(TypeError);
  });

  it("still reads version 1 bundles, from their final heights, with a warning", async () => {
    const scene = await paintedScene();
    const bundle = await entries(await exportBundle(scene, { trees: false }));
    const manifest = parseManifest(bundle.get("manifest.json")!);
    const v1 = await zip([
      { name: "manifest.json", data: new TextEncoder().encode(JSON.stringify({ ...manifest, version: 1, files: (manifest.files as { kind: string }[]).filter((file) => file.kind === "height") })) },
      { name: "height.f32", data: bundle.get("height.f32")!.slice() as Uint8Array<ArrayBuffer>, store: true }
    ]);
    const copy = memoryEngine();
    await loadBundle(copy, v1);
    const heights = scene.exportMap("height").data as Float32Array;
    expect(Array.from(copy.exportMap("sourceHeight").data)).toEqual(Array.from(heights));
    expect(copy.events.find((event) => event.name === "warning")).toMatchObject({ payload: { message: expect.stringContaining("version 1") } });
  });

  it("rejects unknown versions, corrupt CRCs and prototype keys", async () => {
    const bundle = await entries(await exportBundle(await paintedScene(), { trees: false }));
    const text = new TextDecoder().decode(bundle.get("manifest.json")!);
    const withManifest = (json: string) => zip([{ name: "manifest.json", data: new TextEncoder().encode(json) }]);

    await expect(loadBundle(memoryEngine(), await withManifest(text.replace('"version": 2', '"version": 3')))).rejects.toMatchObject({
      code: "INVALID_DEM",
      message: expect.stringContaining("version 3")
    });

    for (const key of ["__proto__", "constructor", "prototype"]) {
      await expect(loadBundle(memoryEngine(), await withManifest(text.replace('"terrain": {', `"terrain": { "${key}": { "polluted": true },`)))).rejects.toMatchObject({
        code: "INVALID_DEM",
        message: expect.stringContaining(`"${key}"`)
      });
    }

    expect(({} as { polluted?: boolean }).polluted).toBeUndefined();
    await expect(loadBundle(memoryEngine(), await withManifest("{"))).rejects.toMatchObject({ code: "INVALID_DEM" });
    await expect(loadBundle(memoryEngine(), await withManifest(`${" ".repeat(BUNDLE_LIMITS.manifestBytes)}{}`))).rejects.toThrow(/over the limit/);

    // Flip one byte of the stored heights: the CRC catches it.
    const bytes = new Uint8Array(await (await exportBundle(await paintedScene(), { trees: false })).arrayBuffer());
    const at = Buffer.from(bytes).indexOf("source-height.f32") + 17;
    bytes[at + 4] ^= 0xff;
    await expect(loadBundle(memoryEngine(), bytes)).rejects.toMatchObject({ code: "INVALID_DEM" });
  });

  it("treats manifest paths as lookup keys only and ignores unknown files", async () => {
    const bundle = await entries(await exportBundle(await paintedScene(), { trees: false }));
    const manifest = parseManifest(bundle.get("manifest.json")!);
    const files = (manifest.files as { path: string; kind: string }[])
      .filter((file) => file.kind === "sourceHeight")
      .map((file) => ({ ...file, path: "../../etc/source-height.f32" }));
    const renamed = await zip([
      { name: "manifest.json", data: new TextEncoder().encode(JSON.stringify({ ...manifest, files })) },
      { name: "../../etc/source-height.f32", data: bundle.get("source-height.f32")!.slice() as Uint8Array<ArrayBuffer> },
      { name: "unknown.bin", data: new Uint8Array(10) }
    ]);
    const copy = memoryEngine();
    await loadBundle(copy, renamed);
    expect(copy.exportMap("sourceHeight").width).toBe(12);

    const missing = await zip([{ name: "manifest.json", data: new TextEncoder().encode(JSON.stringify({ ...manifest, files: [{ path: "nowhere", kind: "sourceHeight" }] })) }]);
    await expect(loadBundle(memoryEngine(), missing)).rejects.toThrow(/has no "nowhere"/);
  });

  it("caps the entry count, each entry's size and the total", async () => {
    const many = await zip(Array.from({ length: BUNDLE_LIMITS.entries + 1 }, (_, index) => ({ name: `file${index}`, data: new Uint8Array(1) })));
    await expect(loadBundle(memoryEngine(), many)).rejects.toThrow(/65 files, over the limit of 64/);

    // A tiny zip whose directory claims a 4 GiB file.
    const small = new Uint8Array(await (await zip([{ name: "manifest.json", data: new Uint8Array(3) }])).arrayBuffer());
    const view = new DataView(small.buffer);
    const directory = view.getUint32(small.length - 6, true);
    view.setUint32(directory + 24, 0xffffffff, true);
    await expect(loadBundle(memoryEngine(), small)).rejects.toThrow(/inflates to 4294967295 bytes, over the limit/);

    // A deflate bomb: 100 bytes claimed, far more inside.
    const bomb = new Uint8Array(await (await zip([{ name: "manifest.json", data: new Uint8Array(4 * 2 ** 20) }])).arrayBuffer());
    const bombView = new DataView(bomb.buffer);
    const record = bombView.getUint32(bomb.length - 6, true);
    bombView.setUint32(record + 24, 100, true);
    await expect(loadBundle(memoryEngine(), bomb)).rejects.toThrow(/more than the 100 bytes/);

    const saved = BUNDLE_LIMITS.totalBytes;
    BUNDLE_LIMITS.totalBytes = 1000;

    try {
      await expect(loadBundle(memoryEngine(), await exportBundle(await paintedScene(), { trees: false }))).rejects.toThrow(/inflate to over 1000 bytes/);
    } finally {
      BUNDLE_LIMITS.totalBytes = saved;
    }
  });

  it("loads a heightmap image with its painted maps in one call", async () => {
    const scene = memoryEngine();
    const grey = (value: number) => makePng({ width: 2, height: 2, colourType: 0, bitDepth: 8, samples: [value, value, value, value] });
    await loadTerrainFromImages(scene, {
      height: makePng({ width: 2, height: 2, colourType: 0, bitDepth: 16, samples: [0, 100, 200, 65535], text: { "vistawasm:range": "[0,10]" } }),
      biome: colourPng([[...BIOME_COLOURS[1]], [...BIOME_COLOURS[2]], [...BIOME_COLOURS[3]], [255, 0, 255]]),
      water: grey(0),
      trees: grey(128),
      grass: grey(255)
    }, { metresPerSample: 5 });
    const painted = scene.getPaintedMaps();
    expect(firstRow(painted.biome!.data)).toEqual([1, 2, 3, expect.any(Number)]);
    expect(Array.from(painted.trees!.data)).toEqual([128, 128, 128, 128]);
    expect(Array.from(painted.grass!.data)).toEqual([255, 255, 255, 255]);
    expect(scene.events.filter((event) => event.name === "warning")).toHaveLength(1);
    await expect(loadTerrainFromImages(scene, {} as never, { metresPerSample: 5 })).rejects.toThrow(TypeError);
  });
});

/** Expect `run` to resolve, or to reject with a typed VistaWASM error. */
async function survives(run: () => Promise<unknown>, counts: Record<string, number>): Promise<void> {
  try {
    await run();
    counts.accepted += 1;
  } catch (error) {
    if (!(error instanceof VistaWasmError)) {
      throw error;
    }

    counts[error.code] = (counts[error.code] ?? 0) + 1;
  }
}

/** Rewrite each chunk's CRC, so a mutation reaches past the CRC check. */
function fixCrcs(bytes: Uint8Array): Uint8Array {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);

  for (let at = 8; at + 12 <= bytes.length;) {
    const length = view.getUint32(at);

    if (length > bytes.length - at - 12) {
      break;
    }

    view.setUint32(at + 8 + length, crc32(bytes.subarray(at + 4, at + 8 + length)));
    at += length + 12;
  }

  return bytes;
}

describe("fuzzing", () => {
  it("decodes 10,000 malformed PNGs without a crash or a hang", async () => {
    const next = random(0x5eed);
    const int = (below: number) => Math.floor(next() * below);
    const bases = [
      makePng({ width: 9, height: 7, colourType: 0, bitDepth: 16, samples: Array.from({ length: 63 }, (_, i) => i * 997), text: { "vistawasm:range": "[0,1]" } }),
      makePng({ width: 5, height: 5, colourType: 3, bitDepth: 4, samples: Array.from({ length: 25 }, (_, i) => i % 6), palette: Array.from({ length: 18 }, (_, i) => i * 9), trns: [0, 255] }),
      makePng({ width: 6, height: 4, colourType: 6, bitDepth: 8, samples: Array.from({ length: 96 }, (_, i) => (i * 31) % 256), interlace: true }),
      makePng({ width: 3, height: 3, colourType: 2, bitDepth: 8, samples: Array.from({ length: 27 }, (_, i) => i) })
    ];
    const counts: Record<string, number> = { accepted: 0 };

    for (let round = 0; round < 10_000; round += 1) {
      const bytes = bases[round % bases.length].slice();
      const kind = round % 4;

      if (kind === 0) {
        // Random bytes flipped anywhere.
        for (let flips = 1 + int(8); flips > 0; flips -= 1) {
          bytes[int(bytes.length)] ^= 1 + int(255);
        }
      } else if (kind === 1) {
        // Header fields and chunk lengths changed, CRCs kept valid.
        const at = 8 + int(Math.min(bytes.length - 8, 40));
        bytes[at] = int(256);
        bytes[16 + int(13)] = int(256);
        fixCrcs(bytes);
      } else if (kind === 2) {
        // The image data replaced with random bytes, compressed or not.
        const view = new DataView(bytes.buffer);
        const idat = Buffer.from(bytes).indexOf("IDAT") - 4;
        const length = view.getUint32(idat);

        for (let index = 0; index < length; index += 1) {
          bytes[idat + 8 + index] = next() < 0.5 ? int(256) : bytes[idat + 8 + index];
        }

        fixCrcs(bytes);
      } else {
        // Truncated, or with junk appended.
        const cut = bytes.subarray(0, int(bytes.length));
        await survives(() => decodePng(next() < 0.5 ? cut : new Uint8Array([...bytes, ...cut])), counts);
        continue;
      }

      await survives(() => decodePng(bytes), counts);
    }

    const total = Object.values(counts).reduce((sum, count) => sum + count, 0);
    expect(total).toBe(10_000);
    expect(counts.INVALID_DEM).toBeGreaterThan(5_000);
    expect(Object.keys(counts).sort()).toEqual(["INVALID_DEM", "accepted"]);
  }, 120_000);

  it("loads 10,000 malformed bundles without a crash or a hang", async () => {
    const next = random(0xb0d1e);
    const int = (below: number) => Math.floor(next() * below);
    const scene = await paintedScene(6, 5);
    const base = new Uint8Array(await (await exportBundle(scene, { trees: false })).arrayBuffer());
    const files = await entries(new Blob([base]));
    const manifestText = new TextDecoder().decode(files.get("manifest.json")!);
    const directory = new DataView(base.buffer).getUint32(base.length - 6, true);
    const counts: Record<string, number> = { accepted: 0 };

    for (let round = 0; round < 10_000; round += 1) {
      const kind = round % 3;
      let bytes: Uint8Array;

      if (kind === 0) {
        // Random bytes flipped anywhere.
        bytes = base.slice();

        for (let flips = 1 + int(6); flips > 0; flips -= 1) {
          bytes[int(bytes.length)] ^= 1 + int(255);
        }
      } else if (kind === 1) {
        // Directory fields changed: sizes, offsets, counts and names.
        bytes = base.slice();
        const view = new DataView(bytes.buffer);
        const field = [directory + int(bytes.length - 22 - directory), bytes.length - 22 + int(22)][int(2)];
        view.setUint8(field, int(256));

        if (next() < 0.5) {
          view.setUint32(Math.min(field, bytes.length - 4), int(2 ** 32), true);
        }
      } else {
        // A valid zip around a damaged manifest or damaged files, so the
        // mutation reaches past the CRCs.
        const text = manifestText.split("");

        for (let edits = 1 + int(4); edits > 0; edits -= 1) {
          text[int(text.length)] = ' {}[]",:0123456789-eE.x_'[int(24)];
        }

        const names = [...files.keys()].filter((name) => name !== "manifest.json");
        const damaged = names[int(names.length)];
        const body = files.get(damaged)!.slice() as Uint8Array<ArrayBuffer>;
        body[int(body.length)] ^= 1 + int(255);
        bytes = new Uint8Array(await (await zip([
          { name: "manifest.json", data: new TextEncoder().encode(next() < 0.5 ? text.join("") : manifestText) },
          ...names.map((name) => ({ name, data: name === damaged ? body : files.get(name)!.slice() as Uint8Array<ArrayBuffer>, store: true }))
        ])).arrayBuffer());
      }

      await survives(() => loadBundle(memoryEngine(), bytes), counts);
    }

    const total = Object.values(counts).reduce((sum, count) => sum + count, 0);
    expect(total).toBe(10_000);
    expect(counts.INVALID_DEM).toBeGreaterThan(5_000);
    expect(Object.keys(counts).sort()).toEqual(["INVALID_DEM", "accepted"]);
  }, 300_000);
});
