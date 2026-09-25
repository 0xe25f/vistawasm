# Plan 6: Tree realism

## Goal

Trees look highly realistic at 5 to 50 m:

- grown branching with believable trunk taper, branch angles and sky
    gaps;
- foliage made of many leaf clumps with real leaf silhouettes, not blobs;
- every tree different in age, lean and colour;
- layered wind: the trunk sways, branches bob and leaves flutter.

Everything is procedural at start-up. The download stays tiny, and
custom models still work through `setTreeModel()`.

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

- Targets: branching structure, leaf cards and clumps, and variation and
    wind. Foliage lighting (translucency and crown self-shadowing) is not
    part of this plan. Keep the existing lighting as it is.
- Size: procedural. A soft target of +60 KB gzipped, and a hard limit of
    +120 KB. Every byte above the soft target must buy visible realism
    that can't be had smaller: removing the hand-built species should
    offset much of the growth engine.
- Generated at start-up:
    - **Textures** (leaf-cluster atlases, bark, impostors) on the GPU.
    - **Geometry growth** (space colonisation) is inherently sequential,
        so it runs in WASM during start-up, within a strict time budget.
        This is the one part not on the GPU, and the report must say so.
        If the repository owner later requires GPU growth, that is a
        follow-up.
- Custom tree models keep working through `setTreeModel()`.
- Priority is near trees, 5 to 50 m. Beyond that, keep good LOD and
    impostors.

## Prerequisites

- **Plan 5** (vegetation density) must be merged. Check for
    `shaders/tree_generate.wgsl` and the cover texture. Near trees now
    come from streamed tiles, and this plan's LODs must fit its budgets.
- **Plan 4** provides grounding with a per-species `root_radius` in the
    cull shader's species table, and the rules that mark trees `stunted`:
    near the tree line, on exposed ridges, and in `alpineTransition`.
- **Plan 5** carries `stunted` in the cover texture's alpha bit 7, and
    into every generated instance: the far set and the near tiles.
    - Check that the instance data has a stunted bit.
    - If plan 5 stored it only in the cover texture, add it to the
        instance's packed variant and age word in this plan. Both
        generators must set it from the same cover-texture bit, and a test
        must check that they agree.
- If any of plans 4 and 5 is missing, carry it out first (it will
    require plans 1, 2, 2b and 3).

### What earlier plans built that this plan must respect

- **Snow on trees** (plan 2): trees on cold ground get snow on their
    upper surfaces through the existing snow path in `trees.wgsl`, driven
    by permanent snow and weather snow cover. Keep it working on the new
    meshes. Snow settles by the geometric (card or branch) normal facing
    up, not the bent foliage normal: otherwise whole crowns turn white.
    Add a before and after capture at `meanTemperatureCelsius: -3`.
- **Dwarf shrubs** (plans 2 and 4): tundra (`iceArctic` fringe) and
    `alpineTransition` plant `Shrub` at 0.35 to 0.6 scale. Give shrubs a
    real dwarf, prostrate form: low (0.3 to 0.8 m), wide, and dense, with
    small leaves. Use it for those biomes and for young shrubs. The
    envelope is a flattened dome, and the tropism is sideways. Scaling
    down the ordinary shrub doesn't read as tundra.
- **Stunted conifers** (plan 4) in `alpineTransition` and near the tree
    line use the krummholz variant in section 4.
- **Grounding:** the root-flare depth below the origin (section 1) must
    match plan 4's `root_radius` sinking rule, so trunks never show a gap
    on slopes. Test it against plan 4's grounding function.
- **The terrain skirt** (plan 2b) never carries trees. There is nothing
    to do here, but don't add fallback placement outside the footprint.
- The WASM is at least 285,511 bytes gzipped after plan 2b. Measure it
    again at the start, after plan 5.

## Current state

- `crates/vista_wasm/src/render/tree_models.rs` (about 1,560 lines):
    - hand-built species (`build_oak`, `build_pine`, `build_spruce`,
        `build_palm`, `build_jungle`, `build_cypress`, `build_acacia`,
        `build_shrub`) using `Builder`, `grow_path` and `taper`;
    - one mesh per species (`SPECIES_COUNT = 8`), merged into
        `TreeLibrary`;
    - `TreeVertex` is 48 bytes: position, normal, uv, and
        `params (layer, sway weight, AO, phase)`.
    - `mesh_from_arrays` builds custom meshes for `setTreeModel`.
