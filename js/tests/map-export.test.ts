import zlib from "node:zlib";
import { beforeAll, describe, expect, it, vi } from "vitest";
import { createVistaEngine, initialiseVistaWasm } from "../src/index";
import {
  MAP_KINDS,
  checkZip32,
  crc32,
  encodePng,
  encodeRaw,
  exportBundle,
  treesToCsv,
  treesToJson
} from "../src/map-export";
import type {
  ExportedMap,
  MapKind,
  TreeRecord,
  VistaEngine,
  VistaWasmGeneratedModule
} from "../src/types";

const FLOAT_KINDS = ["height", "waterDepth", "flow", "discharge", "slope", "normals", "temperature"];

/** A synthetic map of `kind`, as `exportMap()` returns it. */
function fakeMap(kind: MapKind, width = 5, height = 3): ExportedMap {
  const channels = kind === "materials" ? 12 : kind === "normals" ? 3 : 1;
  const count = width * height * channels;
  const float = FLOAT_KINDS.includes(kind);
  const data = float ? new Float32Array(count) : new Uint8Array(count);

  for (let index = 0; index < count; index += 1) {
    data[index] = float ? Math.fround(Math.sin(index) * 800 - 20) : (index * 37) % 256;
  }

  if (kind === "normals") {
    for (let index = 0; index < count; index += 3) {
      const [x, z] = [Math.sin(index) * 0.5, Math.cos(index) * 0.3];
      const y = Math.sqrt(1 - x * x - z * z);
      data.set([x, y, z], index);
    }
  }

  if (kind === "biome" || kind === "water") {
    data.forEach((_, index) => (data[index] = index % (kind === "biome" ? 19 : 7)));
  }

  const legend = kind === "biome" || kind === "water"
    ? Array.from({ length: kind === "biome" ? 19 : 7 }, (_, index) => ({
      index,
      name: `entry${index}`,
      colour: [index / 20, 0.5, 1 - index / 20] as [number, number, number]
    }))
    : undefined;
  let range: [number, number] | undefined;

  if (float) {
    range = [Math.min(...data), Math.max(...data)];
  }

  return {
    kind,
    width,
    height,
    channels,
    type: float ? "float32" : "uint8",
    data,
    encoding: {
      legend,
      range,
      metresPerPixel: [12, 12],
      seaLevelMetres: 0,
      generator: "vistawasm-fractal-0.2.0"
    }
  };
}

interface Png {
  width: number;
  height: number;
  bitDepth: number;
  colourType: number;
  palette?: Uint8Array;
  text: Record<string, string>;
  pixels: Uint8Array;
}

/** Parse a PNG, checking its signature and every chunk's CRC, and undo its filters. */
async function readPng(blob: Blob): Promise<Png> {
  const bytes = new Uint8Array(await blob.arrayBuffer());
  expect(Array.from(bytes.subarray(0, 8))).toEqual([137, 80, 78, 71, 13, 10, 26, 10]);
  const view = new DataView(bytes.buffer);
  const idat: Uint8Array[] = [];
  const png: Partial<Png> = { text: {} };
  let at = 8;

  for (;;) {
    const length = view.getUint32(at);
    const type = String.fromCharCode(...bytes.subarray(at + 4, at + 8));
    const data = bytes.subarray(at + 8, at + 8 + length);
    expect(view.getUint32(at + 8 + length)).toBe(zlib.crc32(bytes.subarray(at + 4, at + 8 + length)));

    if (type === "IHDR") {
      Object.assign(png, {
        width: view.getUint32(at + 8),
        height: view.getUint32(at + 12),
        bitDepth: data[8],
        colourType: data[9]
      });
    } else if (type === "PLTE") {
      png.palette = data.slice();
    } else if (type === "tEXt") {
      const [key, value] = new TextDecoder().decode(data).split("\0");
      png.text![key] = value;
    } else if (type === "IDAT") {
      idat.push(data);
    }

    at += length + 12;

    if (type === "IEND") {
      break;
    }
  }

  expect(at).toBe(bytes.length);
  const raw = zlib.inflateSync(Buffer.concat(idat));
  const channels = { 0: 1, 2: 3, 3: 1, 6: 4 }[png.colourType!]!;
  const stride = (png.width! * channels * png.bitDepth!) / 8;
  const pixels = new Uint8Array(stride * png.height!);

  for (let y = 0; y < png.height!; y += 1) {
    const filter = raw[y * (stride + 1)];
    expect([0, 2]).toContain(filter);

    for (let x = 0; x < stride; x += 1) {
      const value = raw[y * (stride + 1) + 1 + x];
      const above = filter === 2 && y > 0 ? pixels[(y - 1) * stride + x] : 0;
      pixels[y * stride + x] = (value + above) & 255;
    }
  }

  return { ...(png as Png), pixels };
}

