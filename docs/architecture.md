# VistaWASM Architecture

**VistaWASM is a library, not an application.** The host frontend owns layout,
controls, routing, and application state. VistaWASM owns the terrain engine,
DEM ingestion, camera maths, and WebGPU resources for one supplied canvas.

See also: [Building games with VistaWASM](game-development.md),
[Designing worlds](world-design-guide.md), and
[Integrating with web game engines](engine-integration.md) for
application-facing guides. For a field-by-field reference of every public
option, see [`docs/options-reference.md`](options-reference.md). This
document covers the internals.

## Crates

`vista_types` contains shared serialisable types. It has no WebGPU dependency.

`vista_wasm` contains the engine. It exposes `wasm-bindgen` classes on browser
builds and plain Rust APIs for deterministic tests.

## Ownership

One public `VistaEngine` owns one internal `EngineCore`.

`EngineCore` owns:

- Terrain data, plus cached per-sample normals and material weights (see
  [Terrain rendering and level of detail](#terrain-rendering-and-level-of-detail)).
- Camera/projector matrices.
- Sun, atmosphere, water, flora, grass, cloud, mist, and quality controls.
- WebGPU context on browser builds.
- Render statistics.

JavaScript never receives raw GPU objects or memory offsets.

## Terrain generation

Fractal terrain generation is deterministic CPU Rust (`terrain/fractal.rs`)
for a given seed and option set: layered value noise, then shape masks
(island/terrace/basin/canyon/crater), then vertical scaling. Heights are
stored in metres; metadata records scale, sea level, source, and warnings.
No-data samples are kept in a mask.

Erosion (`terrain/erosion.rs` for the CPU reference implementation) runs
last, directly on height-in-metres data. On browser (`wasm32`) builds,
erosion instead runs as GPU compute passes
(`render/erosion_compute.rs` + `shaders/hydraulic_erosion.wgsl` /
`thermal_erosion.wgsl`) for performance on large terrain — a "gather"
reformulation of the same hydraulic/thermal model (each cell only writes
its own output, reading neighbours, so it is race-free across GPU
invocations), not a bit-identical port of the CPU scatter-based algorithm.
If the GPU pass fails for any reason, `EngineCore::generate_fractal_map`
transparently falls back to the CPU implementation, so terrain generation
always produces a result either way. Native (non-wasm32) builds and tests
always use the CPU path.

## DEM Loading

The host fetches files and passes bytes into WASM. Rust decodes raw heightmaps
and a scoped subset of GeoTIFF. Unsupported compression returns a structured
error.

## Rendering

Rust creates the WebGPU instance, adapter, device, queue, surface, and surface
configuration through `wgpu`. Five pipelines run per frame, in this order:

1. **Atmosphere** (`shaders/atmosphere.wgsl`) — a fullscreen-triangle sky
    dome, drawn first with depth writes disabled and a `Less` depth test
    (so terrain/flora/water drawn afterward occlude it normally). It
    reconstructs a view ray per pixel from the camera basis vectors in
    `FrameUniforms`, then computes an analytic Rayleigh gradient, a
    Henyey-Greenstein Mie phase term for the sun disc/glare, a horizon
    haze blend toward `AtmosphereOptions.skyTint`, and — when
    `CloudsOptions.style` is not `"off"` — an optional cloud layer. Both
    cloud styles (`"painted"`, a single noise sample; `"volumetric"`, a
    raymarched density band) are implemented as one function in this same
    shader/pipeline rather than a separate pass, since the atmosphere pass
    already reconstructs the view ray it needs and terrain drawn
    afterwards already occludes clouds behind mountains via the depth
    buffer.
2. **Terrain** — a single CPU-baked mesh (`render/terrain_mesh.rs` +
    `shaders/clipmap_render.wgsl`), see below for its LOD strategy. Also
    applies the height-based ground mist term (`MistOptions`) on top of
    the existing distance-haze blend.
3. **Flora** — tree instances (`render/flora.rs::build_flora_instances` +
    `shaders/flora_instances.wgsl`). `FloraOptions.treeQuality` selects
    between a camera-facing billboard (`"billboard"`, the default, drawing
    6 vertices per instance) and two static, world-oriented crossed quads
    (`"cross-quad"`/`"mesh"`, drawing 12 vertices per instance from the
    same base vertex buffer) — see
    [`docs/vegetation.md`](vegetation.md) for the visual difference this
    makes. Also applies mist tinting and wind sway.
4. **Grass** — an independent, opt-in ground-cover layer
    (`render/grass.rs::build_grass_instances` +
    `shaders/grass_instances.wgsl`), reusing the same `FloraInstance`
    layout as flora but always drawing three crossed, world-oriented blade
    quads (18 vertices per instance). Alpha-blended with depth writes off
    (like water) so it can fade out smoothly at
    `GrassOptions.viewDistanceMetres`, and placement reuses the terrain's
    cached material weights (see [Terrain rendering and level of
    detail](#terrain-rendering-and-level-of-detail)) rather than
    recomputing slope/height thresholds independently.
5. **Water** — an animated, fresnel-shaded plane
    (`render/water.rs::build_water_plane` + `shaders/water.wgsl`). Also
    applies mist tinting.

All five share one `FrameUniforms` uniform buffer/bind group (320 bytes):
`view_proj, camera_position, camera_forward, camera_right, camera_up,
camera_params (tan_half_fov_y, aspect), sun_direction, sun_colour_intensity,
fog, water_params, sky_tint, atmosphere_params, mist_params, mist_colour,
cloud_params, cloud_colour, vegetation_params`. `water_params.z` is an
internal frame-counter-based clock (`frame_counter / 60.0`), not wall
clock, and is reused by flora/grass wind sway and cloud/mist drift rather
than each shader carrying its own animation time. Each WGSL shader file
only declares as much of this struct as the fields it actually reads
require — the fields are laid out in the same order in every file, so a
shader can stop declaring the struct partway through, but never skip or
reorder a field, since that would misalign every field after it. New
fields are always appended at the end of the Rust struct for the same
reason (see the doc comment on `FrameUniforms` in `render/gpu.rs`, and
`docs/environment-upgrade-plan.md` §1.2 for the full rationale). Flora,
grass, and water regenerate on terrain change and on
`setFlora`/`setGrass`/`setWater`/`setRenderQuality` via
`EngineCore::refresh_flora`/`refresh_grass`/`refresh_water`; clouds and
mist have no terrain-dependent placement, so `setClouds`/`setMist` simply
replace state for the next rendered frame.

### Terrain rendering and level of detail

The terrain mesh is a single, regular, always-crack-free grid — never
multiple independent LOD tiles/rings — recentred on the camera as it moves,
with sample spacing that grows exponentially with distance from that
centre (`render/terrain_mesh.rs::build_terrain_mesh_centred` and
`band_sample_offset`). Concretely: the first `LOD_BAND_WIDTH` (24) grid
steps out from the centre sample at native (step-1) resolution; the next 24
steps each advance 2 samples; the next 24 each advance 4 samples; and so on,
doubling every 24 grid steps. Because the mesh is topologically one
connected `513×513` grid regardless of this non-uniform spacing, it can
never develop the T-junction cracks that a classic multi-tier clipmap
needs skirts or index stitching to hide — the tradeoff is that it is one
LOD strategy applied uniformly in a square/radial pattern around the
camera, not independently controllable rings.

`EngineCore::install_terrain` computes normals and material weights for the
*whole* heightmap once per generated/loaded terrain
(`terrain_mesh::bake_terrain_shading`) and caches them, since both are
relatively expensive full-heightmap passes. Every subsequent recentre
reuses that cache — `EngineCore::recentre_terrain_mesh_if_needed` (called
from `render_once()`) only rebuilds and re-uploads the mesh once the camera
has drifted more than 12 samples from where it's currently centred, and
only ever recomputes vertex positions/normals/materials for the new mesh's
`513×513` samples, not the whole heightmap. `RenderStats.terrainTriangles`
and `.clipmapLevels` reflect this real uploaded mesh on browser builds (a
constant `512 × 512 × 2` triangles, and the number of exponential bands the
mesh's half-span spans); native/test builds without a GPU report a
theoretical estimate instead, from `terrain::clipmap::build_clipmap_levels`.

## Lifecycle

`dispose()` is terminal. The TypeScript wrapper (`js/src/index.ts`,
`VistaEngineWrapper`) owns `requestAnimationFrame` and calls `renderOnce()`.
This keeps browser callback lifetime simple and ensures there is no active
animation frame after disposal.

`wasm-bindgen` forbids any call reaching a given exported object while an
`async fn &mut self` method on that same object has not yet resolved (it
panics with "recursive use of an object detected which would lead to
unsafe aliasing in rust"). This only became reachable in practice once
terrain generation could span multiple animation frames (GPU erosion
readback is a genuine multi-frame async gap) — a camera-control `rAF` loop
calling `setCamera()`, or a `ResizeObserver` calling `resize()`, while
`generateFractal()` is still awaiting erosion, would trip it. The wrapper
guards against this with a `pendingCall` mutex: `callAsync()` chains async
calls sequentially (so overlapping async calls queue rather than racing),
and every sync method that touches the raw engine (`setCamera`, `setSun`,
`setAtmosphere`, `setWater`, `setFlora`, `setGrass`, `setClouds`, `setMist`,
`setRenderQuality`, `setDebugView`, `resize`) checks `pendingCall` first and
silently no-ops while it is set, rather than throwing or queuing.
`renderOnce()` returns the last real `RenderStats` during that window
instead of calling into the busy object. `exportHeightmap()` throws a
clear, catchable error instead of silently reading stale/partial data.
`dispose()` marks the wrapper disposed immediately (so no new calls start)
but defers the real `raw.dispose()` until any in-flight call settles.

