import { beforeAll, describe, expect, it, vi } from "vitest";
import { createVistaEngine, initialiseVistaWasm } from "../src/index";
import type { VistaEngine, VistaWasmGeneratedModule } from "../src/types";

// A stand-in for the generated WASM engine that records every call, so the
// wrapper's validation and packing can be tested without a GPU.
const calls: { name: string; args: unknown[] }[] = [];
let weather: string | undefined;

const raw = new Proxy(
  {},
  {
    get(_target, name: string) {
      if (name === "then") {
        return undefined;
      }

      return (...args: unknown[]) => {
        calls.push({ name, args });

        if (name === "renderOnce") {
          return {
            frameIndex: calls.length,
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
  it("packs trees into eight floats each with defaults", () => {
    engine.setTreeInstances([
      { x: 1, y: 2, z: 3, species: "palm" },
      { x: 4, y: 5, z: 6, species: "shrub", scale: 2, rotation: 1, tint: 0.2, dryness: 0.9 }
    ]);
    const [packed] = lastCall("setTreeInstances") as [Float32Array];

    expect(Array.from(packed)).toEqual([
      1, 2, 3, 1, 0, 0.5, 3, 0,
      4, 5, 6, 2, 1, Math.fround(0.2), 7, Math.fround(0.9)
    ]);
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
