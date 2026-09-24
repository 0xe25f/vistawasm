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
    plan states how much it may add on top of that. Every byte must buy
    real value that can't be done smaller. Generate data procedurally at
    start-up instead of embedding it.
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
- Check that each exists. If one is missing, carry out that plan first;
    each is self-contained.

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

- Each candidate cell tries one tree at a jittered position:
    `(x + j1, z + j2)` with `j` in [-0.45, 0.45] cells, from the existing
    hash.
- The CPU placement height is the bilinear height at that point, used
    for rules and culling bounds only. The GPU grounds it exactly.
- Plan 5 raises the number of candidates per cell. Keep the candidate
    loop written so the count per cell is a parameter (1 in this plan).

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
        `= surface moisture + 0.25 x saturate(ln(drainage area) / 12) + aspect term + water-distance term`.
        - Aspect term: slopes facing away from the sun's mean position
            (poleward in the current `SunOptions` hemisphere, or north by
            default) keep up to 0.12 more moisture, in proportion to the
            slope.
        - Water-distance term: up to +0.2 within 30 m of water, and up to
            +0.35 for species with water affinity 1.
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
        - water, and within `river width / 2 + 1.5 m` of a channel;
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
2. Add `mesh_surface_height` (Rust and WGSL) and the uniform, with the
    parity test.
3. Add cull-pass grounding with root radius, and grass grounding.
4. Add jittered candidates.
5. Add the species niche table, the suitability factors, exposure,
    exclusions, clustering, the tree line and stunting.
6. Add `TreePlacement.ground`, with pack/unpack tests.
7. Fix the placement order in `engine.rs`, with a test.
8. Demo: none needed beyond existing controls. Add a "Ground
    hand-placed trees" checkbox to the grove button's options so the
    feature is visible.
9. Docs:
    - `docs/vegetation.md`: how placement works, the niche table, and
        grounding;
    - `docs/hooks.md` (`ground`);
    - `docs/architecture.md` (grounding in the cull pass, and the uniform);
    - `CHANGELOG.md` (Fixed: floating and buried trees; Changed:
        placement).
10. Verify, then commit and push.

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
    higher than 200 m away, at equal moisture.
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
- A top-down forest at 300 m: groves and edges, no grid.
- The reference shots render with no errors. The forest look changes as
    intended.

## Budgets

- WASM growth: at most +6 KB gzipped.
- GPU: tree culling +0.05 ms (five height lookups per surviving tree);
    grass +0.1 ms at most.
- Placement build time: at most +40 ms at 512 x 512.

## Out of scope

Higher densities and the canopy far field (plan 5), and tree shapes and
krummholz meshes (plan 6).
