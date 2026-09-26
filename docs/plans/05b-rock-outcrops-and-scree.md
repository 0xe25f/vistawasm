# Plan 5b: Rock outcrops and scree

## Goal

Bare rock looks like real rock, not grey paint:

- **Placement:** it shows where the soil is thin. That is convex
    shoulders and ridges, steep bands that follow the contours, and
    harder layers that stand out as crags and ledges. It is not
    wherever the slope crosses a threshold.
- **Shading:** faces read as solid stone at every distance, with
    blocky joints, deep cracks, lichen, and dark water streaks down
    vertical faces.
- **Debris:** fallen boulders and a fan of scree lie at the foot of
    outcrops, standing on the ground in 3D.

## Why the rock looks flat today

The owner's render of `continental` seed 2 (after plan 2b), taken from
`[-1500, 1500, 2600]` looking at `[0, 300, 0]`, shows large, smooth-edged
pale grey blotches on green mountainsides.

In `crates/vista_wasm/src/terrain/biomes.rs` (`classify_surface`, about
line 715), rock is:

```text
smoothstep((steep - 0.16) / 0.26) * 0.95 + mountain * 0.35 * (1 - snow)
```

This is a pure function of slope and relative height.

- Wherever the slope crosses about 9 to 24 degrees, rock fades in as a
    soft-edged blob. Rock outcrops have edges and structure; these don't.
- Nothing puts rock on convex ground or bands it along the contours.
    Nothing sheds debris below it.
- In `clipmap_render.wgsl`, rock is the same texture projection at every
    distance. Far away it averages to flat grey.

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

- The owner asked for this because it adds realism. Realism is the test:
    every change here must make rock read more like real rock in
    captures, at no cost to frame rate.
- Hyper-realistic visuals, a solid 60 FPS and tiny files remain the
    targets.
- Size figures are soft targets, and double is the hard limit.

## Prerequisites

- **Plan 5** (vegetation density) must be merged. Check for
    `shaders/tree_generate.wgsl`, `shaders/grass_generate.wgsl`, the
    world-space lattice hash (Rust and WGSL), and the cover texture at
    `@group(1) @binding(14)`.
- **Plan 4** provides `mesh_surface_height` (Rust and WGSL), root
    grounding and the exposure field.
- **Plan 3** provides the distance-to-water field
    (`surface_texture_b.r`) and the river and lake masks.
- **Plan 1** provides drainage on a coarse grid; plan 3 provides
    full-resolution D8 accumulation.
- If any is missing, carry it out first (each is self-contained).
- This plan comes before plan 6. It adds no tree changes.
- **From plan 3b:**
    - Gravel owns material slot 10 and river beds (`RiverNetwork::bed`).
        Scree must not overwrite bed materials.
    - Rock-walled rapids already give rock banks. Keep their look and
        let the soil model agree with them, not fight them.
    - Pipelines are created on demand (`Needs`, `ensure_pipelines`,
        `warm_up`). The boulder pipeline goes through `Needs` (boulders on
        and in view), and the default capture's `first frame ms` must not
        rise by more than 5 %.
    - Boulders must avoid every drawn channel, including streams
        narrower than a sample. Reuse plan 5's binned channel segments
        in the boulder generator.
- **Solid 60 FPS is a hard target.** The whole frame at 1080p on a
    mid-range GPU stays within 12 ms in the default scene and 14 ms in
    rain.

## Design

### 1. Where rock shows: a soil-depth model

Replace the rock rule in `classify_surface` with a soil depth estimate.
Put it in a new `terrain/soil.rs`, and compute it once per terrain
rebuild at height-map resolution.

- **Soil depth** (in metres, clamped to 0 to 3) is
    `base - slope_loss - convexity_loss + accumulation - frost`:
    - `base`: 1.6 m, lower in arid climates (moisture from the surface
        sample) and higher in wet ones.
    - `slope_loss`: `2.2 x smoothstep(18°, 42°, slope)`. Steep ground
        sheds soil.
    - `convexity_loss`: from the height Laplacian at two scales (3 and
        9 samples). Convex shoulders and ridge crests lose up to 1.2 m.
        Concave hollows gain up to 0.8 m.
    - `accumulation`: up to +1 m, rising with `log(drainage area)` from
        plan 3's full-resolution accumulation. Valley floors and
        footslopes hold deep soil.
    - `frost`: above the tree line and in `alpineTransition`, up to
        -1 m, rising with height into the band. Frost shatters bedrock.
- **Hard layers** (lithology): add a banding field `strata(x, y, z)`,
    the sine of height plus a gentle regional dip (a plane tilted 2 to 8
    degrees, direction from the terrain seed), with a period of 25 to
    60 m, warped by 2-octave noise at 400 m.
    - Where a hard band crosses a slope over 22 degrees, soil depth drops
        by up to 1.5 m. Outcrops then run as crags and ledges along the
        contours and repeat down the hillside, as real rock bands do.
    - The dip and period come from the landform: tighter, flatter bands
        for `mesaDesert` (which reads as its terraces), and looser and
        steeper for `alpine`.
