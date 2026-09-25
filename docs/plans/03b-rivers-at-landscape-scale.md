# Plan 3b: Rivers at landscape scale

## Goal

Plan 3 made water drain the land correctly. Photographs of real rivers
show what is still missing. This plan closes those gaps and makes the
first load faster.

1. **Big rivers.** A river 40 to 150 m wide, filling a forested or
    canyon valley floor, is the main feature of many real views. Ours are
    0.6 to 6 m wide.
2. **Reflections.** Calm rivers and lakes mirror their banks, trees and
    hills. Ours reflect only the sky.
3. **Gravel, sand and stones.** Pale gravel bars on inner bends, dark wet
    stones at the waterline, and a cobbled bed seen through clear, shallow
    water, with stones breaking the surface.
4. **Green banks.** A strip of green grass and trees along rivers, even
    in deserts and canyons, and trees growing right to the bank.
5. **Rock-walled rapids.** Whitewater in bedrock channels between rocky
    banks.
6. **Small streams that read at their true size.** Tight meander loops
    a few metres wide, lined with reeds, on maps whose samples are 12 to
    30 m apart; no dashed lines in the distance; plunge pools sized to
    their falls.
7. **Faster first load**, and a cheaper river build and river meshes.

## What plan 3 verification found

Measured on `realism` at commit `0be87b2`, after plan 3.

**Scale.** Discharge is physically right, but every generated map is a
small catchment:

| Map | Largest river |
| --- | --- |
| 512 x 12 m (default) | under 0.1 m³/s, 0.6 m wide |
| Continental 512 x 30 m | 0.33 m³/s, 1.2 m wide |
| Continental 1024 x 30 m | 1 to 1.8 m³/s, 1.6 to 2.5 m wide |
| Continental 512 x 120 m | 4 to 6 m³/s, 3 to 6 m wide |

- With `edges: "coast"` (the default) every map is an island, so no
    water can arrive from beyond it.
- Deltas need a river over 8 m wide meeting the sea on ground flatter
    than 0.5 %. No generated map meets that: rivers are too narrow, and
    coasts rise faster than 0.5 %. A delta was only seen on a synthetic
    coastal plain with `widthScale: 4`.

**Falls and pools.**

- The 35-degree rule gives alpine maps 1,100 to 2,500 falls (1,226 at the
    default climate, 2,459 at -5 °C), with 148k fall vertices.
- Most sit on slopes where no bowl can hold water, so plan 3 draws each
    pool as a thin churned film draped on the ground.
- The plan 3 rule, radius `0.3 x height + w`, gives 10 m pools under
    0.6 m trickles. They read as oversized.

**Meshes and build time.**

- A continental 512 x 120 m map has 7,501 reaches and 605k ribbon
    vertices; 1024-sample maps with fine networks reach about 600k.
- The in-browser `"rivers"` phase is 95 to 226 ms at 512 x 512, inside
    the 300 ms budget. Natively, `carve_reach` takes 169 of 336 ms on the
    continental 120 m map for 137k points.
- `carve_reach` scans a box at least four samples wide around every
    segment, and consecutive segments are about one sample apart, so each
    sample is visited 9 to 25 times.

**Small streams.**

- Streams narrower than a sample are carved one sample wide and drawn as
    a ribbon only.
- Seen from afar they are under a pixel wide and break into dashes.
- Meanders cannot loop tighter than the heightmap grid.

**Reeds.** Reeds need samples no more than 80 m apart; plan 3's last
fix documents this.

**First load.** The default capture spends about 11.5 s from
`generateFractal` to ready under software WebGPU:

- CPU generation takes about 0.5 s, and the surface, mesh, flora and
    grass work after the river build about 0.26 s.
- About 10.9 s is spent inside the first large buffer write
    (`upload_terrain`), waiting for the GPU process to finish start-up
    work queued when the engine was created.
- Pausing 20 s after `createVistaEngine` before generating drops the
    `"finishing"` phase from 11,074 ms to 264 ms.
- Skipping the procedural texture bake saves about 0.5 s. Skipping the
    impostor bake saves nothing measurable.
