# Plan 8: Cloud bases and mid-level clouds

## Goal

Low cumulus bases look like the real thing:

- mostly flat, because they form at a condensation level;
- but uneven from cloud to cloud and within each cloud, with
    cotton-wool lumps;
- softly shaded grey underneath, with wispy fringes;
- never spiky.

An optional mid-level (alto) layer at 2 to 6 km adds:

- **altocumulus:** rippled "mackerel sky" patches;
- **altostratus:** a grey veil that turns the sun into a watery disc.

Both are driven by weather presets and cast shadows onto the low clouds
and the ground. The mid layer is cheap.

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

- Bases are photo-like and mostly flat:
    - the base height varies by ±5 to 10 % of the layer thickness across
        each cloud and between clouds;
    - edges and undersides are eroded into lumps;
    - there's a darker grey underside and wispy fringes.
- Mid-level clouds: both altocumulus and altostratus, each with its own
    amount, height and speed, driven by the presets.
- Budget: a cheap 2.5D layer, not a full raymarch. Its shadows fall on
    the low clouds. About 0.3 ms at 1080p on a mid-range GPU.
- No separate cloud size or spacing controls. Coverage is enough.

## Prerequisites

- **Plan 7** (sky and weather presets) must be merged. Check for
    `crates/vista_wasm/src/weather/presets.rs`, `WeatherPreset`, and the
    regional weather map feeding `cloud_shape`.
- If it is missing, carry out plan 7 (and its prerequisites) first.

## Why cloud bases look flat today

In `crates/vista_wasm/src/shaders/atmosphere.wgsl`, `cloud_shape`:

- The base offset `bottom` is non-zero only when `ragged > 0.001`, which
    is the rain-laden `raggedBase`.
- In fair weather, `bottom = 0`, and `heap` ramps from 0 to 1 over
    `layer_h` 0 to 0.07. Every cloud's base is the same horizontal plane.
- `cloud_density`'s detail erosion (`mix(1 - fbm, fbm, saturate(h * 4))`)
    affects the base only slightly, and in a wispy way, not a lumpy one.

## Design

### 1. Uneven, lumpy bases

All changes are in `cloud_shape` and `cloud_density`. Keep them cheap:
at most one extra 2D noise sample per shape evaluation, and no extra 3D
samples.

- **Per-cloud base height:** add a base offset from a 2D noise at about
    1.8 km wavelength, sampled once per shape evaluation from
    `noise_texture` with `textureSampleLevel`, advected with the wind:
    - `base_offset = (n - 0.5) x 2 x base_variation`, where
        `base_variation` defaults to 0.07 of the layer thickness;
    - so neighbouring clouds sit tens of metres apart in base height.
- **Rounded underside:** raise the base towards each cloud's edge. The
    local base becomes `bottom + base_offset + edge_lift x (1 - low)`,
    where `low` is the existing coarse cloud field and `edge_lift` is
    0.035 of the layer thickness. Bases curl up gently at their rims,
    instead of ending in a sharp flat edge.
- Combine these with the existing `ragged` scud term, which still adds on
    top in rain.
- **Lumpy erosion:** in `cloud_density`, within the lowest 18 % of each
    cloud's own height (from its local base), switch the erosion from
    wispy (`1 - fbm`) to billowy, using the Worley octave (`detail.g`)
    inverted, at 0.6x the top's strength. The underside gets soft
    cotton-wool lumps: bumps of about 30 to 80 m at a 560 m texture
    period, keeping the base mostly flat. Fringes at the very edges
    (`shape` under 0.2) keep the wispy erosion.
- **Underside shading:** in `cloud_ambient`, the base ambient keeps the
    ground bounce. Add a height-within-cloud occlusion term:
    `mix(0.55, 1, smoothstep(0, 0.25, local_h))`. The underside reads a
    soft mid-grey in sunny weather without darkening the sides. Rain
    darkening (`clouds3.z`) stays on top.
- **Parameters:** add these to `CloudsOptions` and to `WeatherPreset`:
    - `baseVariation`: 0 to 0.2, default 0.07;
    - `baseLumpiness`: 0 to 1, default 0.6.
    - Pass them through the existing cloud uniforms, appending a `vec4`
        if no spare lanes exist.