- **Exposure:** rock weight is `1 - smoothstep(0.15, 0.6 m, soil_depth)`.
    Break the edge with 3-sample noise, so outcrop edges are ragged and
    lobed, not soft circles.
- **Kept rules:**
    - `CoastalRocky` stays at least 0.75;
    - snow still buries all but the steepest rock;
    - sea-bed rock is still reduced;
    - plan 2's tundra stoniness stays.
- **Scree** (a new material, `MAT_SCREE = 11`, using reserved slot 11;
    plan 3b gives slot 10 to gravel):
    - For each outcrop sample, walk 2 to 8 samples down the steepest
        descent (plan 3's D8 receivers). While the slope stays over 25
        degrees, deposit scree weight that falls off with distance.
    - Where the slope eases below 25 degrees, stop: that is the foot of
        the talus cone.
    - Scree also covers `alpineTransition` ground at 0.2 to 0.5,
        replacing part of today's plain rock there.
    - Update `MATERIAL_COUNT` usage and the materials debug view. No
        reserved slots remain.
- Keep the old rule as a reference only in a test. The new rule must
    produce broadly similar total rock on mountains (within ±30 %), but in
    structured places.

### 2. What rock looks like: shading

All of this is in `clipmap_render.wgsl`, on the rock and scree
materials only, and uniform-safe (`textureSampleGrad` with the existing
UV derivatives).

- **Scree texture:** generate it in `texture_gen.wgsl` as a new terrain
    layer: angular gravel and cobbles, 2 to 30 cm, with contact shadows
    and a height channel. Raise `TERRAIN_LAYERS` and
    `TERRAIN_TEXTURE_LAYERS` to 12 (plan 3b took them to 11 for gravel),
    and update `replaceTexture` layer validation and `docs/hooks.md`
    (layers 0 to 11).
- **Macro joints:** a second, low-frequency rock pattern at 6 to 20 m.
    - It is Voronoi-based blocky joint planes, taken from the noise
        texture's Worley channel and stretched along the strata direction.
    - It adds dark joint lines and faceted macro normals, so a cliff
        reads as blocks and slabs from 50 m to 2 km, not flat grey.
    - Blend its normal strength up with distance, so far rock keeps
        structure where the near detail texture has averaged out.
- **Crevice occlusion:** darken by the rock height channel plus the joint
    lines, up to 45 %, so cracks read deep. Apply it to ambient light only
    (sky and bounce), not direct sun, to keep lighting plausible.
- **Lichen:**
    - On moist, sun-facing, gently inclined rock, add sparse patches of
        pale grey-green and ochre.
    - The pattern comes from the noise texture at 0.4 m, and it is none
        in arid or `iceArctic` ground.
    - Keep lichen under 20 % coverage, so the rock stays rock.
- **Water streaks:** on faces steeper than 60 degrees, add dark vertical
    streaks running straight down the fall line, from a 1D noise across the
    slope.
    - They are stronger where the surface weather map shows wetness, once
        plan 7 lands.
    - Until then, drive them from moisture, so this plan works on its
        own.
- **Edges:** at the rock-to-grass boundary, grass tufts from plan 5's
    grass generator overlap the rock edge. Raise grass density on the soil
    side of the boundary within 1 m, so turf visibly overhangs the rock
    lip, rather than cross-fading into it.
- **Parallax:** within 120 m, apply 4-step parallax occlusion mapping to
    rock and scree only, from their height channels, with a depth of at
    most 8 cm. Joints and cobbles then read as solid close up.
    - Fade it out between 80 and 120 m.
    - Skip it on pixels where the rock weight is below 0.3, so blends
        don't pay for it.
    - Branch on uniform-safe values only, and use `textureSampleGrad`.

### 3. Boulders and talus (3D)

- **Boulder meshes:** generate them procedurally at start-up.
    - Six variants: an icosphere with 2 subdivisions (162 vertices),
        displaced by 3-octave noise with sharpened peaks, then squashed
        and cut flat on 1 or 2 sides. This gives fractured blocks, not
        spheres.
    - UVs use triplanar mapping in the shader, with no stretching.
    - The material is the rock texture with the macro joints and
        crevice occlusion above, plus lichen on the upper faces.
- **Placement:** use plan 5's world-space lattice and tile streaming,
    with a separate hash salt and its own 32 m tiles. Within
    `boulderDistanceMetres` (default 300 m), candidates on a 2 m pitch
    become boulders when:
    - the sample has scree weight above 0.2, or lies within 12 m
        downslope of an outcrop with rock weight above 0.6;
    - probability is `0.35 x scree weight`, and higher near the outcrop
        foot;
    - size is 0.3 to 3 m, following a power law (many small, few large).
        The largest sit furthest down the talus cone, as fallen blocks
        roll furthest;
    - never in water, on frozen water, within a river's half-width
        + 1 m, on the skirt, on sand or on glacier.
- **Grounding:** plan 4's `mesh_surface_height`, sunk by 25 to 40 % of
    the boulder's height, so they sit bedded in the ground, not balanced
    on it. The rotation is random; the lean follows the slope.
- **Drawing:** instanced, with indirect draws, frustum-culled in the tree
    cull pass or a sibling pass.
    - Within 150 m, boulders cast into the tree shadow map as casters.
    - Beyond `boulderDistanceMetres`, fade out by shrinking, using the
        same hash-band trick as plan 5, so they never pop. The scree
        texture carries the look further out.
- **Budget:** at most 20,000 boulders drawn. Plan 5's detail pressure
    shrinks the boulder distance as it does vegetation.

### 4. Options

Add to `SurfaceOptions`:

```ts
export interface SurfaceOptions {
  // ...existing...
  /**
   * How readily bedrock shows through thin soil, 0 (deep soil everywhere)
   * to 2 (rocky). Defaults to 1.
   */
  rockiness?: number;
  /** Boulders and talus below outcrops. Defaults to true. */
  boulders?: boolean;
  /** Distance in metres within which boulders are drawn, 50 to 1000. Defaults to 300. */
  boulderDistanceMetres?: number;
}
```

- Validate them in `config.rs`.
- `materialTints` gains a twelfth entry (scree), after plan 3b's
    eleventh (gravel), if it doesn't already cover them. Extend the docs list.
- Demo, surface section: a "Rockiness" slider (0 to 2, readout), a
    "Boulders" checkbox, and a "Boulder distance" select (150, 300,
    600 m).

## Steps

1. Capture before shots and record the performance gate:
    - the owner's view: continental seed 2 from `[-1500, 1500, 2600]`
        to `[0, 300, 0]`;
    - an outcrop close-up at 30 m and at 8 m (find one from the top-down
        rock debug view);
    - `mesaDesert` seed 1 oblique from 1,000 m;
    - `alpine` seed 2 at the snow line from 400 m;
    - the reference shots from the ground rules.
2. Add `terrain/soil.rs`: soil depth, strata and scree deposition, with
    tests. Switch `classify_surface` to it, and add `MAT_SCREE`.
3. Add the scree texture layer and the layer-count changes.
4. Add the shading: macro joints, crevice occlusion, lichen, streaks,
    edge turf and parallax.
5. Add the boulder meshes, lattice placement, grounding, culling,
    shadows and fade.
6. Add the options, validation, TypeScript, demo controls and docs:
    - `docs/terrain-data.md`: a new "Rock and soil" section with before
        and after images under 60 KB each;
    - `docs/options-reference.md`;
    - `docs/hooks.md`;
    - `docs/architecture.md`;
    - `CHANGELOG.md`.
7. Run the performance gate, check the size, capture the after shots,
    then commit and push.

## Tests

- **Soil:**
    - on a synthetic ridge and valley, crest soil is shallower than
        hollow soil;
    - valley floors (high drainage) are the deepest;
    - it is deterministic.
- **Structure:** on a synthetic 30-degree cone with strata on, rock
    samples form bands.
    - Most connected rock components are elongated along the contour:
        the mean angle between each component's principal axis and the
        local contour is under 30 degrees.
    - At least 3 separate bands appear down the slope.
- **Old against new:** on `alpine` and `continental` seeds 1 to 3, total
    rock weight on land above 30 % of the relief is within ±30 % of the
    old rule's.
- **Scree:** it lies only downslope of rock, on slopes over 25 degrees,
    and stops where the slope eases.
- **Boulders:**
    - Rust and WGSL lattice parity (reuse plan 5's method);
    - none in water, on the skirt, on sand, on glacier or in river
        channels;
    - sizes follow the power law (the median is under 0.8 m);
    - grounding sinks each boulder by 25 to 40 % of its height against
        `mesh_surface_height`.
- **Validation:** out-of-range values for `rockiness` and
    `boulderDistanceMetres` are rejected.

## Verification

- **The owner's view:** outcrops read as banded crags and ledges with
    ragged edges and dark joints, not grey blotches. Talus fans lie below
    the crags.
- **Close-ups at 30 m and 8 m:** stone with depth (joints, cracks,
    lichen), boulders bedded in the ground, turf overhanging rock lips.
- **`mesaDesert`:** horizontal rock bands along the plateau edges.
- **`alpine` at the snow line:** scree slopes and shattered rock below
    the snow.
- **The reference shots:** no other change.
- **Performance gate:** the terrain pass may grow by at most 0.3 ms, and
    boulders (in the trees or a new pass) by at most 0.4 ms at 1080p on a
    mid-range GPU (scale from software ratios). Every other pass stays
    within 5 %.

## Budgets

- WASM growth: a soft target of +8 KB gzipped, and a hard limit of
    +16 KB.
- GPU: terrain +0.3 ms and boulders +0.4 ms at most.
- Classification and scree: at most +30 ms at 512 x 512. Boulder
    generation at most 0.1 ms per frame.

## Out of scope

Individual cliff meshes and overhangs, and rockfall animation.
