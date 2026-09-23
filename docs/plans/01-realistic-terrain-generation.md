# Plan 1: Realistic terrain generation

## Goal

A random map should look like real land shaped by geology and water, not
a field of spikes:

- broad continental shapes;
- mountain ranges with eroded ridges and dendritic valleys;
- wide, smooth plains and natural coastlines;
- scree slopes below cliffs, and alluvial fans where valleys open out.

Peaks are rare and sit on ranges. Simple landform presets choose the
character of a map. Generation takes at most about 2 seconds at the
default 512 x 512 on a mid-range GPU.

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
- Tiny files: the gzipped `dist/pkg/vista_wasm_bg.wasm` is 237,820 bytes
    at the start of this work. Each plan states how much it may add. Check
    with `gzip -9 -c dist/pkg/vista_wasm_bg.wasm | wc -c`. Generate data
    procedurally at start-up instead of embedding it.

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

- Default look: varied and geology-led. Continents first, then uplifted
    ranges carved by water, with wide plains and coastlines.
- Generation time: up to about 2 s for 512 x 512 on a mid-range GPU,
    with progress reporting.
- Add a high-level `landform` preset. The low-level knobs stay for
    experts.
- The same seed may produce a different, better map. No "classic"
    opt-out is needed, because 1.1.0 is unreleased.

## Prerequisites

None. This is the first plan.

## Why maps look spiky today

Read these before changing anything. The fixes below target each cause.

1. `crates/vista_wasm/src/terrain/fractal.rs`, `octave_noise`: with the
    demo's default `ridged` kind, every octave is ridged (`1 - |n| * 2`).
    That includes the highest frequencies, so ridges are stacked on
    ridges at every scale, which reads as spikes. Real terrain is ridged
    at range scale and smooth at small scale on valley floors.
2. `value_noise` is value noise on a square lattice. Its grid-aligned
    artefacts show as parallel ridges and plateaus.
3. Frequencies are relative to map size (`x * frequency * 6.0` over the
    unit square). Features therefore have no physical size: a 6 km map
    and a 60 km map have the same number of peaks.
4. Detail amplitude is the same everywhere. Plains get as much
    high-frequency roughness as summits.
5. `crates/vista_wasm/src/shaders/hydraulic_erosion.wgsl` and its CPU
    reference in `terrain/erosion.rs` are not hydraulic erosion. They
    move material towards lower neighbours, which acts like diffusion.
    Nothing carries water or sediment, so they can never carve valleys or
    build fans. They mostly blur.
6. `crates/vista_wasm/src/shaders/terrain_noise.wgsl` is an unused
    placeholder (a `sin * cos` pattern). Confirm with
    `grep -rn terrain_noise crates` that nothing dispatches it. Remove the
    file and its entry in `render/pipelines.rs`.

## Design

The generator runs in four stages. Stages A and B run on the CPU in WASM
at a coarse resolution. Stage C adds detail at full resolution on the
CPU. Stage D erodes on the GPU at full resolution. Native (non-WASM)
builds run stage D's CPU reference, so `cargo test` covers the whole
pipeline at small sizes.

### Stage A: tectonic base (coarse grid, 256 x 256 or the map size if smaller)

- Replace `value_noise` with seeded 2D gradient noise: OpenSimplex2-style
    or improved Perlin with a hashed gradient table. Put it in a new
    module, `terrain/noise.rs`. Unit test that its output is in [-1, 1],
    deterministic per seed, and free of lattice artefacts. For the
    artefact test, check that the mean of |n| sampled on lattice points
    differs from its mean at random points by less than 0.05. Value noise
    fails this test.
- Work in metres. A sample's world position is `(x, z)` in metres, and
    every wavelength below is in metres.
- Continents:
    - `continent(p)` is a 3-octave fBm with wavelength `landform.continent_wavelength`.
    - It is domain-warped by a 2-octave fBm of amplitude 0.35 x that wavelength.
    - The value is remapped by a smoothstep around a threshold, chosen so
        the land fraction matches `landform.land_fraction`. Find the
        threshold by sorting samples on the coarse grid, so the land
        fraction is exact.
