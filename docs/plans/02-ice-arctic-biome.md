# Plan 2: Ice and arctic biome (`iceArctic`)

## Goal

Add the missing cold biome. A cold map shows:

- glaciers and ice sheets, with crevasses and blue ice in them;
- a tundra fringe of moss, lichen and dwarf shrubs where the ice thins;
- sea ice on cold ocean;
- snow that stays whatever the weather.

A mild map keeps ice only on its highest, flattest summits. Cold places
snow rather than rain, and the air is crisp.

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

- Contents: glaciers and ice sheets, a tundra fringe, sea ice, and
    permanent snow cover.
- Placement is climate-driven, through a temperature option. Cold maps
    are arctic in the lowlands; mild maps keep ice only on the highest
    peaks.
- Weather follows the climate: snow instead of rain, blowing snow, and
    clear, cold air.

## Prerequisites

Plan 1 (realistic terrain generation) should be merged: check that
`crates/vista_wasm/src/terrain/landforms.rs` exists. This plan does not
depend on its internals. Everything here works on any height map, so if
plan 1 is absent, carry this plan out as written anyway.

## Current state

- Biomes:
    - `BiomeKind` in `crates/vista_types/src/lib.rs` has 15 variants
        (`Ocean = 14`) and `BiomeKind::ALL`.
    - TypeScript mirrors it as `BiomeKind` in `js/src/types.ts` (camelCase
        strings).
- `crates/vista_wasm/src/terrain/biomes.rs`:
    - `ClimateGrid` builds temperature and moisture fields. The
        temperature unit is 0 (polar) to 1 (tropical):
        `0.56 + t * 0.62 + temperature_bias * 0.4`, minus `rel * 0.62` for
        altitude relative to the map's range.
    - `classify_surface` picks the biome and eight material weights
        (`MAT_LUSH_GRASS` … `MAT_VOLCANIC`).
    - `snow_line_metres` gives the snow line.
- Terrain materials:
    - They travel per vertex as two `vec4<f32>` (`materials_a`,
        `materials_b`) in the terrain mesh
        (`render/terrain_mesh.rs`, `shaders/clipmap_render.wgsl`).
    - Textures are generated at start-up into 8 array layers
        (`render/textures.rs`: `TERRAIN_LAYERS = 8`;
        `shaders/texture_gen.wgsl`).
    - `crate::engine::TERRAIN_TEXTURE_LAYERS` must match (a const
        assertion in `gpu.rs`).
- `shaders/material_masks.wgsl` is an unused placeholder. Confirm with
    `grep -rn material_masks crates`, then remove it and its
    `render/pipelines.rs` entry.
- Weather:
    - `crates/vista_wasm/src/weather.rs` (`WeatherSystem`, `next_kind`,
        `profile`).
    - `WeatherOptions.allowSnow` gates snow in the cycle.
    - Snow cover and wetness are global scalars in the uniforms.
- The ocean is shaded in `shaders/water.wgsl` (Gerstner waves, whitecaps).

## Design

### Climate temperature

- Add `BiomeOptions.meanTemperatureCelsius?: number`: the mean annual
    temperature at sea level, in °C, from -30 to 35. It is unset by
    default.
- When it is unset, the climate works exactly as today (from
    `temperatureBias`), so existing maps keep their biomes. The only
    change is that ice may now appear on the highest summits, as below.
- When it is set:
    - The temperature unit at a sample is `(celsius + 30) / 65`, where
        celsius is the sea-level mean, plus the existing climate-noise
        variation (±4 °C), minus a lapse rate of 6.5 °C per 1000 m of real
        altitude above sea level. With a real lapse rate, a 3000 m
        mountain on a 15 °C map is about -4 °C at the top.
    - `temperatureBias` still adds ±0.4 of the unit on top.
- Add `SurfaceSample::celsius()`, which returns the local temperature in
    °C using the same mapping. The default (unset) climate maps its unit
    to °C with the same formula.

### The biome

- Add `BiomeKind::IceArctic = 15` (append only; update `ALL` to 16
    entries), and TypeScript `"iceArctic"`.
