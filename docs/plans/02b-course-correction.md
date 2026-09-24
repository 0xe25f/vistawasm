# Plan 2b: Course correction after plans 1 and 2

## Goal

Fix what verification of plans 1 and 2 found, before rivers and
vegetation build on top of it:

1. **The world ends in a wall.** Land runs into the square map edge. Past
    the edge there is a vertical cliff, then a flat, pale sheet instead of
    sea, and there are smeared streaks along the border.
2. **Alpine and fjords are not alpine** on default-sized maps. They read
    as green rolling domes.
3. **Sea ice looks like floor tiles:** a uniform honeycomb of same-sized
    polygons to the horizon.
4. **Stretched snow on steep slopes,** in the foreground of cold scenes.

Plus a like-for-like performance gate, so no fix costs frame rate.

## What verification found

Verified on `realism` at commit `1cbdc57`.

**What passed:**

- `cargo test` (164 tests), vitest (30), `tsc`, `cargo fmt --check` and
    `npm run lint:indent` all pass.
- Clippy has 29 warnings, the same set as before plan 1.
- Every commit uses the required identity, with no attribution lines.
- All plan 1 modules exist: gradient noise, drainage, tectonics, stream
    power, landforms, virtual-pipe GPU erosion and realism tests. The
    unused shaders are removed.
- All plan 2 features exist:
    - `IceArctic` (index 18);
    - `meanTemperatureCelsius` and `temperatureAt`;
    - twelve `unorm8` material slots;
    - the surface texture at binding 12;
    - glacier smoothing with restore;
    - crevasses, sea ice, and snow instead of rain.
- CPU generation at 512 x 512 takes 0.33 s natively (heightmap 209 ms,
    rivers 62 ms, shading 67 ms).
- **Size:** plan 1 added 31,297 bytes and plan 2 added 7,834 bytes
    gzipped, within the owner's allowances of 32 KB and 12 KB. The WASM
    is now 276,951 bytes gzipped.

**Deliberate deviation, accepted:** plan 2 said mild maps keep small ice
caps on their highest summits. Instead, commit `6854ca7` added three
snowy biomes:

- `alpineTransition` (15);
- `lowerSnowyPeaks` (16);
- `upperSnowyPeaks` (17).

An unset climate puts these on high ground and forms no `IceArctic`, and
`permanent_snow` stays 0 there. This is documented in `docs/biomes.md`,
and it is fine. Plans 3 and 4 have been adjusted to account for it.

**Problems:**

- **Edges.** Share of border samples that are land at 512 x 512 and
    12 m, seeds 1 to 3:

    | Landform | Land on the border |
    | --- | --- |
    | continental | 60 to 75 % |
    | alpine | 86 to 100 % |
    | rollingHills | 77 to 95 % |
    | mesaDesert | 87 to 100 % |
    | fjords | 64 to 82 % |
    | archipelago | 27 to 43 % |
    | volcanicIsland | 0 % |

    The demo defaults to `continental` with `shape.island: 0`, so a
    first-time viewer sees the wall.
- **Relief.** `Landform::for_extent` caps `mountain_relief` at
    `range_wavelength x 0.25`. At the default 6,144 m extent that is about
    920 m, and after erosion `alpine` peaks at 675 to 1,050 m (p99 540 to
    843 m). The preset's 2,600 m never happens on the maps people
    actually make. Real 6 km alpine valleys hold 1,500 to 2,200 m of
    relief.
- **Sea ice.** Captured at `meanTemperatureCelsius: -18` on `fjords`
    seed 2 from 1,600 m up, the sea is a regular Voronoi mosaic of
    equal-sized plates. Real pack ice has floe sizes spanning orders of
    magnitude, irregular rounded edges, long open leads, pressure ridges,
    and brash ice between floes.
- **Snow streaks.** In the same capture, steep snow-covered slopes in the
    foreground show texture stretched down the slope. Triplanar
    projection exists for rock, ice and steep snow material
    (`clipmap_render.wgsl` line 162), so the streaks likely come from a
    path that is not triplanar: the permanent-snow or weather snow-cover
    overlay, or the glacier-smoothed heights' normals. Diagnose it first.

