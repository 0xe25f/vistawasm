# Plan 3: Rivers, lakes and waterfalls

## Goal

Rivers, streams, lakes and waterfalls should read as the result of water
draining the land.

- **Sources:** rain gathering in valleys, snowmelt from glaciers and snow
    fields, springs at the foot of slopes, and lakes that fill basins and
    overflow.
- **Form:** eroded channels that grow wider downstream, meander across
    plains and reach bigger lakes or the sea.
- **Look:**
    - flowing, depth-coloured water with rapids and foam;
    - real waterfalls with a curved sheet, streaks, spray and a churned
        plunge pool;
    - wet banks with bankside plants.
- **Queries:** positions for audio.
- **Authoring:** users can draw their own rivers and lakes as a water
    mask.

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
    plan states how much it may add on top of that. Treat that figure as
    a soft target, and double it as the hard limit, which must never be
    exceeded. Every byte must buy real value that can't be done smaller;
    above the soft target, the report must justify the extra bytes.
    Generate data procedurally at start-up instead of embedding it.
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

- Sources: rainfall and flow accumulation, snowmelt and glaciers, lakes
    filling basins (no dead ends), and springs.
- Rendering: a flowing water surface, real waterfalls, wet banks, and
    river sound hooks.
- Carving is erosion-coupled: rivers come from the terrain's own
    drainage and erosion, with V-shaped valleys in mountains, meanders and
    floodplains in lowlands, and deltas at the sea.
- Authoring: import or paint a water mask; the engine carves and renders
    it. The import and paint UIs are plans 10 and 11. This plan provides
    the complete engine API they call.

## Prerequisites

- **Plan 1** (realistic terrain generation) must be merged. Check that
    `crates/vista_wasm/src/terrain/drainage.rs` and
    `terrain/stream_power.rs` exist, and that `HeightMap` has `aux` with
    the drainage area. If plan 1 is missing, carry out plan 1 first; it is
    self-contained.
- **Plan 2** (ice and arctic biome) must be merged. Check for
    `BiomeKind::IceArctic`, the surface texture at `group(1) binding(12)`,
    and `SurfaceSample.permanent_snow`. If it is missing, carry out plan 2
    first.
- **Plan 2b** (course correction) must be merged. Check for
    `FractalTerrainOptions.edges`, the skirt in `common.wgsl`
    `terrain_height_at`, and `scripts/visual-check/fixed-scene.mjs`. If it
    is missing, carry out plan 2b first.

### What plans 1, 2 and 2b actually built

These notes correct assumptions made before those plans were
implemented:

- `HeightMap.aux` drainage area is on a **coarse grid** (at most 256 per
    side; see `TerrainAux::size` and its bilinear sampler), and it is
    `None` for DEM and raw imports. Use it only as an optional weighting.
    Compute full-resolution D8 receivers and accumulation with
    `terrain/drainage.rs` on the final heights, after erosion, glacier
    smoothing and any water mask, for every terrain source.
- Biomes now include `alpineTransition` (15), `lowerSnowyPeaks` (16),
    `upperSnowyPeaks` (17) and `iceArctic` (18). With an unset climate,
    high ground carries the snowy-peak biomes, and `permanent_snow` is 0
    everywhere. Snowmelt must not rely on `permanent_snow` alone (see
    section 1).
- The surface texture's alpha channel already holds the biome index (read
    with `textureLoad`), so the distance-to-water field gets its own
    texture (see section 6).
- With `edges: "coast"` (the default), every map is ringed by sea, so
    rivers can always reach it. With `"open"`, channels reaching the map
    edge end there, which counts as a valid outlet.
- Beyond the map, a skirt descends into the sea (plan 2b):
    `skirt_height` in `common.wgsl` and `render/terrain_mesh.rs`.
    `terrain_height_at` already returns skirt heights outside the
    footprint, so water depth there is correct. Never build channels,
    lakes, waterfalls or wet banks on the skirt: the river build works
    only on the height map.
- Plan 2b ships a like-for-like performance gate,
    `scripts/visual-check/fixed-scene.mjs`. It loads a fixed 512 x 512
    height map through `loadRawHeightmap`, so rivers are built for it too.
    Its water pass will change with this plan; see Budgets.
