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
- Sun, atmosphere, water, flora, grass, cloud, mist, biome, weather,
  shadow, surface, and quality controls.
- The baked biome/surface map and the carved river network.
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
configuration through `wgpu` (`render/gpu.rs`).

### Start-up work

When the engine is created, and never again:

1. **Procedural textures** (`render/textures.rs` +
    `shaders/texture_gen.wgsl`, mipmapped by `shaders/mipgen.wgsl`): eight
    terrain materials (albedo + height, normal + occlusion + roughness),
    ten bark and foliage layers, water ripples and foam, general 2D noise,
    and a 64³ Perlin-Worley volume for clouds and mist. All are generated
    from seamlessly tiling noise on the GPU; no image files are shipped.
2. **Tree species** (`render/tree_models.rs`): eight species meshes built
    from code with fixed seeds, merged into one vertex/index buffer.
3. **Impostors**: each species mesh is rendered once, orthographically,
    into a texture array used for distant trees.
4. **Ocean grid** (`render/water.rs::build_ocean_grid`): a camera-following
    grid whose spacing doubles every 16 steps and whose outer ring reaches
    the horizon.

### Per-terrain work

`EngineCore::install_terrain` → `rebuild_world` → `rebake_surface`:

1. Rivers and lakes are extracted from the drainage network and carved
    into the heightmap (reversibly; see [`docs/water.md`](water.md)).
2. Normals and the biome/surface map are baked
    (`terrain/biomes.rs::classify_surface`).
3. The LOD terrain mesh, tree and grass instances, river geometry, and a
    height texture (for water depth) are uploaded.

`setBiomes` repeats steps 2–3; changing `WaterOptions.rivers` repeats all
three. Other setters only change uniforms.

### Per frame

0. **Pacing.** If two earlier frames are still on the GPU (tracked with
    `Queue::on_submitted_work_done`), the frame is skipped, so frames never
    queue up behind a slow GPU. If the device was lost (tracked with
    `Device::set_device_lost_callback`), rendering returns
    `WEBGPU_DEVICE_LOST`. The engine smooths the time step (`pacing.rs`,
    `FrameClock`) and picks the render scale (`ResolutionController`): it
    averages the interval between rendered frames over half a second,
    lowers the scale in proportion when frames are late, tries one step
    higher after two calm seconds, and avoids a scale that just dropped
    frames for ten seconds. Changing the scale rebuilds the depth, HDR,
    and cloud targets, so it moves in steps of 0.05.
1. **Weather** (CPU, `weather.rs`) advances by the smoothed time step and is
    applied to the cloud, mist, wind, water, and haze options before they
    reach the GPU. Nothing is uploaded when the weather is off.
2. **Terrain shadow bake** (compute, `shaders/terrain_shadow.wgsl`) runs
    only when the sun, softness, or terrain has changed. It marches across
    the height texture towards the sun and writes a lit fraction per texel.
3. **Tree culling** (compute, `shaders/tree_cull.wgsl`) frustum-tests every
    tree and appends it to per-species mesh and/or impostor lists by
    distance, writing the instance counts of the indirect draw arguments.
    Trees near the camera also go to a shadow-caster list, frustum or not.
4. **Tree shadow pass** (`shaders/shadow.wgsl`) draws each caster as one
    sun-facing, alpha-tested impostor quad into a depth-only orthographic
    shadow map, texel-snapped to stop shimmering.
5. **Opaque pass** into a linear `rgba16float` target plus depth:
    - terrain (`shaders/clipmap_render.wgsl`), texture-splatting the three
      strongest of eight materials with height blending, triplanar rock,
      detail normals, climate tinting, wetness, puddles, and snow;
    - tree meshes and impostors (`shaders/trees.wgsl`) via indirect draws;
    - grass (`shaders/grass_instances.wgsl`), alpha-tested.
