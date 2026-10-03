import { fileURLToPath, URL } from "node:url";
import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";
import vue from "@vitejs/plugin-vue";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// The dev server always aliases to the built package output rather than the
// TypeScript source. The published package resolves its WASM glue relative
// to `dist/index.js`, so aliasing to the same built file keeps every dev
// server (the demo and each framework example) behaving exactly like a real
// consumer that installed `@vista-wasm/vista-wasm` from npm. Run `npm run
// build` once before `npm run dev`.
// The Content Security Policy the built vanilla example ships with: its
// scripts, styles and WASM come only from its own origin, and
// 'wasm-unsafe-eval' lets the engine compile its WASM. The dev server
// injects styles inline for hot reloading, so the tag goes into built
// pages only.
const VANILLA_CSP = "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; object-src 'none'; base-uri 'none'; form-action 'none'";

function vanillaContentSecurityPolicy(): Plugin {
  let vanilla = false;

  return {
    name: "vanilla-content-security-policy",
    apply: "build",
    configResolved(config) {
      vanilla = config.root === fileURLToPath(new URL("./examples/vanilla", import.meta.url));
    },
    transformIndexHtml() {
      return vanilla
        ? [{ tag: "meta", attrs: { "http-equiv": "Content-Security-Policy", content: VANILLA_CSP }, injectTo: "head-prepend" }]
        : [];
    }
  };
}

export default defineConfig({
  plugins: [react(), vue(), svelte(), vanillaContentSecurityPolicy()],
  // Type tests (`*.test-d.ts`) run with `vitest run`, so public
  // declarations are checked alongside the runtime tests.
  test: {
    typecheck: {
      enabled: true,
      tsconfig: "./js/tests/tsconfig.json"
    }
  },
  resolve: {
    alias: {
      "@vista-wasm/vista-wasm": fileURLToPath(new URL("./dist/index.js", import.meta.url))
    }
  },
  server: {
    fs: {
      allow: [fileURLToPath(new URL(".", import.meta.url))]
    },
    // VistaWASM does not need cross-origin isolation today. The dev server
    // sets these anyway, so nothing in the demo or examples comes to rely
    // on a cross-origin resource a threaded build would block.
    headers: {
      "Cross-Origin-Opener-Policy": "same-origin",
      "Cross-Origin-Embedder-Policy": "require-corp"
    }
  }
});