**Cause of the pale sheet:** `common.wgsl` `terrain_height_at` clamps to
the edge sample and lowers it by only `0.08 x distance` outside the
footprint. Next to a 500 m edge, the "sea floor" stays above sea level
for about 6 km, so the ocean there is drawn as shore and foam. The
streaks come from mesh vertices clamped onto the border
(`terrain_mesh.rs`, `sample_x/sample_z .clamp(...)`).

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

The owner has approved these:

- **Size:** plan 1 may use up to 32 KB and plan 2 up to 12 KB, only for
    real value that can't be done smaller, and without lag, frame drops,
    regressions or any loss of realism. Plans 1, 2 and 2b together may
    therefore add at most 44 KB (45,056 bytes) over 237,820. They add
    39,131 today, so this plan may add at most 5,900 bytes gzipped. Aim
    for under 4 KB.
- Hyper-realistic visuals and a solid 60 FPS remain the targets.
- The approved plan 2 deviation (snowy-peak biomes instead of summit ice
    on mild maps) stays.

## Prerequisites

Plans 1 and 2 must be merged. Check that
`crates/vista_wasm/src/terrain/tectonics.rs` and `terrain/glaciers.rs`
exist, and that `BiomeKind::IceArctic` is 18.

## Design

### 1. Coasts inside the map by default, and a skirt for open edges

**Generator side** (`terrain/tectonics.rs`, `terrain/landforms.rs`):

- Add `FractalTerrainOptions.edges?: "coast" | "open"`, defaulting to
    `"coast"` for every landform.
- With `"coast"`, multiply the continent mask by a border falloff before
    stages B to D. Drainage and erosion then see the sea there, and rivers
    reach it.
    - The falloff reaches 0 at the border, over the outer
        `max(6 % of the extent, 300 m)`.
    - It is warped by the existing 2-octave warp noise with amplitude
        4 % of the extent, so the coastline is natural, not a
        rounded square.
- Re-derive the land-fraction threshold after the falloff, so each
    landform still hits its `land_fraction` (the existing test must still
    pass within ±0.05).
- **Target:** at most 2 % of border samples are land for every landform
    with `"coast"`, including `alpine` and `mesaDesert`.
- `"open"` keeps today's behaviour, for users who want land to the edge
    (for example when tiling several maps).
- `rollingHills`, `alpine` and `mesaDesert` keep their inland character.
    The coast is only a rim at the border. Mountains may still meet the
    sea in `fjords` and `alpine` (coastal cliffs), so the falloff there
    lowers the base but keeps uplift near the coast.

**Renderer side** (every map, including `"open"`, DEMs and raw
imports):

- Replace the outside-footprint rule in `common.wgsl`
    `terrain_height_at` with a skirt:
    - Outside the footprint, at distance d from it, the height falls from
        the clamped edge height to `sea_level - 60 m` over
        `SKIRT_METRES = 1500`, with a smoothstep profile, plus low
        noise from `noise_texture` (`textureSampleLevel`) of ±15 %.
    - Beyond the skirt, continue the gentle deep-water slope.
    - Pass sea level in the world uniforms if it isn't there already.
- **Terrain mesh:** instead of clamping sample coordinates at the border,
    let vertices outside the footprint keep their true positions, and
    take their heights from the same skirt function. Mirror it in Rust
    as `skirt_height(edge_height, distance, noise)`, keep it in step with
    the WGSL, and test both against the same values.
    - Outside vertices take the material weights of the nearest edge
        sample, blended towards rock over the first 300 m and towards
        sand at the waterline.
    - Normals come from the skirt heights.
    - This removes both the cliff wall and the stretched streaks.
- **Water:** the ocean over the skirt uses the skirt height for depth,
    so it reads as proper coast, then deep sea. Remove the old
    `0.08 x distance` rule.
- No trees or grass on the skirt. They are generated only inside the
    footprint.

**Demo:** add an "Edges: coast / open" select to the terrain section,
default coast.

### 2. Alpine relief on real map sizes

