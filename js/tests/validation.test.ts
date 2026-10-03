import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import { attachFlyCameraControls } from "../src/camera-controls";
import { MAX_OPTIONS_DEPTH, VistaWasmError } from "../src/errors";
import {
  MAX_PRESET_NAME_LENGTH,
  MAX_WEATHER_PRESETS,
  TEXTURE_LAYER_SIZE,
  createVistaEngine,
  fetchDemBytes,
  imageToRgba,
  initialiseVistaWasm,
  loadBundle
} from "../src/index";
import { encodePng, encodeRaw, exportBundle, treesToCsv, treesToJson } from "../src/map-export";
import { biomeMapFromImage, densityMaskFromImage, waterMaskFromImage } from "../src/map-import";
import { decodePng } from "../src/png-decode";
import { computeHeightmapPixels, exportTerrainObj, readHeightmapFloats } from "../src/terrain-export";
import type { ExportedMap, TerrainMetadata, VistaEngine, VistaWasmGeneratedModule } from "../src/types";
import { makePng } from "./make-png";

// Every public entry point, fed NaN, Infinity, negative, zero, too-large
// and wrong-type values and wrong-length arrays. Each must throw a typed
// error (a TypeError, or a VistaWasmError with a code) whose message
// names what was wrong, before anything reaches the engine.

const calls: string[] = [];
const raw = new Proxy({}, {
  get(_target, name: string) {
    // GPU events are polled after renders and async calls; there are none.
    return name === "then" || name === "takeGpuEvents" ? undefined : (...args: unknown[]) => {
      calls.push(name);
      return name === "renderOnce" ? { frameIndex: calls.length } : name === "setWaterMask" ? undefined : args.length ? undefined : new Float32Array(8);
    };
  }
});
const fakeModule = { VistaEngine: { create: async () => raw } } as unknown as VistaWasmGeneratedModule;
let engine: VistaEngine;
const prototypeBefore = Object.getOwnPropertyNames(Object.prototype).join();

beforeAll(async () => {
  vi.stubGlobal("navigator", { gpu: {} });
  await initialiseVistaWasm({ wasmModule: fakeModule });
  engine = await createVistaEngine({} as HTMLCanvasElement);
});

afterAll(() => {
  // Nothing any test fed in reached Object.prototype.
  expect(Object.getOwnPropertyNames(Object.prototype).join()).toBe(prototypeBefore);
  expect(({} as Record<string, unknown>).polluted).toBeUndefined();
});

/** Expect `call` to throw `type` (and `code`) with a message matching `message`, reaching nothing in the engine. */
function rejects(call: () => unknown, type: typeof TypeError | typeof VistaWasmError, message: RegExp, code?: string): void {
  const before = calls.length;
  let thrown: unknown;

  try {
    call();
  } catch (error) {
    thrown = error;
  }

  expect(thrown, `expected ${message}`).toBeInstanceOf(type);
  expect((thrown as Error).message).toMatch(message);

  if (code) {
    expect((thrown as VistaWasmError).code).toBe(code);
  }

  expect(calls.length).toBe(before);
}

async function rejectsAsync(call: () => Promise<unknown>, type: typeof TypeError | typeof VistaWasmError, message: RegExp, code?: string): Promise<void> {
  const error = await call().then(() => undefined, (error: unknown) => error);
  expect(error, `expected ${message}`).toBeInstanceOf(type);
  expect((error as Error).message).toMatch(message);

  if (code) {
    expect((error as VistaWasmError).code).toBe(code);
  }
}

const BAD_OBJECTS: unknown[] = [undefined, null, 5, "sun", [1, 2]];
const NOT_FINITE = [Number.NaN, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY];