- The rest is shader and pipeline compilation: 14 render pipelines and
    about 8 compute pipelines are created at start-up, whatever the scene
    needs.
- The water shader (37 KB, plus 33 KB of `common.wgsl`) is compiled four
    times: the ocean with sea ice, the ocean without, inland water, and
    waterfalls. The cloud pass is compiled at full and quarter resolution.
    The lens and grass pipelines exist even when rain and grass are off.
- Hardware GPUs compile faster, but pipeline creation is still the
    largest single start-up cost there. Chrome caches compiled shaders
    between visits, so first visits are the ones to fix.

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

- **Inflow is the lever for big rivers.** A map is treated as part of a
    larger basin: water enters from beyond it where a valley meets an open
    map edge, or wherever the host places an inflow. `inflow: "auto"` is
    the default, but it only places an inflow on maps with an open edge,
    so default coast-ringed maps keep their own rivers, and their
    reference shots don't change.
- **Plunge pools scale with discharge as well as height.** This changes
    plan 3's rule, `0.3 x height + w`, which gave 10 m pools under
    trickles.
- **Trickle falls** (under 0.05 m³/s and under 1 m wide) are drawn as
    whitewater down the step, not as a sheet with mist and a pool.
    Consecutive falls close together form one cascade.
- **Gravel is a new terrain material in slot 10.** Plan 5b's scree moves
    to slot 11; this plan updates plan 5b's text to match.
- **Screen-space reflections are on by default** (`reflections:
    "screen"`), with `"sky"` keeping today's sky-only reflection.
- **Riparian greening is on by default** (`riparian: 1`) and changes
    the ground within tens to hundreds of metres of rivers and lakes.
    That is intended: reference shots may change there, and only there.
- **Pipelines are created when the scene needs them**, first-frame
    pipelines first, the rest warmed up after the first frame. No visual
    change, only faster loading.

## Prerequisites

- **Plan 3** must be merged. Check that `terrain/hydrology.rs`,
    `terrain/channels.rs`, `terrain/water_mask.rs` and `water_sounds.rs`
    exist, that `RiverOptions` has `snowmelt`, `springs`, `meanders` and
    `waterfalls`, and that `engine.getWaterfalls()` exists. If it is
    missing, carry out plan 3 first.
- Plan 3's later fixes must be in: plunge pools that only lower the
    ground (`carve_pool` uses `record.lower`), pools that freeze with
    their falls, and reeds that need an unsaturated wet-bank distance.
    Check `git log --oneline | grep -i -E "plunge pools|reeds|freeze"`.

## Current state

Read these before starting:

- `crates/vista_wasm/src/terrain/hydrology.rs`:
    - `build_hydrology` routes water on a flow grid (at most 1024 per
        side) and accumulates discharge from rain, snowmelt and springs;
    - `find_lakes`, `mark_channels`, `streams`.
- `crates/vista_wasm/src/terrain/channels.rs`:
    - `condition_channels`, `shape_stream`, `find_steps`, `meander`,
        `delta`;
    - `carve_reach` (the slow one), `carve_pool`, `carve_oxbow`,
        `CarveRecord`.
- `crates/vista_wasm/src/render/water.rs`:
    - `build_river_network`, `add_ribbon`, `add_fall` (sheet, mist,
        draped pool), `add_lakes`, `mesh_height_at`, `WetBanks`.
- `crates/vista_wasm/src/shaders/water.wgsl`: the `INLAND` and
    `SEA_ICE` override constants, `fragment_fall`, `frozen_surface`, the
    sky-only reflection around `reflect(-view, normal)`.
- `crates/vista_wasm/src/render/gpu.rs`: `create_pipeline` calls for all
    render pipelines in one place (about line 1400), `create_render_targets`
    (`hdr_view` and depth), the draw order, `bake_impostors`.
- `crates/vista_wasm/src/render/textures.rs`: `bake_world_textures`, one
    compute pipeline per `gen_*` entry point, `TERRAIN_LAYERS = 10`.
- `crates/vista_wasm/src/terrain/biomes.rs`: `classify_surface`,
    `reclassify_surface`, `chamfer_distance`.