- In `Landform::for_extent`, replace `mountain_relief.min(range_wavelength x 0.25)`
    with a per-landform steepness allowance:
    - `alpine` and `fjords`: `min(preset, range_wavelength x 0.55)`;
    - `continental`: 0.35;
    - `mesaDesert`: 0.3;
    - `archipelago` and `volcanicIsland`: 0.4;
    - `rollingHills` stays unchanged, at 0 mountain relief.
- Then tune stream-power erodibility and hillslope diffusion for
    `alpine` and `fjords`, so the slope tests still pass: under 3 % of
    land steeper than 60 degrees, and under 5 % steeper than 45 degrees
    outside mapped cliff bands.
- **Targets** at 512 x 512 and 12 m, seeds 1 to 12:
    - `alpine`: p99 height at least 1,500 m, with `upperSnowyPeaks` or
        `lowerSnowyPeaks` present on every seed;
    - `fjords`: p99 at least 1,100 m;
    - `continental`: max from 900 to 1,800 m.
    - Add these as assertions in `terrain/realism_tests.rs`, next to the
        existing ones.
- Keep the existing test `wavelengths_and_relief_shrink_with_small_maps`
    meaningful. Update its bound to the new rule, and keep a 3 km map's
    relief proportionally lower.

### 3. Realistic pack ice

In `shaders/water.wgsl`, replace the single-scale Worley floes. Keep
everything uniform-safe (`textureSampleLevel` only), and add at most 4
extra noise lookups per ice pixel.

- **Multi-scale floes:** combine Worley cells at 3 scales (about 25 m,
    120 m and 600 m).
    - The largest defines big floes. The middle scale breaks them. The
        smallest makes brash and small cakes in the gaps.
    - The resulting floe-size distribution looks power-law, not uniform.
- **Irregular edges:** warp the Worley lookup coordinates with fbm at
    0.3 of each scale, so edges are rounded and irregular, never straight
    polygon sides. Threshold with a soft 0.02 edge.
- **Leads:** long, narrow open-water cracks from a stretched ridged noise
    along a direction that turns slowly across the scene (wavelength about
    2 km), about 5 to 40 m wide, fewer as concentration rises.
- **Pressure ridges:** thin, bright, raised lines along some floe
    boundaries (a normal bump and a whiter albedo), from the middle-scale
    Worley border distance.
- **Brash and grease ice:** between floes at high concentration, a
    darker grey slush with no specular glint, not black open water.
- **Far field:** beyond about 3 km, fade the small scale out and let
    albedo variation and leads carry the look. There must be no visible
    repeating tile, and no mipmapped mush at the horizon.
- **Colour:** snow-covered floes are white with a slight blue shadow
    side; bare floes are blue-grey `(0.62, 0.72, 0.78)`; in about 20 %
    of floes (by cell hash), the snow is thin and patchy.
- **Performance:** with sea ice on screen, the water pass may cost at
    most 0.15 ms more than the current sea ice at 1080p on a mid-range GPU
    (scale from software ratios against the terrain pass). With no
    freezable sea, the existing uniform branch still skips everything.

### 4. Snow stretched on steep slopes

1. **Reproduce:**
    `{"size":[800,450],"engine":{"biomes":{"meanTemperatureCelsius":-18}},"terrain":{"landform":"fjords","shape":null,"seed":2},"shots":[{"position":[-1500,1600,2800],"target":[0,300,0],"ground":false}]}`.
    Streaks show in the lower left foreground.
2. **Diagnose** by switching paths off one at a time in a local build:
    the permanent-snow overlay, the weather snow-cover overlay,
    `MAT_SNOW` below and above the `normal.y < 0.8` cut, `MAT_ICE`, and
    glacier-smoothed normals.
3. **Fix** at the cause:
    - A planar-projected overlay moves to the same triplanar weights as
        the material.
    - The `normal.y < 0.8` cut becomes a smooth blend from 0.9 to 0.7,
        so there is no seam where projection switches.
    - Glacier smoothing that leaves faceted normals is fixed there.
4. Keep the fix to one triplanar path, rather than adding one per
    material, to keep the shader and its cost small.

### 5. Like-for-like performance gate

