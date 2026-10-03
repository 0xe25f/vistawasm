import { beforeAll, describe, expect, it, vi } from "vitest";
import { createVistaEngine, initialiseVistaWasm } from "../src/index";
import type { VistaEngine, VistaWasmGeneratedModule } from "../src/types";

// A stand-in for the generated WASM engine that records every call, so the
// wrapper's validation and packing can be tested without a GPU.
const calls: { name: string; args: unknown[] }[] = [];
let weather: string | undefined;
// When set, renderOnce reports this frame index, as the engine does when it
// skips a frame because the GPU is still busy.
let skippedFrameIndex: number | undefined;
// The weather the engine last reported to `renderFrame`.
let loopWeather: string | null = null;

const raw = new Proxy(
  {},
  {
    get(_target, name: string) {
      if (name === "then") {
        return undefined;
      }

      return (...args: unknown[]) => {
        calls.push({ name, args });

        if (name === "setWaterMask") {
          return args[2] ? "resampled" : undefined;
        }

        if (name === "getWaterSounds") {
          const packed = new Float32Array(20).fill(Number.NaN);
          packed.set([12, 0.75, 1, 2, 3], 0);
          return packed;
        }

        if (name === "getWaterfalls") {
          return new Float32Array([1, 2, 3, 20, 4, 1.5]);
        }

        // An empty array is the engine's "no terrain".
        if (name === "weatherAt") {
          return args[0] === 999 ? new Float32Array(0) : new Float32Array([0.5, 1.25, 0.25, 0.75, 0.5, 0.125, 0]);
        }

        // Polar day: no sunrise or sunset.
        if (name === "getTimeOfDay") {
          return new Float32Array([13.5, 210, 42, Number.NaN, Number.NaN]);
        }

        if (name === "getWeatherPresets") {
          return { clear: { cloudCoverage: 0.08 } };
        }

        // The render loop's lean frame: the index, negated when the
        // weather changed since the last frame.
        if (name === "renderFrame") {
          const changed = (weather ?? null) !== loopWeather;
          loopWeather = weather ?? null;
          return changed ? -calls.length : calls.length;
        }

        if (name === "renderOnce" || name === "getStats") {
          return {
            frameIndex: skippedFrameIndex ?? calls.length,
            frameTimeMs: 0,
            terrainTriangles: 0,
            floraInstances: 0,
            grassInstances: 0,
            clipmapLevels: 0,
            weather
          };
        }

        return undefined;
      };
    }
  }
);

const fakeModule = {
  VistaEngine: {
    create: async () => raw
  }
} as unknown as VistaWasmGeneratedModule;

let engine: VistaEngine;

beforeAll(async () => {
  vi.stubGlobal("navigator", { gpu: {} });
  await initialiseVistaWasm({ wasmModule: fakeModule });
  engine = await createVistaEngine({} as HTMLCanvasElement);
});

function lastCall(name: string): unknown[] {
  const call = calls.filter((entry) => entry.name === name).at(-1);

  if (!call) {
    throw new Error(`${name} was not called.`);
  }

  return call.args;
}

describe("setTreeInstances", () => {
  it("packs trees into nine floats each with defaults", () => {
    engine.setTreeInstances([
      { x: 1, y: 2, z: 3, species: "palm" },
      { x: 4, y: 5, z: 6, species: "shrub", scale: 2, rotation: 1, tint: 0.2, dryness: 0.9, ground: true },
      { x: 7, y: 8, z: 9, species: "oak", ground: false }
    ]);
    const [packed] = lastCall("setTreeInstances") as [Float32Array];

    expect(Array.from(packed)).toEqual([
      1, 2, 3, 1, 0, 0.5, 3, 0, 0,
      4, 5, 6, 2, 1, Math.fround(0.2), 7, Math.fround(0.9), 1,
      7, 8, 9, 1, 0, 0.5, 0, 0, 0
    ]);
  });

  it("rejects a ground flag that is not a boolean", () => {
    expect(() =>
      engine.setTreeInstances([{ x: 0, y: 0, z: 0, species: "oak", ground: 1 as unknown as boolean }])
    ).toThrow(TypeError);
  });

  it("restores procedural placement with undefined", () => {
    engine.setTreeInstances(undefined);
    expect(lastCall("setTreeInstances")).toEqual([undefined]);
  });

  it("rejects unknown species", () => {
    expect(() =>
      engine.setTreeInstances([{ x: 0, y: 0, z: 0, species: "baobab" as "oak" }])
    ).toThrow(TypeError);
  });
});

