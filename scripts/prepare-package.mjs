import { rmSync } from "node:fs";

// wasm-pack writes a `.gitignore` of `*` into its output, which npm also
// reads, so without this `npm pack` would leave out the WASM and its glue.
rmSync("dist/pkg/.gitignore", { force: true });