- `crates/vista_wasm/src/render/flora.rs` and `render/grass.rs`: tree
    and grass placement, reeds.
- `crates/vista_wasm/src/engine.rs`: `rebuild_world_with`,
    `rebake_surface`, `touched_samples`.
- `scripts/visual-check/index.html`: the capture page, which logs the
    generation phases.

## Design

### 1. Faster first load

**Measure first.**

- Extend `scripts/visual-check/index.html` to log, alongside the
    phases:
    - `engine ms`: how long `createVistaEngine` took;
    - `first frame ms`: from the start of `createVistaEngine` to the
        first rendered frame.
- Repeat the elimination tests from "What plan 3 verification found"
    (a pause before generating; skipping the texture bake). Then skip
    each pipeline group in turn to find its share. Do this in scratch
    copies that are never committed.
- Record the per-group cost in the report.

**Create only what the scene needs.**

- Hold the render and compute pipelines in a `Pipelines` struct of
    `Option`s, filled by `ensure_pipelines(&Needs)`.
- `Needs` comes from state the engine already has:

    | Need | Pipeline |
    | --- | --- |
    | Sea can freeze (`sea_ice_possible`) | the sea-ice ocean pipeline, otherwise the open-water one |
    | Rivers, lakes or pools present | `inland_water` |
    | Falls present | `falls` |
    | Clouds on | the cloud pipeline for the resolution in use only |
    | Lens drops possible | `lens` |
    | Grass on | `grass` |
    | Trees present | tree pipelines |
    | Erosion requested | the erosion compute pipelines |

- The impostor bake pipeline is created for the bake and dropped after
    it.
- The cloud texture's compute entry and its 64³ texture are only made
    when clouds are first switched on.
- Drawing code skips a system whose pipeline is `None`. That never
    happens for a system that is visible: `ensure_pipelines` runs before
    the frame whenever options or terrain change.

**Order and warm-up.**

- Create the first frame's pipelines first, in the order the frame
    draws them.
- After the first frame is presented, create the pipelines the scene is
    likely to need soon, one per frame: rain lens drops when the weather
    can reach rain, and the other ocean variant when temperatures are
    near freezing. A later switch then does not hitch.
- If `wgpu` exposes asynchronous pipeline creation on the WebGPU
    backend, use it for the warm-up queue. Otherwise ordering alone is
    fine. Say which in the report.

**Don't block the page.** Before the first large upload after terrain
generation, await the queue's submitted-work-done callback. The page then
stays responsive and keeps receiving progress events while the GPU
process compiles, instead of freezing inside `writeBuffer`.

**Share compiled code.**

- Where two pipelines differ only in an override constant, check that
    they share one shader module (`create_shader_module` once).
- Check that every large per-kind branch is behind an override constant
    or a uniform guard, so each pipeline's backend compile holds only its
    own code.

**Target.** In the default capture under software WebGPU, `first frame
ms` at least 40 % lower than the baseline, with the three reference
shots pixel-identical apart from this plan's river changes.

### 2. A cheaper river build and river meshes

**Carve each sample once.**

- Rewrite `carve_reach` as two passes per reach:
    1. Walk each segment's box, computing only distance, `t` and the
        target height, and keep each sample's lowest target and whether
        it is within the mask distance, in a scratch buffer of touched
        samples (reused, never reallocated per reach).
    2. For each touched sample, check the sea, glacier, lake and no-data
        rules once, and call `record.lower` once.
- The old code read `ground` after earlier segments had lowered it. The
    new code reads it once per sample. Keep the old function in
    `#[cfg(test)]` as a reference, and test that the two differ by at
    most 0.05 m on every fixture map, and that neither ever raises a
    sample.
- Target: `carve_reach` at least 2.5 times faster natively on continental
    512 x 120 m (169 ms at the start).

**Fewer ribbon vertices.**

- In `add_ribbon`, drop a dense point on a straight, uniform run: the
    polyline moves less than `0.05 w` (and 0.1 m) from the chord, and
    width, level and speed change less than 2 %.
