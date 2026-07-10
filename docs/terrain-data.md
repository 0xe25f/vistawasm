# Terrain Data: Generation, Import, and the Data Model

VistaWASM gets its heightmap from exactly one of three sources per engine
instance: procedural fractal generation, a GeoTIFF DEM, or a raw heightmap
buffer. This document covers the data model shared by all three and the
precise scope of each import path. For *how to make fractal terrain look
good creatively*, see
[`docs/world-design-guide.md`](world-design-guide.md); this document is the
data/format reference.

## The shared data model

Every generation/load call returns a `TerrainHandle`:

```ts
interface TerrainHandle {
  id: number;
  metadata: TerrainMetadata;
}
```

`id` is a stable handle for the *current* engine instance (it increments on
each new terrain, so you can detect a stale reference, but it is not a
global/cross-session identifier). `metadata` is the important part — see
the full field table in
[`docs/options-reference.md`](options-reference.md#terrainmetadata-on-terrainhandlemetadata).
Three things worth calling out specifically:

- **`minHeightMetres`/`maxHeightMetres`/`meanHeightMetres` are the real,
  measured height range of what was actually generated or decoded** — not
  an echo of a requested parameter. Always read these back rather than
  assuming e.g. `verticalScale: 1` means "heights range -1 to 1 metres";
  use them to place a camera or set `WaterOptions.seaLevelMetres` (see
  [`docs/water.md`](water.md#sea-level)).
- **`warnings` is a plain `string[]` of non-fatal issues** encountered
  during decode/generation — surface these to a host UI (or at least log
  them) rather than silently discarding them. A DEM missing recognised
  geospatial tags, for example, produces a warning rather than a hard
  failure, since VistaWASM can still render the heights.
- **`geospatial` is only populated for GeoTIFF sources** with recognised
  tags — it is `null` for fractal generation and raw heightmaps.

Only one terrain is active per engine at a time. Generating or loading a
new one fully replaces the previous one (and re-triggers flora/grass/water
placement — see [`docs/architecture.md`](architecture.md#rendering)); there
is no multi-terrain or terrain-streaming support (see
[`docs/game-development.md`](game-development.md#4-multiple-terrains-streaming-worlds-and-world-size)
for how to work within this for large worlds).

## Fractal generation

`engine.generateFractal(options: FractalTerrainOptions)` — deterministic CPU
Rust (`terrain/fractal.rs`) for a given `seed` and option set. The pipeline,
in order:

1. **Noise** (`NoiseOptions`) produces a base `[-1, 1]` height field.
2. **Shape** (`TerrainShapeOptions`) reshapes that field, still in `[-1, 1]`.
3. **Scaling** — multiplied by `verticalScale`, offset by
    `baseHeightMetres`, to produce real height-in-metres.
4. **Erosion** (`ErosionOptions`, optional) runs last, directly on the
    height-in-metres data.

See [`docs/world-design-guide.md`](world-design-guide.md#1-the-generation-pipeline-in-order)
for the full creative walkthrough of what each stage does, and
[`docs/options-reference.md`](options-reference.md#fractalterrainoptions)
for every field.

### Determinism

The same `seed` and options **always** produce the same heights, on both
native and `wasm32` builds — this is verified by the engine's own test
suite (`crates/vista_wasm/tests/deterministic_terrain.rs`). `seed` also
offsets flora/grass/cloud/mist placement (via each system's own
`seedOffset` field), so a saved seed reproduces the whole scene, not just
the terrain shape.

### Erosion

Erosion (`ErosionOptions`) runs as GPU compute passes on browser builds
(`render/erosion_compute.rs` + `shaders/hydraulic_erosion.wgsl`/
`thermal_erosion.wgsl`) for performance on large terrain, falling back
automatically to the CPU reference implementation
(`terrain/erosion.rs`) if the GPU pass fails for any reason — terrain
generation always produces a result either way. Native/test builds always
use the CPU path. The GPU version is a "gather" reformulation of the same
hydraulic/thermal model (each cell writes only its own output, reading
neighbours, so it is race-free across GPU invocations) — it is not a
bit-identical port of the CPU scatter-based algorithm, so do not expect
pixel-identical output between a browser run and a native/test run when
erosion is enabled (undisturbed noise-only generation *is* bit-identical
across both).

`ErosionOptions.quality` caps the total iteration budget regardless of the
requested `hydraulicIterations`/`thermalIterations` — `"preview"` keeps a
UI responsive while scrubbing sliders, `"offline"` allows the full
requested count for a final export.

## DEM import (GeoTIFF)

```ts
const response = await fetch("/terrain/yosemite.tif");
const buffer = await response.arrayBuffer();

const handle = await engine.loadDemFromArrayBuffer(buffer, {
  verticalScale: 1,
  generateNormals: true,
  generateMaterialMasks: true
});
```

Or, if the fetch itself should go through VistaWASM's own error handling:

```ts
const handle = await engine.loadDemFromUrl("/terrain/yosemite.tif");
```

`loadDemFromUrl()` is a thin convenience wrapper: it fetches the URL (using
the `fetch` options in `DemFetchOptions` — `headers`, `signal`) and passes
the resulting bytes to `loadDemFromArrayBuffer()`. Configure CORS on the
serving origin if it differs from your app's origin.

### Exactly what is supported

VistaWASM decodes a **deliberately scoped subset** of GeoTIFF, not the full
format:

- Classic TIFF only (magic value `42` — not BigTIFF).
- **Uncompressed data only.** Any `Compression` tag value other than `1`
  (none) is rejected with `DEM_FORMAT_UNSUPPORTED`. Re-export your DEM
  without compression (e.g. `gdal_translate -co COMPRESS=NONE`) if you hit
  this.
- **One sample per pixel only.** Multi-band GeoTIFFs (e.g. RGB plus a
  separate elevation band) are rejected — extract the elevation band to
  its own single-band file first.
- **Strip-based storage** (not tiled). Most DEM exports use strips by
  default; re-export without tiling if needed.
- 16-bit integer (signed or unsigned) or 32-bit float samples.
- Reads `ModelPixelScaleTag` and `ModelTiepointTag` for geospatial scale,
  `GeoKeyDirectoryTag` presence (recorded, not fully interpreted — see
  `GeospatialMetadata.projectionName`), and GDAL's non-standard no-data tag
  (`42113`) when present.

Anything outside this scope throws a `VistaWasmError` with code
`DEM_FORMAT_UNSUPPORTED` or `DEM_METADATA_MISSING` (see
[`docs/events-errors-and-lifecycle.md`](events-errors-and-lifecycle.md#error-codes)
for the full list) rather than attempting a best-effort partial decode.

### No-data handling

Samples matching the GDAL no-data value (or otherwise flagged during
decode) are tracked in a separate mask, not silently substituted with `0`
or clamped into the height range — this is what `DebugView`'s planned
`"no-data"` overlay is for (see
[`docs/render-quality-and-diagnostics.md`](render-quality-and-diagnostics.md#debug-views-debugview),
noting that overlay is not implemented yet). Flora and grass placement
already skip no-data samples automatically.

## Raw heightmap import

For pre-processed height data with no DEM metadata at all:

```ts
await engine.loadRawHeightmap(buffer, {
  width: 1024,
  height: 1024,
  sampleFormat: "float32",
  metresPerSample: 30,
  heightScaleMetres: 1,
  noDataValue: -9999
});
```

See [`docs/options-reference.md`](options-reference.md#rawheightmapoptions)
for every field. This path has no geospatial metadata concept at all
(`TerrainMetadata.geospatial` is always `null`), and is the right choice
for heightmaps you've already processed yourself (e.g. exported from
another tool, or generated offline) rather than a DEM you want VistaWASM to
parse.

## Choosing a source

| | Fractal | GeoTIFF DEM | Raw heightmap |
| --- | --- | --- | --- |
| Determinism | Fully seeded/reproducible | Whatever the source file contains | Whatever the source buffer contains |
| Real-world accuracy | No — procedural | Yes, for supported files | Only if your own pre-processing preserved it |
| Setup cost | None — just call `generateFractal()` | Needs a compatible, uncompressed, single-band file | Needs you to already have raw samples in a supported format |
| Geospatial metadata | None | Partial (scale/tiepoint/no-data) | None |

Most projects use fractal generation for procedural worlds and DEM import
for real-world locations; raw heightmap import is mainly for
already-processed or non-GeoTIFF pipelines.