### 2. The mid-level layer (2.5D)

- The layer is one slab at height `H = altoHeightMetres` (default 4200
    m, clamped between the top of the cumulus layer + 300 m and the
    cirrus height - 500 m), with thickness `T`:
    - altocumulus: 250 m;
    - altostratus: 600 m.
- For a view ray, intersect the slab's mid-plane, as cirrus does, at
    point `q`.
- **Altocumulus field:** rippled cloudlets.
    - The base pattern is Worley cells about 180 m across. Threshold
        them into cloudlets, with gaps controlled by amount.
    - Multiply by a ripple band: `0.5 + 0.5 x sin(dot(q, wave_dir) x 2π / 900 m + warp)`,
        with the direction perpendicular to the layer wind and a small
        noise warp. The bands give the mackerel look.
    - Coverage modulation comes from a 12 km noise, so patches come and
        go.
    - All samples are `noise_texture` 2D lookups with `textureSampleLevel`:
        three of them.
- **Altostratus field:** a smooth veil.
    - The density is `amount x (0.7 + 0.3 x fibrous)`, where `fibrous` is
        a wind-stretched noise (as cirrus does) at 6 km.
    - It thins towards the edges of the regional coverage.
- **Optical depth:** `tau = density x k x T / max(ray.y, 0.05)`, where k
    is chosen so that altostratus at amount 1 has a transmittance of
    about 0.08 overhead.
    - Opacity is `1 - exp(-tau)`.
    - Fade towards the horizon as cirrus does, `smoothstep(0.01, 0.12, ray.y)`,
        blending into the sky haze.
- **Lighting:**
    - Sun: a cheap self-shadow from one extra density sample offset
        towards the sun by `T / sun.y` in xz, giving `exp(-tau_sun)`.
    - A Henyey-Greenstein phase mix (g = 0.6 and isotropic), and a powder
        term, for silver edges near the sun.
    - Ambient: the sky irradiance, darkened on the underside by
        thickness.
    - Altostratus in front of the sun gives a watery disc: the sun disc
        is attenuated by the transmittance but not blurred away, plus a
        soft corona glow within 3 degrees scaled by
        `(1 - transmittance) x transmittance`.
- **Layer order:** the cloud pass composites back to front:
    - from below the alto layer: cirrus, then alto, then cumulus;
    - above it: cumulus, alto and cirrus in reverse.
    - It lives in the existing cloud pass at cloud resolution
        (`resolutionScale`), with temporal reuse when on. It is cheap
        because it is 2D.
- **Shadows onto the low clouds:** in the cumulus march, compute the
    alto transmittance once per primary sample along the sun direction,
    from the sample up to the alto plane (one 2D lookup), and multiply
    the sun light by it. Not per light sample.
- **Shadows onto the ground:** multiply `cloud_shadow(position)` in
    `common.wgsl` by the alto transmittance (one lookup).
- **Motion:** each alto type drifts with `altoSpeed` (in units of 15 m/s,
    like `cirrusSpeed`), along the layer wind direction, which is the
    resolved wind veered 20 degrees. The drift offset is integrated on
    the CPU and passed as a uniform, as cirrus does.

### 3. Presets

Add `altocumulus`, `altostratus`, `altoHeightMetres` and `altoSpeed` to
`WeatherPreset` and `CloudsOptions`. Built-in values:

| Preset | altocumulus | altostratus |
| --- | --- | --- |
| clear | 0 | 0 |
| fewClouds | 0.15 | 0 |
| partlyCloudy | 0.2 | 0 |
| brokenClouds | 0.45 | 0.1 |
| overcast | 0.1 | 0.65 |
| mist, fog | 0 | 0.3 |
| lightRain | 0 | 0.8 |
| rain, heavyRain | 0 | 0.95 |
| storm | 0.1 | 0.5 |
| snow | 0 | 0.85 |
| blizzard | 0 | 0.9 |

Blending and the regional map apply as for other fields. The regional
coverage modulates alto coverage by ±30 %.

## Public API

Additive:

```ts
export interface CloudsOptions {
  // ...existing...
  /** Variation of low-cloud base height, 0 to 0.2 of the layer thickness. Defaults to 0.07. */
  baseVariation?: number;
  /** Cotton-wool lumps under low clouds, 0 to 1. Defaults to 0.6. */
  baseLumpiness?: number;
  /** Rippled mid-level patches, 0 to 1. Defaults to 0. */
  altocumulus?: number;
  /** Grey mid-level veil that dims the sun, 0 to 1. Defaults to 0. */
  altostratus?: number;
  /** Height of the mid-level layer in metres, 2000 to 7000. Defaults to 4200. */
  altoHeightMetres?: number;
  /** Mid-level drift speed in units of 15 m/s, 0 to 4. Defaults to 1. */
  altoSpeed?: number;
}

export interface WeatherPreset {
  // ...existing...
  baseVariation?: number;
  baseLumpiness?: number;
  altocumulus?: number;
  altostratus?: number;
  altoHeightMetres?: number;
  altoSpeed?: number;
}
```

Validation covers every range above, in `config.rs` and the preset
validator.

## Steps

1. Capture before shots:
    - fair-weather cumulus from below, looking up at 30 degrees:
        `{"set":{"setWeather":{"enabled":true,"state":"partlyCloudy","autoCycle":false,"transitionSeconds":0.1}},"camera":{"position":[0,200,0],"target":[400,900,2000]}}`;
    - the same with a low sun: `setSun` elevation 12 degrees;
    - a side view from 1500 m altitude towards a cloud 8 km away;
    - the reference shots.
    Record the clouds pass GPU time.
2. Add base variation, the rounded underside, lumpy base erosion and
    underside occlusion, with options and uniforms.
3. Add the alto layer function, lighting, the watery sun and layer
    ordering in the cloud pass.
4. Add alto shadows onto cumulus and onto the ground.
5. Add the preset fields and built-in values, the options, validation and
    TypeScript.
6. Demo, in the clouds section:
    - sliders for base variation, base lumpiness, altocumulus,
        altostratus, alto height and alto speed, with readouts;
    - quick buttons "Mackerel sky" (altocumulus 0.7) and "Veiled sun"
        (altostratus 0.7).
7. Docs:
    - `docs/sky-atmosphere-and-weather.md` (bases, the alto layer,
        watery sun);
    - `docs/weather.md` (preset fields);
    - `docs/options-reference.md`;
    - `docs/architecture.md` (cloud pass order and costs);
    - `CHANGELOG.md`.
8. Verify, then commit and push.

## Tests

- A CPU port of the base-height function (base offset plus edge lift)
    over a 20 x 20 km grid:
    - offsets span at least 60 % of ±`baseVariation`;
    - neighbouring clouds 2 km apart differ in base height by at least
        2 % of the thickness on average.
- A CPU port of the alto transmittance: altostratus 1 overhead gives
    0.08 ± 0.02; amount 0 gives exactly 1; transmittance is monotonic in
    amount.
- **Preset resolution:** the built-in table matches the values above, and
    blending halfway gives the mean.
- **Validation:** out-of-range values for every new field are rejected.
- **WGSL:** the shaders still validate in `cargo test`. The browser check
    below catches Tint issues.

## Verification

- The fair-weather shots (before and after): bases uneven between
    clouds, lumpy but mostly flat, softly grey. No spikes or columns
    under clouds. Compare against the reference photos in the request:
    flat-ish bases, lumpy undersides, bright tops.
- The low-sun shot: warm-lit sides, darker undersides.
- **Mackerel sky:** altocumulus 0.7 with the camera looking up at 50
    degrees. Rippled patches, with shadows onto any cumulus below.
- **Veiled sun:** altostratus 0.7 looking towards the sun at elevation
    35 degrees. A watery disc with a soft glow, and ground shadows faint.
- Rain: altostratus deck under the rain preset, with no banding.
- The reference shots render with no errors. With the alto amounts at 0,
    the sky matches the before shots, apart from the intended base
    change.
- **GPU:** clouds pass +0.2 ms for bases and +0.3 ms for the alto layer
    at most, from the software ratio against terrain, scaled.

## Budgets

- WASM growth: at most +6 KB gzipped.
- GPU: at most +0.5 ms in the clouds pass at 1080p on a mid-range GPU.

## Out of scope

Separate cloud size or spacing controls (declined), and volumetric alto
clouds.