- `biome_debug_colour`: pale cyan `[0.75, 0.92, 1.0]`.
- Update `docs/biomes.md` (the biome table and a description).
- Classification in `classify_surface`, checked before the mountain
    rules:
    - Glacier ice where the local mean is below -2 °C, the slope is under
        35 degrees, and the sample is not ocean. The mean must be below
        -6 °C on slopes from 35 to 50 degrees. Steeper slopes stay rock
        with snow.
    - Tundra (still `IceArctic`, with tundra materials) where the local
        mean is from -2 °C to 3 °C, or where the glacier rule fails only
        on slope. Mild maps (unset climate) reach -2 °C only near their
        highest points, so they get small summit ice caps and a tundra
        band below.
- Material weights: add two materials, `MAT_ICE = 8` and
    `MAT_TUNDRA = 9`.
    - Ice is blended with snow by snow depth: fresh snow lies on the ice.
    - Tundra is blended with rock on stony ground.
- Permanent snow: `SurfaceSample.reserved` becomes `permanent_snow`
    (0 to 255):
    - 255 on glacier;
    - 0 to 160 on tundra, by temperature;
    - existing summit snow above `snow_line_metres` stays as it is.

### Twelve material slots, smaller vertices

Ten materials no longer fit two `vec4`s.

- Pack material weights as twelve `unorm8` values in three `u32`s, using
    `pack4x8unorm` on the CPU equivalent and `unpack4x8unorm` in WGSL.
    This replaces `materials_a` and `materials_b`. The vertex shrinks by
    20 bytes, which saves about 5 MB on the 512 x 512 mesh and speeds up
    the streamed mesh upload.
- Slots 10 and 11 are unused (weight 0) and documented as reserved.
- Update:
    - `render/terrain_mesh.rs` (vertex layout and packing, with a unit
        test for round-tripping);
    - `shaders/clipmap_render.wgsl` (unpack, then keep the three
        strongest of the ten as now);
    - `MATERIAL_COUNT = 10`;
    - `TERRAIN_LAYERS` and `TERRAIN_TEXTURE_LAYERS` to 10;
    - `replaceTexture` validation and its docs (`docs/hooks.md`), where
        the layer indices now run 0 to 9;
    - the `"materials"` debug view colours.
- Generate the two new texture layers in `shaders/texture_gen.wgsl`:
    - **Ice:**
        - albedo is blue-white `(0.80, 0.90, 0.97)`, darkening towards a
            deeper blue `(0.35, 0.60, 0.80)` in low areas of the height
            channel;
        - fine air-bubble speckle;
        - roughness 0.15 on bare ice and 0.35 on weathered ice;
        - the height channel carries gentle wind-scoured undulation.
    - **Tundra:**
        - a mosaic of olive and khaki moss `(0.33, 0.36, 0.22)`, lichen
            patches `(0.62, 0.60, 0.45)` and small grey stones;
        - roughness 0.85.
- Both are procedural at start-up, adding no bytes of image data.

### Glacier shading and form

- **Crevasses:** in `clipmap_render.wgsl`, on the ice material, add
    crevasses across the flow direction.
    - Flow is downslope: the terrain normal's horizontal part.
    - Sample the noise texture with coordinates stretched 6 to 1 along
        the flow, so crevasses run across it. Threshold the result into
        thin dark-blue cracks.
    - Only where the slope is 8 to 30 degrees, since crevasses form where
        ice speeds up.
    - Use `textureSampleGrad` with the existing UV derivatives. This
        stays uniform-safe.
- **Blue ice:** the ice colour takes a subsurface blue in cracks and
    shaded areas. Mix towards `(0.25, 0.55, 0.85)` by `(1 - n.l) * 0.35`.
- **Glacier surface:** ice fills valleys smoothly, where rock would be
    rough.
    - Within the glacier mask, raise heights towards a Gaussian-blurred
        surface (radius 60 m), by at most 40 m.
    - Record every changed sample, as `render/water.rs` does for river
        carving (`restore_carving`), so that toggling biomes, changing
        climate or disabling biomes restores the original terrain
        exactly.
    - Run this before rivers are carved.
    - Add a test that turning the climate cold and then back leaves the
        heights bit-identical.

