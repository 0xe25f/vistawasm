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
5. **Raindrops on the lens are cut off** with straight, flat edges, and
    developers can't control how many there are or how big they are.
    Reported by the owner with a screenshot.

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
- **Lens drops** (item 5) are configurable by the developer and exposed
    in the demo:
    - how many drops are on the lens at any one time;
    - the minimum and maximum drop size.
    - They must never be cut off. This is existing-feature work, not plan
        1 or 2 work, so it has its own budget of at most 3,000 bytes
        gzipped, outside the 44 KB cap above.

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

### 5. Lens drops: configurable, and never cut off

**Why they are cut off today.** `present_main` in
`crates/vista_wasm/src/shaders/atmosphere.wgsl` splits the screen into
square cells (16 per screen height for beads, 7 for running drops), and
each pixel only tests the drop of its own cell.

- A drop's centre can sit 0.2 of a cell from an edge, while its radius
    reaches `(0.12 + 0.2) x 1.3 ≈ 0.42` of a cell. Running drops are also
    squashed to 0.8 vertically, so they reach about 0.52 of a cell.
- Any drop that overhangs its cell is sliced off along the cell's
    straight edge.
- Running drops slide down independent columns, so a neighbouring
    column's drop can't be found by a fixed neighbour lookup.
- The owner's screenshot shows exactly this: flat tops and vertical cuts.
- The hashed grid also cannot hold an exact number of drops, or honour a
    size range.

**Design: a small drop simulation, binned into screen tiles.**

- **The CPU owns the drops.** Add `crates/vista_wasm/src/lens_drops.rs`
    with `LensDrops`, which holds up to `MAX_LENS_DROPS = 512` drops.
    - Each drop has a centre (in screen-height units, so it stays round
        on any aspect ratio), a radius, a velocity, an age and a lifetime.
    - It advances with the engine's smoothed time step from `pacing.rs`
        (`FrameClock`), and is deterministic from the weather seed.
    - **Target count** = `lensDropCount x rain intensity at the camera`,
        where rain intensity is 0 to 1 (the existing
        `weather.x x weather3.w x weather3.z` product, computed on the
        CPU). Light rain has fewer drops; the option is the count at full
        rain.
    - New drops spawn at a rate that keeps the live count at the target,
        at uniformly random positions. The spawn rate is spread over time,
        so it never pops.
    - Diameter is drawn between `lensDropMinSize` and `lensDropMaxSize`,
        skewed towards small (the square of a uniform random), as real
        drop populations are.
    - **Behaviour follows size**, as it does on glass:
        - drops in the smaller 60 % of the range are beads: they stay
            still, and evaporate by shrinking over 4 to 9 s;
        - larger drops run: they slide down at a speed that rises with
            size (0.15 to 0.6 screen heights per second), with a small
            sideways wander;
        - a running drop absorbs beads it touches, growing by area
            conservation up to `lensDropMaxSize`;
        - a running drop leaves a trail of tiny beads (one every 0.6
            diameters, at 0.3x its diameter). These count towards the
            total, and the spawner makes room for them.
        - A drop is removed when it leaves the screen, or when it has
            evaporated.
    - When the rain stops, no new drops spawn. The existing ones run off
        or evaporate naturally.
- **Upload and binning, every frame:**
    - Drops go into a storage buffer as `vec4<f32>`: centre x, centre y,
        radius, and w, which packs kind and fade.
    - Bin them into a screen tile grid with 32 columns and 18 rows,
        rescaled to the aspect ratio so tiles are roughly square.
    - Every drop is added to every tile its bounding circle, plus the
        refraction margin, touches. That is what guarantees a drop is never
        clipped.
    - Store a `u32` offset and count per tile, and a flat index list.
    - Each tile holds at most 24 drops. If more overlap, keep the 24
        largest, since tiny beads hidden under big drops don't show.
    - The CPU cost stays under 0.1 ms per frame at 512 drops.
- **Shader:** in `present_main`, replace the two hashed-cell loops.
    - Each pixel finds its tile, loops over that tile's drops, and
        accumulates the same bend, rim and glint as today. Keep today's
        look: a flipped, magnified refraction, a darker rim and a
        highlight.
    - Keep the running drops' slightly taller shape, from the velocity
        direction.
    - The loop count comes from a storage buffer, which is uniform-safe,
        because the only sampling is the existing `textureSampleLevel`
        after the loop.
    - With lens drops off or no rain, the existing uniform branch skips
        everything, and no buffers are touched.