describe("encodePng", () => {
  it("writes byte maps as 8-bit grey exactly, with either filter", async () => {
    const map = fakeMap("occlusion", 7, 4);

    for (const smaller of [false, true]) {
      const png = await readPng((await encodePng(map, { smaller })) as Blob);
      expect([png.width, png.height, png.bitDepth, png.colourType]).toEqual([7, 4, 8, 0]);
      expect(Array.from(png.pixels)).toEqual(Array.from(map.data));
    }
  });

  it("writes 16-bit grey big-endian over the range, stored in a text chunk", async () => {
    const map = fakeMap("height", 9, 6);
    const blob = (await encodePng(map, { bitDepth: 16 })) as Blob & { range?: [number, number] };
    const png = await readPng(blob);
    const [low, high] = map.encoding.range!;
    expect([png.bitDepth, png.colourType]).toEqual([16, 0]);
    expect(blob.range).toEqual([low, high]);
    expect(JSON.parse(png.text["vistawasm:range"])).toEqual([low, high]);

    map.data.forEach((value, index) => {
      const sample = (png.pixels[index * 2] << 8) | png.pixels[index * 2 + 1];
      // Within one step of 65535 over the range.
      expect(Math.abs(low + (sample / 65535) * (high - low) - value)).toBeLessThanOrEqual((high - low) / 65535);
    });
  });

  it("scales float maps to 8 bits over a given range", async () => {
    const map = fakeMap("slope", 4, 4);
    map.data.forEach((_, index) => (map.data[index] = index * 6));
    const png = await readPng((await encodePng(map, { range: [0, 90] })) as Blob);
    expect(Array.from(png.pixels)).toEqual(Array.from(map.data, (value) => Math.round(Math.min(value / 90, 1) * 255)));
  });

  it("writes normals as RGB", async () => {
    const map = fakeMap("normals", 6, 5);
    const png = await readPng((await encodePng(map)) as Blob);
    expect(png.colourType).toBe(2);
    expect(Array.from(png.pixels)).toEqual(Array.from(map.data, (n) => Math.round((n + 1) * 127.5)));
  });

  it("splits the twelve materials into three RGBA files", async () => {
    const map = fakeMap("materials", 3, 2);
    const blobs = (await encodePng(map)) as Blob[];
    expect(blobs).toHaveLength(3);

    for (const [file, blob] of blobs.entries()) {
      const png = await readPng(blob);
      expect(png.colourType).toBe(6);

      for (let pixel = 0; pixel < 6; pixel += 1) {
        expect(Array.from(png.pixels.subarray(pixel * 4, pixel * 4 + 4))).toEqual(
          Array.from(map.data.subarray(pixel * 12 + file * 4, pixel * 12 + file * 4 + 4))
        );
      }
    }
  });

  it("writes biomes and water as palette images in their legend colours", async () => {
    for (const kind of ["biome", "water"] as const) {
      const map = fakeMap(kind, 8, 3);
      const png = await readPng((await encodePng(map)) as Blob);
      expect(png.colourType).toBe(3);
      expect(Array.from(png.pixels)).toEqual(Array.from(map.data));
      const legend = map.encoding.legend!;
      expect(png.palette!.length).toBe(legend.length * 3);
      expect(Array.from(png.palette!.subarray(3, 6))).toEqual(legend[1].colour.map((c) => Math.round(c * 255)));
    }
  });

  it("rejects options it cannot honour", async () => {
    const height = fakeMap("height");
    await expect(encodePng(height, { bitDepth: 12 as 8 })).rejects.toMatchObject({ code: "OPTIONS_INVALID" });
    await expect(encodePng(fakeMap("normals"), { bitDepth: 16 })).rejects.toMatchObject({ code: "OPTIONS_INVALID" });
    await expect(encodePng(height, { range: [5, 1] })).rejects.toMatchObject({ code: "OPTIONS_INVALID" });
    await expect(encodePng(height, { range: [0, Number.NaN] })).rejects.toMatchObject({ code: "OPTIONS_INVALID" });
    await expect(encodePng({ ...height, width: 6 })).rejects.toThrow(TypeError);
  });
});