- Glacial troughs (plan 2b, commit `ac5cf31`) leave shallow basins behind
    their steps, under 15 % of the over-deepening. Section 2 fills them
    as tarns, which is intended. `finish_fractal_heightmap` already
    removes pits smaller than `MIN_BASIN_SAMPLES`, so only real basins
    remain.
- Pack ice is drawn by `pack_ice(xz, concentration, footprint, distance)`
    in `water.wgsl`, behind the uniform guard `sea_ice_possible()`.
    Section 4b reuses both.
- The WASM is 285,511 bytes gzipped after plan 2b. Measure it again at
    the start.

## Current state

Read these files before starting:

- `crates/vista_wasm/src/render/water.rs`:
    - `build_river_network` carves channels into a finished map, from up
        to 600 highest-catchment sources on a flow grid of at most 512.
    - `add_river_ribbon` builds ribbons with per-vertex flow;
        `river_width_metres` grows width with catchment.
    - `carve_river` and `restore_carving` carve and undo; `add_lakes`
        adds lakes.
- `crates/vista_wasm/src/shaders/water.wgsl` renders three kinds:
    - river ribbons with a two-phase flow map and foam when flow speed is
        above 1.6;
    - flat lakes;
    - the ocean.
- There are no waterfalls. Steep reaches are ribbons draped over the
    slope, which is the dated look to replace.
- `RiverOptions` (`enabled`, `minCatchmentKm2`, `widthScale`,
    `currentSpeed`) sits in `WaterOptions.rivers`.
- `engine.rs` `wanted_rivers` rebuilds the network when options change,
    and a test checks that toggling rivers restores the terrain.

## Design

### 1. Discharge, not only catchment

- For every cell, runoff = precipitation x drainage-area contribution.
    - Precipitation comes from moisture: `surface_texture.g` from plan 2,
        mapped to 300 to 3000 mm per year.
    - Accumulate it along the D8 receivers from `terrain/drainage.rs` to
        get a mean discharge Q in m^3/s.
- **Snowmelt:** cells add melt in proportion to their snow, which is
    `snowmelt x snow x 0.6 m per year` over their area. Here `snow` is the
    largest of three values:
    - `permanent_snow / 255`;
    - the snow material weight;
    - 1 in `upperSnowyPeaks`, and 0.6 in `lowerSnowyPeaks`.
    Default (unset-climate) maps then get snowmelt streams from their
    snowy peaks. The lower edge of each connected snowy-peak region
    becomes an explicit source, as glacier snouts do.
    - Glacier snouts (the lowest glacier cells of each connected glacier)
        become explicit sources, even if their catchment is below the
        threshold.
- **Springs:** at slope-foot cells, where the curvature is concave, the
    upslope drainage is above 0.05 km^2 and the slope drops from above
    20 degrees to below 8 degrees, add a small source (0.02 m^3/s).
    - Spacing is deterministic, at most one per 600 m, seeded from the
        terrain seed.
- A cell becomes channel when `Q >= qmin`, where
    `qmin = minCatchmentKm2 x 0.03 m^3/s per km^2`. This keeps the meaning
    of the existing option.

### 2. Lakes that fill and overflow

- Priority-flood the final terrain, keeping depressions:
    - a depression with at least 24 cells, or 1.5 m deep, becomes a lake;
    - its surface is the spill height;
    - its outlet is the spill cell.
- The outlet river starts at the spill cell with the lake's accumulated
    inflow, so there are never dead ends.
    - An endorheic basin, where evaporation exceeds inflow in arid
        climates, keeps its lake without an outlet. Mark it
        `endorheic: true` in the lake data.
- Every channel must end at the sea, at a lake, or at the map edge. Test
    this.

### 3. Channel form (erosion-coupled)

- Carving follows the eroded terrain. Channels run along the valleys
    that plan 1's erosion carved, and conditioning only shapes the beds
    and banks. It stays restorable, because options and masks can change
    after generation.
- Add a final `condition_channels` stage in `terrain/`. It replaces the
    carving inside `build_river_network`, and keeps the order that
    `engine.rs` `rebuild_world` uses today:
    1. `restore_carving`;
    2. `restore_glaciers`;
    3. `shape_glaciers`;
    4. then this stage, whenever rivers are enabled.
