# Options Reference

This is a field-by-field reference for every public VistaWASM option, in
TypeScript's camelCase naming (the Rust engine uses the equivalent
`snake_case` internally; both are the same value once decoded). Types shown
are the TypeScript shapes from `js/src/types.ts`. Every option is validated
by the Rust engine — an invalid value throws a `VistaWasmError` with code
`OPTIONS_INVALID` rather than being silently clamped, except where noted.

For narrative, "why would I change this" guidance, see
[`docs/world-design-guide.md`](world-design-guide.md) (terrain/sky/weather)
and [`docs/vegetation.md`](vegetation.md) (trees/grass). This document is the
precise reference; those documents are the tour.

## Contents

- [`VistaEngineOptions`](#vistaengineoptions-engine-creation)
- [`RenderSizeOptions`](#rendersizeoptions)
- [`CameraOptions`](#cameraoptions)
- [`SunOptions`](#sunoptions)
- [`AtmosphereOptions`](#atmosphereoptions)
- [`CloudsOptions`](#cloudsoptions)
- [`MistOptions`](#mistoptions)
- [`WaterOptions`](#wateroptions)
- [`FloraOptions`](#floraoptions)
- [`GrassOptions`](#grassoptions)
- [`RenderQualityOptions`](#renderqualityoptions)
- [`DebugView`](#debugview)
- [`FractalTerrainOptions`](#fractalterrainoptions)
- [`DemLoadOptions`](#demloadoptions)
- [`RawHeightmapOptions`](#rawheightmapoptions)
- [`ExportHeightmapOptions` / `SnapshotOptions`](#export-options)
- [Read-only shapes](#read-only-shapes-returned-by-the-engine) (`TerrainMetadata`, `TerrainHandle`, `RenderStats`)

## `VistaEngineOptions` (engine creation)

Passed to `createVistaEngine(canvas, options)`. Every field is optional;
omitted fields use the defaults below.

| Field | Type | Default |
| --- | --- | --- |
| `render` | `RenderSizeOptions` | `{ width: 1, height: 1, devicePixelRatio: 1 }` |
| `camera` | `CameraOptions` | see [`CameraOptions`](#cameraoptions) |
| `sun` | `SunOptions` | see [`SunOptions`](#sunoptions) |
| `atmosphere` | `AtmosphereOptions` | see [`AtmosphereOptions`](#atmosphereoptions) |
| `water` | `WaterOptions` | see [`WaterOptions`](#wateroptions) |
| `flora` | `FloraOptions` | see [`FloraOptions`](#floraoptions) |
| `grass` | `GrassOptions` | see [`GrassOptions`](#grassoptions) (disabled by default) |
| `clouds` | `CloudsOptions` | see [`CloudsOptions`](#cloudsoptions) (off by default) |
| `mist` | `MistOptions` | see [`MistOptions`](#mistoptions) (off by default) |
| `quality` | `RenderQualityOptions` | see [`RenderQualityOptions`](#renderqualityoptions) |

Always pass a real `render.width`/`render.height` matching your canvas's
actual CSS size — the `{ width: 1, height: 1 }` default exists only so the
type is safely constructible, not as a usable render size.

## `RenderSizeOptions`

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `width` | `number` | `1` | CSS pixels. Must be at least 1. |
| `height` | `number` | `1` | CSS pixels. Must be at least 1. |
| `devicePixelRatio` | `number?` | `1` | Must be finite, `> 0`, and `<= 8`. |

Also used by `engine.resize(width, height, devicePixelRatio?)`.

## `CameraOptions`

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `position` | `[number, number, number]` | `[0, 120, 300]` | World position in terrain metres. |
| `target` | `[number, number, number]` | `[0, 0, 0]` | Look-at point in terrain metres. |
| `rollDegrees` | `number?` | `0` | Accepted and stored, but **not currently applied** to the view matrix — the camera always uses a fixed world-up vector. Reserved for future use. |
| `fieldOfViewDegrees` | `number` | `55` | Must be between `1` and `160`. |
| `nearMetres` | `number?` | `0.5` | Must be positive and less than `farMetres`. |
| `farMetres` | `number?` | `120000` | Must be positive and greater than `nearMetres`. |
| `minimumHeightAboveTerrainMetres` | `number?` | `2` | Accepted and stored, but **not currently enforced** by the engine — nothing clamps the camera's height against the terrain today. If you need a ground clamp, compute it yourself against an exported heightmap (see [`docs/camera-and-controls.md`](camera-and-controls.md#terrain-clamping-is-not-built-in)). |
| `allowUnderground` | `boolean?` | `false` | Accepted and stored, but **not currently enforced** — see the note above. `attachFlyCameraControls()` always behaves as if this were `true` internally. |

See [`docs/camera-and-controls.md`](camera-and-controls.md) for the
projector model and the bundled fly-camera controller.

## `SunOptions`

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `azimuthDegrees` | `number` | `132` | Compass direction the sun shines from. Must be finite. |
| `elevationDegrees` | `number` | `18` | Angle above the horizon. Negative values put the sun below the horizon. Must be finite. |
| `intensity` | `number` | `1.2` | Light brightness multiplier. Must be `> 0`. |

## `AtmosphereOptions`

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `rayleighStrength` | `number` | `1.0` | Sky gradient saturation. |
| `mieStrength` | `number` | `0.45` | Sun glare/haze size. |
| `hazeDistanceMetres` | `number` | `60000` | Uniform, distance-only blend to sky colour. Must be `> 0`. See the haze-vs-mist distinction in [`docs/sky-atmosphere-and-weather.md`](sky-atmosphere-and-weather.md). |
| `exposure` | `number` | `1.1` | Overall brightness multiplier. Must be `> 0`. |
| `skyTint` | `[number, number, number]` | `[1, 1, 1]` | RGB multiplier applied to the whole sky/haze/cloud result. |

## `CloudsOptions`

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `style` | `"off" \| "painted" \| "volumetric"` | `"off"` | See [`docs/sky-atmosphere-and-weather.md`](sky-atmosphere-and-weather.md#clouds). |
| `coverage` | `number` | `0.45` | `0` clear to `1` overcast. Must be `>= 0`. |
| `speed` | `number` | `1.0` | Drift speed multiplier. |
| `heightMetres` | `number` | `4000` | Cloud layer altitude. |
| `colour` | `[number, number, number]` | `[1, 1, 1]` | Base cloud tint. |
| `seedOffset` | `number \| bigint` | `9007` | Deterministic cloud noise seed. |
| `raymarchSteps` | `number?` | `24` | `"volumetric"` style only. **Hard-clamped server-side to `8..=64`** regardless of the requested value — an out-of-range value throws `OPTIONS_INVALID` rather than being silently clamped, since this value bounds a real shader loop. |

## `MistOptions`

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `style` | `"off" \| "flat" \| "volumetric"` | `"off"` | See [`docs/sky-atmosphere-and-weather.md`](sky-atmosphere-and-weather.md#mist-and-ground-fog). |
| `density` | `number` | `0.5` | `0` to `1`. Must be `>= 0`. |
| `baseHeightMetres` | `number` | `40` | Altitude mist is thickest at. Must be finite. |
| `heightFalloffMetres` | `number` | `120` | How quickly mist thins with altitude. Must be `>= 0`. |
| `colour` | `[number, number, number]` | `[0.82, 0.85, 0.88]` | Mist tint. |
| `riseAboveWater` | `boolean` | `true` | Adds extra mist near `WaterOptions.seaLevelMetres`, independent of `baseHeightMetres`. |
| `seedOffset` | `number \| bigint` | `5303` | Deterministic mist noise seed (`"volumetric"` style only). |

## `WaterOptions`

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `enabled` | `boolean` | `true` | |
| `seaLevelMetres` | `number` | `0` | Set relative to `TerrainMetadata.minHeightMetres`/`meanHeightMetres`, not a hardcoded constant — see [`docs/water.md`](water.md). Must be finite. |
| `waveScale` | `number` | `0.8` | Procedural ripple amplitude. Must be `>= 0`. |
| `reflectivity` | `number` | `0.35` | Fresnel/sky reflection strength. Must be `>= 0`. |
| `shorelineSoftnessMetres` | `number` | `6` | Shoreline blend distance. |

## `FloraOptions`

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `enabled` | `boolean` | `true` | |
| `density` | `number` | `0.35` | `0` to `1`. Must be `>= 0`. Scaled by `RenderQualityOptions.floraDensityScale`. |
| `treeLineMetres` | `number` | `1800` | Altitude above which trees stop spawning. |
| `seedOffset` | `number \| bigint` | `3001` | Deterministic placement seed. |
| `maxInstances` | `number` | `500000` | Upper bound on instance count, also clamped to the active device's limits. |
| `treeQuality` | `"billboard" \| "cross-quad" \| "mesh"` | `"billboard"` | See [`docs/vegetation.md`](vegetation.md#tree-quality). `"mesh"` currently renders identically to `"cross-quad"` (documented, not a bug — see that doc). |
| `speciesVariation` | `number?` | `0` | `0` to `1`. Canopy silhouette/colour variety strength. Must be `>= 0`. |
| `windStrength` | `number?` | `0` | `0` to `1`. Canopy sway strength (trunk never moves). Must be `>= 0`. |

Placement automatically avoids underwater and steep terrain; see
[`docs/vegetation.md`](vegetation.md#placement) for the algorithm.

## `GrassOptions`

Unlike every other environmental option, this defaults fully **disabled** —
grass is a new visual element with no prior equivalent, so existing scenes
are unaffected until a host opts in.

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `enabled` | `boolean` | `false` | |
| `style` | `"billboard-blades" \| "dense-blades"` | `"billboard-blades"` | See [`docs/vegetation.md`](vegetation.md#grass). |
| `density` | `number` | `0.5` | `0` to `1`. Must be `>= 0`. Scaled by `RenderQualityOptions.floraDensityScale`. |
| `viewDistanceMetres` | `number` | `220` | Distance from the camera at which grass fully fades out. Must be `> 0`. |
| `seedOffset` | `number \| bigint` | `7331` | Deterministic placement seed. |
| `maxInstances` | `number` | `200000` | Upper bound on instance count, also clamped to the active device's limits. |

## `RenderQualityOptions`

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `preset` | `"preview" \| "balanced" \| "high" \| "offline"` | `"balanced"` | Caps erosion iteration budget (see [`docs/terrain-data.md`](terrain-data.md#erosion)); does **not** automatically change `treeQuality`/`GrassOptions`/`CloudsOptions`/`MistOptions` defaults — those are set independently per feature. |
| `maxClipmapLevels` | `number?` | `7` | Theoretical LOD-level budget used only by native/test builds without a GPU; browser builds report the real uploaded mesh's stats regardless of this value (see [`docs/render-quality-and-diagnostics.md`](render-quality-and-diagnostics.md)). |
| `floraDensityScale` | `number?` | `1.0` | Global multiplier applied on top of both `FloraOptions.density` and `GrassOptions.density`. The cheapest performance lever for vegetation-heavy scenes. |

## `DebugView`

A plain string, not an object: `"none" | "height" | "slope" | "normals" |
"lod" | "flow" | "materials" | "no-data"`. Passed to `engine.setDebugView()`.
**No value other than `"none"` currently changes what is rendered** — see
[`docs/render-quality-and-diagnostics.md`](render-quality-and-diagnostics.md)
for the current status of each mode.

## `FractalTerrainOptions`

Passed to `engine.generateFractal(options)`.

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `seed` | `number \| bigint` | — (required) | Deterministic seed for terrain, erosion, and flora/grass/cloud/mist placement offsets. |
| `size` | `512 \| 1024 \| 2048 \| 4096 \| 8192 \| number` | — (required) | Square terrain side length in samples. |
| `horizontalScaleMetres` | `number` | — (required) | Metres between adjacent samples. |
| `verticalScale` | `number` | — (required) | Height multiplier applied to the generated `[-1, 1]` noise field. |
| `baseHeightMetres` | `number?` | `0` | Offset added after scaling. |
| `seaLevelMetres` | `number?` | `0` | Initial `TerrainMetadata.seaLevelMetres`; independent from `WaterOptions.seaLevelMetres`, which controls the rendered water plane. |
| `noise` | `NoiseOptions` | see below | |
| `shape` | `TerrainShapeOptions?` | none | |
| `erosion` | `ErosionOptions?` | none (disabled) | |

### `NoiseOptions`

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `kind` | `"simplex" \| "ridged" \| "hybrid" \| "island" \| "canyon" \| "cratered" \| "classic"` | `"ridged"` | See [`docs/world-design-guide.md`](world-design-guide.md#2-noise-kinds) for what each produces. |
| `octaves` | `number` | `7` | Layered noise octave count. |
| `gain` | `number` | `0.5` | Amplitude multiplier between octaves. |
| `lacunarity` | `number` | `2.0` | Frequency multiplier between octaves. |
| `warp` | `number?` | `0` | Domain warp amount. |

### `TerrainShapeOptions`

All fields optional, roughly `0..1` strength dials (see
[`docs/world-design-guide.md`](world-design-guide.md#3-shape-controls)):
`island`, `terrace`, `basin`, `canyon`, `crater`.

### `ErosionOptions`

Passing `erosion` at all enables erosion; omit it entirely to skip erosion.

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `hydraulicIterations` | `number?` | `0` | Rain/transport/deposition passes. |
| `thermalIterations` | `number?` | `0` | Scree/talus slumping passes. |
| `rainAmount` | `number?` | `0.02` | Hydraulic aggressiveness. |
| `evaporation` | `number?` | `0.5` | Hydraulic water loss rate. |
| `sedimentCapacity` | `number?` | `0.04` | Hydraulic transport capacity. |
| `talusAngleDegrees` | `number?` | `35` | Slope angle above which thermal erosion moves material. |
| `quality` | `"preview" \| "balanced" \| "high" \| "offline"?` | `"preview"` | Caps total iteration budget regardless of the requested counts. |

Erosion runs as GPU compute passes on browser builds (CPU fallback on any
GPU error); native/test builds always use the CPU path — see
[`docs/architecture.md`](architecture.md#terrain-generation).

## `DemLoadOptions`

Passed to `engine.loadDemFromArrayBuffer(buffer, options)` /
`engine.loadDemFromUrl(url, options)`.

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `verticalScale` | `number?` | `1` | Height multiplier applied after decode. |
| `generateNormals` | `boolean?` | `true` | |
| `generateMaterialMasks` | `boolean?` | `true` | |

See [`docs/terrain-data.md`](terrain-data.md#dem-import) for the supported
GeoTIFF subset and what decode warnings mean.

## `RawHeightmapOptions`

Passed to `engine.loadRawHeightmap(buffer, options)`.

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `width` | `number` | — (required) | Samples. |
| `height` | `number` | — (required) | Samples. |
| `sampleFormat` | `"uint16" \| "int16" \| "float32"` | — (required) | |
| `byteOrder` | `"little-endian" \| "big-endian"?` | `"little-endian"` | |
| `metresPerSample` | `number` | — (required) | |
| `heightScaleMetres` | `number` | — (required) | |
| `noDataValue` | `number?` | none | Samples equal to this value are marked no-data. |
| `seaLevelMetres` | `number?` | `0` | |

## Export options

### `ExportHeightmapOptions`

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `format` | `"float32-le"?` | `"float32-le"` | Currently the only supported export format. |

### `SnapshotOptions`

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `mimeType` | `string?` | `"image/png"` | Passed to `canvas.toBlob()`. |
| `quality` | `number?` | browser default | Only meaningful for lossy formats. |

See [`docs/export-and-snapshots.md`](export-and-snapshots.md) for the full
set of export helpers, including the ones with their own option types
(`HeightmapImageOptions`, `TerrainObjExportOptions`).

## Read-only shapes returned by the engine

These are never passed *in* — the engine returns them.

### `TerrainMetadata` (on `TerrainHandle.metadata`)

| Field | Type | Notes |
| --- | --- | --- |
| `width`, `height` | `number` | Heightmap dimensions in samples. |
| `metresPerSample` | `number` | |
| `verticalScale` | `number` | |
| `seaLevelMetres` | `number` | |
| `minHeightMetres`, `maxHeightMetres`, `meanHeightMetres` | `number` | Real height range of the loaded/generated terrain — use these, not hardcoded constants, when placing a camera or setting `WaterOptions.seaLevelMetres`. |
| `source` | `string` | `"fractal"`, `"raw-heightmap"`, or `"geotiff"`. |
| `generatorVersion` | `string` | |
| `geospatial` | `GeospatialMetadata \| null` | Present for GeoTIFF sources with recognised tags. |
| `warnings` | `string[]` | Non-fatal decode/generation warnings — see [`docs/events-errors-and-lifecycle.md`](events-errors-and-lifecycle.md#warnings). |

### `RenderStats` (from `engine.renderOnce()` and the `"stats"` event)

| Field | Type | Notes |
| --- | --- | --- |
| `frameIndex` | `number` | Monotonic. |
| `frameTimeMs` | `number` | CPU-measured wall time for the frame, set by the JS wrapper. |
| `gpuFrameTimeMs` | `number \| null` | Not currently populated by any backend. |
| `terrainTriangles` | `number` | Real uploaded mesh triangle count on browser builds; a theoretical estimate on native/test builds. |
| `floraInstances` | `number` | |
| `grassInstances` | `number` | |
| `clipmapLevels` | `number` | Number of exponential LOD bands the terrain mesh's half-span currently spans. |
| `activeGpuMemoryBytes` | `number \| null` | Not currently populated. |

See [`docs/render-quality-and-diagnostics.md`](render-quality-and-diagnostics.md)
for how to use these for a performance HUD.
