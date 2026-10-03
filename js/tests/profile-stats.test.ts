import { describe, expect, it } from "vitest";
// Plain JavaScript modules shared by the GPU profile page (`bench/gpu/`)
// and `scripts/visual-check/fixed-scene.mjs`.
import {
  buildReport,
  isSoftwareAdapter,
  median,
  percentile,
  reportToMarkdown,
  sceneError,
  sceneResult,
  summarise,
  summarisePasses,
  withinBudget
} from "../../bench/gpu/stats.mjs";
import { budgetedGroups, budgetedScenes, skyScenes } from "../../scripts/visual-check/scenes.mjs";

describe("profile statistics", () => {
  it("takes the median of odd and even counts, whatever the order", () => {
    expect(median([5, 1, 3])).toBe(3);
    expect(median([4, 1, 3, 2])).toBe(2.5);
    expect(median([7])).toBe(7);
  });

  it("interpolates the 90th percentile between the nearest ranks", () => {
    const values = Array.from({ length: 11 }, (_, index) => 10 - index);
    expect(percentile(values, 0.9)).toBe(9);
    expect(percentile([1, 2, 3, 4, 5, 6, 7, 8, 9, 10], 0.9)).toBeCloseTo(9.1, 10);
    expect(percentile([3, 1, 2], 0)).toBe(1);
    expect(percentile([3, 1, 2], 1)).toBe(3);
  });

  it("gives the 90th percentile of 120 frames from the slowest dozen", () => {
    const frames = Array.from({ length: 120 }, (_, index) => index + 1);
    // Rank 0.9 x 119 = 107.1, between 108 and 109 ms.
    expect(percentile(frames, 0.9)).toBeCloseTo(108.1, 10);
    expect(summarise(frames)).toEqual({ median: 60.5, p90: 108.1 });
  });

  it("does not reorder the caller's array", () => {
    const values = [3, 1, 2];
    median(values);
    expect(values).toEqual([3, 1, 2]);
  });

  it("rejects empty input, non-finite times and fractions outside 0 to 1", () => {
    expect(() => median([])).toThrow(TypeError);
    expect(() => median([1, Number.NaN])).toThrow(/finite number/);
    expect(() => median([1, Number.POSITIVE_INFINITY])).toThrow(TypeError);
    expect(() => percentile([1, 2], 1.5)).toThrow(RangeError);
    expect(() => percentile([1, 2], Number.NaN)).toThrow(/from 0 to 1/);
  });

  it("summarises each pass over the frames that have it", () => {
    const passes = summarisePasses([
      { terrain: 1, clouds: 4 },
      null,
      { terrain: 3, clouds: 2, grass: 0.5 },
      { terrain: 2, clouds: 3 }
    ]);
    expect(Object.keys(passes)).toEqual(["terrain", "clouds", "grass"]);
    expect(passes.terrain).toEqual({ median: 2, p90: 2.8 });
    expect(passes.grass).toEqual({ median: 0.5, p90: 0.5 });
  });
});

