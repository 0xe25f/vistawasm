# Plan 10: Map import

## Goal

Users can build a world from their own maps:

- a heightmap image (8 or 16-bit PNG, or any browser image);
- a painted biome map;
- a water and river mask;
- tree and grass density masks.

A bundle exported by plan 9 re-imports exactly, recreating the same
scene. A painted biome map is absolute: each biome stays where it was
painted, with natural, irregular borders. Maps of a different size from
the terrain are resampled to fit, with a warning.

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
- Tiny files: measure the gzipped `dist/pkg/vista_wasm_bg.wasm` at the
    start of the plan with `gzip -9 -c dist/pkg/vista_wasm_bg.wasm | wc -c`
    (it was 237,820 bytes before plan 1 and 276,951 after plan 2). Each
    plan states how much it may add on top of that. Every byte must buy
    real value that can't be done smaller. Generate data procedurally at
    start-up instead of embedding it.
- Performance gate: from plan 2b onwards, run
    `node scripts/visual-check/fixed-scene.mjs` before and after the plan.
    No pass may be more than 5 % slower than before, beyond what the plan's
    own GPU budget allows. Put both sets of numbers in the report.

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

- Imports: a painted biome map, a water and river mask, vegetation
    density masks, and export bundle round-trips.
- The biome map is absolute, with natural borders: painted biomes are
    kept, and borders get noise-dithered transitions instead of hard
    lines.
- On size mismatch, resample to the terrain size (nearest for biomes,
    bilinear for masks) and warn.

## Prerequisites

- **Plan 9** (map export) must be merged. Check that
    `js/src/map-export.ts` exists with `encodePng`, `exportBundle` and the
    manifest.
- **Plan 3** provides `setWaterMask`. **Plan 5** provides the optional
    density multiplier input on the cover-texture bake.
- Check that each exists. If one is missing, carry out that plan first.

## Current state

- **Loaders:**
    - `loadDemFromArrayBuffer` and `loadDemFromUrl` (GeoTIFF, in
        `crates/vista_wasm/src/dem/geotiff.rs`);
    - `loadRawHeightmap(buffer, RawHeightmapOptions)` (uint16, int16 or
        float32, either byte order).
- There is no PNG heightmap import, no biome or vegetation mask input,
    and no bundle reader.
- `setWaterMask(mask)` takes a `Uint8Array` mask (plan 3).
- `imageToRgba()` in `js/src/index.ts` converts images to RGBA8 for
    texture replacement.

## Design

### 1. PNG decoding with full precision (TypeScript)

- Add `js/src/png-decode.ts` with `decodePng(bytes)`, returning
    `{ width, height, channels, bitDepth, data: Uint8Array | Uint16Array, text: Record<string, string> }`.
    - It parses the chunks (IHDR, PLTE, tRNS, IDAT, tEXt, IEND) and
        verifies CRCs.
    - It inflates with `DecompressionStream("deflate")`.
    - It reverses all five scanline filters, and supports Adam7
        interlacing.
    - Colour types: grey, grey with alpha, RGB, RGBA and palette, at bit
        depths 1, 2, 4, 8 and 16. 16-bit samples are big-endian.
    - It reads `tEXt` `vistawasm:range`, written by plan 9's 16-bit
        export, so the height range is recovered automatically.
    - Malformed input throws a `VistaWasmError` with `INVALID_DEM` and a
        specific message, such as "PNG chunk CRC mismatch in IDAT".
- Other formats (JPEG, WebP, AVIF): decode through `createImageBitmap`
    and an `OffscreenCanvas` at 8 bits. Report a warning that precision
    is limited to 8 bits.

### 2. Heightmap images

- Add `engine.loadHeightmapImage(source, options)`, where `source` is a
    `Blob`, `ArrayBuffer` or `Uint8Array`, and returns
    `Promise<TerrainHandle>`.
