# Changelog

All notable changes to VistaWASM are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [Unreleased]

## [1.1.0] — 2026-09-23

A realism release: biomes, real trees, simulated water, weather, and
shadows, with every system switchable, tunable, and replaceable. The
public API only grows; see [Upgrading from 1.0.0](#upgrading-from-100)
for changed defaults.

### Added

- **Biomes.** Nineteen climate-driven biomes: grassy meadows, outer
  thicket, outer and inner forest, mountain foothills, mountain proper,
  outer volcanic, caldera, savannah, coastal beach, coastal rocky, outer
  and inner jungle, swamp wetlands, ocean, alpine transition, lower and
  upper snowy peaks, and ice and arctic. New `BiomeOptions`,
  `setBiomes()`, `biomeAt(x, z)`, and a `"biomes"` debug view.
- **Procedural textures**, generated on the GPU at start-up with nothing
  to download: ten terrain materials with height, normal, occlusion, and
  roughness; bark, leaf, needle, frond, and moss textures; water ripples;
  2D and 3D noise.
- **Trees.** Eight procedurally modelled species (oak, pine, spruce, palm,
  jungle, cypress, acacia, shrub) with 3D meshes near the camera, baked
  impostors in the distance, GPU culling, and indirect draws.
- **Water.** Gerstner wave simulation (`WaterOptions.waves`), surface
  currents, depth-based colour and clarity, foam, and shoaling surf; rivers
  and lakes from the terrain's drainage network, carved into the terrain,
  with flowing currents (`WaterOptions.rivers`).
- **Weather.** Clear, partly cloudy, overcast, fog, rain, storm, and snow,
  with smooth transitions, optional automatic cycling, and per-effect
  control. It drives clouds, mist, wind, waves, falling rain and snow, wet
  ground and puddles, settled snow, and lightning. New `WeatherOptions`,
  `setWeather()`, `getWeather()`, `RenderStats.weather`, and the
  `"weatherChanged"` event.
- **Cloud types**: `CloudsOptions.stratiform`, `towering`,
  `baseDarkness`, `raggedBase`, and `rainShafts`. Each weather state has
  its own clouds: stratus when overcast, dark ragged nimbostratus with rain
  shafts in rain, and cumulonimbus towers with anvils, rain shafts, and
  lightning that lights the clouds from inside in storms.
- **High cirrus**: `CloudsOptions.cirrus` and `cirrusHeightMetres`.
- **Shadows** from terrain (a baked horizon map), trees (a sun shadow
  map), and clouds, each configurable through `ShadowOptions` and
  `setShadows()`.
- **Replacement hooks** for your own assets: `setTreeModel()`,
  `resetTreeModel()`, `setTreeInstances()`, `replaceTexture()`,
  `resetTextures()`, `FloraOptions.speciesRules`, and the `imageToRgba()`
  helper.
- **Surface options**: `SurfaceOptions` and `setSurface()` for flat-colour
  mode, detail normals, texture scale, and per-material tints.
- `CloudsOptions.resolutionScale`: clouds render at a reduced resolution,
  half by default.
- Mist wind drift and sun scattering.
- New guides: `docs/weather.md`, `docs/shadows.md`, `docs/hooks.md`,
  `docs/biomes.md`, and `docs/frameworks.md` (React, Vue, and Svelte
  components, moved out of the README).
- A rewritten README with a screenshot, a quick start, and clear paths for
  people using the package and people changing it; a documentation index
  (`docs/README.md`); and `CONTRIBUTING.md` with a fresh-setup guide.
- **A frame profiler.** `RenderStats.gpuPassTimesMs` reports GPU time per
  pass (terrain, trees, grass, clouds, sky and fog, water, shadows, tree
  culling, upscale and lens), and `gpuFrameTimeMs` their sum, from
  timestamp queries read back without stalling. The demo lists them in its stats panel.
- **Render, detail, and cloud distances.**
  `RenderQualityOptions.renderDistanceMetres` (terrain, trees, and water
  beyond it are not shaded, hidden by distance fog over
  `renderFadeMetres`),
  `detailDistanceMetres` (distant terrain takes one texture sample per
  material instead of up to eight), and `cloudDistanceMetres` (how far
  clouds are marched, thinning out over `cloudFadeMetres`). `preset` now
  fills in whichever are unset, and the demo has a control for each.
- **A frame-rate cap.** `RenderQualityOptions.maxFrameRate` (default 60,
  `0` for uncapped) renders evenly spaced frames, so a 60 cap on a 120 or
  144 Hz display gives a steady 60 rather than a rate that swings with the
  scene.
- **Dynamic resolution.** `renderScale` renders the scene below the canvas
  resolution, and `dynamicResolution` (on by default, down to
  `minRenderScale`, default 0.5) lowers it when frames arrive late and
  raises it when there is time to spare, to hold the frame rate. A final
  pass upscales with contrast-adaptive sharpening.
  `RenderStats.renderScale` reports the scale in use; the demo has
  controls and shows it.
- `CloudsOptions.temporal`: reuse distant clouds between frames. A
  quarter-size pass marches one sky pixel of every 2 x 2 block, a different
  one each frame, and the rest are reprojected from the previous frame,
  cutting the cost of sky clouds by about three quarters. Off by default;
  a checkbox in the demo.
- `WeatherOptions.lensDrops`: raindrops that land on the lens, refract the
  scene, and run down the screen. Off by default.
- A three.js guide (`docs/threejs.md`) and example (`examples/threejs/`,
  `npm run dev:threejs`): three.js objects drawn over a VistaWASM world
  with a shared camera, a matching sun, and hills that hide them; and
  VistaWASM terrain drawn by three.js, including in browsers without
  WebGPU.
- Benchmarks (`bench/`): production download size and terrain generation
  speed compared with THREE.Terrain and three-terrain, with the method,
  results, and a feature comparison. Run them with `npm run size` and
  `npm run speed` inside `bench/`.
- **Demo.** A link to the GitHub repository; collapsible sections with
  remembered state; value readouts on every slider; one-click weather
  presets; and controls for every option, including weather effects,
  shadows, surface, cloud types, and cloud quality. A "Custom assets"
  panel demonstrates each hook: a custom tree model built in JavaScript, a
  species rule, a hand-placed grove, and texture replacement from an
  image file.
- **Snow biomes.** High ground now rises through `alpineTransition`
  (scree, patchy snow and dwarf shrubs below the snow line) into
  `lowerSnowyPeaks` (snowfields broken by rock) and `upperSnowyPeaks`
  (permanent snow and ice). `biomeAt()` reports them, and the `"biomes"`
  debug view colours them.
- **Landforms.** `FractalTerrainOptions.landform` picks the character of a
  generated map: `"continental"` (the default), `"alpine"`,
  `"rollingHills"`, `"archipelago"`, `"mesaDesert"`, `"fjords"` or
  `"volcanicIsland"`. See `docs/terrain-data.md`.
- Terrain generation progress: `generateFractal()` now emits `"progress"`
  events for its `"tectonics"`, `"drainage"`, `"detail"`, `"erosion"` and
  `"finishing"` phases, with erosion reported at least every 10 %.
- **Demo.** A Landform select, an Advanced terrain sub-section for the
  detail noise, an erosion quality select (default `"high"`), and the time
  of each generation phase in the status line.
- **Ice and arctic biome.** `iceArctic`: glaciers and ice sheets with
  crevasses and blue ice, a tundra fringe of moss, lichen, dwarf shrubs,
  and stones, snow that never melts, and sea ice with drifting floes and
  fast ice on cold coasts. Glaciers fill valleys with smooth ice and are
  removed exactly when the climate warms.
- **Climate temperature.** `BiomeOptions.meanTemperatureCelsius` (-30 to
  35 °C at sea level) drives every biome, cooling 6.5 °C per 1000 m. New
  `temperatureAt(x, z)` returns the mean temperature in °C at a world
  position, or `null` off the terrain.
- **Cold weather.** Rain falls as snow below 0.5 °C at the camera and as
  sleet up to 2.5 °C; cold climates cycle towards snow and clear spells
  and always allow snow; gales lift blowing snow off snowy ground; and
  cold air is crisp and clear.
- Glacier ice and tundra terrain textures, generated at start-up.
  `replaceTexture` accepts terrain layers 0 to 9, and `materialTints`
  accepts 8 or 10 colours.
- **Demo.** A climate temperature slider with an automatic setting, the
  temperature under the camera in the biome readout, and a biome colour
  legend for the biomes debug view.
- **Map edges.** `FractalTerrainOptions.edges`: `"coast"` (the default)
  rings the land with sea along a natural, wandering coastline, and
  `"open"` lets the land run to the map edge for tiling several maps. The
  demo's terrain section has an Edges select.

### Changed

- **Fractal terrain is geology-led.** Continents with an exact land
  fraction and uplifted ranges are carved by a stream-power model into
  dendritic valleys and ridge spurs, with flat valley floors and glacial
  troughs, then detailed and eroded. Noise is seeded gradient noise instead
  of value noise, and features have real sizes in metres. Maps have
  plains, coastlines and ranges instead of a field of spikes. The same seed
  gives a different map from earlier builds; `generatorVersion` is now
  `vistawasm-fractal-0.2.0`.
- `NoiseOptions` now controls the detail layer on top of the landform, and
  `TerrainShapeOptions` applies in units of the landform's relief.
- The coast is generated at `seaLevelMetres`, and `verticalScale`
  stretches heights about sea level.
- Generated maps are ringed by sea by default (`edges: "coast"`), for
  every landform, so land no longer runs into the map edge. Each landform
  keeps its land fraction. `edges: "open"` gives the previous heights bit
  for bit.
- **Erosion** is a virtual-pipe shallow-water model that cuts gullies,
  aggrades valley floors and builds alluvial fans, plus talus-angle
  thermal erosion with soil creep. It runs at half and then full
  resolution, on the GPU in browsers with the CPU as a fallback. Unset
  `ErosionOptions` fields take the landform's defaults, unset iteration
  counts follow `quality`, and the quality caps are now 120, 240, 400 and
  5000 iterations.
- Fractal and erosion options are validated with messages that name the
  field and its valid range.
- The automatic snow line (`BiomeOptions.snowLineMetres` unset) is now at
  least 400 m above sea level, so low hills no longer turn white.
- Terrain vertices are 36 bytes instead of 40: the normal is
  octahedron-encoded and the ten material weights are packed into twelve
  `unorm8` slots, so the streamed terrain mesh uploads faster (see
  `docs/architecture.md`).
- `FrameUniforms` grows to 784 bytes, and render shaders gain the surface
  texture at `@group(1) @binding(12)`.
- Heavy sleet and snow now darken the sky to a full overcast, as heavy
  rain does.

- The default `"balanced"` render preset now draws terrain beyond 2 km with
  one texture sample per material, and marches clouds to 60 km (from 90
  km). In testing this saved about 12 % of GPU time with no visible
  difference; `preset: "offline"` restores the previous behaviour.

- Volumetric clouds are rebuilt: an adaptive march that refines cloud
  edges, multiple-scattering lighting with bright tops and darker bases,
  distance-aware detail, and a stable dither. They no longer look grainy
  or like cotton wool, and they form rounded domes rather than columns.
- Rendering is linear HDR with ACES tone mapping and a single-scattering
  sky model shared by every shader; haze and mist are applied in a
  depth-aware composite pass.
- Water extends to the horizon instead of stopping at the terrain edge.
- Animation uses a real-time clock instead of a frame counter, and wind
  drift is integrated over time, so changing the wind never makes clouds,
  mist, or currents jump. The time step is smoothed, and a pause longer
  than 0.2 s (a background tab) no longer jumps the scene forward.
- Overcast, rain, and storm skies march cloud lighting with 3 samples
  instead of 5, and sky lighting skips a second sky evaluation that did
  not change the result.
- `setWater`, `setFlora`, `setGrass`, `setClouds`, `setMist`, and every
  new setter validate their input at the JavaScript boundary.
- Smaller, faster builds: release builds use link-time optimisation, one
  codegen unit, size optimisation, and `panic = "abort"`; shaders are
  minified at build time, `common.wgsl` is embedded once instead of once
  per shader, and each shader module is compiled once. The optimised WASM
  binary is 585 KB (226 KB gzipped).
- The minimum Rust version for building from source is 1.87, which wgpu
  30 requires. The development notes list exact tool versions and a
  fresh-setup sequence.

### Fixed

- **Terrain hitches while moving.** The camera-centred terrain mesh was
  rebuilt, reallocated, and re-uploaded (16 MB) in a single frame whenever
  the camera drifted 12 samples. It now streams: the next mesh starts at
  6 samples, centred ahead of the camera by its velocity, is built and
  uploaded 64 rows per frame into a second buffer, and swaps in when
  complete. In testing, a full rebuild took 147 ms of CPU time; while
  streaming, no frame took more than 2 ms.

- **Rain, storms, and snow.** Under rain the sky is now a true overcast
  (brightest overhead, darker at the horizon, no direct sun), so the
  distant sea darkens under rain instead of glowing white, and sharp
  shadows and sun glints on the water disappear. The circular pulse in
  the sky above the camera is gone. Falling rain and snow were smeared
  more the longer a scene ran; rain now falls in short streaks and snow in
  round, chunky flakes. Storms, and `precipitationScale` above 1, now look
  heavier, with a grey veil that cuts visibility. Rain and snow were hidden
  wherever water was in view; they now fall in front of it. Rain curtains
  cost less.
- **Cirrus** drifted at twice the speed of the low clouds, which the
  weather raises in a gale. It now has its own `CloudsOptions.cirrusSpeed`.

- **Lag with a high frame-rate reading.** The demo and examples showed
  `1000 / frameTimeMs` as FPS, but `frameTimeMs` only covers the CPU time
  to submit a frame, so a slow GPU showed tens of thousands of FPS. They
  now show the time between drawn frames. The engine also paces itself:
  `renderOnce()` draws nothing while two earlier frames are still on the
  GPU, so frames can no longer queue up behind a slow GPU and make the
  picture lag behind the camera.
- **Lost GPU devices went unnoticed.** Rendering carried on silently into a
  lost device. The engine now reports `WEBGPU_DEVICE_LOST` on the next
  frame, `start()` emits `"deviceLost"`, and the demo says so.
- GPU erosion falling back to the CPU now adds a warning to the terrain
  metadata, and `generateFractal()` emits `"warning"` events like the
  loaders.

- `npm run dev` and every `npm run dev:*` script failed in a fresh clone
  with "Failed to resolve import \"@vista-wasm/vista-wasm\"" until
  `npm run build` had been run. They now build first when `dist/` is
  missing, explain what to install if the build fails, and warn when the
  build is older than the Rust or TypeScript sources.

- Documentation checked against the code. Corrected: `RenderQualityOptions.preset`
  never capped erosion (that is `ErosionOptions.quality`);
  `RenderStats.frameTimeMs` covers CPU time only; the `"stats"` event also
  fires for `renderOnce()` loops; the fly camera's `←`/`→` keys turn; the
  default `RiverOptions.minCatchmentKm2` is `0.15`; `toVistaWasmError()` and
  `assertVistaWasmSupport()` are not exported; `GPU_LIMIT_EXCEEDED` is not
  raised. The framework components now render at the canvas size and clean
  up when unmounted mid-load, the README and getting-started cameras no
  longer start inside a hill, and every code example type-checks.

- The `"height"`, `"slope"`, `"normals"`, and `"materials"` debug views
  now render.
- The sun disc no longer shines through thick cloud or an overcast sky.
- Cloud edges no longer look hairy: the march bisects to each cloud's
  edge and takes short steps just inside it.
- The README gave the licence as AGPL-3.0-or-later; it is
  AGPL-3.0-only, as `LICENSE`, `NOTICE`, and the package metadata say.
- The declared minimum Rust version (1.82) was too old to build the
  project.

### Removed

- `shaders/flora_instances.wgsl`, replaced by `shaders/trees.wgsl`.
- `shaders/terrain_noise.wgsl`, which nothing used.
- The unused `shaders/material_masks.wgsl` placeholder.

### Upgrading from 1.0.0

No code changes are needed; every new option is optional. Some defaults
changed, so scenes look (and cost) different:

- `FloraOptions.treeQuality` defaults to `"mesh"`, `speciesVariation` to
  `0.6`, and `windStrength` to `0.3`. `"billboard"` and `"cross-quad"`
  draw impostors of the real tree models.
- `CloudsOptions.heightMetres` defaults to `1800` and `raymarchSteps` to
  `32`. Clouds render at half resolution (`resolutionScale: 0.5`) and add
  a thin cirrus layer (`cirrus: 0.35`); set `cirrus: 0` to remove it.
- Terrain, tree, and cloud shadows are on by default. Pass
  `shadows: { terrain: { enabled: false }, trees: { enabled: false } }`
  for the 1.0.0 look, or lower `trees.resolution` on weak GPUs.
- The weather system is off by default, so existing cloud, mist, and water
  settings behave as before until you enable it.
- Fractal terrain comes from the new generator, so saved seeds give new
  maps. Erosion iteration counts from 1.0.0 still work but do much less at
  the old values; leave them unset and pick a `quality` instead.

## [1.0.0] — 2026-07-10

### Added

- WebGPU terrain engine compiled to WebAssembly via Rust and wasm-bindgen.
- Seeded fractal terrain generation with configurable noise type (simplex,
  ridged), octaves, gain, lacunarity, and shaping passes (island, terrace,
  basin, canyon, crater).
- Hydraulic and thermal erosion. Runs as GPU compute passes in the browser
  (falling back to the CPU implementation automatically); runs CPU-side on
  native/test builds.
- GeoTIFF DEM import: uncompressed strips, 16-bit integer, 32-bit integer,
  and 32-bit float samples; ModelPixelScaleTag, ModelTiepointTag, and
  GDAL no-data values.
- Raw heightmap loading (float32, uint16, int16, uint8, int8 sample formats).
- Single-mesh terrain renderer with exponentially increasing sample spacing
  from the camera, giving near-detail plus far reach without the T-junction
  cracks that multi-tier clipmaps need skirts to hide. Mesh recentres on the
  camera as it moves.
- Analytic Rayleigh/Mie sky dome with sun disc/glare, horizon haze, and an
  optional cloud layer (off/painted/volumetric).
- Height-based ground mist (off/flat/volumetric) applied across terrain,
  flora, grass, and water.
- Tree billboard renderer with billboard/cross-quad/mesh quality dial,
  per-tree canopy variety, and wind sway. Placement respects slope, tree
  line, water level, and flora density.
- Grass ground-cover renderer: crossed blade tufts driven by terrain material
  weights, with configurable view-distance fade.
- Animated fresnel/specular water plane, sized to the terrain footprint.
- Fly-camera controller: WASD movement, pointer-drag look, scroll-to-zoom,
  Space/Shift and middle-mouse-drag for vertical travel.
- Terrain export: hypsometric minimap PNG (`exportHeightmapImage`), Wavefront
  OBJ 3D model (`exportTerrainObj`), raw heightmap download, and canvas
  screenshot (`exportSnapshot`).
- TypeScript wrapper (`js/src`) with full types and a pending-call mutex that
  prevents wasm-bindgen reentrancy panics when async calls overlap with the
  render/resize loop.
- Demo as a plain static site (no bundler at request time) deployable to
  GitHub Pages via `.github/workflows/deploy-demo.yml`.
- Four runnable framework examples: vanilla TypeScript, React, Vue, Svelte.
- Rust workspace with separate `vista_types` and `vista_wasm` crates.
- Comprehensive documentation in `docs/` covering every public option,
  terrain data, vegetation, sky and weather, water, camera and controls,
  render quality, export, events and errors, architecture, and contributing.

[Unreleased]: https://github.com/0xe25f/vistawasm/compare/v1.1.0...HEAD
[1.1.0]: https://github.com/0xe25f/vistawasm/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/0xe25f/vistawasm/releases/tag/v1.0.0
