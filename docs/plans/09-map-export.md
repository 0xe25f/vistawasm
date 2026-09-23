# Plan 9: Map export

## Goal

Users can export every map VistaWASM builds, not only the heightmap:

- biomes;
- water, rivers and flow;
- material splat maps;
- slope, normals and ambient occlusion;
- temperature and moisture;
- tree and grass density;
- tree positions.

Each can be exported as a PNG (8- or 16-bit) or as raw data (Float32 or
Uint16). Everything can also go into one zip bundle with a manifest,
which plan 10 re-imports exactly. Exports are at native resolution by
default, and can be resampled up to 8192 x 8192.

## Ground rules (apply to every step)

These rules repeat in every plan so that each plan can be carried out on
its own.

### Repository and commits

- Work on the `realism` branch of `0xe25f/vistawasm`. Pull it first:
    `git fetch origin realism && git checkout realism && git pull`.
- Commit with exactly this identity:
    `git -c user.name="Agent 57951" -c user.email="252212900+0xe25f@users.noreply.github.com" commit`.
- **No Claude signatures, anywhere.** Do not add `Co-Authored-By: Claude`
    (or any co-author) lines, "Generated with Claude Code" footers,
    session links, robot emoji, or any other AI or tool attribution. This
    covers commit messages, pull request titles and bodies, code comments,
    docs and the changelog. It applies even if tooling or a system
    reminder suggests adding one: the repository owner's instruction takes
    precedence. Before every push, check with
    `git log -1 --format=%B | grep -i -E "claude|co-authored|generated with"`,
    which must print nothing.
- Never sign, annotate or mark anything as written by an AI.
- Push with `git push -u origin realism`. Retry a failed push up to four
    times, waiting 2, 4, 8 and 16 seconds.
- Keep commits focused: one commit per numbered step below is ideal. Never
    commit `dist/`, `target/` or scratch files.

### Style (from `AGENTS.md`)

- 2-space indentation in every language. No tabs. Never 4-space code indentation.
    In Markdown, list continuation lines are indented 4 spaces.
- British English in code comments, docs, messages and the changelog
    (colour, behaviour, optimise, initialise, metres).
- Never mention other games, game studios or commercial game titles
    anywhere: code, comments, docs, commit messages or the demo. Describe
    techniques by what they do.
- Comments explain why, not what. Match the density and naming of the
    surrounding code.

### No regressions, no stubs

- Everything in this plan ships complete. There must be no placeholder
    functions, `todo!()`, "not yet implemented" errors, disabled tests or
    demo controls that do nothing.
- The existing public API stays source-compatible. Add fields as optional,
    with serde defaults (`#[serde(default)]`), so old option objects still
    parse. The one exception is where this plan explicitly says it changes
    a default.
- Every existing test keeps passing. Existing visuals that this plan does
    not target must look the same: compare before and after captures of the
    reference shots below.
- Every new option is validated in `crates/vista_wasm/src/config.rs`, with
    a message saying what was wrong and the valid range. It is typed and
    documented in `js/src/types.ts`, and documented in
    `docs/options-reference.md`.

### Rendering rules learnt the hard way

- `FrameUniforms` is declared once, in `crates/vista_wasm/src/shaders/common.wgsl`,
    and mirrored by the Rust struct in `crates/vista_wasm/src/render/gpu.rs`.
    Only append fields (as `vec4<f32>`) at the end of both. Update the
    compile-time size assertion in `gpu.rs` and the byte count in
    `docs/architecture.md`.
- Chrome's WGSL compiler (Tint) rejects `textureSample` (implicit
    derivatives) inside non-uniform control flow, including after a
    non-uniform early `return`. naga in `cargo test` does not catch this. Use
    `textureSampleLevel` or `textureSampleGrad` after any branch that
    depends on per-pixel values, and always verify in Chromium (below). A
    shader that fails there renders black.
- GPUs shade pixels in 2 x 2 quads and larger groups. Skipping work on
    scattered single pixels saves nothing. Skip work for whole regions or
    in separate, smaller passes.
- Per-pixel `time * something` terms that differ between neighbouring
    pixels shimmer. Derive such values from uniforms, not per pixel.