- Uplift:
    - `uplift(p)` marks the mountain ranges. It is a ridged 2-octave noise
        (ridged only here) with wavelength `landform.range_wavelength`,
        warped strongly (amplitude 0.5 x wavelength) so ranges bend and
        branch.
    - It is multiplied by a low-frequency range mask, so only
        `landform.range_coverage` of the land carries ranges.
    - Uplift is 0 at sea and fades in over the first 20 % of the distance
        inland, except for landforms that ask for coastal cliffs (fjords,
        volcanic island).
- Base elevation = `sea_floor + continent * lowland_relief + uplift * mountain_relief`.
    The relief values come from the landform, in metres.

### Stage B: stream-power erosion on the coarse grid

This carves the dendritic valley network and ridge spurs that make
mountains look eroded.

- Implement the implicit stream-power solver in a new module,
    `terrain/stream_power.rs`. It follows Braun and Willett (2013): for
    each iteration,
    1. Fill depressions with priority-flood plus epsilon. There is
        already a flood-fill `FloodCell` in `render/water.rs`; move it to
        a shared `terrain/drainage.rs` and reuse it there.
    2. Compute the steepest-descent receiver of every cell (D8).
    3. Build the stack order from the outlets upwards.
    4. Accumulate drainage area A.
    5. Walk the stack from outlets upwards and solve implicitly:
        `h_i = (h_i + dt * (U_i + K * A^m / dx * h_r)) / (1 + dt * K * A^m / dx)`.
        Use m = 0.45 and n = 1, uplift U from stage A, erodibility K from
        the landform, and dt chosen so that 40 iterations reach a steady
        relief.
    6. Apply hillslope diffusion with coefficient `landform.hillslope_diffusion`,
        one explicit step.
- Sea and lake outlets are boundary cells with fixed height.
- Keep `drainage_area` from the last iteration. Stage D uses it, and
    later plans reuse it for rivers and moisture. Store it in the height
    map's aux data (see the data-model section).
- Budget: at most 150 ms in WASM at 256 x 256. Measure it in a browser
    and log it in the demo's status line with the other generation phases.

### Stage C: full-resolution detail

- Upsample stage B's heights bicubically to the full size.
- Add detail with derivative-damped fBm: accumulate the gradient of the
    octaves so far, and scale each new octave by `1 / (1 + k * |grad|^2)`.
    Valleys and plains stay smooth while ridges keep crisp detail.
- Scale detail amplitude by a roughness mask:
    `mix(plains_roughness, mountain_roughness, smoothstep(slope and uplift))`.
- Keep the existing `NoiseOptions`:
    - `octaves`, `gain`, `lacunarity` and `warp` control this detail layer.
    - `kind` picks its flavour: `simplex` smooth, `ridged` ridged only
        above the range mask, `hybrid` in between, `classic` the old
        billowy look, `island`/`canyon`/`cratered` their existing shape
        masks.
    - This keeps every existing option meaningful.
- The existing `TerrainShapeOptions` (`island`, `basin`, `canyon`, `crater`,
    `terrace`) apply after this stage, as today.

### Stage D: full-resolution hydraulic and thermal erosion (GPU)

- Replace `hydraulic_erosion.wgsl` with a virtual-pipe shallow-water
    erosion model (Mei, Decaudin and Hu, 2007), split into these compute
    passes per iteration, all at full resolution:
    1. Rain: add water, weighted towards high drainage area from stage B,
        so existing valleys keep carving.
    2. Outflow flux: pipe model with gravity and a pipe cross-section.
    3. Water height and velocity update.
    4. Erosion and deposition: sediment capacity
        `C = Kc * sin(slope) * |v| * limit(depth)`. Erode `Ks * (C - s)`
        when under capacity, deposit `Kd * (s - C)` when over it. This
        builds alluvial fans where valleys open onto plains.
    5. Semi-Lagrangian sediment advection.
    6. Evaporation.
