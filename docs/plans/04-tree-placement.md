# Plan 4: Tree placement

## Goal

Every tree stands on the ground you see, at every distance, with its
roots sunk into the slope. Trees grow only where the species would grow:

- never in water, on cliffs, on beaches (except beach palms), on
    glaciers, on lava or on bare rock;
- in patterns that follow the land: denser in sheltered, moist valleys
    and on shaded slopes, thinner and stunted on exposed ridges and near
    the tree line, in groves rather than an even scatter.

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

- Symptoms to fix: floating or buried trees, and trees in the wrong
    places.
- Placement follows ecological rules per species:
    - altitude, slope and temperature;
    - sun exposure (denser on shaded slopes);
    - soil moisture from drainage and rivers, and distance to water;
    - wind exposure;
    - clustering into groves, forest edges and seedlings near parents.
- Grounding: sink the trunk base to the lowest ground under the root
    radius. Trees stay upright, and density drops to zero above each
    species' maximum slope.

## Prerequisites

- **Plan 1** (drainage area in `HeightMap.aux`, `terrain/drainage.rs`).
- **Plan 2** (the surface texture and `SurfaceSample::celsius()`,
    permanent snow, `IceArctic`).
- **Plan 2b** (map edges with a skirt, and the performance gate).
- **Plan 3** (the distance-to-water field in `surface_texture_b.r` at
    `group(1) binding(13)`, and the carved rivers).
- **Plan 3b** (rivers at landscape scale). Check for
    `RiverOptions.riparian`, `MAT_GRAVEL` in `terrain/biomes.rs`, and
    `ensure_pipelines` and `warm_up` in `render/gpu.rs`.
- Check that each exists. If one is missing, carry out that plan first;
    each is self-contained.

### What plans 2b and 3 changed that this plan must match

The terrain mesh was rebuilt in plan 2b, so section 1 must mirror today's
`build_terrain_mesh_centred` in `render/terrain_mesh.rs` exactly:

- **Bands:** vertex grid offsets come from
    `band_sample_offset(grid_distance)`, with `LOD_BAND_WIDTH = 24`. The
    step doubles every 24 grid steps out from the centre.
- **Centre:** the centre sample is `centre.round()`. It is not in any
    uniform yet; this plan adds it.
- **Clamping at the skirt:** offsets are clamped to
    `[-reach, size - 1 + reach]`, where
    `reach = ceil(SKIRT_MESH_METRES / metres_per_sample)`.
- **Inside the footprint,** a vertex takes the exact sample height,
    `map.heights[index]`, with no filtering. Fetch it with `textureLoad`
    from `height_texture`.
- **Outside the footprint,** a vertex keeps its true position, and its
    height is `skirt_ground(map, x, z)`: the clamped edge height, bilinear,
    fed through `skirt_height` with `skirt_noise`. WGSL has matching
    `skirt_height` and `skirt_noise`. `mesh_surface_height` must return the
    same values there, so grass tufts at the very edge of the map are
    grounded too.
- **Triangles:** read the triangle split from the centred mesh's index
    buffer. The index builder in `terrain_mesh.rs` splits each quad into
    `(top_left, bottom_left, top_right)` and
    `(top_right, bottom_left, bottom_right)`, so the shared diagonal runs
    from bottom-left to top-right. Confirm that the centred mesh uses the
    same builder, and mirror whatever it does.
- **Placement:** trees and grass are placed only inside the footprint.
    None on the skirt, and nothing beyond the coast that `edges: "coast"`
    produces.
- **Distance to water** comes from plan 3's `surface_texture_b.r`
    (`@group(1) @binding(13)`, distance / 40 m). Its CPU copy is
    `RiverNetwork::wet` (`WetBanks`); use that for placement rules.
    `surface_texture_b.g` is snow and ice cover, and b and a are
    reserved.
- **Frozen water** (plan 3, section 4b) is still water: the water
    exclusion applies to frozen lakes and rivers too.

### What plan 3b built that this plan must match

Verified on `realism` after plan 3b. The WASM is 348,926 bytes gzipped,
and 248 Rust and 44 TypeScript tests pass.