- Hold 60 FPS. Budgets are given per plan in milliseconds of GPU time at
    1920 x 1080 on a mid-range GPU (roughly an Apple M-series base chip or
    a desktop RTX 3060). Dynamic resolution (`RenderQualityOptions.dynamicResolution`)
    is a safety net, not a budget.
- Tiny files: the gzipped `dist/pkg/vista_wasm_bg.wasm` is 237,820 bytes
    at the start of this work. Each plan states how much it may add. Check
    with `gzip -9 -c dist/pkg/vista_wasm_bg.wasm | wc -c`. Generate data
    procedurally at start-up instead of embedding it.

### Commands

Run all of these before every push. All must pass with no new warnings:

```sh
cargo fmt --all
cargo clippy -q --all-targets     # no warnings beyond the existing ones
cargo test -q
npm run build                     # wasm-pack plus tsc
npx tsc -p tsconfig.json --noEmit
npx vitest run
npm run lint:indent
```

### Visual verification in headless Chromium

`scripts/visual-check/` renders the engine with software WebGPU and saves
PNGs. Serve the repository root, then capture:

```sh
python3 -m http.server 8124 &
node scripts/visual-check/capture.mjs /tmp/shot.png '{"size":[960,600]}' 3
```

- The config JSON takes `size`, `engine` (`createVistaEngine` options),
    `terrain` (`generateFractal` options), `set` (`{ "setWeather": {...} }`
    calls), `camera`, and `shots` (a list of camera or `set` changes, one
    PNG each). See the header of `capture.mjs`.
- The script exits non-zero on any page error or console error. Treat
    that as a failed check.
- If Playwright is not resolvable, set `PLAYWRIGHT_MODULE` to its
    `index.mjs`, and `CHROMIUM_PATH` to a Chromium binary.
- Software rendering takes seconds per frame. Use small sizes (480 x 300)
    for iteration and 960 x 600 for final shots. Relative GPU times
    between passes are meaningful; absolute times are not.
- Open every PNG you produce and look at it. A check that was not looked
    at did not happen.
- Reference shots to capture before starting and again at the end, to
    prove nothing else changed:
    1. The default config: `{"size":[960,600]}`.
    2. Rain: `{"size":[960,600],"set":{"setWeather":{"enabled":true,"state":"rain","autoCycle":false,"transitionSeconds":0.1}}}`
        with 4 frames.
    3. A forest close-up:
        `{"size":[960,600],"shots":[{"biome":"innerForest","back":40,"height":6}]}`.

### Documentation and demo

- Update `CHANGELOG.md` under the `## [1.1.0]` heading (it is unreleased).
    Put new features under `### Added` and behaviour changes under
    `### Changed`.
- Update the relevant guide in `docs/` and `docs/options-reference.md`.
    Add a short, runnable example for each new public function.
- Every new option gets a working control in the demo (`demo/index.html`
    and `demo/src/main.js`), in the right collapsible section, with a
    visible value readout for sliders.

### Finishing

Report what changed, how it was tested (commands and shots), the size and
GPU-time deltas, and any risks. Keep the report short.

## Decisions already made

- Maps: biome map, water/rivers/flow, material splat and slope (plus
    normals and occlusion), vegetation (tree positions and species, and
    tree and grass density maps).
- Formats: PNG (8 and 16-bit), raw Float32 and Uint16, and one zip bundle
    with `manifest.json`.
- Resolution: native by default, with optional resampling to any size up
    to 8192 x 8192.
- GeoTIFF export was not chosen.

## Prerequisites

- **Plans 1 to 5** provide the data:
    - drainage area and flow (plan 1);
    - temperature and permanent snow (plan 2);
    - the water, river and waterfall data and the distance-to-water field
        (plan 3);
    - the tree niches (plan 4);
    - the cover texture and the lattice tree generator (plan 5).
- Check that each exists. If one is missing, carry out that plan first.

## Current state

- `engine.exportHeightmap(options?)` returns a `Uint8Array` of
    little-endian Float32 heights (`ExportHeightmapOptions.format:
    "float32-le"`).
- `js/src/terrain-export.ts` has:
    - `readHeightmapFloats`, `computeHeightmapPixels`,
        `renderHeightmapToCanvas` and `exportHeightmapImage` (8-bit PNG
        via `canvas.toBlob`);
    - `exportTerrainObj`;
    - `downloadRawHeightmap`, `downloadBlob` and `downloadText`.