- Channels do not run over glacier ice (`IceArctic` samples with
    `permanent_snow` 255). Water there flows beneath the ice: follow the
    drainage, but carve and draw nothing until the first non-glacier
    sample, which is the snout source from section 1.
- The stage covers:
    - **Hydraulic geometry:** width `w = 2.7 x Q^0.5 x widthScale` metres
        and depth `d = 0.35 x Q^0.4` metres, both clamped to
        [0.6, 400] m.
    - **Bed profile:** the bed must descend monotonically from source to
        mouth. Clamp each point to at most its upstream neighbour, then
        smooth the longitudinal profile with a 5-point average that never
        rises.
    - **Cross-section:**
        - Where the valley slope is above 6 %, carve a V: the bank slope
            follows the terrain, and the channel stays narrow.
        - Where it is below 2 %, carve a flat-bottomed trapezoid with a
            floodplain 4 x w wide on each side, flattened towards the bank
            height.
        - Blend between the two.
    - **Meanders:** on reaches with slope under 1.5 % and w above 4 m,
        displace the centreline with a Kinoshita curve.
        - The wavelength is 11 x w, and the amplitude grows to 2.5 x w on
            flat ground, scaled by `meanders`.
        - The curve is seeded per reach, and its ends stay pinned so
            joins stay connected.
        - Carve along the displaced path, and add oxbow-lake depressions
            (flat, 0.6 d deep) at 10 % of the sharpest loops.
    - **Deltas:** where a channel with w above 8 m meets the sea on a
        slope under 0.5 %, split it into 2 or 3 distributaries fanning out
        at ±25 degrees over the last 8 x w. Deposit a low fan (0.5 m
        above sea level) between them.
    - **Waterfall steps:** where the bed drops more than
        `max(3 m, 1.5 w)` within two samples, or the valley slope is over
        35 degrees, do not smooth the step away. Record a waterfall there
        instead (section 5), and carve a plunge pool at its foot:
        radius `0.3 x height + w`, depth `0.15 x height`.
- Record every changed sample in the existing carving record, so
    `restore_carving` returns the exact original terrain. The existing
    toggle test must keep passing, and must also cover meanders and
    plunge pools.
- Budget: the whole river build, including lakes, takes at most 300 ms
    at 512 x 512 in WASM. Log it in the generation phases.

### 4. Flowing water surface

- **Surface height:** the river surface follows the bed plus depth,
    smoothed along the path so it never steps up. Lakes are flat at the
    spill height, and a river entering a lake meets it at lake level.
- **Ribbons:**
    - Make them 1.3 x w wide, so the edge always lies on the bank.
    - The shader fades alpha by water depth: sample the terrain height
        texture (already used for absorption) and use `smoothstep(0, 0.25 m, depth)`,
        so banks meet the water with no visible edge.
    - Subdivide steep reaches more finely, keeping segments at most
        `0.5 x w` long.
- **Speed:** Manning's equation,
    `v = (1 / n) x R^(2/3) x S^(1/2)`, with n = 0.035 and R ≈ d.
    - Clamp v to [0.2, 6] m/s, multiplied by `currentSpeed`.
    - Store it in the vertex flow vector along the path tangent.
- **Shading** in `water.wgsl`:
    - Keep the two-phase flow map. Scale ripple size with speed: smooth
        and glassy under 0.5 m/s, choppy above 2 m/s.
    - **Rapids** (slope 2 to 8 %): add standing-wave normals, a
        stationary pattern with the texture offset fixed in space, plus
        whitewater foam in proportion to `saturate((v - 1.5) / 2.5)`.
    - **Bends:** extra foam and speed on the outer bank, from the path
        curvature sign stored in a vertex attribute.
    - **Clarity:** clear shallows showing the bed, with the colour taken
        from depth absorption (existing), and the absorption colour tinted
        by the sediment load of fast rivers.
    - Keep every texture sample uniform-safe. The existing code uses
        `textureSample` at top level, which is fine; any new sample inside
        a per-pixel branch must use `textureSampleLevel` or
        `textureSampleGrad`.
- **Snowmelt fullness:** a uniform `melt_factor` (0.4 to 1.4) from the
    weather temperature at the camera scales speed and foam, and raises
    the river surface by up to `0.2 x d` inside the carved channel. The
    geometry stays the same, so no rebuild is needed.