6. **Cloud pass** (`shaders/atmosphere.wgsl`, `cloud_main`) raymarches the
    clouds at a reduced resolution (`CloudsOptions.resolutionScale`,
    default half) into an `rgba16float` target, blending cumulus, flat
    sheets, and storm towers by the cloud-type options, and adds rain
    shafts below the cloud base when they are enabled. Skipped when there
    are no clouds. With `CloudsOptions.temporal`, a quarter-size pass
    (`cloud_quarter_main`) first marches one sky pixel of every 2 x 2
    block, a different one each frame; the cloud pass then takes that
    sample or the previous frame's clouds, reprojected with the previous
    view-projection, from a second, ping-ponged cloud image. Marching one
    pixel in four inside the full pass would not help: GPUs shade pixels
    in groups, and a group costs as much as its slowest pixel. Pixels
    with terrain in front are always marched, and reuse turns off within
    300 m of the cloud layer.
7. **Composite pass** (`shaders/atmosphere.wgsl`) onto the canvas: reads
    the HDR target, depth, and upsampled clouds, draws sky and sun, applies
    haze and mist along each pixel's true view ray, adds rain and snow, and
    tone maps (ACES).
8. **Water pass** (`shaders/water.wgsl`): ocean grid, rivers, and lakes,
    depth-tested against the opaque scene, alpha-blended, fogged, and tone
    mapped in the same way.
9. **Present pass** (`shaders/atmosphere.wgsl`, `present_main`), only when
    the scene is rendered below the canvas resolution or lens drops are on.
    Steps 5 to 8 then draw into an off-screen image at the render scale,
    and this pass upscales it to the canvas with contrast-adaptive
    sharpening, refracting it through raindrops on the lens. At full
    resolution without lens drops there is no extra pass.

Every render shader is compiled with `shaders/common.wgsl` prepended, which
declares the one `FrameUniforms` struct (752 bytes), the shared world
textures (bind group 1), shadow receivers (bind group 2), the sky model,
lighting, fog integrals, and every shadow lookup. Because there is exactly
one declaration, the Rust struct in `render/gpu.rs` and the WGSL struct
cannot drift apart per shader, and a compile-time size assertion guards the
Rust side. `build.rs` strips comments and indentation from each shader, and
`common.wgsl` is embedded once and prepended at run time, which keeps the
binary small. Every shader is validated with naga in `cargo test`. Each
shader module is compiled once and shared by every pipeline that uses it.

Animation uses a smoothed real-time clock (`camera_position.w`), and wind-driven
offsets are integrated over time, so wind, water, clouds, and mist move at
the same speed regardless of frame rate and never jump when the wind
changes.

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
reuses that cache and only recomputes the new mesh's `513×513` vertices,
not the whole heightmap.

Recentring streams, the way a tile engine streams chunks around a moving
player (`EngineCore::recentre_terrain_mesh_if_needed`, called from
`render_once()`):

- Once the camera drifts 6 samples from the displayed mesh's centre,
  the next mesh is started, centred ahead of the camera by its smoothed
  velocity (half a second of travel, at most 8 samples;
  `terrain_mesh::next_mesh_centre`), so a moving camera flies into detail
  that is already there.
- It is built and uploaded 64 rows per frame
  (`terrain_mesh::build_centred_mesh_rows`) into the second of two vertex
  buffers while the first is drawn, then the buffers swap. The index
  buffer never changes and is uploaded once.
- If the camera drifts 18 samples before the stream finishes (a teleport,
  or very fast flight), the rest is built at once, as before.

This replaces a full rebuild, fresh buffers, and a 16 MB upload in a
single frame every 12 samples. `RenderStats.terrainTriangles`
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
`setWeather`, `setShadows`, `setSurface`, `setBiomes`, `setRenderQuality`,
`setDebugView`, `resize`) checks `pendingCall` first and silently no-ops
while it is set, rather than throwing or queuing; `biomeAt()` and
`getWeather()` return `undefined`. The replacement hooks (`setTreeModel`,
`setTreeInstances`, `replaceTexture`, and their resets) throw instead, so
an asset change is never silently dropped.
`renderOnce()` returns the last real `RenderStats` during that window
instead of calling into the busy object. `exportHeightmap()` throws a
clear, catchable error instead of silently reading stale/partial data.
`dispose()` marks the wrapper disposed immediately (so no new calls start)
but defers the real `raw.dispose()` until any in-flight call settles.

