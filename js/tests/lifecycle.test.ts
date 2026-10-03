import { afterEach, describe, expect, it, vi } from "vitest";
import { VistaWasmError } from "../src/errors";
import { createVistaEngine, initialiseVistaWasm } from "../src/index";
import type { VistaEngine, VistaWasmGeneratedModule, VistaWasmRawEngine } from "../src/types";

// What the wrapper does when the WASM instance traps: release builds
// abort on a panic, which surfaces as a `WebAssembly.RuntimeError` and
// leaves the instance unusable.

const handle = { id: 1, metadata: { warnings: [] } };
let raw: Partial<VistaWasmRawEngine> = {};
const fakeModule = {
  VistaEngine: { create: async () => new Proxy({}, { get: (_target, name: string) => name === "then" ? undefined : (raw as Record<string, unknown>)[name] ?? (() => undefined) }) }
} as unknown as VistaWasmGeneratedModule;

async function engine(calls: Partial<VistaWasmRawEngine>): Promise<VistaEngine & { fatal: unknown[] }> {
  raw = calls;
  vi.stubGlobal("navigator", { gpu: {} });
  await initialiseVistaWasm({ wasmModule: fakeModule });
  const created = Object.assign(await createVistaEngine({} as HTMLCanvasElement), { fatal: [] as unknown[] });
  created.on("fatalError", (error) => created.fatal.push(error));
  return created;
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("a WebAssembly trap", () => {
  it("marks the engine dead, reports it once, and refuses every later call", async () => {
    const trapped = await engine({
      renderOnce: () => {
        throw new WebAssembly.RuntimeError("unreachable");
      }
    });

    expect(() => trapped.renderOnce()).toThrow(/stopped after an internal error \(unreachable\)/);
    expect(trapped.fatal).toHaveLength(1);
    expect(trapped.fatal[0]).toBeInstanceOf(VistaWasmError);

    for (const call of [() => trapped.renderOnce(), () => trapped.setSun({ azimuthDegrees: 0, elevationDegrees: 10, intensity: 1 }), () => trapped.exportHeightmap()]) {
      expect(call).toThrow(expect.objectContaining({
        code: "ENGINE_DISPOSED",
        message: expect.stringMatching(/stopped after an internal error \(unreachable\) and must be recreated/)
      }));
    }

    await expect(trapped.generateFractal({ seed: 1, size: 16, horizontalScaleMetres: 12, verticalScale: 1 })).rejects.toMatchObject({ code: "ENGINE_DISPOSED" });
    expect(trapped.fatal).toHaveLength(1);
    // Disposing a dead engine does not call into the trapped instance.
    trapped.dispose();
  });

  it("ends an async call whose promise a trap left unsettled", async () => {
    const trapped = await engine({ generateFractal: () => new Promise(() => undefined) });
    // Node has no page to raise error events on, so a stand-in does.
    const page = new EventTarget();
    vi.stubGlobal("addEventListener", page.addEventListener.bind(page));
    vi.stubGlobal("removeEventListener", page.removeEventListener.bind(page));
    const generating = trapped.generateFractal({ seed: 1, size: 16, horizontalScaleMetres: 12, verticalScale: 1 });
    await Promise.resolve();
    await Promise.resolve();
    page.dispatchEvent(Object.assign(new Event("error"), { error: new WebAssembly.RuntimeError("unreachable") }));
    await expect(generating).rejects.toThrow(/stopped after an internal error/);
    expect(trapped.fatal).toHaveLength(1);
    expect(() => trapped.renderOnce()).toThrow(expect.objectContaining({ code: "ENGINE_DISPOSED" }));
  });

  it("is told apart from ordinary engine errors, which leave the engine usable", async () => {
    let fail = true;
    const usable = await engine({
      setSun: () => {
        if (fail) {
          throw Object.assign(new Error("Options are not valid: sun.intensity"), { code: "OPTIONS_INVALID" });
        }
      },
      generateFractal: async () => handle
    });

    expect(() => usable.setSun({ azimuthDegrees: 0, elevationDegrees: 10, intensity: -1 })).toThrow(expect.objectContaining({ code: "OPTIONS_INVALID" }));
    fail = false;
    usable.setSun({ azimuthDegrees: 0, elevationDegrees: 10, intensity: 1 });
    await expect(usable.generateFractal({ seed: 1, size: 16, horizontalScaleMetres: 12, verticalScale: 1 })).resolves.toBe(handle);
    expect(usable.fatal).toHaveLength(0);
  });
});

describe("GPU events", () => {
  it("turn uncaught GPU errors into gpuError events and a lost device into one deviceLost", async () => {
    let pending: string | undefined = "\nValidation Error: buffer usage is 0";
    let frame = 0;
    const gpu = await engine({
      renderOnce: () => ({ frameIndex: (frame += 1) }),
      takeGpuEvents: () => {
        const events = pending;
        pending = undefined;
        return events;
      }
    });
    const [errors, lost]: unknown[][] = [[], []];
    gpu.on("gpuError", (error) => errors.push(error));
    gpu.on("deviceLost", (error) => lost.push(error));

    gpu.renderOnce();
    expect(errors).toEqual([expect.objectContaining({ code: "GPU_ERROR", message: "The GPU reported an error: Validation Error: buffer usage is 0" })]);
    expect(lost).toHaveLength(0);

    pending = "the browser lost the device: power off";
    gpu.renderOnce();
    pending = "the browser lost the device: again";
    gpu.renderOnce();
    expect(lost).toEqual([expect.objectContaining({ code: "WEBGPU_DEVICE_LOST", message: expect.stringContaining("power off") })]);
    expect(gpu.fatal).toHaveLength(0);
  });
});
