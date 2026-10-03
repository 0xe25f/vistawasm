import type { VistaWasmGeneratedModule } from "./types.js";

/**
 * Import the generated `wasm-pack` glue from `url`. This sits in a module
 * of its own because the build strips comments from the others (see
 * `tsconfig.strip.json`), and the comment here tells Vite to leave a URL
 * known only at run time alone.
 */
export function importGlue(url: string | URL): Promise<VistaWasmGeneratedModule> {
  return import(/* @vite-ignore */ url.toString());
}