### 4b. Frozen lakes, rivers and falls

Verification of plan 2 found lakes staying liquid and turquoise at
-18 °C. Water follows the climate:

- **Lakes** freeze where the lake's mean temperature (`celsius()` at its
    outlet) is below 0 °C.
    - Draw them with plan 2b's `pack_ice` at concentration 1, called with
        a larger floe scale (lake ice is continuous: large, smooth sheets
        with pressure cracks), and snow cover rising as the temperature
        falls.
    - Guard it the way `sea_ice_possible()` guards sea ice: a new uniform
        flag that is set only when some lake or river is below 0 °C.
    - Clear blue-black ice shows where the snow is thin (by noise and
        wind exposure).
    - There are no ripples, flow or foam.
- **Rivers** freeze below -5 °C: an ice surface following the channel,
    snow-dusted, with open dark leads over the fastest reaches (speed
    above 2 m/s).
- **Waterfalls** below -8 °C become icefalls: the sheet mesh is shaded as
    static blue-white ice with vertical ribbing. There is no spray, no
    plunge-pool foam, and no sound in `getWaterSounds`.
- Between -2 and 0 °C (and -7 to -5 °C for rivers), freezing is partial:
    ice concentration rises linearly, so margins freeze first.
- All of these are uniform-branch-guarded: nothing is evaluated on maps
    with no water below 0 °C, like the existing sea-ice guard.
- **Tests:**
    - a CPU port of the freeze function (fully frozen at -3 °C, open at
        1 °C, partial at -1 °C);
    - `getWaterSounds` returns null for a waterfall at -10 °C.
- **Verification:** at `meanTemperatureCelsius: -18` on `fjords` seed
    2, first find a lake with a top-down shot, because plan 2b changed
    this terrain. Then capture from 30 m above it. It must show a frozen,
    snow-covered lake with cracks, not open water.

### 5. Real waterfalls

For each recorded waterfall (lip position, lip height, foot height,
width w, discharge Q, flow direction):

- **Sheet mesh:**
    - A curved strip following the projectile path of water leaving the
        lip at speed v: `x(t) = v t`, `y(t) = -g t^2 / 2`.
    - Make it w wide, with 12 vertical segments and 3 across (more for
        wide falls), clamped to the cliff face so it never cuts into the
        rock: sample the height map along the path and push the sheet
        outwards.
- Give waterfall geometry its own vertex buffer and draw it in the water
    pass with a new kind, `WATER_KIND_FALL`.
- **Shading:**
    - Vertical streaks scroll downwards at `sqrt(2 g drop)` in UV space.
        The offset is `time x speed`, where speed is a vertex attribute:
        constant along the sheet, so there's no neighbour shimmer.
    - The sheet breaks up into aerated white towards the bottom, with
        alpha from streak noise. It is thinner and translucent at the
        edges, and lit by the sun with back-lighting through it.
- **Spray:**
    - At the foot, 16 to 64 camera-facing mist sprites, depending on Q and
        drop, drawn in the same water pass with soft depth fade.
    - They rise and drift with the wind, with motion derived from
        uniforms.
    - Stop drawing them beyond 1.5 km.
- **Plunge pool:** in the shader, a ring of churned foam around the foot
    that expands and fades, with radial texture flow outwards.
- **Cascades:** consecutive steps under 3 m become rapids with extra
    foam, not separate falls.

### 6. Wet banks

- Add a distance-to-water field: an r8 texture at terrain resolution.
    - It stores the distance to the nearest river, lake or waterfall edge,
        in metres, encoded 0 to 40 m.
    - Build it on the CPU with a two-pass distance transform after the
        river build.
    - Upload it as a new `@group(1) @binding(13) surface_texture_b`, an
        rgba8unorm texture at terrain resolution:
        - r: distance to water / 40 m;
        - g, b, a: reserved (0), for later plans.
    - Add it to every bind group layout that uses group 1. Document it
        next to the surface texture in `docs/architecture.md`. Sample it
        with `textureSampleLevel`.
- **Terrain shader:** within 0 to 6 m of water, darken albedo by up to
    35 %, lower roughness to 0.35, and blend mud on gentle banks.