- Never drop a point at a join, a fall, a lake, the ends, or where
    `|curvature|` is over 0.2.
- Target: at least 40 % fewer ribbon vertices on continental 512 x
    120 m (605k at the start), with no visible change in the river
    shots.

**Fewer fall vertices:** see section 5.

### 3. Big rivers: inflow

**Options.**

- `RiverOptions.inflow` is `"auto"` (the default), `"none"`, or a list
    of up to 8 `{ position: [x, z], dischargeCubicMetresPerSecond }`.
- An explicit inflow snaps to the nearest land sample on the height map,
    never the skirt. Its discharge is added there, and water is forced
    downstream from it like any source.
- `"auto"` places at most one inflow, and only where the map has an open
    edge (`edges: "open"`, or an imported map whose edge is land):
    - Candidates are land samples on the map border at local minima of
        height along the border (valley mouths), whose D8 receiver points
        inwards.
    - Pick the lowest.
    - Its discharge comes from a virtual upstream area of 10 times the
        map's land area, at the map's mean precipitation:
        `Q = 10 x A_land x P_mean / seconds per year`.
    - A 512 x 30 m open map then gets a trunk river of roughly 100 to
        300 m³/s, 25 to 45 m wide at `widthScale: 1`.
- Validation:
    - positions must be finite and on the map;
    - discharge must be from 0 to 100,000 m³/s;
    - at most 8 entries;
    - messages give the valid range.
- `engine.getInflows()` returns the inflows in use, including the one
    `"auto"` placed: `{ position: [x, y, z], dischargeCubicMetresPerSecond
    }[]`.

**Valley floors for big rivers.**

- A river 40 m wide in a V-shaped valley reads as a canal. For reaches
    with `w >= 20 m` on slopes under 1 %, lower the ground within `3 w`
    of the bank to at most bank level + 0.5 m, tapering back to the
    ground over a further `6 w`.
- Only ever lower, and record every change so `restore_carving` stays
    exact.

**What follows on its own.**

- Width, depth, speed, meander wavelength (about 11 w), oxbows, rapids
    and sound all scale from discharge.
- Deltas become reachable where a big river meets flat coast.
- The river build budget still holds (section 2 pays for the wider
    carving).

**Demo.**

- A "River valley" preset in the terrain section: continental, `edges:
    "open"`, 512 x 30 m, `inflow: "auto"`.
- A "Jump to the main river" button that places the camera 120 m above
    the main river, 1 km downstream of `getInflows()[0]`, looking
    downstream. Disable it when there are no inflows.
- An inflow select (auto, none, custom) with a discharge slider (1 to
    2,000 m³/s, readout) for the custom inflow, placed at the camera's
    target.

### 4. Reflections

**Scene copy.** After the opaque passes and before the water pass, a
small render pass writes a half-resolution `rgba16float` texture:

- RGB is the HDR scene colour.
- A is linear view depth, from the depth buffer.
- Water is not in it, so water never reflects water. That is intended.

**Tracing in the water fragment.**

- March the reflected ray in view space: 16 steps with increasing
    stride, then 4 binary refinement steps against the copy's depth.
- On a hit, take the colour with `textureSampleLevel`. Its level comes
    from the surface roughness, from ripples and wind, so choppy water
    blurs its reflection.
- Fade the hit towards the screen edges, with ray length, and where the
    hit faces away. Fall back to today's sky and cloud reflection
    wherever the fade is incomplete.
- Weight by the existing Fresnel term and `reflectivity`.

**Guards and rules.**

- Behind a uniform guard (`reflections == "screen"`). Rivers, lakes,
    pools and the ocean all use it; frozen surfaces keep their own
    shading.
- All samples after per-pixel branches use `textureSampleLevel`.
- Verify in Chromium.

**Budget.** The water pass may grow by at most 0.8 ms with a river
filling a third of the screen. The copy pass is counted in it.

### 5. Falls and pools at their size

**Pools scale with discharge.**

- Plan 3's pool radius, `0.3 x height + w`, and depth, `0.15 x height`,
    are both multiplied by `clamp(sqrt(Q) / 2, 0.15, 1)`.
