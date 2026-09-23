import { fileURLToPath, URL } from "node:url";
import { defineConfig } from "vite";

// VistaWASM is linked from the repository root (`file:..`) and loads its
// WASM glue relative to its own module URL, so it must not be pre-bundled,
// and the dev server must be allowed to serve files from the parent folder.
export default defineConfig({
  optimizeDeps: {
    exclude: ["@vista-wasm/vista-wasm"]
  },
  server: {
    fs: {
      allow: [fileURLToPath(new URL("..", import.meta.url))]
    }
  }
});