- **A reference for the mesh height already exists.**
    `render/water.rs` `mesh_height_at` gives the full-detail band's
    triangle interpolation, splitting each sample square from top-right to
    bottom-left, and rivers use it to sit on the drawn ground.
    - Move it into `render/terrain_mesh.rs`, and generalise it into
        `mesh_surface_height` for every band and the skirt (section 1).
    - Make `water.rs` call the shared function, so there is exactly one
        copy of the rule in Rust and one in WGSL.
    - The existing river tests must still pass unchanged.
- **Tree placement already jitters, but grounds trees wrongly.**
    `render/flora.rs` `build_tree_instances` now:
    - walks a stride grid, with up to 2 candidates per cell (`per_cell`)
        on fine grids;
    - jitters each candidate's position by up to half a cell;
    - moves trees next to a channel back to the sample centre (the
        `beside` rule).
    But the height stays that of the rounded sample. On a 30-degree
    slope with 12 m cells, trees therefore float or sink by up to about
    3.5 m, even near the camera. This is the first bug to fix (section 1
    fixes it on the GPU, and section 3 on the CPU).
- **Thinning over the instance cap makes stripes.** When candidates
    exceed `max_instances`, `build_tree_instances` keeps every Nth
    candidate in row order (`step_by`), which thins whole rows. Replace it
    with thinning by hashed rank: keep the candidates whose hash is below
    `max_instances / count`. It is deterministic and spatially even.
    Add a test that the kept set shows no row striping (the row-to-row
    variance of the kept count is within Poisson noise).
- **Channels narrower than a sample aren't in the channel mask.**
    - Plan 3b draws small streams at their true size, including tight
        loops, as reaches (`RiverNetwork::reaches`, `terrain/channels.rs`
        `Reach` with `points`) and bank strips.
    - The mask and the `beside` test miss these streams, so trees can
        stand in them.
    - `water_sounds.rs` already builds a spatial grid of river segments.
        Reuse it, or share its builder, as a channel-distance query: the
        distance to the nearest drawn centreline, minus that point's half
        width.
    - Exclude trees within `half width + 1.5 m` of every drawn channel,
        brooks included, and within `half width + 1 m` of plunge pools.
        Remove the `beside` snap: with the exact grounding and exclusion,
        trees stand where they fall.
- **The riparian field** (`RiverNetwork::riparian`, 0 to 255 per
    sample, from `riparian_field`):
    - Classification already adds `0.45 x value` to moisture.
    - 3b raises tree density by up to `1.5 x value` in savannah, grassy
        meadows and mesa-desert ground.
    - Trees must never grow on gravel or sand bars, or below the bank
        top.
    Keep all three behaviours. Section 4's water term must use the
    riparian value, and must not add its own distance-based moisture on
    top, or moisture near water would be counted twice.
- **Bed materials** (`RiverNetwork::bed`, and `MAT_GRAVEL = 10`): no
    trees where gravel weight is above 0.4, or on bar samples from `bed`.
- **Pipelines** are created on demand through `Pipelines`, `Needs`,
    `ensure_pipelines` and a one-per-frame `warm_up`. This plan changes the
    tree cull and grass shaders but adds no pipelines. If a new one proves
    necessary, it goes through `Needs`, and the default capture's
    `first frame ms` must not rise by more than 5 %.
- **Screen-space reflections** (`reflections: "screen"`) reflect a copy
    of the scene, so grounded trees are reflected grounded. Check it in
    verification: a riverside tree's reflection meets its trunk.
- **Clippy:** count the warnings at the start, and add none.


## Why trees float, sink and grow in the wrong places

Confirm each cause with a test or capture before fixing it.

1. **Distance mismatch.**
    - `render/flora.rs` `candidate_at` puts a tree exactly on a height-map
        sample, at that sample's height.
    - The terrain mesh (`render/terrain_mesh.rs`) samples heights at
        vertices whose spacing doubles in each LOD band away from the
        camera. It interpolates linearly across each triangle between
        them.
    - Far from the camera, the rendered ground between coarse vertices
        can be metres above or below the true sample, so trees float or
        sink.
    - The mesh is also double-buffered and recentred as the camera
        moves, so the error changes as you fly.
2. **Slopes.** The tree's origin is the trunk centre. On a slope, the
    downhill side of the trunk base floats.
3. **Grid.** Trees sit exactly on sample points, one per cell at most.
    That gives a visible grid and makes jittered positions impossible
    without the right height lookup.