describe("encodeRaw", () => {
  it("writes float32 exactly and uint16 over a range", () => {
    const map = fakeMap("height", 10, 10);
    const { bytes } = encodeRaw(map);
    expect(Array.from(new Float32Array(bytes.buffer))).toEqual(Array.from(map.data));

    const { bytes: words, range } = encodeRaw(map, { type: "uint16" });
    const [low, high] = range!;
    const samples = new Uint16Array(words.buffer);
    expect(samples.length).toBe(100);
    expect(Math.max(...samples)).toBe(65535);
    expect(Math.min(...samples)).toBe(0);
    map.data.forEach((value, index) => {
      expect(Math.abs(low + (samples[index] / 65535) * (high - low) - value)).toBeLessThanOrEqual((high - low) / 65535);
    });

    expect(() => encodeRaw(map, { type: "int8" as "uint16" })).toThrow(/float32/);
    // Past the largest terrain no map comes from exportMap(), which keeps
    // the largest raw output (twelve floats a pixel at 2048) far under 1 GiB.
    const huge = { ...fakeMap("materials", 2, 2), width: 4096, height: 4096, data: new Uint8Array(4096 * 4096 * 12) };
    expect(() => encodeRaw(huge)).toThrow(/width and height whole numbers from 1 to 2048/);
  });
});

const trees: TreeRecord[] = [
  { x: 1.5, y: 20.25, z: -3, species: "oak", variant: 2, scale: 1.1, rotation: 0.3, tint: 0.5, dryness: 0.25 },
  { x: -40, y: 12, z: 7.125, species: "pine", variant: 0, scale: 0.8, rotation: 6, tint: 0.4, dryness: 0, handPlaced: true }
];

describe("trees as text", () => {
  it("writes CSV with a header row and a row a tree", () => {
    const lines = treesToCsv(trees).trim().split("\n");
    expect(lines[0]).toBe("x,y,z,species,variant,scale,rotation,tint,dryness,handPlaced");
    expect(lines).toHaveLength(3);
    expect(lines[1]).toBe("1.5,20.25,-3,oak,2,1.1,0.3,0.5,0.25,false");
    expect(lines[2].endsWith(",true")).toBe(true);
  });

  it("writes JSON that parses back to the same records", () => {
    expect(JSON.parse(treesToJson(trees))).toEqual(trees);
  });
});

interface ZipEntry {
  name: string;
  method: number;
  data: Uint8Array;
}

/** Read a zip from its central directory, checking every entry's CRC. */
async function readZip(blob: Blob): Promise<Map<string, ZipEntry>> {
  const bytes = new Uint8Array(await blob.arrayBuffer());
  const view = new DataView(bytes.buffer);
  const end = bytes.length - 22;
  expect(view.getUint32(end, true)).toBe(0x06054b50);
  const count = view.getUint16(end + 10, true);
  let at = view.getUint32(end + 16, true);
  expect(at + view.getUint32(end + 12, true)).toBe(end);
  const entries = new Map<string, ZipEntry>();

  for (let index = 0; index < count; index += 1) {
    expect(view.getUint32(at, true)).toBe(0x02014b50);
    const method = view.getUint16(at + 10, true);
    const crc = view.getUint32(at + 16, true);
    const packed = view.getUint32(at + 20, true);
    const size = view.getUint32(at + 24, true);
    const nameLength = view.getUint16(at + 28, true);
    const local = view.getUint32(at + 42, true);
    const name = new TextDecoder().decode(bytes.subarray(at + 46, at + 46 + nameLength));
    expect(view.getUint32(local, true)).toBe(0x04034b50);
    expect(view.getUint32(local + 14, true)).toBe(crc);
    const start = local + 30 + view.getUint16(local + 26, true);
    const body = bytes.subarray(start, start + packed);
    const data = method === 8 ? new Uint8Array(zlib.inflateRawSync(body)) : body;
    expect(data.length).toBe(size);
    expect(zlib.crc32(data)).toBe(crc);
    entries.set(name, { name, method, data });
    at += 46 + nameLength;
  }

  return entries;
}

