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

- **Biomes.** Fifteen climate-driven biomes: grassy meadows, outer
  thicket, outer and inner forest, mountain foothills, mountain proper,
  outer volcanic, caldera, savannah, coastal beach, coastal rocky, outer
  and inner jungle, swamp wetlands, and ocean. New `BiomeOptions`,
  `setBiomes()`, `biomeAt(x, z)`, and a `"biomes"` debug view.
- **Procedural textures**, generated on the GPU at start-up with nothing
  to download: eight terrain materials with height, normal, occlusion, and
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

### Changed

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
  mist, or currents jump.
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

- Documentation checked against the code. Corrected: `RenderQualityOptions.preset`
  has no effect (the erosion cap is `ErosionOptions.quality`);
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