- `shaders/trees.wgsl`:
    - near meshes drawn instanced;
    - wind is one `wind_offset` function (a sway weight and phase per
        vertex);
    - impostors are camera-facing cards from a baked impostor texture
        array (`gpu.rs`: `impostor_texture`, `tree_impostor` pipeline).
- Leaf and bark textures are generated in `shaders/texture_gen.wgsl`
    into the flora texture array.

## Design

### 1. Growth: space colonisation per species

- Replace the hand-built species with one growth engine in a new module,
    `render/tree_growth.rs`, following Runions, Lane and Prusinkiewicz
    (2007).
- **Crown envelope:** attraction points are sampled in the species'
    crown envelope, at a density set per species:

    | Species | Envelope | Notes |
    | --- | --- | --- |
    | Oak | Broad, flattened ellipsoid; the trunk forks low | Heavy, crooked limbs, wide sky gaps |
    | Pine | Tall trunk; the crown sits in the top 35 %, irregular and flat-topped | Self-pruned lower trunk |
    | Spruce | Narrow cone; branches in whorls every 0.5 to 0.8 m | Droop increases lower down |
    | Palm | No colonisation: a curved, ringed trunk and 12 to 18 fronds | Improve the fronds (below) |
    | Jungle emergent | A tall, straight trunk; an umbrella crown | Buttress roots at the base |
    | Cypress | A columnar crown | Knee roots around the base when on wet ground |
    | Acacia | Flat-topped, wide umbrella | Branches fork from a short trunk at shallow angles |
    | Shrub | Multi-stem hemisphere | Dense, low |

- **Growth loop:**
    1. Each attraction point pulls its nearest node within the influence
        radius.
    2. Nodes grow a step `D` in the mean direction, plus the tropism
        vector (up for most species, outwards and down for spruce
        droop).
    3. Points within the kill distance are removed.
    4. Stop when no nodes grow or when the node budget is reached.
- **Radii:** use the pipe model. `r_parent^2.5 = sum(r_child^2.5)`,
    with tip radius per species.
- **Trunk:**
    - Taper with the pipe-model radii.
    - Add a root flare below 0.6 m, where the radius grows by up to 1.6x,
        matching plan 4's root radius. The mesh extends 0.4 m below the
        origin, so grounded trees never show a gap.
- **Branch mesh:**
    - Generalised cylinders along the node paths, with 6 to 10 sides by
        radius.
    - Parallel-transport frames, so there's no twisting.
    - Smooth joints: extend the child a little into the parent.
    - Bark UVs are continuous along each branch, with u around and v
        along, scaled by circumference, so the texture density is
        uniform.
- **Variants:** generate 4 variants per species with different seeds,
    and 3 age classes per variant (young, mature and old: the envelope
    and node budget scale), for 12 meshes per species.
    - Budget the vertices so the whole library at LOD0 has at most
        900,000 vertices.
    - Growth must take at most 250 ms in total in WASM at start-up.
        Measure it in the browser, log it in the start-up phases, and
        grow the variants lazily (one per frame after the first frame)
        if needed.

### 2. Leaves: clumps with real silhouettes

- **Leaf atlas:** generated on the GPU in `texture_gen.wgsl`. Each layer
    holds clusters of 5 to 30 leaves drawn with signed-distance leaf
    shapes per species:
    - oak: lobed;
    - broadleaf: ovate with a serrated edge;
    - pine: needle bundles;
    - spruce: flat needle sprays;
    - palm: pinnate leaflets;
    - jungle: large glossy ovate;
    - cypress: scale-leaf sprays;
    - acacia: tiny bipinnate leaflets.
    - Channels: albedo with alpha, and normals with a midrib bump. Colour
        varies per leaf within a cluster.
- **Placement:** clump cards (quads, or two crossed quads for needles)
    are placed at twig tips from the growth graph (nodes of the last 2
    orders), 1 to 3 per tip, oriented roughly outwards and randomly
    rolled. Card size is per species (0.4 to 1.2 m).