/** An engine that only exports, from synthetic maps. */
function fakeEngine(overrides: Partial<VistaEngine> = {}): VistaEngine {
  const height = fakeMap("height", 16, 12);
  return {
    exportMap: (kind: MapKind, options: { size?: [number, number] } = {}) =>
      kind === "height" && !options.size ? height : fakeMap(kind, ...(options.size ?? [16, 12])),
    exportHeightmap: () => new Uint8Array(height.data.buffer.slice(0)),
    exportTrees: () => trees,
    getOptionsSnapshot: () => ({ flora: { density: 2 }, terrain: { seed: 7 } }),
    ...overrides
  } as unknown as VistaEngine;
}

describe("exportBundle", () => {
  it("writes a zip whose manifest describes every file", async () => {
    const engine = fakeEngine();
    const entries = await readZip(await exportBundle(engine, { previews: true }));
    const names = [...entries.keys()];
    expect(names[0]).toBe("manifest.json");
    expect(names).toEqual(expect.arrayContaining([
      "height.f32", "biome.png", "water.png", "flow.f32", "discharge.f32",
      "splat0.png", "splat1.png", "splat2.png", "slope.png", "normals.png",
      "occlusion.png", "temperature.f32", "moisture.png", "treeDensity.png",
      "grassDensity.png", "trees.csv", "preview-height.png", "preview-biome.png"
    ]));

    const manifest = JSON.parse(new TextDecoder().decode(entries.get("manifest.json")!.data));
    expect(manifest.format).toBe("vistawasm-bundle");
    expect(manifest.version).toBe(2);
    expect(manifest.generator).toBe("vistawasm-fractal-0.2.0");
    expect(manifest.terrain).toMatchObject({ width: 16, height: 12, metresPerSample: 12, seaLevelMetres: 0 });
    expect(manifest.options).toEqual({ flora: { density: 2 }, terrain: { seed: 7 } });

    for (const file of manifest.files) {
      expect(entries.has(file.path)).toBe(true);
    }

    const biome = manifest.files.find((file: { path: string }) => file.path === "biome.png");
    expect(biome).toMatchObject({ kind: "biome", type: "uint8", width: 16, height: 12, channels: 1 });
    expect(biome.encoding.legend).toHaveLength(19);
    expect(manifest.files.find((file: { path: string }) => file.path === "splat2.png").firstChannel).toBe(8);
    expect(manifest.files.find((file: { path: string }) => file.path === "slope.png").range).toEqual([0, 90]);

    // The heights are the exported heightmap, byte for byte.
    expect(Array.from(entries.get("height.f32")!.data)).toEqual(Array.from(engine.exportHeightmap()));
    // PNGs are stored, as they are compressed already; the rest deflated.
    expect(entries.get("biome.png")!.method).toBe(0);
    expect(entries.get("flow.f32")!.method).toBe(8);
    await readPng(new Blob([entries.get("splat0.png")!.data]));
    expect(new TextDecoder().decode(entries.get("trees.csv")!.data)).toBe(treesToCsv(trees));
  });

  it("resamples the maps but keeps the heights exact", async () => {
    const entries = await readZip(await exportBundle(fakeEngine(), { size: [20, 30], trees: false }));
    const manifest = JSON.parse(new TextDecoder().decode(entries.get("manifest.json")!.data));
    expect(manifest.files.find((file: { path: string }) => file.path === "flow.f32")).toMatchObject({ width: 20, height: 30 });
    expect(entries.get("height.f32")!.data.length).toBe(16 * 12 * 4);
    expect(entries.has("trees.csv")).toBe(false);
  });

  it("leaves out a forest over the cap unless trees are asked for", async () => {
    const over = () => {
      throw Object.assign(new Error("too many"), { code: "OPTIONS_INVALID" });
    };
    const entries = await readZip(await exportBundle(fakeEngine({ exportTrees: over })));
    expect(entries.has("trees.csv")).toBe(false);
    await expect(exportBundle(fakeEngine({ exportTrees: over }), { trees: true })).rejects.toThrow("too many");
  });

  it("validates its options", async () => {
    await expect(exportBundle(fakeEngine(), { size: [1, 5] })).rejects.toMatchObject({ code: "OPTIONS_INVALID" });
    await expect(exportBundle(fakeEngine(), { maxTrees: 0 })).rejects.toMatchObject({ code: "OPTIONS_INVALID" });
  });
});