- Options:
    - `metresPerSample` (required);
    - `minHeightMetres` and `maxHeightMetres` (the defaults come from the
        PNG's `vistawasm:range` if present; otherwise both are required);
    - `seaLevelMetres`;
    - `channel`: `"luminance"` (default), `"r"`, `"g"`, `"b"` or `"a"`.
- Implement it by converting to float32 in TypeScript and calling the
    existing `loadRawHeightmap` path, so validation, normals and every
    downstream stage are shared.
- Width and height are 2 to 8192. Non-square images are allowed if the
    engine supports them; if it doesn't, reject them with a clear message.

### 3. Painted biome maps

- Add `engine.setBiomeMap(map: BiomeMap | null)`:

    ```ts
    export interface BiomeMap {
      width: number;
      height: number;
      /** One BiomeKind index per sample, row-major, north row first; 255 = not painted. */
      data: Uint8Array;
      /** Width of the dithered border in samples, 0 to 8. Defaults to 3. */
      borderSamples?: number;
    }
    ```

- Add the helper `biomeMapFromImage(image, options?)`, returning
    `Promise<BiomeMap>`:
    - Palette PNGs from plan 9 map indices directly, through the legend.
    - Colour images map each pixel to the nearest legend colour in
        CIELAB. Pixels further than ΔE 25 from every legend colour count
        as unmatched, and still take the nearest. The helper returns
        `{ map, unmatchedFraction }`, and warns when the fraction is
        above 1 %.
    - Fully transparent pixels become 255 (not painted: the engine
        classifies those itself).
    - `options.legend` accepts custom colour-to-biome pairs, for users
        painting in their own colours.
- **Engine behaviour** (Rust, `terrain/biomes.rs`):
    - `classify_surface` gains `forced: Option<&[u8]>`. Where a sample is
        forced, its biome is the painted one, and its materials follow
        that biome's rules with the local slope, height and moisture.
        Where the value is 255, classification runs as usual.
    - **Natural borders:** the forced lookup is domain-warped. Sample the
        painted map at `p + warp(p) x borderSamples`, with a 2-octave
        seeded noise (wavelengths 7 and 23 samples). Borders become
        irregular and interlocking, while interiors keep their painted
        biome. Material weights then blend across borders over
        `borderSamples` with a box average, so ground textures
        transition softly.
    - **Absolute with one physical exception:** `ocean` painted above sea
        level can't hold sea water. Those samples fall back to automatic
        classification, and a warning names the count (paint water with
        the water mask instead). Every other painted biome is kept
        exactly.
    - Individual trees still obey plan 4's physical exclusions: no trunk
        on a 60-degree cliff, even in painted forest. Document that the
        biome is absolute, but trees need somewhere to stand.
    - Climate-driven extras follow the painted biome: `iceArctic` gets
        permanent snow and glacier smoothing (plan 2); volcanic biomes get
        heat.
- Size mismatch: nearest resample to the terrain size, with a warning,
    as for the water mask.
- `null` clears the map, and the terrain reclassifies. It survives
    option changes and clears when new terrain loads, like the water mask.

### 4. Water masks from images

- Add the helper `waterMaskFromImage(image, options?)`, returning
    `Promise<WaterMask>`. The default convention is the same colours as
    plan 9's `water.png` legend: lake blue and river cyan.
    - A river's brightness sets its strength (1 to 127).
    - `options.mode: "grey"` reads raw values with the `WaterMask`
        semantics instead (0 none, 1 to 127 river, 128 to 255 lake).
- The result feeds the existing `engine.setWaterMask`.

### 5. Vegetation density masks

- Add `engine.setVegetationMasks(masks: { trees?: DensityMask | null; grass?: DensityMask | null })`,
    where `DensityMask` is `{ width, height, data: Uint8Array }`.
    - Values: 0 means none, 128 means unchanged, and 255 means 2x, capped
        at the equivalent of density 4.
    - Apply it as the multiplier input of plan 5's cover-texture bake,
        for trees and grass separately.
    - Resample bilinearly on a size mismatch, with a warning.
    - Omitted keys keep their current mask; `null` clears one.
- Add the helper `densityMaskFromImage(image, options?)`, which uses
    luminance by default.

### 6. Bundles, exactly round-tripped

- **Writer changes (plan 9's `exportBundle`):**
    - Bump the manifest to `"version": 2`.
    - Add `source-height.f32`: the heights before any carving (rivers,
        glacier smoothing, water mask). The engine gets these by applying
        its carving records in reverse to a copy. Add `engine.exportMap("sourceHeight")`
        as a new `MapKind` for this.
    - When set, add the painted inputs: `painted-biome.png` (palette),
        `water-mask.png` (grey), `tree-mask.png` and `grass-mask.png`.
    - `height.f32` stays, as the final heights, for other tools.
- **Reader:** `loadBundle(engine, source, options?)` returns
    `Promise<TerrainHandle>`.
    1. Read the zip, verifying CRCs, and inflate with
        `DecompressionStream("deflate-raw")`.
    2. Validate the manifest. Unknown or older versions are rejected with
        a message: version 1 bundles came from unreleased builds, so there
        is no migration.
    3. Load `source-height.f32` through `loadRawHeightmap` with the
        manifest's terrain metadata.
    4. Apply `options` from the manifest through the public setters:
        biomes, water, flora, grass, weather, quality and so on.
        `options.applySettings: false` skips this step.
    5. Apply the painted maps, if present, with their setters.
    - The derived maps (biome, water, flow and so on) are not imported:
        they are recomputed deterministically from source heights,
        options and painted inputs. That is what makes the round trip
        exact.
    - Emit `"progress"` events with phase `"bundle"`.
- **Loading several images at once:**
    `loadTerrainFromImages(engine, { height, biome?, water?, trees?, grass? }, options)`
    does sections 2 to 5 in one call, with one set of warnings.

## Public API

Additive:

```ts
export interface HeightmapImageImportOptions {
  metresPerSample: number;
  minHeightMetres?: number;
  maxHeightMetres?: number;
  seaLevelMetres?: number;
  channel?: "luminance" | "r" | "g" | "b" | "a";
}

export interface DensityMask { width: number; height: number; data: Uint8Array }

export interface VistaEngine {
  // ...existing...
  loadHeightmapImage(source: Blob | ArrayBuffer | Uint8Array, options: HeightmapImageImportOptions): Promise<TerrainHandle>;
  setBiomeMap(map: BiomeMap | null): void;
  setVegetationMasks(masks: { trees?: DensityMask | null; grass?: DensityMask | null }): void;
}

export function decodePng(bytes: Uint8Array): Promise<DecodedPng>;
export function biomeMapFromImage(image: Blob | ArrayBuffer | Uint8Array, options?: { legend?: { colour: [number, number, number]; biome: BiomeKind }[]; borderSamples?: number }): Promise<{ map: BiomeMap; unmatchedFraction: number }>;
export function waterMaskFromImage(image: Blob | ArrayBuffer | Uint8Array, options?: { mode?: "legend" | "grey" }): Promise<WaterMask>;
export function densityMaskFromImage(image: Blob | ArrayBuffer | Uint8Array, options?: { channel?: "luminance" | "r" | "g" | "b" | "a" }): Promise<DensityMask>;
export function loadBundle(engine: VistaEngine, source: Blob | ArrayBuffer | Uint8Array, options?: { applySettings?: boolean }): Promise<TerrainHandle>;
export function loadTerrainFromImages(engine: VistaEngine, images: { height: Blob | ArrayBuffer | Uint8Array; biome?: Blob | ArrayBuffer | Uint8Array; water?: Blob | ArrayBuffer | Uint8Array; trees?: Blob | ArrayBuffer | Uint8Array; grass?: Blob | ArrayBuffer | Uint8Array }, options: HeightmapImageImportOptions): Promise<TerrainHandle>;
```

- The heightmap import options are named `HeightmapImageImportOptions`,
    because `HeightmapImageOptions` already exists for export colour modes.
- Validate everything at the boundary:
    - types (`TypeError`);
    - sizes 2 to 8192;
    - data length equal to width x height;
    - finite metre values, and min below max;
    - `borderSamples` from 0 to 8;
    - biome indices below the count, or 255.
- Calls while terrain is generating fail with the existing message
    style.

## Steps

1. Add `png-decode.ts`, with exhaustive tests.
2. Add `loadHeightmapImage`, with tests.
3. Add forced biome classification with domain-warped borders, the
    ocean fallback, and `setBiomeMap`, with Rust tests. Then add
    `biomeMapFromImage`, with TypeScript tests.
4. Add `waterMaskFromImage`.
5. Add `setVegetationMasks`, wired into plan 5's bake, and
    `densityMaskFromImage`.
6. Add the bundle writer changes (version 2, source height, painted
    inputs), and the `sourceHeight` map kind.
7. Add `loadBundle` and `loadTerrainFromImages`.
8. Demo, a new "Import" section:
    - file pickers for the heightmap image (with metres-per-sample, min
        and max height fields, pre-filled from `vistawasm:range` when
        present), biome map, water mask, tree mask and grass mask;
    - an "Import bundle (.zip)" picker;
    - warnings shown in the status line.
9. Docs:
    - a new `docs/import.md`, covering each import, the colour
        conventions (with the legend table and colours), border
        dithering, the absolute-biome rule and its ocean exception, the
        bundle format v2, and an end-to-end example;
    - update `docs/export-and-snapshots.md` (bundle v2);
    - `docs/options-reference.md`;
    - `docs/README.md` (index);
    - `CHANGELOG.md`.
10. Verify, then commit and push.

## Tests

TypeScript (vitest):

- **PNG decoding** of PNGs made with `node:zlib`:
    - every colour type, bit depths 1, 2, 4, 8 and 16, all five filters,
        and Adam7;
    - a CRC mismatch is rejected;
    - a plan 9 `encodePng` output round-trips exactly.
- **`biomeMapFromImage`:**
    - exact legend colours map exactly;
    - colours within ΔE 10 map to the right biome;
    - a far colour takes the nearest biome and raises `unmatchedFraction`;
    - transparent pixels become 255.
- **`waterMaskFromImage`** and **`densityMaskFromImage`** conventions.
- **Bundle round trip** in Node, using a mocked engine for the zip layer:
    manifest parsing, a version 1 bundle rejected, and a corrupt CRC
    rejected.
- **Validation** of every boundary rule.

Rust:

- Forced classification:
    - interior painted samples keep their biome;
    - border samples within `borderSamples` of a painted edge vary
        (irregular), and none further away change;
    - it is deterministic.
- Painted ocean above sea level falls back to automatic classification,
    and is counted.
- The density multiplier: 0 gives no trees, and 255 doubles the target,
    capped.
- Source heights: after rivers, glacier smoothing and a water mask,
    `sourceHeight` equals the originally loaded heights exactly.

Browser round trip (harness):

- Generate the default map. Paint a synthetic biome map (a quarter of
    the map set to `savannahExpanse`) and a water mask. Export a bundle.
    Reload the page, and `loadBundle`.
- Then `exportMap` for height, biome, water, treeDensity and trees must
    equal the originals exactly (byte-compare them in the page and
    report).

## Verification

- A PNG heightmap import at 16-bit: a 1024 x 1024 image generated in the
    test, rendered top-down and at a low angle.
- A painted biome map with three large regions: borders irregular and
    natural, not straight lines; ground textures blend.
- A water mask with a painted lake and river: carved and flowing (plan 3
    rendering).
- A tree mask with a painted clearing: trees absent there.
- The bundle round trip passes.
- The reference shots are unaffected when nothing is imported.

## Budgets

- WASM growth: at most +6 KB gzipped.
- `dist/index.js`: at most +5 KB gzipped.
- A 4096 x 4096 16-bit PNG decodes in under 1.5 s in the browser; report
    the measured value.

## Out of scope

The paint UI (plan 11) and GeoTIFF changes.