describe("setTreeModel", () => {
  it("converts plain arrays to typed arrays", () => {
    engine.setTreeModel("oak", {
      positions: [0, 0, 0, 1, 0, 0, 0, 1, 0],
      normals: [0, 0, 1, 0, 0, 1, 0, 0, 1],
      uvs: [0, 0, 1, 0, 0, 1],
      indices: [0, 1, 2]
    });
    const [species, positions, , , indices, layers] = lastCall("setTreeModel");

    expect(species).toBe("oak");
    expect(positions).toBeInstanceOf(Float32Array);
    expect(indices).toBeInstanceOf(Uint32Array);
    expect(layers).toBeUndefined();
  });

  it("rejects unknown species", () => {
    expect(() => engine.resetTreeModel("elm" as "oak")).toThrow(TypeError);
  });
});

describe("replaceTexture", () => {
  it("validates the target, layer, and data type", () => {
    const texels = new Uint8ClampedArray(512 * 512 * 4);

    engine.replaceTexture("flora", 2, texels);
    expect(lastCall("replaceTexture")[2]).toBeInstanceOf(Uint8Array);
    expect(() => engine.replaceTexture("sky" as "flora", 0, texels)).toThrow(TypeError);
    expect(() => engine.replaceTexture("flora", -1, texels)).toThrow(TypeError);
    expect(() => engine.replaceTexture("flora", 0, [] as unknown as Uint8Array)).toThrow(TypeError);
  });
});

describe("temperatureAt", () => {
  it("returns null when the engine reports no terrain", () => {
    expect(engine.temperatureAt(10, -20)).toBeNull();
    expect(lastCall("temperatureAt")).toEqual([10, -20]);
  });

  it("rejects positions that are not finite numbers", () => {
    expect(() => engine.temperatureAt(Number.NaN, 0)).toThrow(TypeError);
    expect(() => engine.temperatureAt(0, Number.POSITIVE_INFINITY)).toThrow(TypeError);
    expect(() => engine.temperatureAt("1" as unknown as number, 0)).toThrow(TypeError);
  });
});

describe("weatherChanged", () => {
  it("fires only when the dominant weather changes", () => {
    const seen: (string | null)[] = [];
    const off = engine.on("weatherChanged", (kind) => seen.push(kind));

    engine.renderOnce();
    weather = "rain";
    engine.renderOnce();
    engine.renderOnce();
    weather = undefined;
    engine.renderOnce();
    off();

    expect(seen).toEqual(["rain", null]);
  });
});

describe("frame pacing", () => {
  it("returns the previous stats and emits nothing for a skipped frame", () => {
    const seen: number[] = [];
    const off = engine.on("stats", (stats) => seen.push(stats.frameIndex));

    const drawn = engine.renderOnce();
    skippedFrameIndex = drawn.frameIndex;
    const skipped = engine.renderOnce();
    skippedFrameIndex = undefined;
    const next = engine.renderOnce();
    off();

    expect(skipped).toBe(drawn);
    expect(seen).toEqual([drawn.frameIndex, next.frameIndex]);
  });
});

describe("render loop", () => {
  it("reads stats only when something listens or the weather changed", () => {
    const frames: FrameRequestCallback[] = [];
    vi.stubGlobal("window", {
      requestAnimationFrame: (callback: FrameRequestCallback) => frames.push(callback),
      cancelAnimationFrame: () => undefined
    });
    let now = 0;
    const tick = (count: number) => {
      for (let frame = 0; frame < count; frame++) {
        now += 100;
        frames.shift()?.(now);
      }
    };
    const count = (name: string) => calls.filter((call) => call.name === name).length;

    engine.start();
    tick(1);
    const statsBefore = count("getStats");
    const framesBefore = count("renderFrame");

    // Nothing listens: frames are drawn, but no stats object is built.
    tick(5);
    expect(count("renderFrame") - framesBefore).toBe(5);
    expect(count("getStats")).toBe(statsBefore);

    // A stats listener gets every frame's stats.
    const seen: number[] = [];
    const off = engine.on("stats", (stats) => seen.push(stats.frameIndex));
    tick(3);
    off();
    expect(seen).toHaveLength(3);
    expect(count("getStats")).toBe(statsBefore + 3);

    // A change of weather is reported with nothing listening for stats.
    const changes: (string | null)[] = [];
    const offWeather = engine.on("weatherChanged", (kind) => changes.push(kind));
    weather = "snow";
    tick(2);
    weather = undefined;
    tick(1);
    offWeather();
    engine.stop();
    vi.unstubAllGlobals();
    vi.stubGlobal("navigator", { gpu: {} });

    expect(changes).toEqual(["snow", null]);
    expect(count("getStats")).toBe(statsBefore + 5);
  });
});