- **Grass:**
    - Denser and greener within 12 m of water.
    - **Reeds:** a new tall, thin tuft style (1.4 to 2.2 m) within 3 m of
        slow water (lakes, and rivers under 0.6 m/s), in temperate and
        warm climates.
    - Add it as a `GrassInstance` style flag, drawn with the existing
        grass pipeline and the same wind.

### 7. Sound hooks

- Build a spatial grid (256 m cells) of river segments (with speed and
    width), waterfalls (with height and discharge), lake shores and the
    coastline.
- Add `engine.getWaterSounds(x, y, z)`, returning
    `{ river, waterfall, lakeShore, surf }`. Each is
    `{ distanceMetres, loudness, position: [x, y, z] } | null`, where:
    - `loudness` is 0 to 1, from the source strength over distance
        squared;
    - river strength is `v x w`; waterfall strength is `Q x drop`; surf
        strength is wave height;
    - the search radius is 400 m for rivers, 1500 m for waterfalls and
        600 m for surf.
- Add `engine.getWaterfalls()`, returning
    `{ position, heightMetres, widthMetres, dischargeCubicMetresPerSecond }[]`.
- Each query costs under 0.2 ms: it is CPU-only and reads just the nine
    cells around the position.

### 8. Water mask authoring (engine API)

Add `engine.setWaterMask(mask: WaterMask | null)`:

```ts
export interface WaterMask {
  width: number;
  height: number;
  /**
   * One byte per sample, row-major, north row first:
   * 0 = no water, 1 to 127 = river brush strength, 128 to 255 = lake/pond.
   */
  data: Uint8Array;
}
```

- Validate:
    - `width` and `height` are from 2 to 8192, and `data.length` equals
        `width x height`;
    - a `TypeError` is thrown for a wrong type, and a `VistaWasmError`
        with `INVALID_OPTIONS` for sizes.
- Resample the mask to terrain size: nearest for classes, bilinear for
    strength. When the sizes differ, emit a warning through the existing
    warnings mechanism.
- **Lakes:** each connected lake region is flattened below its
    surroundings.
    - The bed is `rim - max(1.5 m, 0.05 x sqrt(area))`, smoothed.
    - The surface is at the lowest rim height.
    - The outlet runs from the lowest rim point into the natural drainage
        (it joins the network in step 2).
- **Rivers:** skeletonise the painted strokes (Zhang-Suen thinning) into
    centrelines.
    - Orient each one downhill: follow the terrain from its higher end.
        If both ends are equal, flow towards the nearest sea or lake.
    - Width comes from stroke strength (1 → 1 m, 127 → 60 m) or from
        discharge, whichever is larger.
    - Carve with the same channel-form code, and join the natural
        network.
- Painted water always wins over generated water.
- `null` removes the mask and restores the terrain exactly.
- The mask survives changes to `setWater` options, and is cleared when
    new terrain loads.

## Public API

Additive only:

```ts
export interface RiverOptions {
  // ...existing...
  /** Streams start at glacier snouts and snow fields. 0 to 2, default 1. */
  snowmelt?: number;
  /** Small springs at the foot of slopes. Default true. */
  springs?: boolean;
  /** Lowland meander strength, 0 (straight) to 1. Default 0.6. */
  meanders?: number;
  /** Waterfalls where rivers cross cliffs. Default true. */
  waterfalls?: boolean;
}

export interface WaterSound {
  distanceMetres: number;
  loudness: number;
  position: [number, number, number];
}

export interface WaterSounds {
  river: WaterSound | null;
  waterfall: WaterSound | null;
  lakeShore: WaterSound | null;
  surf: WaterSound | null;
}

export interface Waterfall {
  position: [number, number, number];
  heightMetres: number;
  widthMetres: number;
  dischargeCubicMetresPerSecond: number;
}

export interface VistaEngine {
  // ...existing...
  setWaterMask(mask: WaterMask | null): void;
  getWaterSounds(x: number, y: number, z: number): WaterSounds;
  getWaterfalls(): Waterfall[];
}
```

Mirror these in Rust (`vista_types`), with serde camelCase and defaults.
Validate ranges in `config.rs`.

## Steps