- The depth is at least 0.3 m.
- Update `docs/water.md` and plan 3's tests.

**Trickle falls.**

- Falls under 0.05 m³/s and under 1 m wide get no sheet, mist or pool.
- Their step is drawn as a whitewater ribbon down the fall line instead:
    rapids foam on the existing ribbon, not skipped.
- `getWaterfalls` still lists them, and their sound stays.

**Cascades.**

- Consecutive falls on one reach, where each foot is within three pool
    radii of the next lip, form one cascade. It has:
    - one sheet mesh following the steps;
    - one mist cloud of at most 64 sprites at the lowest foot;
    - intermediate pools as churned films;
    - one full pool at the bottom.
- `getWaterfalls` lists a cascade once. Its height is the total drop and
    its position the lowest foot. Note this under `### Changed`.

**Target.** At least 50 % fewer fall vertices on alpine 512 x 12 m at
the default climate (148k at the start).

### 6. Small streams at their true size

**A sub-sample centreline.**

- For reaches narrower than a heightmap sample, on slopes under 1 %,
    with `meanders > 0`, give the ribbon its own Kinoshita meander at the
    stream's own wavelength (about 11 w). Keep it inside a corridor of
    ±0.45 samples around the carved path, so the water stays in the
    carved ground.
- Loops a few metres across then show on 12 to 30 m grids.
- The hydrology, carving and sounds keep using the carved path.

**Bank strips.**

- Beside every reach narrower than a heightmap sample, draw a strip on
    each side of the water. It runs from `0.5 w` out to `0.5 w + b`,
    where `b = max(0.5 m, 0.4 w)`.
- Strip heights come from `mesh_height_at` plus 2 cm.
- Draw it in its own small pipeline after the terrain, created only when
    such streams exist.
- The fragment shades a wet, muddy or gravelly bank with the terrain's
    material functions and lighting:
    - normals tilt towards the water;
    - alpha fades to 0 at the outer edge;
    - the strip fades out between 300 and 500 m.
- This gives a crisp small channel where the heightmap has only a
    one-sample trench.

**No dashes at a distance.**

- In the ribbon and bank-strip vertex shaders, widen each side to at
    least 0.75 pixel footprints.
- Pass `coverage = true width / drawn width` to the fragment and
    multiply alpha by it. Far brooks become continuous faint lines.

**Reeds on true banks.**

- For reaches narrower than a heightmap sample, grass placement measures
    distance to the true (sub-sample) water edge from the reach geometry,
    not from the coarse wet-bank field.
- Reeds then line tight meanders, and grow on maps of any sample
    spacing within 3 m of still or slow water. This removes plan 3's
    80 m limit; update `docs/water.md`.

### 7. Gravel, sand and stones

**A gravel material.**

- `MAT_GRAVEL = 10`, in reserved slot 10. `TERRAIN_LAYERS` and
    `TERRAIN_TEXTURE_LAYERS` (in `engine.rs`) go from 10 to 11, and
    `replaceTexture` and `docs/hooks.md` accept layers 0 to 10.
- Generate it in `texture_gen.wgsl` as one more layer of the terrain
    arrays, with no new pipeline:
    - rounded cobbles from cell noise, 3 to 15 cm;
    - grey-brown with per-stone tint;
    - dark crevices in the occlusion;
    - a bumpy height and normal;
    - rough, except where wet.
- `materialTints` gains an eleventh entry. Update `docs/options-reference.md`,
    the materials debug view, `replaceTexture` (accepts layer 10), and
    plan 5b's text (scree becomes slot 11, `MAT_SCREE = 11`, and the
    twelfth tint).

**Where it goes.** Weights, never hard switches, blended with what is
already there. Only on samples the channel stage touched, above the
water, below the bank top. Only where the channel is at least 0.75
samples wide; narrower streams get their bed look from the bank strips.

- **Sorted by speed:**
    - at 1 m/s and over, gravel;
    - 0.4 to 1 m/s, sand;
    - under 0.4 m/s, mud, which the wet banks already darken.