4. **Rules.**
    - `MAX_PLANTING_SLOPE = 0.75` (about 37 degrees) is measured over a
        wide 4-neighbour stencil, so it misses cliff bands.
    - Rock, sand, glacier and lava are only excluded indirectly, by
        biome.
    - There is no aspect, exposure, soil or water-distance rule.
5. **Order.** Check in `engine.rs` that tree and grass placement always
    runs after every height change: erosion, glacier smoothing, river
    carving, the water mask and imports. Fix any path that places before
    a later carve.

## Design

### 0. Carried over from plan 3b

Plan 3b's report left three things open. They sit where trees meet water,
so they are fixed here, first, with before and after measurements.

1. **The mesa river build is over budget.**
    - The in-browser `"rivers"` phase on `mesaDesert` with open edges and
        a 123 m³/s inflow takes 390 to 413 ms, against the 300 ms limit.
        The base plan 3 build already took 288 to 298 ms there.
    - Profiling found the extra 40 ms (native) in the small-stream loops.
    - Bring it to 300 ms or less, without removing or flattening any
        loop. Profile first. Likely levers:
        - build loop geometry once per reach, not per segment;
        - reuse the channel spatial grid instead of rescanning;
        - avoid per-point allocation;
        - skip loop work on reaches whose samples the loops can't
            change.
    - Output must stay bit-identical: add a test that hashes the river
        network (heights, reaches, vertices) for a fixed mesa seed, before
        and after.
    - The continental case (250 to 278 ms today) must not get slower.
2. **Bank-strip vertex counts can explode on coarse maps.**
    - A flat coastal plain reached about 1.5 million bank-strip vertices,
        after a device loss that 3b worked around by capping loop points
        per map.
    - That many vertices risks frame drops on phones and integrated GPUs,
        which breaks the solid 60 FPS rule.
    - Give bank strips a hard budget of 300,000 vertices per map:
        - Strips are built once, not per view, so simplify them when
            they are built: a curvature-aware decimation merges consecutive
            segments where the heading changes by less than 4 degrees.
        - If still over budget, lower strip resolution proportionally on
            the least visible streams first: the narrowest, slowest, and
            those furthest from the map centre.
    - Test that the synthetic coastal plain stays within 300,000, and that
        the continental reference map loses no visible strip (the pixel
        difference in the reference shot is within 0.05 %).
3. **Ground by water is too dark.**
    - 3b's report says the extra bankside moisture darkens lakeside
        ground, and that plan 3's brown mud band along small streams
        remains on 30 m maps.
    - Real riverbanks are greener and fresher, not darker; mud shows
        only as a thin wet margin at the waterline.
    - Fix it by:
        - capping the wet-bank darkening to the first 2 m from the water
            edge (it currently reaches further on coarse maps, because one
            sample spans 30 m);
        - scaling the mud band to the stream's true width from
            `RiverNetwork::reaches`, not the sample size;
        - letting riparian moisture raise greenness and saturation, not
            lower albedo.
    - Verify with before and after close-ups of a lake shore and a 30 m
        map's small stream from 40 m. The ground by water must read as
        fresh green turf with a thin darker margin.

### 1. Ground at the rendered surface (GPU)

- Add `mesh_surface_height(xz)` to `common.wgsl`. It returns the height
    of the terrain mesh actually being drawn at a world position:
    1. Using the drawn (front) mesh's centre and band layout, work out
        which LOD band contains xz, and the band's sample step.
    2. Snap to that step's grid cell, fetch its corner vertex heights from
        the height texture with `textureLoad` (exact texels, as the mesh
        builder does), pick the triangle using the same diagonal the mesh
        uses, and interpolate barycentrically.
- Add the front mesh's centre (in samples) and its band parameters to
    `FrameUniforms`, appended as one `vec4<f32>`. Update them when the
    streamed mesh swaps front and back buffers.
- Implement the same function in Rust (`render/terrain_mesh.rs`,
    `mesh_surface_height`). Test it against the triangles the mesh
    builder actually produces: at 10,000 random points in every band it
    must agree within 1 cm. The WGSL copy must follow the Rust one line
    for line; comment each to the other.
- **Tree cull pass** (`shaders/tree_cull.wgsl`): for every surviving
    instance, compute the grounded base height (below) and write it into
    the output instance's position. Trees, impostors and shadow casters
    all read the output, so each tree is grounded once per frame.
