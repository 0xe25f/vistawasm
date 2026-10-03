import { describe, expect, it } from "vitest";
import { VistaWasmError } from "../src/errors";
import { initialiseVistaWasm } from "../src/index";
import type { VistaWasmGeneratedModule } from "../src/types";

// The loader's state is per module, and vitest gives each test file its
// own, so these run in order against a loader that has loaded nothing.

describe("initialiseVistaWasm()", () => {
  it("checks its options before loading anything", async () => {
    await expect(initialiseVistaWasm(null as never)).rejects.toThrow(/initialiseVistaWasm\(\) expects an options object/);
    await expect(initialiseVistaWasm({ moduleUrl: 42 as never })).rejects.toThrow(/moduleUrl must be a string or a URL/);
    await expect(initialiseVistaWasm(JSON.parse('{ "__proto__": { "moduleUrl": "https://example.com/x.js" } }'))).rejects.toMatchObject({ code: "OPTIONS_INVALID" });
  });

  it("loads the glue next to the package by default, and reports a failure with a typed error", async () => {
    const error = await initialiseVistaWasm().then(() => undefined, (error: unknown) => error);
    const expected = new URL("../src/pkg/vista_wasm.js", import.meta.url).href;
    expect(error).toBeInstanceOf(VistaWasmError);
    expect(error).toMatchObject({ code: "WASM_LOAD_FAILED", message: expect.stringContaining(`module (${expected})`) });
    expect((error as VistaWasmError).message).toMatch(/application\/wasm, or pass wasmUrl or moduleUrl/);
    // The original error is kept for debugging.
    expect((error as VistaWasmError).details).toBeInstanceOf(Error);
  });

  it("does not swallow a WASM that fails to instantiate, and does not cache the failure", async () => {
    const failure = new WebAssembly.CompileError("bad magic number");
    const broken = { default: async () => { throw failure; }, VistaEngine: {} } as unknown as VistaWasmGeneratedModule;
    await expect(initialiseVistaWasm({ wasmModule: broken })).rejects.toMatchObject({
      code: "WASM_LOAD_FAILED",
      message: expect.stringContaining("bad magic number"),
      details: failure
    });

    const seen: unknown[] = [];
    const working = { default: async (url: unknown) => void seen.push(url), VistaEngine: {} } as unknown as VistaWasmGeneratedModule;
    await initialiseVistaWasm({ wasmModule: working, wasmUrl: "/assets/vista.wasm" });
    // The URL reaches the generated loader as given, in the object form it
    // expects; with none and a module of the caller's own, it uses its own
    // default next to itself.
    expect(seen).toEqual([{ module_or_path: "/assets/vista.wasm" }]);
  });
});