describe("profile report", () => {
  const scene = { id: "meadow-0.5-4-m", name: "meadow 0.5, 4 m", group: "meadow-0.5", budget: { grass: 1 } };

  it("uses GPU timestamps for the whole frame when every frame has them", () => {
    const result = sceneResult(scene, {
      passFrames: [{ grass: 0.8, terrain: 1 }, { grass: 1.2, terrain: 1 }, { grass: 0.9, terrain: 1 }],
      gpuFrameMs: [1.8, 2.2, 1.9],
      wallFrameMs: [5, 6, 7]
    });
    expect(result.frameSource).toBe("gpu-timestamps");
    expect(result.frame).toEqual({ median: 1.9, p90: 2.14 });
    expect(result.wallClock).toEqual({ median: 6, p90: 6.8 });
    expect(result.frames).toBe(3);
    expect(withinBudget(result, "grass")).toBe(true);
    expect(withinBudget(result, "frame")).toBeNull();
  });

  it("falls back to the wall clock, with no passes, without timestamps", () => {
    const result = sceneResult(scene, {
      passFrames: [null, null],
      gpuFrameMs: [null, null],
      wallFrameMs: [10, 20]
    });
    expect(result.frameSource).toBe("wall-clock");
    expect(result.frame).toEqual({ median: 15, p90: 19 });
    expect(result.passes).toEqual({});
    expect(withinBudget(result, "grass")).toBeNull();
  });

  it("builds JSON naming the browser, adapter, canvas and library version", () => {
    const report = buildReport({
      date: "2026-09-30T12:00:00.000Z",
      userAgent: "Test browser",
      info: { vendor: "acme", architecture: "gen-1", device: "", description: "Acme GPU", extra: 1 },
      timestampQueries: true,
      canvas: { width: 1920, height: 1080, limitedByScreen: false },
      libraryVersion: "1.1.0",
      warmFrames: 40,
      frames: 120,
      scenes: [sceneError(scene, "Device lost")]
    });
    const parsed = JSON.parse(JSON.stringify(report));
    expect(parsed).toEqual({
      format: "vistawasm-gpu-profile",
      formatVersion: 1,
      date: "2026-09-30T12:00:00.000Z",
      library: { name: "@vista-wasm/vista-wasm", version: "1.1.0" },
      browser: { userAgent: "Test browser" },
      adapter: { vendor: "acme", architecture: "gen-1", device: "", description: "Acme GPU", software: false },
      timestampQueries: true,
      canvas: { width: 1920, height: 1080, limitedByScreen: false },
      settings: { renderScale: 1, dynamicResolution: false, devicePixelRatio: 1, warmFrames: 40, frames: 120 },
      scenes: [
        {
          id: "meadow-0.5-4-m",
          name: "meadow 0.5, 4 m",
          group: "meadow-0.5",
          budget: { grass: 1 },
          frames: 0,
          frame: null,
          frameSource: null,
          wallClock: null,
          passes: {},
          error: "Device lost"
        }
      ]
    });
  });

  it("writes Markdown rows that flag a broken budget", () => {
    const over = sceneResult(scene, {
      passFrames: [{ grass: 1.5, terrain: 1 }],
      gpuFrameMs: [2.5],
      wallFrameMs: [4]
    });
    const markdown = reportToMarkdown(buildReport({
      date: "2026-09-30T12:00:00.000Z",
      userAgent: "Test browser",
      info: { vendor: "acme", architecture: "", device: "", description: "Acme GPU" },
      timestampQueries: true,
      canvas: { width: 1920, height: 1080 },
      libraryVersion: "1.1.0",
      warmFrames: 40,
      frames: 120,
      scenes: [over]
    }));
    expect(markdown).toContain("2026-09-30: acme, Acme GPU; Test browser; 1920 x 1080;");
    expect(markdown).toContain("| meadow-0.5-4-m | 2.50 | 2.50 | grass 1 | **over** | grass 1.50, terrain 1.00 |");
  });

  it("recognises software adapters, whose times are not judged against budgets", () => {
    expect(isSoftwareAdapter({ vendor: "google", architecture: "swiftshader" })).toBe(true);
    expect(isSoftwareAdapter({ vendor: "mesa", description: "llvmpipe (LLVM 19.1.1, 256 bits)" })).toBe(true);
    expect(isSoftwareAdapter({ vendor: "apple", architecture: "metal-3" })).toBe(false);
    expect(isSoftwareAdapter(null)).toBe(false);
  });

  it("reports an empty adapter when the browser gives no adapter info", () => {
    const report = buildReport({
      date: "",
      userAgent: "",
      info: null,
      timestampQueries: false,
      canvas: { width: 480, height: 270 },
      libraryVersion: "1.1.0",
      warmFrames: 0,
      frames: 1,
      scenes: []
    });
    expect(report.adapter).toEqual({ vendor: "", architecture: "", device: "", description: "", software: false });
    expect(report.canvas.limitedByScreen).toBe(false);
  });
});

describe("budgeted scenes", () => {
  it("names every scene with a unique id that URL lists can hold", () => {
    const ids = budgetedScenes.map((scene: { id: string }) => scene.id);
    expect(new Set(ids).size).toBe(ids.length);

    for (const id of ids) {
      expect(id).toMatch(/^[A-Za-z0-9.-]+$/);
    }
  });

  it("covers the fixed, grass, jungle, meadow and sky scenes", () => {
    expect(budgetedGroups).toEqual(["fixed", "fixed-grass", "jungle", "meadow-0.5", "meadow-4", "sky"]);
    expect(budgetedScenes).toHaveLength(3 + 3 + 3 + 4 + Object.keys(skyScenes).length);
    expect(Object.keys(skyScenes)).toEqual([
      "cumulus from below",
      "low sun",
      "mackerel sky",
      "veiled sun",
      "rain deck"
    ]);
  });
});