- Add `scripts/visual-check/fixed-scene.mjs`. It loads a fixed heightmap,
    generated once and committed as `scripts/visual-check/fixed-512.f32.gz`
    (under 600 KB), through `loadRawHeightmap` with fixed options and a
    fixed camera. It then prints `gpuPassTimesMs` averaged over 6 frames,
    after 3 warm-up frames.
- Record the numbers before this plan's changes and after, and put both
    in the report. No pass may grow by more than 5 %, except water with
    sea ice on screen (section 3).
- Later plans reuse this gate. Mention it in the ground rules of plans 3
    to 11, which this plan's commit updates.

## Public API

Additive:

```ts
export interface FractalTerrainOptions {
  // ...existing...
  /**
   * "coast" (default) keeps the land inside the map, ringed by sea.
   * "open" lets land run to the map edge, for tiling several maps.
   */
  edges?: "coast" | "open";
}
```

- Rust: `TerrainEdges`, with serde camelCase and a `Coast` default.
    Unknown values are rejected with a message listing the valid ones.
- Document it in `docs/terrain-data.md` (landforms) and
    `docs/options-reference.md`.
- `CHANGELOG.md` gets a Changed entry for edges, relief and sea ice, and
    a Fixed entry for the edge wall, the pale sheet and the stretched snow.

## Steps

1. Build the fixed-scene gate and record the "before" numbers.
2. Add the edge falloff in the generator, `edges`, the tests, the demo
    select and docs.
3. Add the skirt: WGSL, the Rust mirror, mesh vertices outside the
    footprint, water depth and tests.
4. Change the relief allowance and retune `alpine` and `fjords`, with
    new realism assertions.
5. Rewrite the pack ice.
6. Diagnose and fix the snow stretching.
7. Record the "after" numbers, check the size, capture the verification
    shots, then commit and push.

## Tests

- **Border land:** at most 2 % of border samples are land for every
    landform with `edges: "coast"` (seeds 1 to 12). `"open"` reproduces
    today's heights bit-identically.
- **Land fraction:** still within ±0.05 with `"coast"`.
- **Relief:** the targets in section 2, and the existing slope, peak,
    drainage and smoothness tests keep passing.
- **Skirt:**
    - `skirt_height` at distance 0 equals the edge height;
    - it is below sea level by 1,500 m;
    - it is monotonic for a flat edge without noise;
    - the Rust and WGSL constants match (a test reads the WGSL source and
        checks `SKIRT_METRES`).
- **Mesh:** no vertex outside the footprint shares a position with an
    edge vertex, so there are no clamped duplicates.
- **Sea ice:** a CPU port of the floe mask over a 4 x 4 km area at
    concentration 0.8 gives:
    - an ice fraction within ±0.08 of 0.8;
    - a floe-area distribution (connected components) spanning at least
        two orders of magnitude;
    - at least one lead longer than 400 m.

## Verification

Open every capture:

- **Continental seed 2:** top-down from 7,000 m and oblique from 1,500 m.
    Sea all around, a natural coastline, no wall, no pale sheet, no
    streaks.
- **`edges: "open"` alpine:** the same oblique. The land runs to the
    edge, then the skirt descends into sea, with no wall and no void.
- **Alpine seed 2:** the oblique from `[-1500, 1500, 2600]`. High, eroded
    ranges with snowy peaks, not domes.
- **Pack ice** at -18 °C, `fjords` seed 2, from 1,600 m and from 30 m.
    Irregular floes of many sizes, leads, ridges, brash, and no visible
    tiling.
- **The snow-streak shot:** no stretching.
- **Reference shots** (default, rain, forest close-up): unchanged apart
    from the edge coastline.
- **Performance gate:** within 5 % per pass.
- **Size:** plans 1, 2 and 2b total at most +44 KB over 237,820 bytes
    gzipped. Report the exact figure.

## Budgets

- WASM: at most +5,900 bytes gzipped (aim under 4 KB).
- GPU: no pass more than 5 % slower on the fixed scene; sea ice at most
    +0.15 ms.
- Generation time: at most +20 ms at 512 x 512.

## Out of scope

Rivers, lakes and lake ice (plan 3, adjusted to freeze lakes), and
vegetation (plans 4 to 6).