- **Bent normals:** the normal is blended 65 % towards the direction
    from the crown centre (ellipsoid-scaled) and 35 % towards the card
    normal. The crown then shades as a volume, but the clumps still read.
    Do not change the lighting model itself.
- **Mips:** alpha is kept for distance. Preserve alpha-test coverage in
    each mip by rescaling alpha, so the alpha-tested fraction matches mip
    0 at the 0.5 threshold. This prevents distant trees thinning out and
    looking like skeletons.

### 3. Levels of detail

- **LOD0 (under 50 m):** full mesh.
- **LOD1 (50 to 150 m):** branches of order 3 and above are removed,
    their leaf clumps are merged into fewer, larger cards (half the
    count, 1.4x size), and trunk sides are halved.
- **Impostors (beyond 150 m, or `meshDistanceMetres`):**
    - Bake 8 views around, plus one from 40 degrees above, per variant,
        with albedo, alpha and normal, on the GPU at start-up.
    - The shader picks and blends the two nearest views. The existing
        impostor array grows by variants x views. Keep it within 64 MB
        of GPU memory, or reduce resolution.
- **Crossfades:**
    - Fade between LODs with a hash-based dithered alpha over a 10 %
        distance band, so there's no popping.
    - Plan 5's thinning and canopy crossfade stay as they are.
- `tree_cull.wgsl` picks LOD0, LOD1 or impostor per instance, and writes
    three indirect draws per species variant group. Update the
    indirect-argument layout and its docs.

### 4. Variation

- **Per instance:**
    - variant and age class from the instance hash;
    - scale jitter (existing);
    - lean up to 6 degrees, biased downslope and with the prevailing
        wind. Roots stay grounded, because the lean pivots at the
        origin;
    - hue ±4 %, saturation ±10 % and brightness ±8 % from the tint
        (existing), plus per-clump variation baked as vertex colour;
    - `dryness` (existing).