describe("zip limits", () => {
  it("rejects anything a zip without ZIP64 cannot record", () => {
    expect(() => checkZip32(2 ** 32 - 1)).not.toThrow();
    expect(() => checkZip32(2 ** 32)).toThrow(/4 GiB/);
  });

  it("computes the CRC-32 zip and PNG use", () => {
    const bytes = new TextEncoder().encode("123456789");
    expect(crc32(bytes)).toBe(0xcbf43926);
    expect(crc32(bytes.subarray(4), crc32(bytes.subarray(0, 4)))).toBe(0xcbf43926);
  });
});

// The generated engine, recording calls, so the wrapper's validation can
// be tested without a GPU.
const calls: { name: string; args: unknown[] }[] = [];
let generating: (() => void) | undefined;

const raw = new Proxy({}, {
  get(_target, name: string) {
    if (name === "then") {
      return undefined;
    }

    return (...args: unknown[]) => {
      calls.push({ name, args });

      if (name === "generateFractal") {
        return new Promise((resolve) => {
          generating = () => resolve({ id: 1, metadata: { warnings: [] } });
        });
      }

      if (name === "loadRawHeightmap") {
        return Promise.resolve({ id: 2, metadata: { warnings: [] } });
      }

      if (name === "exportMap") {
        const map = fakeMap(MAP_KINDS[args[0] as number], (args[1] as number) ?? 4, (args[2] as number) ?? 4);
        const legend = map.encoding.legend ?? [];
        return {
          width: map.width,
          height: map.height,
          channels: map.channels,
          data: map.data,
          units: "m",
          range: map.encoding.range && Float32Array.from(map.encoding.range),
          legendNames: legend.map((entry) => entry.name).join("\n"),
          legendColours: Float32Array.from(legend.flatMap((entry) => entry.colour)),
          metresPerPixel: new Float32Array([12, 12]),
          seaLevelMetres: 0,
          generator: "vistawasm-fractal-0.2.0"
        };
      }

      if (name === "exportTrees") {
        return new Float32Array([1, 2, 3, 4, 3, 1.5, 0.5, 0.25, 0.75, 0, 9, 8, 7, 7, 0, 1, 0, 0.5, 0, 1]);
      }

      return undefined;
    };
  }
});

let engine: VistaEngine;

beforeAll(async () => {
  vi.stubGlobal("navigator", { gpu: {} });
  await initialiseVistaWasm({ wasmModule: { VistaEngine: { create: async () => raw } } as unknown as VistaWasmGeneratedModule });
  engine = await createVistaEngine({} as HTMLCanvasElement, {
    flora: { density: 2, seedOffset: 5n } as never,
    quality: { preset: "balanced" }
  });
});

