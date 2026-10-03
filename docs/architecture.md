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

Fractal generation is deterministic Rust for a given seed and option set.
It runs in four stages; the landform preset (`terrain/landforms.rs`) sets
the numbers each stage uses.

- **Stage A, tectonics** (`terrain/tectonics.rs`). On a coarse grid (at
  most 256 samples per side, never finer than 40 m), warped gradient noise
  (`terrain/noise.rs`) lays out continents, thresholded by sorting so the
  land fraction is exact. Warped ridged noise, masked to part of the land
  and faded in from the coast, gives the uplift of the mountain ranges.
- **Stage B, drainage** (`terrain/stream_power.rs`). An implicit
  stream-power solver (Braun and Willett, 2013) carves the coarse grid:
  stochastic D8 routing, a stack order from the outlets, drainage-area
  accumulation (`terrain/drainage.rs`, shared with river extraction) and an
  implicit update towards each receiver. Lowlands erode for 10 iterations;
  ranges run 40 iterations to the steady state between uplift and erosion,
  with threshold hillslopes and glacial `n = 2` carving under ice. Valleys
  then get flat floors, glacial troughs are over-deepened (the sea floods
  them as fjords) by a depth that grows smoothly down each channel, so
  their floors step down with shallow basins rather than rows of pits,
  and the drainage area is kept in `HeightMap::aux`.
- **Stage C, detail** (`terrain/fractal.rs`). Bicubic upsampling to full
  resolution, derivative-damped fBm limited by slope and ruggedness, then
  the shape masks and vertical scaling.
- **Stage D, erosion**. Virtual-pipe hydraulic erosion and talus-angle
  thermal erosion, 60 % of iterations at half resolution and 40 % at full
  resolution. Browser builds run it as GPU compute passes
  (`render/erosion_compute.rs`, `shaders/hydraulic_erosion.wgsl`,
  `shaders/thermal_erosion.wgsl`), submitted in chunks so progress is
  reported at least every 10 %. The CPU reference (`terrain/erosion.rs`)
  runs the same passes with the same constants for native builds, tests,
  and as a fallback if the GPU pass fails. Every pass is a gather: each
  cell writes only its own output.

Finally, minor summits are levelled, specks of land drowned and small pits
filled, so the map drains. Heights are stored in metres; metadata records
scale, sea level, source, and warnings. No-data samples are kept in a mask.
Stages report `"tectonics"`, `"drainage"`, `"detail"`, `"erosion"`,
`"finishing"` and, inside it, `"rivers"` progress to the JavaScript
`"progress"` event.

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
    `shaders/texture_gen.wgsl`, mipmapped by `shaders/mipgen.wgsl`):
    twelve terrain materials (albedo + height, normal + occlusion +
    roughness), fifteen bark and foliage layers and eight layers of leaf
    normal maps, water ripples and foam,
    general 2D noise, and a 64³ Perlin-Worley volume for clouds and mist.
    All are generated from seamlessly tiling noise on the GPU; no image
    files are shipped. Most are baked only when first needed: the terrain
    layers of the materials the surface uses (plus rock and sand for the
    skirt, and mud and gravel when there are bank strips), the bark and
    foliage of the species present, and the cloud volume when clouds or
    volumetric mist sample it. The terrain bake writes heights in one
    pass and normals from them in a second.
