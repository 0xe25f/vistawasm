# Changelog

All notable changes to VistaWASM are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [Unreleased]

### Added

- Climate-driven biomes: grassy meadows, outer thicket, outer and inner
  forest, mountain foothills, mountain proper, outer volcanic, caldera,
  savannah, coastal beach, coastal rocky, outer and inner jungle, swamp
  wetlands, and ocean. New `BiomeOptions`, `engine.setBiomes()`,
  `engine.biomeAt(x, z)`, and a `"biomes"` debug view.
- Procedural textures generated on the GPU at start-up: eight terrain
  materials with height, normal, occlusion, and roughness; bark, leaf,
  needle, frond, and moss textures; water ripples; 2D and 3D noise.
- Eight procedurally modelled tree species (oak, pine, spruce, palm,
  jungle, cypress, acacia, shrub) with 3D meshes near the camera, baked
  impostors in the distance, GPU culling, and indirect draws.
- Gerstner wave simulation (`WaterOptions.waves`), surface currents,
  depth-based colour and clarity, foam, and shoaling surf.
- Rivers and lakes from the terrain drainage network, carved into the
  terrain, with flowing currents (`WaterOptions.rivers`).
- Volumetric clouds with Perlin-Worley shapes, self-shadowing, wind
  direction, billowing evolution, thickness, density, and moving cloud
  shadows.
- Mist wind drift and sun scattering.
- `"height"`, `"slope"`, `"normals"`, and `"materials"` debug views now
  render.
- Weather system: clear, partly cloudy, overcast, fog, rain, storm, and
  snow, with smooth transitions, optional automatic cycling, and per-effect
  control. Rain and snow fall, ground gets wet with puddles, snow settles,
  and storms bring lightning. New `setWeather()`, `getWeather()`,
  `RenderStats.weather`, and the `"weatherChanged"` event.
- Shadows from terrain (baked horizon map), trees (sun shadow map), and
  clouds, each configurable through `ShadowOptions` and `setShadows()`.
- Replacement hooks: `setTreeModel()`, `resetTreeModel()`,
  `setTreeInstances()`, `replaceTexture()`, `resetTextures()`,
  `FloraOptions.speciesRules`, and the `imageToRgba()` helper.
- `SurfaceOptions` and `setSurface()`: flat-colour mode, detail normals,
  texture scale, and per-material tints.
- `CloudsOptions.resolutionScale`: clouds render at half resolution by
  default.
- High cirrus layer: `CloudsOptions.cirrus` and `cirrusHeightMetres`.
- The demo links to the GitHub repository and has weather, shadow, and
  surface controls, including texture replacement from an image file.

### Changed

- `FloraOptions.treeQuality` defaults to `"mesh"`, `speciesVariation` to
  `0.6`, and `windStrength` to `0.3`. `"billboard"` and `"cross-quad"` now
  draw impostors of the real tree models.
- Rendering is linear HDR with ACES tone mapping and a single-scattering
  sky model shared by every shader; haze and mist are applied in a
  depth-aware composite pass.
- `CloudsOptions.heightMetres` defaults to `1800` and `raymarchSteps` to
  `32`.
- Water now extends to the horizon instead of stopping at the terrain edge.
- Animation uses a real-time clock instead of a frame counter.
- `setWater`, `setFlora`, `setGrass`, `setClouds`, and `setMist` now
  validate their input at the JavaScript boundary.
- Volumetric clouds are rebuilt for realism: an adaptive march that
  refines cloud edges, multiple-scattering lighting with darker bases and
  bright tops, distance-aware detail, and a stable dither. They no longer
  look grainy or like cotton wool, and they narrow into rounded domes.
- The sun disc no longer shines through thick cloud or an overcast sky.
- Release builds use link-time optimisation, one codegen unit, size
  optimisation, and `panic = "abort"`. Shaders are minified at build time
  and `common.wgsl` is embedded once instead of once per shader.

### Removed

- `shaders/flora_instances.wgsl` (replaced by `shaders/trees.wgsl`).

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

[Unreleased]: https://github.com/0xe25f/vistawasm/compare/v1.0.0...HEAD
[1.0.0]: https://github.com/0xe25f/vistawasm/releases/tag/v1.0.0