- Replace `thermal_erosion.wgsl` with a talus-angle model: material moves
    from a cell to lower neighbours in proportion to the excess over
    `talus_angle_degrees`, using the gather formulation (each cell writes
    only its own output), as the current shader does, to stay race-free.
    This makes scree slopes below cliffs.
- Run the multi-scale schedule: 60 % of iterations at half resolution
    (downsampled, then the height difference upsampled back), then 40 %
    at full resolution. Large features settle cheaply; fine gullies form
    at full resolution.
- Budget:
    - default `ErosionOptions.quality: "preview"` at most 0.6 s at 512 x 512;
    - `"balanced"` at most 1.2 s;
    - `"high"` at most 2 s (the demo default);
    - `"offline"` unbounded.
    - Report progress through the existing `"progress"` event with phase
        `"erosion"`, at least every 10 %.
- The CPU reference in `terrain/erosion.rs` implements the same passes
    with the same constants, for native builds and tests.
- Write back into the height map with the existing async readback in
    `render/erosion_compute.rs`.

### Landform presets

Add `FractalTerrainOptions.landform?: LandformKind` (serde default
`"continental"`), with a Rust table in `terrain/landforms.rs`. Every preset
has these fields:

- `land_fraction`
- `continent_wavelength` (m)
- `range_wavelength` (m)
- `range_coverage`
- `sea_floor` (m)
- `lowland_relief` (m)
- `mountain_relief` (m)
- `erodibility`
- `hillslope_diffusion`
- `plains_roughness`
- `mountain_roughness`
- `talus_angle_degrees`
- `rain`
- `terrace`
- `coastal_cliffs`
- `glacial` (0 to 1)

Wavelengths are clamped to the map extent (continent at most 1.5 x
extent, range at most 0.6 x extent), so a small map still gets a coherent
composition.

| Landform | Character | Key values |
| --- | --- | --- |
| `continental` (default) | Mixed plains, hills and one or two ranges | land 0.7, relief 1400 m |
| `alpine` | High, heavily eroded ranges, deep valleys | land 0.95, relief 2600 m, glacial 0.4 |
| `rollingHills` | Gentle downs and broad vales, no ranges | land 0.9, relief 250 m, range coverage 0 |
| `archipelago` | Many islands of varied size | land 0.35, relief 600 m |
| `mesaDesert` | Plateaus, buttes and canyons | terrace 0.7, low rain, high talus |
| `fjords` | Steep ranges cut by flooded U-shaped glacial valleys | glacial 1, coastal cliffs |
| `volcanicIsland` | A central cone with a caldera, radial gullies and a reef shelf | a cone profile added in stage A |

- Glacial carving (`glacial > 0`): during stage B, cells above a snowline
    use `n = 2` erosion with a wider receiver kernel (a 3 x 3 average of
    receivers). This gives U-shaped valleys, which the sea floods in
    `fjords`.
- `ErosionOptions` keeps every field. Where a field is unset, the
    landform supplies the default value.

### Data model

Add to `HeightMap` in `terrain/heightmap.rs` an `aux: TerrainAux` field
holding the drainage area (a `Vec<f32>`, empty when not computed). Later
plans reuse it. Keep `HeightMap` cloning cheap: `aux` is an
`Option<Arc<...>>`.

## Public API

Additive only.

```ts
export type LandformKind =
  | "continental"
  | "alpine"
  | "rollingHills"
  | "archipelago"
  | "mesaDesert"
  | "fjords"
  | "volcanicIsland";

export interface FractalTerrainOptions {
  // ...existing fields...
  /** Character of the land. Defaults to "continental". */
  landform?: LandformKind;
}
```

- Rust: `LandformKind` in `crates/vista_types/src/lib.rs`, with
    `#[serde(rename_all = "camelCase")]` and `#[default] Continental`.
- Validation in `config.rs`: unknown values are rejected by serde with a
    message listing the valid names. Add a test.
- Bump `GENERATOR_VERSION` in `fractal.rs` to `vistawasm-fractal-0.2.0`,
    so that caches and metadata record the change.

## Steps