- **Stunted** (plan 4's flag): use the young age class, flagged by the
    wind: branches on the windward side are removed and the crown is
    offset leeward. This gives krummholz near the tree line and on
    exposed ridges. Implement it as a growth parameter (attraction points
    culled on the windward half), so it is a real variant, not a
    transform.

### 5. Layered wind

Extend `TreeVertex` to 56 bytes, or keep 48 by packing. The new data:

- the branch pivot (the position where the vertex's branch leaves its
    parent);
- branch level (0 trunk, 1 limb, 2 and above twig or leaf);
- stiffness;
- phase.

In the vertex shader, the displacement adds up three layers:

1. **Trunk sway:** bend around the base with amplitude
    `wind^2 x height^2 x k_species`, at 0.2 to 0.5 Hz, with gusts from
    the weather wind (`frame` wind uniforms), phase from instance
    position.
2. **Branch bob:** rotate around the branch pivot, perpendicular to the
    branch, with amplitude falling off by stiffness, at 0.8 to 1.5 Hz,
    with per-branch phase.
3. **Leaf flutter:** small rotation of each card around its attachment
    point, at 3 to 6 Hz, scaled by wind and species (aspen-like broadleaf
    most, needles least).

- All phases come from vertex attributes and instance data, never from
    per-pixel values.
- Displacement is 0 at the ground, and the total is bounded to 8 % of
    tree height.
- Impostors get trunk sway only, as a whole-card shear (existing
    behaviour, retuned to match LOD0 amplitude).
- Shadow casters use the same trunk sway, so shadows move with trees.
- **Custom meshes** (`setTreeModel`): `mesh_from_arrays` derives the wind
    data automatically. Pivot at the origin; level from height
    (trunk below 30 % of height); flutter on vertices whose texture
    layer is a foliage layer. Document this in `docs/hooks.md`. The
    `setTreeModel` API is unchanged.

## Public API

No breaking changes. Additive:

```ts
export interface FloraOptions {
  // ...existing...
  /** Distinct grown shapes per species, 1 to 4. Defaults to 4. Lower uses less GPU memory. */
  variantsPerSpecies?: number;
}
```

Validate it in [1, 4]. `setTreeModel` replaces every variant of that
species with the custom mesh, as it replaces the single mesh today.

## Steps

1. Capture before shots: each species at 8, 20 and 50 m. Use the harness
    with `set: { setTreeInstances: [...] }` to plant one tree of each
    species on flat ground in a row, with the camera pointed at each.
    Also capture a forest at 30 m, and the reference shots. Record
    per-pass GPU times.
2. Add `render/tree_growth.rs`: colonisation, pipe radii, branch meshing,
    root flare and envelopes, with tests.
3. Add the species parameter table and all 8 species, with the palm and
    jungle buttress special cases, and variants and age classes.
4. Add the leaf atlas generation and clump cards, bent normals, and
    coverage-preserving mips.
5. Add LOD1 generation, the impostor bake with 9 views, and cull-pass
    LOD selection, with crossfades.
6. Add variation, lean and stunted krummholz.
7. Add the layered wind in the tree, impostor and shadow shaders, and the
    `TreeVertex` layout.
8. Remove the old hand-built builders. Keep `mesh_from_arrays`, and keep
    `build_species_mesh` as the entry point, now returning grown meshes.
9. Demo:
    - a "Tree variants" slider from 1 to 4;
    - a "Tree showcase" button that plants one of each species in front
        of the camera on flat ground (using `setTreeInstances` with
        `ground: true`) and frames it. This helps people judge the
        result.
10. Docs:
    - `docs/vegetation.md` (growth, LODs, variants, wind);
    - `docs/hooks.md` (custom-mesh wind derivation);
    - `docs/architecture.md` (vertex layout, indirect-argument layout,
        impostor bake);
    - `CHANGELOG.md`.
11. Verify, then commit and push.

## Tests

- **Determinism:** the same species, variant and seed give identical
    vertex buffers.
- **Pipe model:** for every branching node,
    `|r_parent^2.5 - sum(r_child^2.5)|` is below 1 %.
- **Budget:** the library's total LOD0 vertices are at most 900,000.
    LOD1 has at most 45 % of LOD0's triangles per variant.
- **Envelope:** at least 90 % of leaf cards lie inside their species
    envelope scaled by 1.1.
- **Root flare:** the mesh extends at least 0.3 m below the origin, and
    the base radius is at least 1.4x the radius at 1 m.
- **Wind:** a CPU port of the displacement function gives 0 at y = 0,
    and is bounded by 8 % of height at the maximum wind.
- **Custom mesh:** `mesh_from_arrays` output has valid wind data (pivots
    finite, levels in range).
- **Growth time:** measured in the browser (the report states it). Also
    a native benchmark test (ignored by default) that prints time per
    species.

## Verification

- After shots for every species at 8, 20 and 50 m, compared with the
    before shots. Visible branching and sky gaps, leaf silhouettes at
    8 m, no blob crowns, no skeleton trees at 50 m.
- The forest at 30 m: variation between neighbouring trees.
- Wind: capture two frames 0.5 s apart at a wind of 12 m/s. Trunks,
    branches and leaves move by different amounts, and the ground
    contact does not move.
- The reference shots render with no errors.
- GPU:
    - the trees pass at the default density is at most +0.8 ms compared
        with before (scaled estimate);
    - at plan 5's density 4 in the jungle, the trees pass is still within
        plan 5's 5 ms budget;
    - impostor memory is at most 64 MB.
- Size: WASM gzipped growth within the soft target of +60 KB, and never
    above the hard limit of +120 KB. Removing the old builders
    should offset part of it.

## Budgets

- WASM growth: a soft target of +60 KB gzipped, and a hard limit of
    +120 KB. Above the soft target, the report must say what the extra
    bytes buy and why it can't be done smaller.
- **Performance gate** (`scripts/visual-check/fixed-scene.mjs`): only
    the trees, shadows and tree culling passes may grow, and within the
    per-frame budgets above. Every other pass stays within 5 %.
- Start-up: growth at most 250 ms in WASM; texture and impostor bakes at
    most 300 ms on the GPU.
- GPU per frame: as above.

## Out of scope

Foliage translucency and crown self-shadowing (not chosen), seasons, and
falling leaves.
