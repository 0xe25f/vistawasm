import { VistaWasmError } from "./errors.js";

/**
 * Browser feature support returned before engine creation.
 */
export interface VistaWasmSupport {
  /**
   * Whether WebGPU is exposed by the browser.
   */
  webGpu: boolean;

  /**
   * Whether WebAssembly is available.
   */
  webAssembly: boolean;

  /**
   * Whether ES modules can use `import.meta.url`.
   */
  esModules: boolean;
}

/**
 * Detect browser support needed by VistaWASM.
 */
export function detectVistaWasmSupport(globalObject: typeof globalThis = globalThis): VistaWasmSupport {
  const navigatorLike = globalObject.navigator as Navigator & { gpu?: unknown } | undefined;

  return {
    webGpu: Boolean(navigatorLike?.gpu),
    webAssembly: typeof globalObject.WebAssembly === "object",
    esModules: typeof import.meta.url === "string"
  };
}

/**
 * Throw a clear error when the browser cannot run VistaWASM.
 */
export function assertVistaWasmSupport(globalObject: typeof globalThis = globalThis): void {
  const support = detectVistaWasmSupport(globalObject);

  if (!support.webAssembly) {
    throw new VistaWasmError(
      "INTERNAL_ERROR",
      "WebAssembly is not available in this browser."
    );
  }

  if (!support.webGpu) {
    throw new VistaWasmError(
      "WEBGPU_UNAVAILABLE",
      "WebGPU is not available in this browser. VistaWASM requires WebGPU and cannot fall back to WebGL2."
    );
  }
}