1. Capture the three reference shots from the ground rules, plus a
    top-down shot of seeds 1, 2 and 3:
    `{"terrain":{"seed":N},"camera":{"position":[0,9000,1],"target":[0,0,0]}}`.
    Keep them for comparison.
2. Add `terrain/noise.rs` (gradient noise with tests) and switch
    `fractal.rs` to it.
3. Add `terrain/drainage.rs` (priority-flood, receivers, stack,
    accumulation) by moving the existing code out of `render/water.rs`.
    Rivers must keep working: run their tests.
4. Add `terrain/landforms.rs` and `LandformKind` in the types, TypeScript
    and validation.
5. Implement stage A and stage B (`terrain/stream_power.rs`), with tests.
6. Implement stage C in `fractal.rs`.
7. Replace the erosion model: WGSL passes, `render/erosion_compute.rs`
    and the CPU reference in `terrain/erosion.rs`. Keep the async
    readback and progress events.
8. Remove `terrain_noise.wgsl` if it is unused.
9. Demo:
    - add a "Landform" select at the top of the terrain section, default
        Continental;
    - move "Noise kind", "Octaves" and the other low-level controls into
        an "Advanced terrain" sub-section;
    - set the demo's erosion default to `"high"`;
    - show the generation phases and times in the status line.
10. Docs:
    - `docs/terrain-data.md`: a new "Landforms" section with a top-down
        image per landform (capture them with the harness, save them as
        JPEG under 60 KB in `docs/images/`, and keep the total under
        400 KB);
    - `docs/options-reference.md`;
    - `docs/architecture.md` "Terrain generation" section, rewritten for
        the four stages;
    - `CHANGELOG.md`.
11. Verify (below), then commit and push.

## Tests

Rust, in `crates/vista_wasm/src/terrain/`. Run each on seeds 1 to 12 at
256 x 256 and 12 m per sample, on the full CPU pipeline:

- **Not spiky:**
    - fewer than 5 % of land cells steeper than 45 degrees, and fewer
        than 0.5 % steeper than 60 degrees;
    - `alpine` and `fjords` may reach 3 % above 60 degrees;
    - `rollingHills`: none steeper than 30 degrees.
- **Few isolated peaks:** strict 8-neighbour local maxima on land, at
    most 4 per km^2 (today's ridged default is far above this; record
    the old value in the test comment).
- **Drains:** after priority-flood on the final map, at least 97 % of
    land cells reach the sea or a basin with at least 24 cells (a lake).
- **Smooth lowlands:** on cells with slope under 5 degrees, the mean
    absolute Laplacian is below `0.02 * metres_per_sample`.
- **Land fraction:** within ±0.05 of the landform's `land_fraction`.
- **Determinism:** the same seed and options give bit-identical heights.
- **Noise:** in [-1, 1], deterministic, and passes the lattice-artefact
    check.
- **Stream power:** a tilted plane with uniform uplift develops channels,
    with drainage area following Hack's law (exponent 0.5 to 0.6).
- **Erosion CPU reference:**
    - total sediment is conserved within 1 % (eroded minus deposited
        minus evaporated sediment);
    - a V-shaped valley with an open end develops a fan, with net
        deposition in the lowest 20 % of cells.

TypeScript: a type test that `landform` accepts every documented value
and rejects others (`// @ts-expect-error`).

## Verification

- Top-down shots of seeds 1, 2 and 3 for every landform. Look at each:
    no spikes, and visible dendritic valleys on `continental` and
    `alpine`.
- The same seeds from a low camera: `{"camera":{"position":[0,300,2500],"target":[0,200,0]}}`.
- Browser generation times, printed by the harness log line
    `generation ms`. Swiftshader is slow; also record the stage
    breakdown, and state the expected mid-range GPU time in the report
    from the per-stage timings.
- The reference shots still render with no errors.

## Budgets

- WASM growth: at most +10 KB gzipped.
- No change to per-frame GPU cost.
- Generation: as listed in stage D.

## Out of scope

River rendering, lakes and waterfalls (plan 3) and arctic climate
(plan 2). This plan only provides the drainage area they build on.