- **Grass** (`shaders/grass_instances.wgsl`): ground each tuft at
    `mesh_surface_height` in the vertex shader. There's one lookup per
    tuft, since all its vertices share the instance position.

### 2. Root grounding

- Root radius r = trunk radius x 2.5, per species. Add `root_radius` to
    the species bounds table read by the cull shader, alongside the
    existing height and radius bounds.
- Base height = the minimum of `mesh_surface_height` at the trunk centre
    and at four points at distance r (along ±x and ±z), minus
    `0.05 x r`. The downhill side then touches the ground, and the uphill
    side is buried slightly, as real root flares are.
- Trees stay vertical. Never tilt them to the slope normal.
- **Hand-placed trees:** `setTreeInstances` keeps the given `y` as today.
    Add an optional `ground?: boolean` field to `TreePlacement` (default
    false). When true, the tree is grounded by the same rule. Pack it into
    the existing packed instance array; widen the stride if needed, and
    update the TypeScript packer and the Rust unpacker together with a
    round-trip test.

### 3. Jittered, sub-cell candidates

- Keep today's stride grid, `per_cell` (up to 2 candidates per cell on
    fine grids) and jitter of up to ±0.45 of a cell. They are plan 3's,
    and the tree count must not change noticeably.
- **Fix the height.** The CPU height of a candidate is
    `mesh_surface_height` at its jittered position for the full-detail
    band: the triangle interpolation `mesh_height_at` does today. It is
    not the rounded sample's height.
    - Rules (slope, substrate, suitability) use the sample under the
        jittered position, rounded as today.
    - Culling bounds use the corrected height. The GPU then grounds the
        tree exactly for whatever band is drawn (section 1).
- **Remove the `beside` snap.** It moved trees next to a channel back to
    the sample centre. The channel exclusion (see "What plan 3b built"),
    applied at the jittered position, replaces it.
