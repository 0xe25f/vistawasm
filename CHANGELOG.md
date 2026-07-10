# Changelog

All notable changes to VistaWASM are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


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