### Tundra vegetation

- In `render/flora.rs` `choose_species`: on `IceArctic` tundra (not
    glacier), use `Shrub` at 0.35 to 0.6 scale with a brownish tint, at
    10 % of the normal tree density.
- In `render/grass.rs`: sparse, short tufts (0.4 x height) tinted
    ochre-green, at 40 % density.
- Nothing grows on glacier.

### Permanent snow

- Add `group(1) @binding(12) surface_texture: texture_2d<f32>`: an
    rgba8unorm texture at terrain resolution. Channels:
    - r: temperature unit;
    - g: moisture;
    - b: permanent snow;
    - a: biome index / 255.
- Upload it with the other per-terrain textures. Update every bind group
    layout that uses group 1. Later plans reuse this texture, so document
    its channels in `docs/architecture.md`.
- Terrain, tree and grass shaders use `max(weather snow cover, permanent snow)`.
    Trees on permanent snow ground get snow on their upper surfaces
    through the existing snow path.

### Sea ice

- In `shaders/water.wgsl`, for the ocean kind only, compute an ice
    concentration:
    - inside the terrain area, from `surface_texture.r`, sampled with
        `textureSampleLevel`;
    - beyond it, from a new world uniform: the sea-level mean temperature
        unit.
    - Concentration = `saturate((-1.5 - celsius) / 6.0)`: open water
        above -1.5 °C, full pack ice below -7.5 °C.
- **Floes:**
    - Worley cells from the noise texture's Worley channel, at a 40 m
        scale near the camera, blended with a 300 m scale far away.
    - A cell is ice when its hash is below the concentration.
    - Floes are thresholded with a 0.04 soft edge.
    - They drift with the wind at 2 % of the wind speed: offset the UV
        by a uniform-derived drift, never per-pixel time.
- **Shading:**
    - Floes are snow-white to blue-grey, using the snow material's
        lighting response, with a bevelled rim from the Worley distance
        gradient.
    - Water between floes is dark, and waves and whitecaps there are
        scaled by `1 - concentration`.
    - Within 200 m of the coast, when the mean is below -10 °C, fast ice
        is continuous (concentration 1).
- Keep all sampling uniform-safe (`textureSampleLevel`).
- Add a test in the WGSL validation suite for the new bindings.

### Weather in the cold

- In `weather.rs`, precipitation phase follows temperature at the camera:
    - the camera's surface sample, via a new `Engine::celsius_at(x, z)`
        exposed as `temperatureAt(x, z)` in the API;
    - rain turns to snow below 0.5 °C, and falls as sleet (a mix of both
        particle types) between 0.5 and 2.5 °C.
    - Apply this in the resolved `FrameWeather` (`rain` and `snow` swap
        by phase). The weather state names stay the same.
- In `next_kind`, when the camera is below 0 °C, bias the cycle:
    - Rain and Storm become Snow;
    - Clear is weighted 1.5 x;
    - `allowSnow` is treated as true, whatever it is set to. Document
        that cold climates always allow snow.
- **Blowing snow:** when snow cover is at least 0.5 and the wind is
    above 8 m/s, draw low drifting snow streaks in the composite pass's
    precipitation layer, 0 to 2 m above the ground, moving with the wind.
    Reuse the snow particle code with a flat, fast profile. Derive
    per-pixel motion from uniforms (camera heading and wind) to avoid
    shimmer.
- **Crisp air:** below 0 °C at the camera, multiply the haze distance by
    1.4 and the Mie strength by 0.7.

## Public API

Additive only:

```ts
export type BiomeKind = /* ...existing... */ | "iceArctic";

export interface BiomeOptions {
  // ...existing...
  /**
   * Mean annual temperature at sea level in °C, from -30 to 35. Unset
   * keeps the default climate. Below about -2 °C lowlands freeze into
   * ice sheets; around 15 °C only the highest summits hold ice.
   */
  meanTemperatureCelsius?: number;
}

export interface VistaEngine {
  // ...existing...
  /** Mean annual temperature in °C at a world position, or null off the terrain. */
  temperatureAt(x: number, z: number): number | null;
}
```