- Replace `step_by` thinning with hash-rank thinning (see "What plan 3b
    built").
- Plan 5 replaces this loop with a world lattice. Keep the candidate
    count per cell a parameter.

### 4. Ecological rules

- Add a species table to `render/flora.rs`, `SpeciesNiche`, with one
    entry per `TreeSpecies`:
    - `celsius: (min, optimum, max)`: annual mean from plan 2;
    - `moisture: (min, optimum, max)`;
    - `max_slope_degrees`: oak 32, pine 38, spruce 40, palm 20,
        jungle 30, cypress 12, acacia 25, shrub 45;
    - `water_affinity` from -1 to 1: cypress 1 (wet ground), acacia -0.6;
    - `shade_tolerance` from 0 to 1;
    - `exposure_tolerance` from 0 to 1;
    - `beach_ok`: palm only;
    - `cluster_radius_metres`.
- Suitability at a candidate is a product of factors, each from 0 to 1:
    - **Temperature:** a triangular response between min, optimum and max.
    - **Moisture:** effective moisture
        `= surface moisture + 0.25 x saturate(ln(drainage area) / 12) + aspect term + water term`.
        - Surface moisture already includes plan 3b's `0.45 x riparian`
            near rivers and lakes.
        - Drainage area comes from plan 3's full-resolution accumulation
            (`terrain/hydrology.rs`). `HeightMap.aux` is coarse and is
            missing for imported maps.
        - Aspect term: slopes facing away from the sun's mean position
            (poleward in the current `SunOptions` hemisphere, or north by
            default) keep up to 0.12 more moisture, in proportion to the
            slope.
        - Water term: `0.35 x water_affinity x riparian`. It is
            positive for water-loving species (cypress), and negative for
            dry-ground species (acacia). It holds only the species
            preference. Do not add a separate distance-to-water moisture:
            the riparian value already carries it, and it must not be
            counted twice.
        - Keep plan 3b's riparian density boost of up to `1.5 x riparian`
            in savannah, grassy meadows and mesa-desert ground, as a
            multiplier on suitability.
    - **Slope:** 1 up to 60 % of the maximum slope, falling to 0 at the
        maximum. Use the fine per-sample normal, not the 4-neighbour
        stencil.
    - **Exposure:** topographic exposure is the height above the mean of
        a 300 m neighbourhood, computed once with a box filter on the
        coarse grid.
        - Exposed ridges reduce suitability by
            `(1 - exposure_tolerance) x saturate(exposure / 60 m)`.
        - On exposed ridges and within 150 m below the tree line, trees
            shrink (scale x 0.5 to 1) and flag `stunted` for plan 6's
            krummholz shaping (an unused bit here).
    - **Substrate** (hard exclusions):
        - water, and within `half width + 1.5 m` of any drawn channel
            centreline, including streams narrower than a sample and
            their loops (the channel-distance query in "What plan 3b
            built"), and within `half width + 1 m` of plunge pools;
        - gravel weight above 0.4, any bar sample in
            `RiverNetwork::bed`, and anything below the bank top;
        - sand weight above 0.5, unless the species is `beach_ok`;
        - rock weight above 0.6;
        - permanent snow above 0.5;
        - the `lowerSnowyPeaks` (16) and `upperSnowyPeaks` (17) biomes;
        - snow material weight above 0.5;
        - glacier;
        - the skirt outside the terrain footprint (plan 2b): nothing
            grows off the map.
        - volcanic heat above 0.2;
        - the plunge pools, deltas and lake beds from plan 3.
    - **Tree line:** 1 at 200 m below `treeLineMetres`, falling to 0 at
        the tree line, with the plan 2 temperature rule as a second
        limit: a mean below -1 °C means no trees.
    - **`alpineTransition`** (the band below the snow line added after
        plan 2): only shrubs (dwarf, scale 0.35 to 0.6) and stunted
        conifers (pine and spruce, flagged `stunted`), at 15 % of the
        normal density. They thin to none at the band's upper edge. The
        tree line and this band must agree: whichever is lower wins, so no
        full-size tree ever stands in the transition band.
- Species choice: among species with suitability above 0.05 in this
    biome, weighted by suitability. `FloraRule` overrides still apply
    first, and still respect the hard exclusions.
- **Clustering:**
    - Pass 1 places parent trees by jittered candidates with probability
        `density x suitability x grove(x)`, where `grove` is a
        low-frequency noise (wavelength 180 m) remapped to 0.35 to 1.
    - Pass 2 adds up to 3 seedlings or young trees per parent, within
        `cluster_radius` (3 to 15 m), with scale 0.45 to 0.8 and the same
        species, each subject to the same exclusions.
    - Forest edges: where suitability falls below 0.3, trees are
        smaller, and shrubs become likelier.
- Determinism: everything hashes from the terrain seed and
    `seedOffset`, as now.

## Public API

Additive only:

```ts
export interface TreePlacement {
  // ...existing...
  /** Snap this tree to the rendered ground, sinking its roots on slopes. Defaults to false. */
  ground?: boolean;
}
```

No other API changes. The existing `FloraOptions` fields keep their
meaning.

## Steps

1. Capture the reference shots, plus:
    - a slope close-up at 25 m on a forested hillside;
    - a hillside 2 to 4 km away with trees, taken from 300 m up.
    These are the before images. Measure a floating-tree count on the far
    shot: use a debug build flag that tints trees whose base differs by
    more than 0.3 m from `mesh_surface_height` (via a temporary
    comparison in the cull shader). Remove the flag before committing,
    and keep the number for the report.
2. Section 0: the mesa river build, the bank-strip budget and bank
    shading, each with its test and before and after numbers or shots.
3. Move `mesh_height_at` into `render/terrain_mesh.rs`, and generalise
    it into `mesh_surface_height` (Rust and WGSL, every band and the
    skirt) with the uniform and the parity test. Switch `water.rs` to it.
4. Add cull-pass grounding with root radius, and grass grounding.
5. Fix the jitter height, remove the `beside` snap, add the
    channel-distance exclusion, and replace the thinning.
6. Add the species niche table, the suitability factors, exposure,
    exclusions, clustering, the tree line and stunting.
7. Add `TreePlacement.ground`, with pack/unpack tests.
8. Fix the placement order in `engine.rs`, with a test.
9. Demo: none needed beyond existing controls. Add a "Ground
    hand-placed trees" checkbox to the grove button's options so the
    feature is visible.
10. Docs:
    - `docs/vegetation.md`: how placement works, the niche table, and
        grounding;
    - `docs/hooks.md` (`ground`);
    - `docs/architecture.md` (grounding in the cull pass, and the uniform);
    - `CHANGELOG.md` (Fixed: floating and buried trees; Changed:
        placement).
11. Verify, then commit and push.

## Tests

- **`mesh_surface_height` parity:** within 1 cm of the mesh triangles, in
    every band, including just after a recentre.
- **Grounding:** on a synthetic 30-degree plane, the grounded base equals
    the height at the downhill root point minus 0.05 r.
- **Exclusions:** no tree within a river's half-width + 1.5 m, on
    sand > 0.5 (except palms), on rock > 0.6, on glacier, on permanent
    snow > 0.5, in heat > 0.2, or above the tree line.
- **Slope:** no tree steeper than its species' maximum.
- **Aspect:** on a synthetic cone at moderate moisture, the shaded
    (poleward) half holds at least 20 % more trees than the sunny half.
- **Water:** density within 30 m of a synthetic river is at least 15 %
    higher than 200 m away. The riparian boost holds (up to 1.5 x in
    savannah beside a river).
    - Effective moisture beside the river equals the classification's
        moisture plus only the species' water term: no double counting.
- **Jitter grounding:** on a synthetic 30-degree plane with 12 m cells,
    every generated tree's CPU height is within 0.05 m of the full-detail
    `mesh_surface_height` at its jittered position. Today's code is off by
    metres; record the old error in the test comment.
- **Small streams:** on a synthetic plain with a 2 m wide looping brook
    (narrower than the 12 m samples), no tree lies within
    `half width + 1.5 m` of its centreline, including inside loops.
- **Bars and gravel:** none on gravel weight above 0.4 or on `bed` bars.
- **No stripes:** with an instance cap at half the candidate count, the
    per-row kept counts vary within Poisson noise, with no row runs.
- **Shared height rule:** `water.rs` uses the shared
    `mesh_surface_height`, and plan 3's river tests pass unchanged.
- **Clustering:** the Clark-Evans nearest-neighbour ratio is below 0.9
    (clustered) on a uniform-suitability plain.
- **Order:** after toggling rivers and after `setWaterMask`, every
    generated tree's CPU height is within 0.05 m of the final bilinear
    height.
- **Determinism:** the same seed gives identical instances.

## Verification

- The far hillside shot (before and after): the floating-tree count from
    step 1 drops to 0 in the after build. Check it with the temporary
    flag, then remove the flag.
- The slope close-up: roots meet the ground, and the downhill side does
    not float.
- A beach, a river bank, a cliff band and a glacier edge at 40 m: no
    trees where excluded.
- A top-down forest at 300 m: groves and edges, no grid, no stripes.
- **Riverside trees at 25 m:** trees reach the bank but never stand in
    any channel, including small-stream loops and brooks. They're absent
    from gravel bars. Their screen-space reflections meet their trunks.
- **Plan 3b's verification shots,** 1 (boreal valley), 4 (canyon) and 5
    (meadow): no trees in water, and green banks as intended.
- **The mesa river-build time** in the generation phases: at most 300 ms.
- The reference shots render with no errors. The forest look changes as
    intended.

## Budgets

- WASM growth: a soft target of +6 KB gzipped, and a hard limit of
    +12 KB. Above the soft target, the report must say what the extra
    bytes buy and why it can't be done smaller.
- **Performance gate** (`scripts/visual-check/fixed-scene.mjs`): only
    the tree culling, trees, shadows and grass passes may grow, and within
    the amounts below, scaled from software ratios against the terrain
    pass. Every other pass stays within 5 %.
- GPU: tree culling +0.05 ms (five height lookups per surviving tree);
    grass +0.1 ms at most.
- Placement build time: at most +40 ms at 512 x 512.
- River build: at most 300 ms in the browser on every map, including
    `mesaDesert` with an open edge and an inflow (section 0).
- Bank strips: at most 300,000 vertices on any map.
- **Solid 60 FPS is a hard target.** The whole frame at 1080p on a
    mid-range GPU must stay within 12 ms in the default scene and 14 ms in
    rain. Estimate it from the fixed-scene ratios against the terrain
    pass. No pass on the gate may grow beyond this plan's amounts.

## Out of scope

Higher densities and the canopy far field (plan 5), and tree shapes and
krummholz meshes (plan 6).