- **Bindings:** add the drop and tile buffers to the present pass's bind
    group only. No other pipeline changes.
- **Options,** added to `WeatherOptions` next to `lensDrops`, flat like
    the other weather fields:

    | Option | Type | Default | Range | Meaning |
    | --- | --- | --- | --- | --- |
    | `lensDropCount` | `number` | 60 | 0 to 512, integer | Drops on the lens at once in full rain. It scales with the rain's intensity. |
    | `lensDropMinSize` | `number` | 0.008 | 0.002 to 0.2 | Smallest drop diameter, as a fraction of the canvas height (0.008 is about 9 px at 1080p). |
    | `lensDropMaxSize` | `number` | 0.05 | 0.002 to 0.2 | Largest drop diameter, as a fraction of the canvas height. It must be at least `lensDropMinSize`. |

    - Sizes are fractions of the canvas height, so drops look the same
        at 1080p, 4K and on phones, whatever the device pixel ratio.
    - Validate them in `config.rs`. `lensDropMinSize > lensDropMaxSize`
        is rejected with a message naming both values.
    - The defaults give a look close to today's, for users who never set
        them.
- **Demo,** in the weather section under the existing "Lens drops"
    checkbox:
    - a "Drops on the lens" slider (0 to 300, step 1, readout);
    - "Smallest drop" and "Largest drop" sliders (0.2 % to 10 % of screen
        height, readouts in %);
    - they are enabled only while "Lens drops" is checked;
    - if the minimum is dragged above the maximum, the maximum follows,
        and vice versa.
- **Docs:**
    - `docs/weather.md`: a lens drops section with the options, the
        size-follows-behaviour rule, and an example;
    - `docs/options-reference.md`;
    - `CHANGELOG.md` (Added: lens drop count and sizes; Fixed: drops cut
        off at straight edges).

### 6. Like-for-like performance gate

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

export interface WeatherOptions {
  // ...existing...
  /** Drops on the lens at once in full rain, 0 to 512. Scales with rain intensity. Defaults to 60. */
  lensDropCount?: number;
  /** Smallest lens drop diameter as a fraction of the canvas height, 0.002 to 0.2. Defaults to 0.008. */
  lensDropMinSize?: number;
  /** Largest lens drop diameter as a fraction of the canvas height, 0.002 to 0.2. Defaults to 0.05. */
  lensDropMaxSize?: number;
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
7. Add lens drops: the options and validation, `lens_drops.rs`, binning,
    buffers, the shader rewrite, demo controls and docs.
8. Record the "after" numbers, check the size, capture the verification
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
- **Lens drops** (`lens_drops.rs`, native tests):
    - After 20 s of full rain at `lensDropCount: 120`, the live count is
        within ±5 % of 120. At half intensity it is within ±5 % of 60.
        With no rain, it is 0 within 10 s.
    - Every spawned drop's diameter is within the minimum and maximum
        sizes, and so is every merged drop.
    - Only drops in the upper 40 % of the size range move.
    - **Never clipped:** for random drop sets (1,000 cases, including
        drops overlapping tile edges and screen corners), a CPU port of
        the shader's per-tile evaluation gives exactly the same coverage
        mask as brute force over all drops, on a 640 x 360 pixel grid. This
        test is the proof that no drop is cut off.
    - The per-tile cap keeps the largest drops.
    - Deterministic for the same seed and time steps.
- **Validation:** out-of-range count and sizes, and min above max, are
    rejected. TypeScript type tests cover the three new options.

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
- **Lens drops:** rain with lens drops at 1280 x 720, with
    `lensDropCount` 40 and 250, and sizes 0.01 to 0.08. Drops are fully
    round with no flat edges (look closely at tile and cell boundaries,
    and at the top of the screen). The count visibly follows the setting.
    Then capture a 390 x 844 portrait: drops stay round, and are sized
    relative to the screen height.
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
- Lens drops: WASM at most +3,000 bytes gzipped, separate from the plan
    1 and 2 cap. The present pass costs at most 0.25 ms at 1080p on a
    mid-range GPU with 120 drops (report the software-renderer ratio
    against the terrain pass, before and after). CPU time at most 0.1 ms
    per frame.

## Out of scope

Rivers, lakes and lake ice (plan 3, adjusted to freeze lakes), and
vegetation (plans 4 to 6).