describe("VistaEngine exports", () => {
  it("names each map's kind and type", () => {
    const map = engine.exportMap("normals", { size: [3, 2] });
    expect(calls.at(-1)).toEqual({ name: "exportMap", args: [8, 3, 2] });
    expect(map).toMatchObject({
      kind: "normals",
      type: "float32",
      width: 3,
      height: 2,
      channels: 3,
      encoding: { units: "m", metresPerPixel: [12, 12], seaLevelMetres: 0, generator: "vistawasm-fractal-0.2.0" }
    });
    expect(map.encoding.legend).toBeUndefined();
    const biome = engine.exportMap("biome");
    expect(biome.type).toBe("uint8");
    expect(calls.at(-1)!.args).toEqual([1, undefined, undefined]);
    // The legend comes back whole, colours to their decimals.
    const expected = fakeMap("biome").encoding.legend!;
    expect(biome.encoding.legend!.map((entry) => entry.name)).toEqual(expected.map((entry) => entry.name));
    biome.encoding.legend!.forEach((entry, index) => {
      expect(entry.index).toBe(index);
      entry.colour.forEach((component, k) => expect(component).toBeCloseTo(expected[index].colour[k], 6));
    });
    expect(MAP_KINDS).toHaveLength(15);
  });

  it("rejects unknown kinds and bad sizes", () => {
    expect(() => engine.exportMap("rivers" as MapKind)).toThrow(TypeError);
    expect(() => engine.exportMap("height", { size: [4] as unknown as [number, number] })).toThrow(TypeError);

    for (const size of [[1, 4], [4, 2049], [4096, 4096], [2.5, 4], [Number.NaN, 4]]) {
      expect(() => engine.exportMap("height", { size: size as [number, number] })).toThrow(
        expect.objectContaining({ code: "OPTIONS_INVALID", message: expect.stringContaining("from 2 to 2048 (WebAssembly's limit on terrain size)") })
      );
    }
  });

  it("unpacks trees and validates the region and the cap", () => {
    const records = engine.exportTrees({ region: { minX: -5, minZ: -5, maxX: 5, maxZ: 5 }, maxCount: 10 });
    const [region, maxCount] = calls.at(-1)!.args as [Float32Array, number];
    expect(Array.from(region)).toEqual([-5, -5, 5, 5]);
    expect(maxCount).toBe(10);
    expect(records).toEqual([
      { x: 1, y: 2, z: 3, species: "jungle", variant: 3, scale: 1.5, rotation: 0.5, tint: 0.25, dryness: 0.75 },
      { x: 9, y: 8, z: 7, species: "shrub", variant: 0, scale: 1, rotation: 0, tint: 0.5, dryness: 0, handPlaced: true }
    ]);
    expect(calls.at(-1)).toMatchObject({ name: "exportTrees" });
    engine.exportTrees();
    expect(calls.at(-1)!.args).toEqual([undefined, 2_000_000]);

    expect(() => engine.exportTrees({ region: { minX: 0, minZ: 0, maxX: Infinity, maxZ: 1 } })).toThrow(TypeError);
    expect(() => engine.exportTrees({ region: { minX: 0, minZ: 0, maxX: 1 } as never })).toThrow(TypeError);
    expect(() => engine.exportTrees({ region: { minX: 3, minZ: 0, maxX: 1, maxZ: 1 } })).toThrow(
      expect.objectContaining({ code: "OPTIONS_INVALID" })
    );

    for (const maxCount of [0, 10_000_001, 1.5, Number.NaN]) {
      expect(() => engine.exportTrees({ maxCount })).toThrow(expect.objectContaining({ code: "OPTIONS_INVALID" }));
    }
  });

  it("snapshots the options the engine was given and accepted since", async () => {
    engine.setGrass({ enabled: true, density: 0.75 } as never);
    engine.setCamera({ position: [1, 2, 3], target: [0, 0, 0] } as never);
    const snapshot = engine.getOptionsSnapshot();
    expect(snapshot).toMatchObject({
      flora: { density: 2, seedOffset: 5 },
      quality: { preset: "balanced" },
      grass: { enabled: true, density: 0.75 },
      camera: { position: [1, 2, 3] }
    });
    expect(snapshot.terrain).toBeUndefined();
    // A copy, as plain JSON.
    snapshot.flora!.density = 4;
    expect(engine.getOptionsSnapshot().flora!.density).toBe(2);
    expect(JSON.parse(JSON.stringify(engine.getOptionsSnapshot()))).toEqual(engine.getOptionsSnapshot());
  });

  it("refuses to export while terrain is generating", async () => {
    const pending = engine.generateFractal({ seed: 1, size: 64 } as never);
    await Promise.resolve();
    await Promise.resolve();

    for (const call of [() => engine.exportMap("height"), () => engine.exportTrees()]) {
      expect(call).toThrow(/while terrain is generating/);
    }

    // The snapshot never reaches into the engine.
    expect(engine.getOptionsSnapshot().flora).toBeDefined();

    generating!();
    await pending;
    expect(() => engine.exportMap("height")).not.toThrow();
    // The terrain it generated is part of the snapshot, until a heightmap
    // is loaded.
    expect(engine.getOptionsSnapshot().terrain).toEqual({ seed: 1, size: 64 });
    await engine.loadRawHeightmap(new ArrayBuffer(8), {} as never);
    expect(engine.getOptionsSnapshot().terrain).toBeUndefined();
  });
});