- **Point bars:** on the inner side of bends (from the sign of
    `curvature`), gravel or sand reaches out to `1.5 w` from the water
    edge.
- **Mouths:** deltas and sea mouths are sand.

Plan 3 removed a sand and mud rule that made blocky beads along streams.
Check the river close-up for beads again.

**Wet stones.** Within the wet-bank field's first 2 m, gravel darkens
more than other ground and turns glossy.

**Emergent stones and clear beds.**

- In the river branch of `water.wgsl`, where depth is under 1 m and
    speed is over 0.5 m/s, add procedural stones in the bed: cell noise,
    0.3 to 0.8 m across, seeded by `rest_xz`, larger in high-power reaches
    (section 8).
- Where a stone's top is above the surface, draw it as an opaque wet
    stone with a foam ring on its upstream side (from the flow direction).
- Where it is within 10 cm below, draw a bright riffle.
- Shallow clear water already shows the bed, which now carries gravel.
- Keep it inside the existing river branch, and use `textureSampleLevel`
    only.

### 8. Rock-walled rapids

**Stream power.** Compute unit stream power per channel point,
`omega = 1000 x 9.81 x Q x S / w`, in W/m².

**Where it is over 300 W/m² and the slope is over 2 %:**

- **Rock banks:** bank-band samples get rock material weight rising with
    power, and the V-shape blend steepens towards near-vertical walls
    within `r` to `r + w`. Lower only, and record every change.
- **Bigger stones:** the emergent stones (section 7) grow to 0.8 to
    2 m, standing waves and whitewater scale up with power, and foam
    streaks run downstream of each stone.

**Coordination.** Plan 5b adds outcrops and boulders later. Keep this
rule in the channel stage so plan 5b can read it.

### 9. Green banks

**A riparian field.**

- The river build computes a riparian value per heightmap sample, from 0
    to 1:
    - influence radius `R = clamp(25 + 12 sqrt(Q), 25, 400)` metres for
        rivers, and 40 m around lakes;
    - value `(1 - d / R)^2 x riparian`.
- Use a two-pass chamfer that propagates the largest remaining reach
    (`R - d`), keeping each source's `R` alongside.
- It lives on the heightmap grid, beside the channel mask, and is never
    built on the skirt.

**Climate.**

- Classification adds `0.45 x riparian value` to moisture (clamped to 1)
    above the bank top, except on glacier ice.
- Dry biomes near water turn to grassy meadows or thicket, so a canyon
    floor gets a green strip. No new biome.
- `reclassify_surface` covers every sample with a riparian value, so the
    patch stays equal to a full classification. Extend plan 3's test.

**Trees.**

- Tree density rises by up to `1.5 x value` in savannah, grassy meadows
    and mesa-desert ground, with the species of the new local biome.
- Trees grow from 1 m beyond the channel mask outwards, never on bars or
    below the bank top. Forests keep their density but now reach the
    bank.

**Grass.** The existing within-12 m density boost also follows the
riparian value.

**Option.** `RiverOptions.riparian`, from 0 to 2, default 1. 0 turns it
off.

## Public API

Additive only.

```ts
export interface RiverInflow {
  /** World x and z in metres. Snaps to the nearest land sample. */
  position: [number, number];
  /** 0 to 100,000. */
  dischargeCubicMetresPerSecond: number;
}

export interface RiverOptions {
  // ...existing fields
  /**
   * Water arriving from beyond the map. "auto" (default) adds one inflow
   * where a valley meets an open map edge, sized from a basin ten times
   * the map's land area; "none" adds nothing; or up to 8 explicit inflows.
   */
  inflow?: "auto" | "none" | RiverInflow[];
  /** Bankside greening and trees, 0 (off) to 2. Default 1. */
  riparian?: number;
}

export interface WaterOptions {
  // ...existing fields
  /** "screen" (default) reflects the scene; "sky" reflects the sky only. */
  reflections?: "screen" | "sky";
}

export interface WaterInflow {
  position: [number, number, number];
  dischargeCubicMetresPerSecond: number;
}

export interface VistaEngine {
  // ...existing methods
  /** The inflows in use, including the one "auto" placed. */
  getInflows(): WaterInflow[];
}
```