- `docs/export-and-snapshots.md` documents these.
- Current gzipped sizes: `dist/index.js` 5,020 bytes; the WASM file
    237,820 bytes (plus whatever plans 1 to 8 added).

## Design

### 1. Engine side: raw map data (Rust, `export.rs`)

- Add `engine.exportMap(kind, options?)`, which returns an `ExportedMap`:
    `{ kind, width, height, channels, type: "float32" | "uint8" | "uint16", data, encoding }`.
    - `encoding` describes how to read the values: units, scale and
        offset where quantised, and a legend for categorical maps.

| Kind | Type and channels | Meaning |
| --- | --- | --- |
| `height` | float32 x 1 | Metres above the datum. Same values as `exportHeightmap`. |
| `biome` | uint8 x 1 | `BiomeKind` index. The legend lists name and colour for every index. |
| `water` | uint8 x 1 | 0 none, 1 river, 2 lake, 3 ocean, 4 waterfall, 5 painted river, 6 painted lake. The legend is included. |
| `waterDepth` | float32 x 1 | Water depth in metres (0 on dry land). |
| `flow` | float32 x 1 | Upstream drainage area in km^2. |
| `discharge` | float32 x 1 | Mean discharge in m^3/s (plan 3). |
| `materials` | uint8 x 12 | Material weights in `MAT_*` order (10 used, 2 reserved). |
| `slope` | float32 x 1 | Slope in degrees. |
| `normals` | float32 x 3 | World-space unit normals (x, y, z). |
| `occlusion` | uint8 x 1 | Ambient occlusion, 0 occluded to 255 open. |
| `temperature` | float32 x 1 | Mean annual °C (plan 2). |
| `moisture` | uint8 x 1 | 0 arid to 255 saturated. |
| `treeDensity` | uint8 x 1 | Trees per hectare / 4, clamped to 255, at the current density (plan 5's `target_density x suitability`). |
| `grassDensity` | uint8 x 1 | 0 to 255 cover at the current grass density. |

- Every map comes from the same data the renderer uses, so exports match
    what you see.
- **Resampling:** `options.size` is `[width, height]`, each from 2 to
    8192.
    - Continuous maps are bicubic (height, flow and so on).
    - Normals are bilinear, then renormalised.
    - Categorical maps (`biome`, `water`) are nearest.
    - `materials` are bilinear, then renormalised to sum 255.
    - The default is native size.
    - Resampling runs in Rust, row by row.
    - Reject sizes that would need more than 1 GB of output, with a clear
        error.
- **Trees:** `engine.exportTrees(options?)` returns `TreeRecord[]`:
    `{ x, y, z, species, variant, scale, rotation, tint, dryness }`.
    - It covers the whole map, or `options.region`
        (`{ minX, minZ, maxX, maxZ }` in metres), at the current density,
        from the complete lattice (plan 5). This includes near-field trees
        that the renderer streams, generated on the CPU with the same
        hash, so the list is exact.
    - `y` is the grounded height using plan 4's root rule against the
        full-resolution height map.
    - `options.maxCount` defaults to 2,000,000. Exceeding it throws
        `INVALID_OPTIONS`, with a message suggesting `region`.
    - Hand-placed trees from `setTreeInstances` are included, marked
        `handPlaced: true`.

### 2. Encoders in TypeScript (no dependencies)

Add `js/src/map-export.ts`:

- **`encodePng(map, options?)`** returns `Promise<Blob>`, writing PNG
    directly:
    - the IHDR, IDAT and IEND chunks;
    - a CRC-32 table computed once;
    - zlib compression through the platform `CompressionStream("deflate")`,
        which produces zlib-wrapped data as PNG requires;
    - scanline filter 0 for speed, or filter 2 (up) when `options.smaller`
        is set.
- **Colour types:**
    - Gray 8 or 16 for single-channel maps. With `bitDepth: 16`, values
        are scaled to 0 to 65535 over `options.range`, which defaults to
        the map's min and max. The range is written to a `tEXt` chunk
        (`vistawasm:range`) and returned.
    - RGB 8 for `normals`, encoded as (n + 1) / 2 x 255.
    - RGBA 8 for `materials`. It is split into three files: `splat0`
        (lush, dry, forest, sand), `splat1` (rock, snow, mud, volcanic)
        and `splat2` (ice, tundra, reserved, reserved). `encodePng`
        returns an array of three Blobs for `materials`.
    - Palette (PLTE) for `biome` and `water`, with the legend colours, so
        the files open as readable colour maps.
- **`encodeRaw(map, options?)`** returns `Uint8Array`:
    - Float32 little-endian as is;
    - Uint16 little-endian scaled over a range, when
        `options.type: "uint16"` is given; the range is returned.
- **`treesToCsv(trees)`** returns a string with a header row.
    **`treesToJson(trees)`** returns a string.
- **`exportBundle(engine, options?)`** returns `Promise<Blob>`, a zip.
    - Write the zip directly: local headers, the central directory and
        the end record, with deflate through
        `CompressionStream("deflate-raw")` and CRC-32. Use ZIP64 when over
        4 GB, or reject: size checks keep bundles under 4 GB unless
        ZIP64 is implemented. Pick one, and test it.
    - Contents:
        - `manifest.json`;
        - `height.f32` (exact);
        - `biome.png`, `water.png`, `flow.f32`, `discharge.f32`;
        - `splat0.png`, `splat1.png`, `splat2.png`;
        - `slope.png` (8-bit, 0 to 90 degrees);
        - `normals.png`, `occlusion.png`, `temperature.f32`,
            `moisture.png`;
        - `treeDensity.png`, `grassDensity.png`;
        - `trees.csv` (when `options.trees` is true, which is the
            default when the count is under `maxCount`);
        - optional previews (`preview-height.png` hypsometric and
            `preview-biome.png`) when `options.previews` is set.
- **`manifest.json` schema**, versioned for plan 10:

    ```json
    {
      "format": "vistawasm-bundle",
      "version": 1,
      "generator": "vistawasm-fractal-0.2.0",
      "terrain": {
        "width": 512,
        "height": 512,
        "metresPerSample": 12,
        "seaLevelMetres": 0,
        "minHeightMetres": -140.2,
        "maxHeightMetres": 1893.4
      },
      "options": { "...": "the full engine options in effect, as JSON" },
      "files": [
        { "path": "height.f32", "kind": "height", "type": "float32", "width": 512, "height": 512, "channels": 1 },
        { "path": "biome.png", "kind": "biome", "type": "uint8", "legend": [{ "index": 0, "name": "grassyMeadows", "colour": [0.4, 0.7, 0.3] }] }
      ]
    }
    ```

    - `options` holds everything needed to rebuild the same scene:
        seed, fractal options, biome, water, flora, grass, weather and
        quality options. `engine.getOptionsSnapshot()` returns them.
        Add it if missing, as a public method returning a
        JSON-serialisable `VistaEngineOptions`.
- **`downloadBundle(engine, filename, options?)`**: a convenience
    wrapper using the existing `downloadBlob`.

## Public API

Additive:

```ts
export type MapKind =
  | "height" | "biome" | "water" | "waterDepth" | "flow" | "discharge"
  | "materials" | "slope" | "normals" | "occlusion" | "temperature"
  | "moisture" | "treeDensity" | "grassDensity";

export interface MapEncoding {
  units?: string;
  range?: [number, number];
  legend?: { index: number; name: string; colour: [number, number, number] }[];
}

export interface ExportedMap {
  kind: MapKind;
  width: number;
  height: number;
  channels: number;
  type: "float32" | "uint8" | "uint16";
  data: Float32Array | Uint8Array | Uint16Array;
  encoding: MapEncoding;
}

export interface ExportMapOptions { size?: [number, number] }

export interface TreeRecord {
  x: number; y: number; z: number;
  species: TreeSpecies; variant: number;
  scale: number; rotation: number; tint: number; dryness: number;
  handPlaced?: boolean;
}

export interface ExportTreesOptions {
  region?: { minX: number; minZ: number; maxX: number; maxZ: number };
  maxCount?: number;
}

export interface VistaEngine {
  // ...existing...
  exportMap(kind: MapKind, options?: ExportMapOptions): ExportedMap;
  exportTrees(options?: ExportTreesOptions): TreeRecord[];
  getOptionsSnapshot(): VistaEngineOptions;
}

export function encodePng(map: ExportedMap, options?: { bitDepth?: 8 | 16; range?: [number, number]; smaller?: boolean }): Promise<Blob | Blob[]>;
export function encodeRaw(map: ExportedMap, options?: { type?: "float32" | "uint16"; range?: [number, number] }): { bytes: Uint8Array; range?: [number, number] };
export function treesToCsv(trees: TreeRecord[]): string;
export function treesToJson(trees: TreeRecord[]): string;
export function exportBundle(engine: VistaEngine, options?: { size?: [number, number]; trees?: boolean; previews?: boolean; maxTrees?: number }): Promise<Blob>;
export function downloadBundle(engine: VistaEngine, filename: string, options?: Parameters<typeof exportBundle>[1]): Promise<void>;
```

- Validate every input at the boundary (`TypeError` or `VistaWasmError`
    with `INVALID_OPTIONS`):
    - an unknown kind;
    - sizes that aren't integers, or are outside 2 to 8192;
    - a region that isn't finite, or has min greater than max;
    - `maxCount` not from 1 to 10,000,000;
    - calling while terrain is generating (reuse the existing message
        style used by `exportHeightmap`).
- `exportHeightmap` and the existing helpers are unchanged.

## Steps

1. Add the Rust `export.rs` with every map kind and resampling, the
    wasm-bindgen methods and the TypeScript wrapper methods, with tests.
2. Add `exportTrees`, with determinism and consistency tests against the
    renderer's lattice.
3. Add `getOptionsSnapshot` if it is missing.
4. Add `js/src/map-export.ts` with the PNG, raw and zip encoders, CSV and
    JSON, and the bundle, and export them from `js/src/index.ts`.
5. Demo, a new "Export" section:
    - a map-kind select, a size select (native, 1024, 2048, 4096,
        8192), and a format select (PNG 8-bit, PNG 16-bit, raw
        Float32, raw Uint16);
    - "Export map", "Export trees (CSV)" and "Export bundle (.zip)"
        buttons;
    - a status line with file sizes. Keep the existing heightmap and OBJ
        exports.
6. Docs:
    - `docs/export-and-snapshots.md`: new sections for maps, trees and
        bundles, with examples and the manifest schema;
    - `docs/options-reference.md` (types);
    - `CHANGELOG.md`.
7. Verify, then commit and push.

## Tests

Rust:

- Every map kind's length and type match `width x height x channels`.
- The biome legend covers `BiomeKind::ALL` in order.
- Materials sum to 255 ± 2 per sample.
- A synthetic 30-degree plane exports slope 30 ± 0.5; normals are unit
    length within 1e-3.
- Nearest resampling of `biome` never produces a value absent from the
    source. Bicubic height resampling at native size is the identity.
- `exportTrees`:
    - is deterministic;
    - matches the renderer's lattice selection for a region (compare with
        plan 5's CPU emulation);
    - `maxCount` overflow is an error.

TypeScript (vitest, Node):

- **PNG:** verify the signature and chunk CRCs, inflate IDAT with
    `node:zlib` and compare pixels, for 8 and 16-bit grey (big-endian
    samples), RGB, RGBA and palette.
- **Zip:** parse the central directory, check CRCs, and inflate each
    entry with `node:zlib` (`inflateRawSync`). The manifest is valid JSON
    matching the schema. `height.f32` bytes equal `exportHeightmap()`.
- CSV has a header row and the right number of rows. JSON parses back to
    the same records.
- Every validation case above.

## Verification

- In the browser harness, call `exportBundle` through `page.evaluate`,
    return the zip as base64, save and unzip it, and open `biome.png`,
    `water.png`, `splat0.png`, `normals.png` and `treeDensity.png`. Each
    must look right against a top-down render of the same map.
- 16-bit PNG and raw Float32 heights round-trip to the same values:
    within 1 / 65535 of the range for 16-bit, and exactly for Float32.
- Export at 4096 x 4096 in the demo and check the time. It should be
    under 3 s in a browser; report the measured value.
- The reference shots are unaffected: exports don't change rendering.

## Budgets

- WASM growth: at most +8 KB gzipped.
- `dist/index.js`: at most +4 KB gzipped.
- No per-frame cost.

## Out of scope

Importing (plan 10) and GeoTIFF.