- Validation: `meanTemperatureCelsius` is finite and in [-30, 35].
- `temperatureAt` returns `null` with no terrain, and validates that
    both arguments are finite numbers (`TypeError` otherwise).

## Steps

1. Capture the reference shots. Also capture a mountain summit at
    default settings:
    `{"shots":[{"biome":"mountainProper","back":400,"height":120}]}`.
2. Add the types, validation and TypeScript for the new option, biome and
    API method, with tests.
3. Pack the materials into twelve `unorm8` slots: mesh, shader, textures
    and constants. Verify that the default shots are pixel-identical in
    material appearance, allowing unorm8 rounding: the mean absolute RGB
    difference must be under 1.0.
4. Add the climate temperature, the classification, the ice and tundra
    materials, and permanent snow.
5. Add the surface texture binding.
6. Add glacier smoothing with restore, and its test.
7. Add crevasses and blue ice.
8. Add tundra flora and grass.
9. Add sea ice.
10. Add weather phase, cycle bias, blowing snow and crisp air.
11. Demo:
    - a "Climate temperature" slider (-30 to 35 °C) with an "Auto"
        checkbox (unset) in the biomes section;
    - `iceArctic` in the biome debug legend.
12. Docs:
    - `docs/biomes.md` and `docs/weather.md` (cold phase, blowing snow);
    - `docs/water.md` (sea ice);
    - `docs/options-reference.md` and `docs/hooks.md` (texture layers);
    - `docs/architecture.md` (surface texture, vertex packing,
        `FrameUniforms` if changed);
    - `CHANGELOG.md`.
13. Verify, then commit and push.

## Tests

- **Classification:**
    - A flat map at -15 °C is at least 90 % `IceArctic` on land.
    - At 15 °C (unset climate) with a 3000 m peak, `IceArctic` appears
        only where the height is above 85 % of the relief.
    - At 30 °C there is no `IceArctic`.
- **Permanent snow:** 255 on glacier samples.
- **Glacier smoothing:** restoring gives bit-identical heights, and the
    maximum raise is at most 40 m.
- **Packing:** `pack`/`unpack` of twelve weights round-trips within 1/255.
- **Flora:** no trees on glacier samples; tundra trees are shrubs only.
- **Weather:** rain at -3 °C resolves to snow; at 1.5 °C it resolves to
    both, which is sleet.
- **`temperatureAt`:** returns null with no terrain, and matches the
    surface sample.
- **TypeScript:** a type test for the new members; `temperatureAt`
    rejects `NaN`.

## Verification

Capture these, and open every one:

- `{"engine":{"biomes":{"meanTemperatureCelsius":-18}},"shots":[{"position":[0,0,2500],"target":[0,0,0],"height":250}]}`:
    an ice sheet with crevasses and sea ice on the coast.
- `{"engine":{"biomes":{"meanTemperatureCelsius":-3}}}`: a tundra and ice
    mosaic.
- The default map summit shot: small ice caps, the rest as before.
- `{"engine":{"biomes":{"meanTemperatureCelsius":-18}},"set":{"setWeather":{"enabled":true,"state":"rain","autoCycle":false,"transitionSeconds":0.1}}}`:
    snow falls, not rain.
- The reference shots, unchanged apart from summit ice caps.

## Budgets

- WASM growth: at most +8 KB gzipped.
- GPU: terrain +0.2 ms at most (the crevasse branch runs only on ice;
    keep it cheap); water +0.2 ms at most with sea ice visible, and 0 ms
    extra with none.
- Memory: the terrain vertex buffer shrinks.

## Out of scope

Snowmelt streams from glaciers (plan 3 uses the glacier mask and
temperature this plan provides) and preset-based weather (plan 7 moves
this plan's cold-weather rules into presets).