- Mirror these in `vista_types` with serde defaults.
- Validate them in `config.rs`.
- Add wasm-bindgen methods (flat `Float32Array` at the boundary, as
    `getWaterfalls` does) and typed wrappers.
- `materialTints` accepts 8, 10 or 11 entries.

## Steps

0. **Baseline.** Record:
    - gzipped WASM size;
    - `fixed-scene.mjs` (twice);
    - the reference shots;
    - `first frame ms` and `engine ms` for the default capture (after
        adding the logging in step 1, measured on the unchanged engine);
    - native `carve_reach` time, ribbon vertices and fall vertices on
        the maps named in section 2;
    - the four waterfall and river shots from plan 3's verification.
1. **Faster first load** (section 1). Commit the logging separately,
    first.
2. **River build and mesh efficiency** (section 2's carve and ribbons).
3. **Inflow and big rivers** (section 3), with `getInflows`.
4. **Falls and pools at their size** (section 5).
5. **Small streams** (section 6).
6. **Gravel material, bars and wet stones** (section 7, first half).
7. **Emergent stones and rock-walled rapids** (section 7, second half,
    and section 8).
8. **Green banks** (section 9).
9. **Reflections** (section 4).
10. **Demo:**
    - the River valley preset and the jump to the main river;
    - the inflow select and discharge slider;
    - a riparian slider (0 to 2, readout);
    - a reflections select.
11. **Docs:**
    - `docs/water.md`: inflow, big rivers, pools, trickles and cascades,
        small streams, bed materials, rapids, green banks, reflections;
    - `docs/options-reference.md`;
    - `docs/architecture.md`: need-driven pipelines, warm-up, the scene
        copy and the bank-strip pass in the draw order, binding changes;
    - `docs/render-quality-and-diagnostics.md`: load time and pipeline
        warm-up;
    - `CHANGELOG.md`;
    - plan 5b's slot change;
    - this plan's row in `docs/plans/README.md`.
12. **Verification and report.**

Run `fixed-scene.mjs` after each major step.

## Tests

**Load.**

- `Needs` from a scene without rivers, falls, clouds, grass or rain
    creates none of those pipelines.
- A scene with them creates each once.
- Switching grass on later creates the grass pipeline.

**Carve.**

- The new `carve_reach` stays within 0.05 m of the reference on three
    fixture maps, never raises a sample, and the toggle test stays
    bit-identical.

**Ribbons.**

- Simplification keeps every join, fall and end point.
- It removes at least 40 % of points on a long straight fixture reach.

**Inflow.**

- An explicit inflow on a valley fixture yields a river whose discharge
    at the outlet is at least the inflow.
- `"auto"` on an open-edge fixture places one inflow at the lowest
    border valley mouth, and on a coast map places none.
- Validation rejects 9 inflows, negative discharge, and positions off
    the map.

**Valley floors.** Ground within `3 w` of a 40 m river on a gentle
fixture sits at most 0.5 m above bank level, and restore is exact.

**Pools.** Pool radius under a 0.03 m³/s fall is at most 0.2 of plan
3's value; under a 4 m³/s fall it is unchanged.

**Trickles and cascades.**

- A trickle fall has no sheet or pool vertices.
- Three close falls become one cascade whose `getWaterfalls` height is
    their total drop.

**Small streams.**

- The sub-sample centreline stays within ±0.45 samples of the carved
    path.
- On a flat fixture its sinuosity is at least 1.3 with `meanders: 1`.

**Reeds.** On a 120 m map, reeds grow within 3 m of a slow stream's true
edge and nowhere else.

**Materials.**

- Fast, bending reaches get gravel on the inner bend and none on the
    outer bank.
- Slow reaches get sand or mud.
- Channels under 0.75 samples wide get no bed material.
- The texture bake produces 11 terrain layers.

**Stream power.** Rock bank weight appears only where power is over
300 W/m² and the slope is over 2 %.

**Riparian.**

- The value is 1 at the water and 0 beyond `R`.
- A savannah fixture's samples within 30 m of a river classify greener.
- The patched classification equals a full classification.
- Trees never stand inside the channel mask.

**Reflections.** A CPU port of the ray step and fade (screen-edge and
distance fades) gives 0 at the edges and 1 in the middle.

**TypeScript.**

- Type tests for the new fields and `getInflows`.
- Runtime validation tests for `inflow` and `riparian`.

## Verification

Capture each at 960 x 600, open it, and describe it in the report. The
targets are the owner's reference photographs:

1. **Boreal valley.**
    - Continental, `edges: "open"`, 512 x 30 m, `inflow: "auto"`,
        `meanTemperatureCelsius: 3`.
    - 120 m above the main river, looking downstream.
    - A wide, calm river reflecting the forest and hills, a gravel bar on
        an inner bend, and trees to the bank.
2. **Cobbled stream.**
    - Rolling hills with an explicit 15 m³/s inflow.
    - 1.5 m above the water.
    - A clear, shallow bed of cobbles, stones breaking the surface with
        foam rings, and wet dark stones at the edge.
3. **Rapids.**
    - Alpine, `edges: "open"`, `inflow: "auto"`.
    - At the bank of a reach steeper than 2 %.
    - Whitewater, standing waves, and rock banks.
4. **Canyon.**
    - `mesaDesert`, `edges: "open"`, `inflow: "auto"`.
    - Above the river.
    - A silty river with sandbars and a green strip of grass and trees
        through the dry land.
5. **Meadow meanders.**
    - Rolling hills, 512 x 30 m.
    - About 60 m above a slow stream a few metres wide.
    - Tight loops narrower than a heightmap sample, reeds lining them,
        and crisp banks.
6. **Distance.** The same stream from 1.5 km: a continuous faint line,
    no dashes.
7. **Delta.**
    - Continental, `edges: "open"`, `inflow: "auto"`, at the river's
        mouth.
    - Distributaries around a low fan.
    - If no generated map has a coast flatter than 0.5 %, say so and
        show it on the synthetic coastal plain from plan 3 with the auto
        inflow instead.
8. **Plan 3 regressions:**
    - the default waterfall close-up (pool sized to its fall, mist,
        churn);
    - alpine cascades (fewer, merged, no shelves);
    - the frozen lake at -18 °C on `fjords` seed 2;
    - reeds on hills seed 3.
9. **Reference shots 1 to 3:** unchanged except rivers, their banks and
    riparian ground.
10. **Load time:** `first frame ms` before and after, twice each.

## Budgets

- **WASM:** a soft target of +14 KB gzipped over the size at the start,
    and a hard limit of +28 KB. Efficiency work may give bytes back;
    count the net figure.
- **GPU:**
    - water pass +0.8 ms at most with screen reflections and a river
        filling a third of the screen, including the scene copy;
    - terrain pass +0.15 ms (gravel layer, bars, rock banks);
    - the bank-strip pass 0.1 ms at most;
    - trees and grass +3 % at most in the default scene;
    - every other pass within 5 % on `fixed-scene.mjs`, scaled from
        software ratios against the terrain pass as in plan 3.
- **Load:** default capture `first frame ms` at least 40 % lower under
    software WebGPU.
- **River build:** the in-browser `"rivers"` phase at most 300 ms at 512
    x 512, including an auto inflow on an open-edge map. `carve_reach` at
    least 2.5 times faster.
- **Meshes:** ribbon vertices at least 40 % fewer on continental 512 x
    120 m; fall vertices at least 50 % fewer on alpine 512 x 12 m.
- **CPU per frame:** near zero. The warm-up creates at most one pipeline
    per frame.

## Out of scope

- **Mist rising from rivers and lakes on cool mornings:** plan 7 (sky
    and weather presets) owns time of day and humidity. Plan 7 should use
    the riparian field from this plan.
- **Other effects:**
    - seasonal flow and floods;
    - sediment transport over time;
    - reflections of water in water;
    - planar reflections.
- **3D boulder meshes:** plan 5b.
- **Painting inflows in the UI:** plan 11. This plan provides the API.