describe("option objects", () => {
  const setters = ["setCamera", "setSun", "setAtmosphere", "setWater", "setFlora", "setGrass", "setClouds", "setMist", "setRenderQuality", "setBiomes", "setWeather", "setShadows", "setSurface"] as const;

  it.each(setters)("%s() rejects anything but an object", (setter) => {
    for (const value of BAD_OBJECTS) {
      rejects(() => (engine[setter] as (value: unknown) => void)(value), TypeError, new RegExp(`${setter}\\(\\) expects an options object, but it was given`));
    }
  });

  it.each(setters)("%s() rejects prototype keys at any depth", (setter) => {
    for (const key of ["__proto__", "constructor", "prototype"]) {
      const top = JSON.parse(`{ "${key}": { "polluted": true } }`);
      const nested = JSON.parse(`{ "a": { "b": { "${key}": { "polluted": true } } } }`);
      rejects(() => (engine[setter] as (value: unknown) => void)(top), VistaWasmError, new RegExp(`may not hold the key "${key}"`), "OPTIONS_INVALID");
      rejects(() => (engine[setter] as (value: unknown) => void)(nested), VistaWasmError, new RegExp(`"${key}" \\(in a\\.b\\)`), "OPTIONS_INVALID");
    }
  });

  it("rejects options that nest too deeply or contain themselves", () => {
    const cyclic: Record<string, unknown> = {};
    cyclic.self = cyclic;
    let deep: Record<string, unknown> = {};

    for (let depth = 0; depth <= MAX_OPTIONS_DEPTH; depth += 1) {
      deep = { deep };
    }

    rejects(() => engine.setSun(cyclic as never), VistaWasmError, /nest more than 8 levels deep at \.self\.self/, "OPTIONS_INVALID");
    rejects(() => engine.setClouds(deep as never), VistaWasmError, /nest more than 8 levels deep/, "OPTIONS_INVALID");
    // Typed arrays are data and are not walked.
    engine.setSun({ data: new Float32Array(1_000_000) } as never);
    expect(calls.at(-1)).toBe("setSun");
  });

  it("checks engine options, terrain options and export options the same way", async () => {
    await rejectsAsync(() => createVistaEngine({} as HTMLCanvasElement, null as never), TypeError, /createVistaEngine\(\) expects an options object/);
    await rejectsAsync(() => createVistaEngine({} as HTMLCanvasElement, JSON.parse('{ "weather": { "__proto__": {} } }')), VistaWasmError, /"__proto__"/, "OPTIONS_INVALID");
    await rejectsAsync(() => engine.generateFractal(null as never), TypeError, /generateFractal\(\) expects an options object/);
    await rejectsAsync(() => engine.loadRawHeightmap(new ArrayBuffer(16), "float32" as never), TypeError, /loadRawHeightmap\(\) expects an options object/);
    rejects(() => engine.exportMap("height", 5 as never), TypeError, /exportMap\(\) expects an options object/);
    rejects(() => engine.exportTrees(JSON.parse('{ "constructor": 1 }')), VistaWasmError, /"constructor"/, "OPTIONS_INVALID");
  });

  it("rejects terrain bytes that are not an ArrayBuffer", async () => {
    for (const buffer of [new Uint8Array(16), [1, 2, 3], "bytes", null]) {
      await rejectsAsync(() => engine.loadRawHeightmap(buffer as never, { width: 2, height: 2 } as never), TypeError, /expects the file's bytes as an ArrayBuffer/);
      await rejectsAsync(() => engine.loadDemFromArrayBuffer(buffer as never), TypeError, /expects the file's bytes as an ArrayBuffer/);
    }

    await rejectsAsync(() => fetchDemBytes(42 as never), TypeError, /fetchDemBytes\(\) expects the DEM's URL/);
  });
});

describe("weather presets", () => {
  it("rejects prototype keys as preset names, in next weights and inside presets", () => {
    for (const key of ["__proto__", "constructor", "prototype"]) {
      for (const json of [
        `{ "presets": { "${key}": { "cloudCoverage": 1 } } }`,
        `{ "presets": { "calm": { "next": { "${key}": 1 } } } }`,
        `{ "presets": { "calm": { "${key}": { "polluted": true } } } }`
      ]) {
        rejects(() => engine.setWeather(JSON.parse(json)), VistaWasmError, new RegExp(`"${key}"`), "OPTIONS_INVALID");
      }
    }
  });

  it("rejects presets that are not a plain object", () => {
    // A literal __proto__ key sets the prototype: the preset would vanish.
    rejects(() => engine.setWeather({ presets: { __proto__: { calm: {} } } as never }), TypeError, /presets must be a plain object/);
    rejects(() => engine.setWeather({ presets: new Map() as never }), TypeError, /presets must be a plain object/);
    rejects(() => engine.setWeather({ presets: [] as never }), TypeError, /presets must be a plain object/);
    rejects(() => engine.setWeather({ presets: "calm" as never }), TypeError, /presets must be a plain object/);
  });

  it("bounds the number and length of preset names", () => {
    const many = Object.fromEntries(Array.from({ length: MAX_WEATHER_PRESETS + 1 }, (_, index) => [`p${index}`, {}]));
    rejects(() => engine.setWeather({ presets: many }), VistaWasmError, /at most 64 presets, but it holds 65/, "OPTIONS_INVALID");
    rejects(() => engine.setWeather({ presets: { ["x".repeat(MAX_PRESET_NAME_LENGTH + 1)]: {} } }), VistaWasmError, /1 to 64 characters/, "OPTIONS_INVALID");
    rejects(() => engine.setWeather({ presets: { "": {} } }), VistaWasmError, /1 to 64 characters/, "OPTIONS_INVALID");
    rejects(() => engine.setWeather({ state: "y".repeat(65) }), VistaWasmError, /1 to 64 characters/, "OPTIONS_INVALID");
    rejects(() => engine.setWeather({ state: 3 as never }), VistaWasmError, /preset names and state must be strings/, "OPTIONS_INVALID");

    const allowed = Object.fromEntries(Array.from({ length: MAX_WEATHER_PRESETS }, (_, index) => [`p${index}`.padEnd(MAX_PRESET_NAME_LENGTH, "x"), {}]));
    engine.setWeather({ presets: allowed, state: "clear" });
    expect(calls.at(-1)).toBe("setWeather");
  });
});

describe("engine methods", () => {
  it("resize() rejects sizes that are not finite, negative or too large, before the canvas changes", () => {
    for (const [width, height] of [[Number.NaN, 10], [10, Number.POSITIVE_INFINITY], [-1, 10], [8_193, 10], ["10", 10]]) {
      rejects(() => engine.resize(width as number, height as number, 1), VistaWasmError, /width and height must be from 0 to 8192 CSS pixels/, "OPTIONS_INVALID");
    }

    for (const ratio of [...NOT_FINITE, 0, -1, 9, "2"]) {
      rejects(() => engine.resize(10, 10, ratio as number), VistaWasmError, /devicePixelRatio must be over 0 and at most 8/, "OPTIONS_INVALID");
    }

    engine.resize(0, 10, 2);
    expect(calls.at(-1)).toBe("resize");
  });

  it("setTreeModel() rejects arrays of the wrong type, values that would wrap, and too many vertices", () => {
    const model = { positions: [0, 0, 0, 1, 0, 0, 0, 1, 0], normals: new Float32Array(9), uvs: new Float32Array(6), indices: [0, 1, 2] };
    rejects(() => engine.setTreeModel("oak", { ...model, positions: [0, "1", 2] as never }), TypeError, /positions must be a Float32Array or an array of numbers/);
    rejects(() => engine.setTreeModel("oak", { ...model, normals: new Float64Array(9) as never }), TypeError, /normals must be a Float32Array/);

    for (const index of [-1, 1.5, 2 ** 32, Number.NaN]) {
      rejects(() => engine.setTreeModel("oak", { ...model, indices: [0, 1, index] }), TypeError, /indices must be a Uint32Array or an array of whole numbers from 0 to 4294967295/);
    }

    rejects(() => engine.setTreeModel("oak", { ...model, indices: new Int32Array(3) as never }), TypeError, /indices must be a Uint32Array/);
    rejects(() => engine.setTreeModel("oak", { ...model, positions: new Float32Array(65_537 * 3) }), VistaWasmError, /at most 65536 vertices/, "OPTIONS_INVALID");
    rejects(() => engine.setTreeModel("elm" as never, model), TypeError, /Unknown tree species/);
  });

  it("setTreeInstances() rejects trees that are not objects or hold values that are not numbers", () => {
    rejects(() => engine.setTreeInstances([null as never]), TypeError, /Tree 0 has an unknown species/);
    rejects(() => engine.setTreeInstances([{ x: "1" as never, y: 0, z: 0, species: "oak" }]), TypeError, /Tree 0 needs numbers for x, y and z/);
    rejects(() => engine.setTreeInstances([{ x: 0, y: 0, z: 0, species: "oak" }, { x: 0, y: 0, z: 0, species: "pine", scale: "2" as never }]), TypeError, /Tree 1 needs numbers/);
    rejects(() => engine.setTreeInstances({} as never), TypeError, /expects an array of trees/);
  });

  it("replaceTexture() rejects layers that would wrap and texels of the wrong length", () => {
    const texels = new Uint8Array(TEXTURE_LAYER_SIZE * TEXTURE_LAYER_SIZE * 4);

    for (const layer of [Number.NaN, -1, 1.5, 256, 2 ** 32]) {
      rejects(() => engine.replaceTexture("flora", layer, texels), TypeError, /layer must be a whole number from 0 to 255/);
    }

    for (const length of [0, 4, texels.length - 1, texels.length + 4]) {
      rejects(() => engine.replaceTexture("flora", 0, new Uint8Array(length)), VistaWasmError, /expects 512 x 512 RGBA texels \(1048576 bytes\), but it was given/, "OPTIONS_INVALID");
    }

    rejects(() => engine.replaceTexture("flora", 0, new Uint16Array(texels.length) as never), TypeError, /RGBA texels in a Uint8Array/);
  });

  it("setDebugView() and on() reject unknown names", () => {
    rejects(() => engine.setDebugView("wireframe" as never), TypeError, /setDebugView\(\) expects one of none, height/);
    rejects(() => engine.on("stat" as never, () => undefined), TypeError, /on\(\) expects an event name, one of ready/);
    rejects(() => engine.on("stats", "listener" as never), TypeError, /and a listener function/);
  });

  it("masks reject wrong types, sizes and lengths", () => {
    for (const call of [
      (mask: never) => engine.setWaterMask(mask),
      (mask: never) => engine.setBiomeMap(mask),
      (mask: never) => engine.setVegetationMasks({ trees: mask })
    ]) {
      rejects(() => call({ width: 2, height: 2, data: [0, 0, 0, 0] } as never), TypeError, /expects \{ width, height, data \}/);
      rejects(() => call({ width: 2.5, height: 2, data: new Uint8Array(5) } as never), TypeError, /whole-number sizes/);
      rejects(() => call({ width: Number.NaN, height: 2, data: new Uint8Array(4) } as never), TypeError, /whole-number sizes/);

      for (const [width, height] of [[1, 2], [0, 2], [-2, 2], [2049, 2]]) {
        rejects(() => call({ width, height, data: new Uint8Array(Math.max(0, width * height)) } as never), VistaWasmError, /width and height must be from 2 to 2048 \(WebAssembly's limit on terrain size\)/, "OPTIONS_INVALID");
      }

      rejects(() => call({ width: 3, height: 2, data: new Uint8Array(5) } as never), VistaWasmError, /must hold width x height = 6 bytes, but it holds 5/, "OPTIONS_INVALID");
    }
  });

  it("the other number checks hold", () => {
    for (const value of NOT_FINITE) {
      rejects(() => engine.temperatureAt(value, 0), TypeError, /finite x and z/);
      rejects(() => engine.weatherAt(0, value), TypeError, /finite x and z/);
      rejects(() => engine.getWaterSounds(0, value, 0), TypeError, /finite x, y and z/);
      rejects(() => engine.advanceWeather(value), VistaWasmError, /0 to 86400 seconds/, "OPTIONS_INVALID");
    }

    rejects(() => engine.exportTrees({ maxCount: 0 }), VistaWasmError, /maxCount must be a whole number from 1 to 10000000/, "OPTIONS_INVALID");
    rejects(() => engine.exportMap("height", { size: [2, 2049] }), VistaWasmError, /from 2 to 2048/, "OPTIONS_INVALID");
    rejects(() => engine.setTimeOfDay({ dayOfYear: 367 }), VistaWasmError, /from 1 to 366/, "OPTIONS_INVALID");
  });

  it("exportSnapshot() checks its type and quality", async () => {
    await rejectsAsync(() => engine.exportSnapshot({ mimeType: "text/html" }), VistaWasmError, /mimeType must be an image type/, "OPTIONS_INVALID");
    await rejectsAsync(() => engine.exportSnapshot({ mimeType: "image/png\"><script>" }), VistaWasmError, /mimeType must be an image type/, "OPTIONS_INVALID");

    for (const quality of [...NOT_FINITE, -0.1, 1.1, "0.5"]) {
      await rejectsAsync(() => engine.exportSnapshot({ quality: quality as number }), VistaWasmError, /quality must be from 0 to 1/, "OPTIONS_INVALID");
    }
  });
});

describe("allocation before validation", () => {
  it("loadRawHeightmap() rejects a buffer longer than any map before the engine copies it", async () => {
    // A stand-in for a 2 GiB buffer, so the test allocates nothing.
    const huge = Object.create(ArrayBuffer.prototype, { byteLength: { value: 2 ** 31 } }) as ArrayBuffer;
    const before = calls.length;
    await rejectsAsync(() => engine.loadRawHeightmap(huge, { width: 4, height: 4, sampleFormat: "float32", metresPerSample: 1, heightScaleMetres: 1 }), VistaWasmError, /2147483648 bytes, more than the 16777216 the largest map .* 4 x 4 float32 samples need 64/, "OPTIONS_INVALID");
    await rejectsAsync(() => engine.loadRawHeightmap(new ArrayBuffer(10), { width: 4, height: 4, sampleFormat: "int16", metresPerSample: 1, heightScaleMetres: 1 }), VistaWasmError, /given 10 bytes, but 4 x 4 int16 samples need 32/, "OPTIONS_INVALID");
    await rejectsAsync(() => engine.loadDemFromArrayBuffer(huge), VistaWasmError, /more than the 83886080 a GeoTIFF/, "DEM_FORMAT_UNSUPPORTED");
    expect(calls.length).toBe(before);
    // A buffer merely longer than needed reaches the engine, which reads
    // only what the samples need and warns.
    await engine.loadRawHeightmap(new ArrayBuffer(100), { width: 4, height: 4, sampleFormat: "float32", metresPerSample: 1, heightScaleMetres: 1 }).catch(() => undefined);
    expect(calls.at(-1)).toBe("loadRawHeightmap");
  });

  it("setTreeInstances() counts the trees before allocating for them", () => {
    rejects(() => engine.setTreeInstances(new Array(1_000_001)), VistaWasmError, /at most 1000000 trees, but it was given 1000001/, "OPTIONS_INVALID");
  });

  it("reads a JPEG's size from its header and refuses one past 2048 before the browser decodes it", async () => {
    const decode = vi.fn();
    vi.stubGlobal("createImageBitmap", decode);
    // SOI, then a baseline frame header for 30000 x 30000.
    const jpeg = new Uint8Array([0xff, 0xd8, 0xff, 0xc0, 0, 11, 8, 0x75, 0x30, 0x75, 0x30, 1, 1, 0x11, 0, 0]);
    await rejectsAsync(() => densityMaskFromImage(jpeg), VistaWasmError, /30000 x 30000, but painted maps \(WebAssembly's limit on terrain size\) must be from 2 to 2048/, "INVALID_DEM");
    await rejectsAsync(() => densityMaskFromImage(new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12])), VistaWasmError, /not a PNG, JPEG, WebP, GIF or BMP/, "INVALID_DEM");
    expect(decode).not.toHaveBeenCalled();
    vi.unstubAllGlobals();
    vi.stubGlobal("navigator", { gpu: {} });
  });

  it("fetchDemBytes() refuses a long Content-Length unread, and cancels a body that grows past its limit", async () => {
    let pulled = 0;
    let cancelled = false;
    const endless = () => new ReadableStream<Uint8Array>({
      pull(controller) {
        pulled += 1;
        controller.enqueue(new Uint8Array(64));
      },
      cancel() {
        cancelled = true;
      }
    });
    const respond = (headers: Record<string, string>) => vi.fn(async () => new Response(endless(), { headers }));

    vi.stubGlobal("fetch", respond({ "content-length": String(2 ** 40) }));
    await rejectsAsync(() => fetchDemBytes("https://example.com/dem.tif"), VistaWasmError, /1099511627776 bytes, over the limit of 83886080/, "DEM_FETCH_FAILED");
    expect(cancelled).toBe(true);

    [pulled, cancelled] = [0, false];
    vi.stubGlobal("fetch", respond({}));
    await rejectsAsync(() => fetchDemBytes("https://example.com/dem.tif", { maxBytes: 1000 }), VistaWasmError, /over 1000 bytes, over the limit of 1000/, "DEM_FETCH_FAILED");
    expect(cancelled).toBe(true);
    expect(pulled).toBeLessThan(40);
    await rejectsAsync(() => fetchDemBytes("https://example.com/dem.tif", { maxBytes: 2 ** 40 }), VistaWasmError, /maxBytes must be a whole number from 1 to 83886080/, "OPTIONS_INVALID");

    vi.stubGlobal("fetch", vi.fn(async () => new Response(new Uint8Array([1, 2, 3]))));
    expect(new Uint8Array(await fetchDemBytes("https://example.com/dem.tif"))).toEqual(new Uint8Array([1, 2, 3]));
    vi.unstubAllGlobals();
    vi.stubGlobal("navigator", { gpu: {} });
  });
});

describe("options are read once", () => {
  it("passes the engine a copy taken with one read of each property", () => {
    let reads = 0;
    const grass = { get density() {
      reads += 1;
      return reads;
    } };
    engine.setGrass(grass as never);
    expect(reads).toBe(1);
    expect(calls.at(-1)).toBe("setGrass");
    // What the engine was given, and what the snapshot keeps, is the copy.
    expect(engine.getOptionsSnapshot().grass).toMatchObject({ density: 1 });
    expect(reads).toBe(1);
  });

  it("warns about an unknown key, naming the closest valid one, and leaves it out", async () => {
    const warnings: string[] = [];
    const stop = engine.on("warning", ({ message }) => warnings.push(message));
    engine.setGrass({ enabled: true, densty: 2, density: 1 } as never);
    expect(warnings).toEqual([
      'setGrass() option "densty" is not one the engine reads, so it was ignored. Did you mean "density"? Unknown options become errors in the next major version.'
    ]);
    expect(engine.getOptionsSnapshot().grass).toEqual({ enabled: true, density: 1 });
    warnings.length = 0;
    engine.setWater({ rivers: { meanderz: 1 }, colour: [0, 0, 1] } as never);
    expect(warnings.map((message) => message.match(/"[^"]+"/g))).toEqual([['"rivers.meanderz"', '"meanders"'], ['"colour"']]);
    expect(engine.getOptionsSnapshot().water).toEqual({ rivers: {} });
    // Lists of objects are checked item by item; maps of names are not.
    warnings.length = 0;
    engine.setWater({ rivers: { inflow: [{ position: [1, 2], dischargeCubicMetresPerSecond: 3, wide: 1 }] } } as never);
    engine.setWeather({ presets: { calm: { anything: 1 } } } as never);
    expect(warnings.map((message) => message.match(/"[^"]+"/)?.[0])).toEqual(['"rivers.inflow[0].wide"']);
    stop();
  });

  it("rejects options that are not plain data", () => {
    rejects(() => engine.setSun({ intensity: () => 1 } as never), TypeError, /setSun\(\) options must be plain data/);
  });
});

describe("module functions", () => {
  const map = (overrides: Partial<ExportedMap> = {}): ExportedMap => ({
    kind: "moisture",
    width: 2,
    height: 2,
    channels: 1,
    type: "uint8",
    data: new Uint8Array(4),
    encoding: { units: "", metresPerPixel: [1, 1] } as never,
    ...overrides
  });

  it("encodePng() and encodeRaw() reject maps that are not whole, in range and the right length", async () => {
    for (const bad of [{ width: Number.NaN }, { width: 2.5, data: new Uint8Array(5) }, { width: 0, data: new Uint8Array(0) }, { width: 2049, data: new Uint8Array(2049 * 2) }, { channels: 13, data: new Uint8Array(52) }, { data: new Uint8Array(3) }, { data: [0, 0, 0, 0] as never }]) {
      await rejectsAsync(() => encodePng(map(bad)), TypeError, /expects a map from exportMap\(\): width and height whole numbers from 1 to 2048, 1 to 12 channels/);
      expect(() => encodeRaw(map(bad))).toThrow(TypeError);
    }

    await rejectsAsync(() => encodePng(null as never), TypeError, /expects a map from exportMap/);
    await rejectsAsync(() => encodePng(map(), JSON.parse('{ "__proto__": {} }')), VistaWasmError, /"__proto__"/, "OPTIONS_INVALID");
  });

  it("encodePng() rejects a legend a palette cannot hold", async () => {
    const legend = (entries: unknown[]) => map({ kind: "biome", encoding: { legend: entries } as never });
    const colour = [0.1, 0.2, 0.3];

    for (const entries of [
      [{ index: 256, colour }],
      [{ index: -1, colour }],
      [{ index: 1.5, colour }],
      [{ index: 0, colour: [0, Number.NaN, 0] }],
      [{ index: 0, colour: [0, 0] }],
      Array.from({ length: 257 }, (_, index) => ({ index: index % 256, colour }))
    ]) {
      await rejectsAsync(() => encodePng(legend(entries)), VistaWasmError, /at most 256 entries, each with an index from 0 to 255/, "OPTIONS_INVALID");
    }
  });

  it("treesToCsv() and treesToJson() write only numbers and known species", () => {
    const tree = { x: 1, y: 2, z: 3, species: "oak", variant: 0, scale: 1, rotation: 0, tint: 0.5, dryness: 0 } as const;
    expect(treesToCsv([tree])).toContain("1,2,3,oak");

    for (const bad of [{ ...tree, species: "=HYPERLINK(\"x\")" }, { ...tree, x: "1,2" }, null]) {
      expect(() => treesToCsv([bad as never])).toThrow(/tree 0 needs numbers for x, y, z/);
      expect(() => treesToJson([bad as never])).toThrow(TypeError);
    }

    expect(() => treesToCsv("trees" as never)).toThrow(/expects an array of trees/);
  });

  it("exportBundle() and loadBundle() check their options", async () => {
    await rejectsAsync(() => exportBundle(engine, 3 as never), TypeError, /exportBundle\(\) expects an options object/);
    await rejectsAsync(() => exportBundle(engine, { maxTrees: Number.POSITIVE_INFINITY }), VistaWasmError, /maxTrees must be a whole number/, "OPTIONS_INVALID");
    await rejectsAsync(() => loadBundle(engine, new Uint8Array(4), "yes" as never), TypeError, /loadBundle\(\) expects an options object/);
    await rejectsAsync(() => loadBundle(engine, 42 as never), TypeError, /must be a Blob, an ArrayBuffer or a Uint8Array/);
  });

  it("the image readers check their options and inputs", async () => {
    const png = makePng({ width: 2, height: 2, colourType: 0, bitDepth: 8, samples: [0, 1, 2, 3] });
    await rejectsAsync(() => decodePng([1, 2] as never), TypeError, /decodePng\(\) expects the PNG file as a Uint8Array/);
    await rejectsAsync(() => biomeMapFromImage(png, null as never), TypeError, /biomeMapFromImage\(\) expects an options object/);
    await rejectsAsync(() => biomeMapFromImage(png, { borderSamples: Number.NaN }), VistaWasmError, /borderSamples must be a whole number from 0 to 8/, "OPTIONS_INVALID");
    await rejectsAsync(() => biomeMapFromImage(png, { legend: Array.from({ length: 257 }, () => ({ colour: [0, 0, 0], biome: "ocean" as const })) }), VistaWasmError, /at most 256 colours, but it holds 257/, "OPTIONS_INVALID");
    await rejectsAsync(() => biomeMapFromImage(png, { legend: [null as never] }), VistaWasmError, /legend entries need a colour/, "OPTIONS_INVALID");
    await rejectsAsync(() => waterMaskFromImage(png, { mode: "gray" as never }), VistaWasmError, /mode must be "legend" or "grey"/, "OPTIONS_INVALID");
    await rejectsAsync(() => densityMaskFromImage(png, { channel: "alpha" as never }), VistaWasmError, /channel must be one of/, "OPTIONS_INVALID");
    await rejectsAsync(() => densityMaskFromImage("file.png" as never), TypeError, /must be a Blob, an ArrayBuffer or a Uint8Array/);

    for (const metresPerSample of [...NOT_FINITE, 0, -1, "1"]) {
      await rejectsAsync(() => engine.loadHeightmapImage(png, { metresPerSample: metresPerSample as number, minHeightMetres: 0, maxHeightMetres: 1 }), VistaWasmError, /metresPerSample must be/, "OPTIONS_INVALID");
    }
  });

  it("imageToRgba() checks its size before decoding anything", async () => {
    for (const size of [...NOT_FINITE, 0, -1, 1.5, 8193]) {
      await rejectsAsync(() => imageToRgba(new Blob(), size), VistaWasmError, /size must be a whole number from 1 to 8192/, "OPTIONS_INVALID");
    }
  });

  it("the heightmap helpers check sizes, bytes and modes", () => {
    const metadata = { width: 2, height: 2, minHeightMetres: 0, maxHeightMetres: 1, seaLevelMetres: 0, metresPerSample: 1 } as TerrainMetadata;
    const bytes = new Uint8Array(16);
    expect(() => readHeightmapFloats([0, 0, 0, 0] as never)).toThrow(/heightBytes must be a Uint8Array/);

    for (const width of [...NOT_FINITE, 0, -2, 1.5]) {
      expect(() => computeHeightmapPixels({ ...metadata, width }, bytes)).toThrow(/whole numbers greater than 0/);
      expect(() => exportTerrainObj({ ...metadata, width }, bytes)).toThrow(/whole numbers greater than 1/);
    }

    expect(() => computeHeightmapPixels(metadata, bytes, { colourMode: "rainbow" as never })).toThrow(/colourMode must be "grayscale" or "hypsometric"/);
    expect(() => exportTerrainObj({ ...metadata, metresPerSample: Number.NaN }, bytes)).toThrow(/metresPerSample must be a finite number/);

    for (const maxSamplesPerSide of [Number.NaN, Number.POSITIVE_INFINITY, 2.5, 2049]) {
      expect(() => exportTerrainObj(metadata, bytes, { maxSamplesPerSide })).toThrow(/maxSamplesPerSide must be a whole number up to 2048/);
    }

    expect(exportTerrainObj(metadata, bytes, { maxSamplesPerSide: 1 })).toContain("4 vertices");
  });

  it("attachFlyCameraControls() checks its engine, canvas and every option before listening", () => {
    const canvas = { addEventListener: vi.fn() } as unknown as HTMLCanvasElement;
    const target = { setCamera: vi.fn() };
    const attach = (options: unknown, on: unknown = target) => () => attachFlyCameraControls(on as never, canvas, options as never);
    rejects(attach({}, {}), TypeError, /expects an engine with setCamera\(\) and a canvas element/);
    rejects(attach(null), TypeError, /expects an options object/);
    rejects(attach({ initialPosition: [0, Number.NaN, 0] }), TypeError, /initialPosition expects a position \[x, y, z\] of three finite numbers/);
    rejects(attach({ initialPosition: [0, 0] }), TypeError, /three finite numbers/);
    rejects(attach({ onCameraChange: "log" }), TypeError, /onCameraChange must be a function/);

    for (const value of [...NOT_FINITE, "5"]) {
      rejects(attach({ moveSpeedMetresPerSecond: value }), VistaWasmError, /moveSpeedMetresPerSecond must be a finite number, but it is/, "OPTIONS_INVALID");
    }

    for (const value of [0, -10, 180, 500]) {
      rejects(attach({ fieldOfViewDegrees: value }), VistaWasmError, /fieldOfViewDegrees must be a finite number from 1 to 179 degrees/, "OPTIONS_INVALID");
    }

    rejects(attach({ minFieldOfViewDegrees: 90, maxFieldOfViewDegrees: 30 }), VistaWasmError, /minFieldOfViewDegrees must not be above maxFieldOfViewDegrees/, "OPTIONS_INVALID");
    expect(canvas.addEventListener).not.toHaveBeenCalled();
  });
});