1. Capture the reference shots and a river close-up on the default map.
    To find a river, take a top-down shot
    (`{"camera":{"position":[0,9000,1],"target":[0,0,0]}}`) and read
    world coordinates off it: the map is centred on (0, 0), and
    `size x horizontalScaleMetres` metres wide. Then capture
    `{"shots":[{"position":[X,0,Z],"target":[TX,0,TZ],"height":30}]}`.
    Save that camera in the plan's working notes, for the after shot.
2. Add discharge, springs and snowmelt sources, with tests.
3. Add lakes (fill, overflow, endorheic), with tests.
4. Add `condition_channels` (geometry, profile, cross-section, meanders,
    deltas, waterfall steps, plunge pools), with restore tests.
5. Update ribbons, speed and the water-shader changes.
6. Add waterfalls: data, mesh, shader, spray and pool.
7. Add the wet-bank field, terrain shading, grass and reeds.
8. Add sound hooks and `getWaterfalls`.
9. Add `setWaterMask`, with tests.
10. Demo:
    - in the water section: sliders for snowmelt and meanders,
        checkboxes for springs and waterfalls, a "Jump to a waterfall"
        button (camera to the nearest one from `getWaterfalls`), and a
        "Water sounds" readout line in the stats panel (the nearest
        source and its loudness);
    - no audio playback in the core demo.
11. Docs:
    - `docs/water.md`: sources, channel form, waterfalls, wet banks, the
        water mask with an example, and sound hooks with a Web Audio
        example that sets a gain from `loudness`;
    - `docs/options-reference.md`;
    - `docs/architecture.md`: river build stages, draw order, waterfall
        pass, and the surface-texture channel layout;
    - `CHANGELOG.md`.
12. Verify, then commit and push.

## Tests

Rust:

- **Termination:** every channel cell reaches the sea, a lake or the map
    edge, except in endorheic basins.
- **Monotonic beds:** along every path, the bed height never rises
    downstream.
- **Width grows downstream:** at each confluence, the downstream width is
    at least the widest input.
- **Lakes:** a synthetic bowl fills to its spill height, and its outlet
    starts at the lowest rim.
- **Springs:** deterministic per seed, and at most one per 600 m.
- **Snowmelt:** a glacier snout on a synthetic slope creates a source.
- **Meanders:** sinuosity (path length over straight length) is at least
    1.3 on a synthetic flat plain with `meanders: 1`, and at most 1.05
    with `0`.
- **Waterfalls:** a synthetic 20 m cliff step yields one waterfall of
    height 20 ± 2 m, and a plunge pool of the expected radius.
- **Restore:** toggling rivers, and setting then clearing a water mask,
    both restore heights bit-identically.
- **`setWaterMask`:** rejects a wrong length; a painted line becomes one
    river oriented downhill; a painted blob becomes one lake.
- **`getWaterSounds`:** a position beside a synthetic river gives
    loudness above 0.5, and 2 km away gives null.

TypeScript: type tests for the new members, and runtime validation
tests for `setWaterMask` (wrong type, wrong length).

## Verification

- A river close-up (before and after): banks meet the water with no
    edge, and the flow visibly runs downstream.
- A waterfall close-up: use "Jump to a waterfall", or place the camera
    from `getWaterfalls()[0]` at 3 x its height away. You should see the
    sheet, streaks, spray and plunge-pool foam.
- A lake with an outlet, from above.
- A delta on the `archipelago` or `continental` landform.
- The `alpine` landform at -5 °C: snowmelt streams from glaciers.
- Wet banks and reeds at 5 m height.
- The reference shots: no changes except the rivers.
- The water GPU pass with two waterfalls in view: at most 0.6 ms more
    than before (measure the ratio under software rendering against the
    terrain pass, and scale it).

## Budgets

- WASM growth: a soft target of +20 KB gzipped, and a hard limit of
    +40 KB. Above the soft target, the report must say what the extra
    bytes buy and why it can't be done smaller.
- GPU:
    - water pass +0.6 ms at most with waterfalls in view;
    - terrain +0.1 ms for wet banks.
    - On the fixed-scene gate, only the water and terrain passes may
        grow, and within these amounts, scaled from software ratios
        against the terrain pass. Every other pass stays within 5 %.
- River build: at most 300 ms at 512 x 512.

## Out of scope

Water painting UI (plan 11), file import (plan 10), audio playback, and
dynamic flooding.