describe("water", () => {
  it("validates water masks before they reach the engine", () => {
    const before = calls.length;

    expect(() => engine.setWaterMask({ width: 2, height: 2, data: [0, 0, 0, 0] } as never)).toThrow(TypeError);
    expect(() => engine.setWaterMask(undefined as never)).toThrow(TypeError);
    expect(() => engine.setWaterMask({ width: 3, height: 2, data: new Uint8Array(5) })).toThrow(
      expect.objectContaining({ code: "OPTIONS_INVALID" })
    );
    expect(() => engine.setWaterMask({ width: 1, height: 2, data: new Uint8Array(2) })).toThrow(
      expect.objectContaining({ code: "OPTIONS_INVALID" })
    );
    expect(calls.length).toBe(before);
  });

  it("passes masks through, reports warnings, and clears with null", () => {
    const warnings: string[] = [];
    const stop = engine.on("warning", ({ message }) => warnings.push(message));
    const data = new Uint8Array([0, 60, 200, 0]);

    engine.setWaterMask({ width: 2, height: 2, data });
    expect(lastCall("setWaterMask")).toEqual([2, 2, data]);
    expect(warnings).toEqual(["resampled"]);

    engine.setWaterMask(null);
    expect(lastCall("setWaterMask")).toEqual([0, 0, undefined]);
    stop();
  });

  it("unpacks water sounds and waterfalls", () => {
    const sounds = engine.getWaterSounds(0, 0, 0);

    expect(sounds.river).toEqual({ distanceMetres: 12, loudness: 0.75, position: [1, 2, 3] });
    expect(sounds.waterfall).toBeNull();
    expect(sounds.lakeShore).toBeNull();
    expect(sounds.surf).toBeNull();
    expect(() => engine.getWaterSounds(Number.NaN, 0, 0)).toThrow(TypeError);
    expect(engine.getWaterfalls()).toEqual([
      { position: [1, 2, 3], heightMetres: 20, widthMetres: 4, dischargeCubicMetresPerSecond: 1.5 }
    ]);
  });
});

describe("weather and time of day", () => {
  it("rejects skips that are negative, not a number or longer than a day", () => {
    const before = calls.length;

    for (const seconds of [-1, Number.NaN, 86401, Number.POSITIVE_INFINITY, "600" as unknown as number]) {
      expect(() => engine.advanceWeather(seconds)).toThrow(
        expect.objectContaining({ code: "OPTIONS_INVALID" })
      );
    }

    expect(calls.length).toBe(before);
    engine.advanceWeather(600);
    expect(lastCall("advanceWeather")).toEqual([600]);
  });

  it("unpacks the weather at a place, and null with no terrain", () => {
    expect(engine.weatherAt(10, 20)).toEqual({
      coverage: 0.5,
      precipitation: 1.25,
      storminess: 0.25,
      humidity: 0.75,
      wetness: 0.5,
      puddles: 0.125,
      snowDepth: 0
    });
    expect(engine.weatherAt(999, 0)).toBeNull();
    expect(() => engine.weatherAt(Number.NaN, 0)).toThrow(TypeError);
  });

  it("fills in time of day defaults and checks the day of the year", () => {
    engine.setTimeOfDay({ enabled: true, hours: 18 });
    expect(lastCall("setTimeOfDay")).toEqual([true, 18, 24, 45, 172]);
    expect(() => engine.setTimeOfDay({ dayOfYear: 0 })).toThrow(
      expect.objectContaining({ code: "OPTIONS_INVALID" })
    );
    expect(() => engine.setTimeOfDay({ dayOfYear: 12.5 })).toThrow(
      expect.objectContaining({ code: "OPTIONS_INVALID" })
    );
    expect(() => engine.setTimeOfDay({ hours: "noon" as unknown as number })).toThrow(TypeError);
  });

  it("reports polar day or night as null sunrise and sunset", () => {
    expect(engine.getTimeOfDay()).toEqual({
      hours: 13.5,
      sunAzimuthDegrees: 210,
      sunElevationDegrees: 42,
      sunriseHours: null,
      sunsetHours: null
    });
  });

  it("passes the resolved presets through", () => {
    expect(engine.getWeatherPresets().clear.cloudCoverage).toBe(0.08);
  });
});
