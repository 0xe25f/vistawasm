import { describe, expect, it } from "vitest";
import { VistaWasmError } from "../src/errors";
import { detectVistaWasmSupport } from "../src/feature-detect";

describe("VistaWasmError", () => {
  it("keeps stable error codes", () => {
    const error = new VistaWasmError("WEBGPU_UNAVAILABLE", "WebGPU is missing.");

    expect(error.code).toBe("WEBGPU_UNAVAILABLE");
    expect(error.name).toBe("VistaWasmError");
  });
});

describe("detectVistaWasmSupport", () => {
  it("reports WebGPU support from navigator", () => {
    const support = detectVistaWasmSupport({
      navigator: {
        gpu: {}
      },
      WebAssembly
    } as unknown as typeof globalThis);

    expect(support.webGpu).toBe(true);
    expect(support.webAssembly).toBe(true);
  });
});