2. **Tree growth** (`render/tree_growth.rs`, merged by
    `render/tree_models.rs`): the first variant of each of the eight
    species, in four age classes and two levels of detail, grown by space
    colonisation with fixed seeds and merged into one vertex/index
    buffer. The other variants grow after the first frame, one mesh a
    frame, and join the buffer together (see [Trees](#trees)).
3. **Impostors**: each variant's mature tree is rendered once into a
    nine-view atlas used for distant trees, when its species first
    appears.
4. **Ocean grid** (`render/water.rs::build_ocean_grid`): a camera-following
    grid whose spacing doubles every 16 steps and whose outer ring reaches
    the horizon.

### Pipelines

Render and compute pipelines are created when the scene needs them
(`render/pipelines.rs`). Each frame the engine describes what the scene
draws as `Needs` (trees, grass, clouds, rivers, falls, bank strips, sea
ice, screen reflections, the final pass), and every pipeline those need
that does not exist yet is created, in the order the frame draws. What
the scene is likely to need soon (weather that can reach rain, clouds
and lens drops) is created after the first frame has been presented, at
most one pipeline per frame, so the browser compiles it while the scene
is already on screen. Pipelines no longer needed are kept, so switching a
system back on never compiles it again. `wgpu` has no public
asynchronous pipeline creation, so the warm-up relies on this ordering.

### Per-terrain work

`EngineCore::install_terrain` → `rebuild_world` → `rebake_surface`.
First, `release_world` drops everything this work rebuilds (the rivers,
drainage, ground textures, surface and normals), keeping the heights,
options and painted maps. WebAssembly memory never shrinks, so holding
the old world while the new one is built would raise the page's memory
for good.


1. Glaciers raise the ground they cover towards a smooth ice surface, by
    at most 40 m (`terrain/glaciers.rs`). Every raised sample is recorded
    so it can be restored exactly.
2. When water and rivers (or a water mask) are on, the river build runs
    (the `"rivers"` progress phase; see [`docs/water.md`](water.md)):
    1. the surface is classified once, before any channel is cut, for the
        climate's rain, snow and temperature;
    2. a painted water mask flattens its lakes and yields its river
        centrelines (`terrain/water_mask.rs`);
    3. hydrology (`terrain/hydrology.rs`) routes water on a flow grid at
        full resolution up to 1024 per side: inflows from beyond an open
        edge are placed, a priority flood fills basins to their spill
        height (inflow cells are not outlets), discharge is accumulated
        from rain, snowmelt, springs and inflows, lakes overflow or stay
        endorheic, channels are marked, and the network is split into
        streams, main stems first;
    4. the channel stage (`terrain/channels.rs::condition_channels`)
        shapes beds and banks: hydraulic geometry, a level that never
        rises, V or floodplain cross-sections (rock walls on powerful,
        steep reaches), valley floors for big rivers, meanders and
        oxbows, deltas, waterfall steps and cascades, and plunge pools
        sized by discharge. The carve visits each sample once per
        segment, with a scratch running height;
    5. geometry is built (`render/water.rs`): river ribbons (with
        sub-sample loops for streams too narrow to migrate, straight
        runs merged, and inner edges kept within 0.9 of a bend's radius)
        as 56-byte `WaterVertex`es: position, flow, `params` (kind,
        across, this side's half width, metres along the centreline),
        `extra` (slope, curvature, depth, °C) and `swirl` (the signed
        eddy strength below bend apexes, beside joins and below falls,
        plus the catchment's colour as whole multiples of 4), lake and
        oxbow surfaces and plunge pools in one buffer, waterfall sheets
        and mist in another, and banks in a third: five 44-byte
        `BankVertex`es a row (the water's edge, the margin's foot, the
        face's top under the lip, the lip and the turf's back), within a
        300,000-vertex budget, the least visible streams thinned first.
        Steep streams gain two rows at each step-pool lip first
        (`step_pools`). Each drawn stream also records its bed stones and
        riparian band per point, and each sea mouth its plume. Then come
        the wet-bank field, the riparian field (a two-pass chamfer on the
        heightmap grid), the bed materials, the channel field (see
        below) and the sound map (`water_sounds.rs`).

    Every changed sample (mask, channels, pools, delta fans) is recorded
    in one `CarveRecord`, so the terrain can be restored exactly.
3. Normals and the biome/surface map are baked
    (`terrain/biomes.rs::classify_surface`), with the riparian field
    adding moisture near water, then the bed materials are blended in
    (`apply_bed_materials`). Bare rock and scree come from the soil model
    (`terrain/soil.rs::soil_field`), run over the whole map first: soil
    depth from slope, convexity, drainage, frost and the bedrock's hard
    beds, then scree walked down the steepest descent below each outcrop,
    and the talus field boulders are placed from. After a river build,
    only the samples within two samples of a change or a channel, those
    with a riparian value, and those whose soil changed are classified
    again (`reclassify_surface`), which gives the same result as a full
    pass.
4. The LOD terrain mesh, tree and grass instances, river and waterfall
    geometry, a height texture (for water depth), and the surface
    textures are uploaded.
    Trees are placed last, after every height change, from the final
    heights, the surface map, the drawn channels and the drainage area:
    the cover texture is baked (`render/flora.rs::bake_cover`), then the
    far set is placed from the lattice
    (`render/vegetation.rs::lattice_trees`; see
    [`docs/vegetation.md`](vegetation.md#placement)). The grass texture
    (`render/grass.rs::bake_grass`) and the reeds follow. A test holds
    this order.

`setBiomes`, `setWaterMask` and changing `WaterOptions.rivers` restore the
carving and the glaciers, in that order, and repeat all four steps. Other
setters only change uniforms.

### The ground layers

`@group(1) @binding(12) ground_layers` is a three-layer `rgba8unorm`
array at the height texture's resolution: the surface texture (layer 0),
the banks (layer 1) and tree cover (layer 2). WebGPU allows 16 sampled
textures per shader stage by default and the composite and cloud passes
used all 16, so the three same-sized textures share one binding, which
frees bindings 13 and 14 for the weather (below). Render shaders read the
layers through `common.wgsl::ground_layer_at`; the generators bind each
layer as its own 2D view.

### The surface texture

Layer 0 of `ground_layers`, uploaded with the other per-terrain data by
`GpuContext::upload_surface`. Later work reuses it, so its channels are
fixed:

| Channel | Contents |
| --- | --- |
| r | Temperature unit, `(°C + 30) / 65`: 0 is -30 °C, 1 is 35 °C. |
| g | Moisture, 0 (arid) to 1 (saturated). |
| b | Permanent snow, 0 to 1: 1 on glacier, up to 0.63 on tundra. On ocean samples, 1 marks snow-covered fast ice. |
| a | Biome index (`BiomeKind` as `u8`) / 255. Read it with `textureLoad`: filtering blends indices. |

Shaders sample it with `textureSampleLevel` and the clamp sampler
(`common.wgsl::surface_at`), which is safe in non-uniform control flow.

Layer 1 of `ground_layers`, the banks, is uploaded by
`GpuContext::upload_surface` with the first:

| Channel | Contents |
| --- | --- |
| r | Distance to the nearest river, lake or waterfall edge / 40 m: 0 at the water, 1 at 40 m or more (and everywhere when there is no water). |
| g | Snow and ice cover, 0 to 1: the sample's snow and ice material weights or its permanent snow, whichever is more (`SurfaceSample::snow_cover`). Bank strips fade out under it. |
| b | Bankside greening, 0 to 1 (`RiverNetwork::riparian`): the terrain shader freshens grass by it and leaves it out of its wetness. |
| a | The talus field (`soil::talus_byte`): the top four bits the chance of a boulder per 2 m candidate, the bottom four how far down its talus cone. 0 where no boulder lies (`SurfaceSample::talus_here`). The generators read it with `textureLoad`. |

The distance is built on the CPU with a two-pass chamfer distance
transform (`render/water.rs::WetBanks`), seeded from each water's edge:
half a sample beyond lakes, pools and wide rivers, and from the drawn
edge of streams narrower than a sample. The channels are sampled with
`textureSampleLevel` (`common.wgsl::water_banks_at` and
`snow_cover_at`).

### The channel field

`render/channel_field.rs::ChannelField` holds, at four times the
heightmap's resolution, the signed distance to the nearest drawn water
edge (±16 m in a byte, negative under water) and a flow byte: the
owning channel's speed (two bits, 0.5 m/s steps), whether its banks are
meshed or rock (a bit each), and the loose bed's band (a nibble, 0.07 n²
metres). It is sparse: only 16 x 16-texel tiles that hold water within
the field's reach are stored, at most 8 MB, widest channels first. The
tiles go up as an `rg8` atlas 64 tiles across, and a storage buffer
maps each map tile to its slot (0 for none) after a four-word header:
texels per metre, map tiles across and down, and 1 when there are
tiles. The terrain and bank shaders bind both as group 3, beside the
grass mask, and read them with `textureLoad` (`channel_field_at`),
so wet margins, beds and banks follow the drawn edges, not the
heightmap's samples.

### The channel bins

`render/vegetation.rs::channel_bins` packs the drawn channels and
plunge pools for the generators: a grid of cells at least 64 m across,
then 11 words a segment (its ends, half widths, clearance, the bed
stones' median size and chance, the riparian band's density for the
grass, and the scrub's after the tree mask). Trees, scrub, grass,
boulders and stream stones all read them (`near_channel`,
`in_channel`).

### The cover texture

Layer 2 of `ground_layers`, tree cover, is baked on the CPU once per
terrain and uploaded by `GpuContext::upload_cover`. The tree generator reads it with
`textureLoad`; render shaders read it through `common.wgsl::cover_at`.

| Channel | Contents |
| --- | --- |
| r | The square root of the forest share / 4: tree probability is `r² x p_scale`, up to 4 times the density table. |
| g | The likeliest species (`TreeSpecies` as `u8`). |
| b | The second species. |
| a | Bits 0 to 6: the second species' share / 127. Bit 7: stunted (tree line and exposed ridges). |

`common.wgsl::canopy_at` turns red into canopy cover,
`1 - exp(-crowns per square metre x red share)`, which caps the forest
floor material in the terrain vertex shader and drives the canopy layer.

The tree, grass and boulder generators (`shaders/tree_generate.wgsl`,
`shaders/grass_generate.wgsl`, `shaders/boulder_generate.wgsl`) bind the
height, surface, bank and cover textures they read, plus the grass
texture for grass, at their own group 0, with the drawn channels as bins
of segments (`render/vegetation.rs::channel_bins`) in a storage buffer.
Trees and grass leave room for boulders by testing the boulder lattice
round each candidate (`generate_common.wgsl::under_boulder`).

### The grass density mask

`engine.setVegetationMasks({ grass })` scales the grass. The CPU resamples
the mask to the ground layers' texels, caps each value so the grass is
never denser than at density 4, and uploads it as an `r8unorm` texture
(`GpuContext::upload_grass_mask`); without a mask it is 1 x 1 and
neutral. It is rebuilt only when the mask or the grass settings change,
never per frame. A byte of 0 is no grass, 128 unchanged and 255 twice as
dense (`ground.wgsl::density_multiplier`, mirroring
`painted::density_multiplier`).

Two passes read it with `textureSampleLevel`, so the tufts and the
ground's grass sheen stay matched:

- the grass generator scales each lattice point's chance of a tuft by
    it (`@group(0) @binding(12)`, with its own sampler at binding 13);
- the terrain scales its distant grass sheen by it
    (`@group(3) @binding(0)`, in the terrain's own pipeline layout). The
    composite and cloud passes use every sampled texture a stage may
    have, so the mask cannot join the world group.

The tree mask needs no texture: it scales the cover texture's red
channel when the cover is baked (`flora::bake_cover`).

### The weather textures

`@group(1) @binding(13) regional_texture` is the regional weather map
(`weather/regional.rs`): 128 x 128 `rgba16float`, coverage,
precipitation, storminess and humidity, over `regionSizeKm` centred on
the terrain. The CPU refreshes 16 rows a frame (all 128 after a sudden
change) and uploads only the rows that changed. `FrameUniforms.regional`
holds its corner, 1 / its size, and a flag that is 1 while clouds, rain
curtains and the wet ground read it; `common.wgsl::regional_weather_at`
samples it.

`@group(1) @binding(14) surface_weather_texture` is the surface weather
map, `rgba8unorm` at the terrain shadow's resolution (the height
texture's, capped at 1024 on the long side):

| Channel | Contents |
| --- | --- |
| r | Wetness, 0 to 1. |
| g | Puddle water: a hollow's fill times how much it can hold. |
| b | Snow depth, 0 to 1. |
| a | A hollow's fill, 0 to 1, which the pass needs to keep. |

`shaders/surface_weather.wgsl` steps it four times a second with the
weather time since its last step, reading the previous state, the
heights (slope and the Laplacian for hollows), the surface layer
(temperature), the cover layer (canopy shade) and the regional map
(precipitation), and writing a second texture that is then copied back,
so the render shaders always read one. Its maths is `weather/surface.rs`'s
`step`, which the CPU also runs on a 64 x 64 mirror for `weatherAt`.
Values are rounded up or down at random in proportion, so slow drying is
not lost to the 8-bit steps. `common.wgsl::surface_weather_at` returns
the map's values, or the weather's single values off the terrain or
while the weather does not drive the ground.

### Terrain vertices

Each terrain vertex is 36 bytes:

| Bytes | Attribute | Contents |
| --- | --- | --- |
| 0–11 | `float32x3` | Position in terrain metres. |
| 12–15 | `snorm16x2` | Octahedron-encoded normal (`terrain_mesh::encode_normal`). |
| 16–27 | `uint32x3` | Twelve `unorm8` material weights, unpacked with `unpack4x8unorm`: lush grass, dry grass, forest floor, sand, rock, snow, mud, volcanic, ice, tundra, gravel, scree. |
| 28–31 | `unorm8x4` | Moisture, temperature, volcanic heat, occlusion. |
| 32–35 | `uint8x4` | Biome index, tree cover, the canopy layer's lift in metres (`terrain_mesh::CANOPY_LIFT_BYTE`), permanent snow. |

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
1. **Weather** (CPU, `weather/`) advances by the smoothed time step. The
    time of day (`weather/sun.rs`) moves the sun. The preset table
    (`weather/presets.rs`) is blended field by field; the regional field
    (`weather/regional.rs`) drifts with the winds aloft and refreshes 16
    rows; the camera's own place on it gives the local coverage,
    precipitation, humidity and storminess, and four samples towards the
    sun give the direct light. The gust front's travel and the sea state
    (`weather/wind.rs`) come from the one resolved wind. The result is
    applied to the cloud, mist, wind, water, haze and light options before
    they reach the GPU, and every quarter of a second the wet ground steps
    on the CPU mirror and queues a GPU step. Nothing is uploaded when the
    weather is off, and the uniforms it adds are then neutral, so shading
    is exactly as without weather.
2. **Terrain shadow bake** (compute, `shaders/terrain_shadow.wgsl`) runs
    only when the sun, softness, or terrain has changed. It marches across
    the height texture towards the sun and writes a lit fraction per texel.
3. **Grounding** (compute, `shaders/grounding.wgsl`) runs only when the
    drawn terrain mesh recentres or the heights change. It stands every
    procedural tree (and every hand-placed tree with `ground: true`) on
    the mesh as it is drawn, sunk to the lowest ground within 2.5 trunk
    radii less 5 % of that radius, and every grass tuft on the ground under
    it, writing the heights into their instance buffers. The height rule
    is `mesh_height` in `shaders/ground.wgsl`: the LOD band and grid cell
    of the drawn mesh at a position (from the `FrameUniforms.ground`
    uniform: the mesh centre, metres per sample and skirt reach), the
    triangle split from bottom-left to top-right, and exact texel loads.
    `render/terrain_mesh.rs::mesh_surface_height` is the Rust copy, tested
    against the built mesh's triangles.
4. **Vegetation streaming** (CPU, `render/vegetation.rs::Streams`, and
    compute, the `"generation"` pass). The CPU estimates the trees and
    tufts drawn from the cover and grass textures, lets the budgets and
    detail pressure set the near radii, and picks the tiles to fill,
    nearest first: at most 8 tree tiles, 16 grass tiles and 8 boulder
    tiles a frame. The generators fill those tiles' slots in the tile
    pools and ground each plant and boulder on the drawn mesh as they
    write it (boulders sunk by 25 to 40 % of their height), so grounding
    never runs per frame; live tiles are re-grounded when the mesh
    recentres. With no tiles to fill, the pass costs nothing.
5. **Tree culling** (compute, `shaders/tree_cull.wgsl`) runs over the far
    set and every live tile slot. Lattice trees are thinned by distance
    and rank and grown to keep crown area; between 0.8 and 1 times the
    canopy distance they drop out by hash. Each survivor is frustum-tested
    and appended by distance to a mesh list (one per species, variant, age
    class and level of detail: full meshes within 50 m, light ones to
    150 m) or the impostor list, and to both across the last tenth of each
    band, writing the instance counts of the indirect draw arguments (see
    [Trees](#trees)); understorey saplings have their own mesh lists, and
    are impostors beyond 15 m.
    Trees near the camera also go to a shadow-caster list, frustum or not,
    capped at a quarter of `maxTreeTriangles`. The draw arguments are read
    back a frame or two late, without stalling, for the triangle budget
    and `RenderStats.treeTriangles`. Grass
    tiles are culled the same way (`grass_generate.wgsl`, `cull_main`)
    into the grass draw list, and boulders (`boulder_generate.wgsl`,
    `cull_main`, in the `"generation"` pass) into one indexed indirect
    draw per mesh variant and level of detail, at most 20,000, with a
    shadow list per variant within 150 m.
6. **Tree shadow pass** (`shaders/trees.wgsl`, `vertex_shadow` and
    `fragment_shadow`) draws each caster as one sun-facing quad, swaying
    and leaning with its tree and alpha-tested against the impostor view
    nearest the sun, into a depth-only orthographic shadow map,
    texel-snapped to stop shimmering. Boulders within 150 m
    cast into the same map (`shaders/boulders.wgsl`, `shadow_main`).
7. **Opaque pass** into a linear `rgba16float` target plus depth:
    - terrain (`shaders/clipmap_render.wgsl`, with the material sampling
      and bare rock of `shaders/materials.wgsl`), texture-splatting the
      three strongest of twelve materials with height blending, triplanar
      rock, scree, ice and steep snow (one projection path, blending
      smoothly from top-down as slopes steepen), detail normals, climate
      tinting, wetness, puddles, snow, and glacier crevasses; rock adds
      macro joints and bedding ledges, crevice shade on sky light, lichen
      and water streaks, and within 120 m rock and scree take four steps
      of parallax;
    - the canopy layer (`clipmap_render.wgsl`, `canopy_vertex_main` and
      `canopy_fragment_main`), when trees are streamed and the view
      reaches it: the terrain's far bands drawn again at every other
      vertex, raised by the canopy height and faded in crown by crown as
      the trees drop out;
    - bank strips beside streams narrower than a sample
      (`clipmap_render.wgsl`, `vertex_bank` and `fragment_bank`), right
      after the terrain, alpha-blended with a depth bias and no depth
      writes;
    - boulders (`shaders/boulders.wgsl`) via indexed indirect draws, in
      their own timed pass, before the trees and grass they hide;
    - tree meshes and impostors (`shaders/trees.wgsl`) via indirect draws:
      full meshes, light meshes (the same module with the `LIGHT`
      override constant), understorey saplings, then impostors;
    - grass, ferns and undergrowth (`shaders/grass_instances.wgsl`),
      alpha-tested.

    Trees and the canopy are drawn before the scene copy, so water
    reflects them.
8. **Cloud pass** (`shaders/atmosphere.wgsl`, `cloud_main`) raymarches the
    clouds at a reduced resolution (`CloudsOptions.resolutionScale`,
    default half) into an `rgba16float` target, blending cumulus, flat
    sheets, and storm towers by the cloud-type options, and adds rain
    shafts below the cloud base when they are enabled. The mid-level
    layer (altocumulus and altostratus) is one 2D layer in the same pass:
    at most five 2D noise lookups per pixel (two for altostratus alone),
    plus two of the regional map where the weather varies, and none at
    amounts of 0. It shades the sunlight in the cumulus march with one
    lookup per lit sample. The pass composites back to front: from below the mid-level
    layer, the mid-level layer and then the low clouds and rain shafts in
    front of it; from above it, the reverse. Cirrus is drawn behind them
    all in the composite pass. Skipped when there are neither low nor
    mid-level clouds. With `CloudsOptions.temporal`, a quarter-size pass
    (`cloud_quarter_main`) first marches one sky pixel of every 2 x 2
    block, a different one each frame; the cloud pass then takes that
    sample or the previous frame's clouds, reprojected with the previous
    view-projection, from a second, ping-ponged cloud image. Marching one
    pixel in four inside the full pass would not help: GPUs shade pixels
    in groups, and a group costs as much as its slowest pixel. Pixels
    with terrain in front are always marched, and reuse turns off within
    300 m of the cloud layer.
9. **Composite pass** (`shaders/atmosphere.wgsl`) onto the canvas: reads
    the HDR target, depth, and upsampled clouds, draws sky and sun (a
    watery disc with a corona behind a mid-level veil), applies
    haze and mist along each pixel's true view ray, adds rain and snow, and
    tone maps (ACES).
10. **Scene copy pass** (`shaders/atmosphere.wgsl`, `scene_copy_main`),
    only with `WaterOptions.reflections` `"screen"`: copies the HDR target
    and linear view depth into a half-resolution `rgba16float` image for
    water to reflect. Water is not in it, so water never reflects water.
    Its GPU time is counted in the water pass.
11. **Water pass** (`shaders/water.wgsl`): the ocean grid, then rivers,
    lakes and plunge pools, then waterfall sheets and mist, depth-tested
    against the opaque scene, alpha-blended, fogged, and tone mapped in
    the same way. The ocean draws with one of two pipelines from the same
    shader: one with sea ice, and one compiled with the `SEA_ICE` override
    constant off, drawn whenever no sea can freeze. Rivers and lakes draw
    with a third, compiled with the `INLAND` constant on, so the ocean's
    pipelines contain no river, pool or lake-ice code and the inland one
    no waves or sea ice. Waterfalls draw with a fourth, with their own
    fragment entry point (`fragment_fall`). Frozen water and waterfalls
    are also behind uniform guards (`frame.rivers`), so maps without them
    skip that code. The water pipelines bind the scene copy as group 3
    (`@group(3) @binding(0) scene_copy`), and trace reflected rays through
    it behind the `frame.water_origin.w` guard. Shallow water also reads
    it along the refracted ray, with caustics, behind the
    `frame.waterside.w` guard, and the ocean tints the plumes of the
    eight river mouths in `frame.mouths`.
12. **Present pass** (`shaders/atmosphere.wgsl`, `present_main`), only when
    the scene is rendered below the canvas resolution or lens drops are on.
    Steps 6 to 10 then draw into an off-screen image at the render scale,
    and this pass upscales it to the canvas with contrast-adaptive
    sharpening, refracting it through raindrops on the lens. The drops
    are simulated on the CPU (`lens_drops.rs`) and uploaded with a screen
    tile grid in two storage buffers bound to this pass only; each pixel
    tests the drops of its own tile. At full resolution without drops on
    the lens there is no extra pass.

Every render shader is compiled with `shaders/ground.wgsl` (the ground
height rules, which the grounding pass also uses) and `shaders/common.wgsl`
prepended; `common.wgsl`
declares the one `FrameUniforms` struct (1,248 bytes), the shared world
textures (bind group 1), shadow receivers (bind group 2), the sky model,
lighting, fog integrals, and every shadow lookup. Because there is exactly
one declaration, the Rust struct in `render/gpu.rs` and the WGSL struct
cannot drift apart per shader, and a compile-time size assertion guards the
Rust side. The weather appended the last five `vec4`s: `regional` (the
regional map's mapping), `air` (the Mie colour and phase asymmetry),
`light` (direct sun, shadow strength, indirect light and flat sky light
under cloud), `gust` (the gust front) and `weather4` (the surface weather
flag, whitecaps and spray). Without weather they hold values that leave
shading exactly as before. The cloud bases and the mid-level layer
appended two more: `clouds5` (base variation, base lumpiness, and the
altocumulus and altostratus amounts) and `alto` (the mid-level layer's
drift and height; its fourth component is unused, since the composite's
watery sun now looks through the veil per pixel, within 3 degrees of the
sun only). Rivers appended `rivers2`, whose `x` is the eddy strength
(`WaterOptions::eddies`; 0 skips all eddy code). `build.rs` strips comments and indentation from each shader and
gives the names the shaders declare short ones (entry points and
`override` constants keep theirs), and `common.wgsl` is embedded once and
prepended at run time, which keeps the binary small. After `wasm-pack`,
`scripts/shorten-names.mjs` gives the functions the module imports from
its JavaScript glue, and its closure shims, short names in both files. Every shader is validated with naga in `cargo test`. Each
shader module is compiled once and shared by every pipeline that uses it.

Animation uses a smoothed real-time clock (`camera_position.w`), and wind-driven
offsets are integrated over time, so wind, water, clouds, and mist move at
the same speed regardless of frame rate and never jump when the wind
changes.

### Trees

Every tree is grown in WASM, which is sequential work, and drawn on the
GPU. `TreeVertex` is 56 bytes:

| Offset | Attribute | Format | Holds |
| --- | --- | --- | --- |
| 0 | `position` | `float32x3` | Model-space metres, trunk base at the origin |
| 12 | `normal` | `snorm16x4` | Normal (xyz); w: branch level / 4 plus 0.2 x the clump's colour (or a cypress knee's flag) |
| 20 | `uv` | `float32x2` | Bark: around and along; leaves: v from the twig out |
| 28 | `params` | `float32x4` | Flora layer, stiffness, occlusion, branch phase |
| 44 | `pivot` | `float32x3` | Where the vertex's limb leaves the trunk |

The library holds `8 species x 4 variants x 4 age classes x 2 levels of
detail` mesh slots (`tree_models::mesh_slot`; shrubs share their mature
mesh for the krummholz slot). Each tree's species word carries its
variant (bits 10 and 11), age class (12 and 13) and lean (14 to 19)
from the generators; the grounding pass, culling and exports read the
species through `TREE_SPECIES_BITS`.

The cull pass writes one indexed indirect draw (5 words) per mesh list:
the 256 canopy lists in mesh-slot order, then 32 understorey lists (per
species and variant, the young mesh), then the impostor draw (4 words at
word 1440) and the shadow-caster draw (4 words at 1444): 1,448 words in
all. A storage buffer holds each list's first slot and slots; each list
has twice its share of its species' trees, and a full list passes trees
on to the lighter level, then to impostors. Drawn trees are 9 floats:
position, scale, rotation, tint, `species | variant << 3 | lean << 5`
plus half the fade (negated for the complementary dither), dryness and
extra crown width.

Impostors live in one `rgba8` array: per species and variant, a colour
layer and a normal layer, each a 3 x 3 atlas of 80 x 160 views with a
margin that keeps mips from bleeding across views (39 MB at 4 variants,
5 mips). Eight views look in from the horizon at 45 degrees apart, and
the ninth down from 40 degrees above, projected onto the plane in front
of the crown where the impostor shader draws it. The bake pipeline (two
colour targets) is made for each bake and dropped after it. Mips keep
each view's leaf coverage (`mipgen.wgsl`), and the colour of each
species' views from above, averaged on the GPU, becomes the canopy layer's
colour in `WorldInfo` (624 bytes; the CPU writes all but those colours).

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

Spacing grows separately along x and z, so away from the diagonals the
outer cells are long and thin. A vertex's normal is therefore the slope
averaged over its own spacing in each direction (a separable box filter,
from per-row prefix sums), not the normal of the one sample it sits on,
which would alias and streak down slopes. Vertex heights stay exact, so
the mesh matches the height texture that water, shadows and trees use.

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
`getWeather()` return `undefined`, and `temperatureAt()` returns `null`. The replacement hooks (`setTreeModel`,
`setTreeInstances`, `replaceTexture`, and their resets) throw instead, so
an asset change is never silently dropped.
`renderOnce()` returns the last real `RenderStats` during that window
instead of calling into the busy object. `exportHeightmap()` throws a
clear, catchable error instead of silently reading stale/partial data.
`dispose()` marks the wrapper disposed immediately (so no new calls start)
but defers the real `raw.dispose()` until any in-flight call settles.

