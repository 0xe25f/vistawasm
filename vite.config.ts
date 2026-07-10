import { fileURLToPath, URL } from "node:url";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import vue from "@vitejs/plugin-vue";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// The dev server always aliases to the built package output rather than the
// TypeScript source. The published package resolves its WASM glue relative
// to `dist/index.js`, so aliasing to the same built file keeps every dev
// server (the demo and each framework example) behaving exactly like a real
// consumer that installed `@vista-wasm/vista-wasm` from npm. Run `npm run
// build` once before `npm run dev`.
export default defineConfig({
  plugins: [react(), vue(), svelte()],
  resolve: {
    alias: {
      "@vista-wasm/vista-wasm": fileURLToPath(new URL("./dist/index.js", import.meta.url))
    }
  },
  server: {
    fs: {
      allow: [fileURLToPath(new URL(".", import.meta.url))]
    },
    headers: {
      "Cross-Origin-Opener-Policy": "same-origin",
      "Cross-Origin-Embedder-Policy": "require-corp"
    }
  }
});
