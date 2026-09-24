# Plan 7: Sky and weather presets

## Goal

Weather becomes one system. A table of weather presets, which you can
edit and extend, describes each kind of weather. At any moment, every
system reads the same blend of presets, so transitions stay consistent:

- sky colour and haze;
- clouds;
- fog and mist;
- rain and snow;
- wind on trees, grass and the sea;
- sea state;
- wet and snowy ground;
- light levels.

Weather varies across the map: you can watch a storm approach while the
coast is in sunshine. Ground wetness builds up and dries where rain
actually fell. Overcast skies soften and dim the light. A time-of-day
cycle is added, and the weather prefers to clear before sunset, so storms
end in wet, golden evenings.

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

- Rebuild weather around a single blended preset table. It is editable,
    and you can add your own presets.
- Effects:
    - humidity and turbidity haze;
    - wet surfaces that dry over time, with puddles;
    - wind driving trees, grass and sea state;
    - overcast dimming of direct and indirect light.
- Regional weather cells: a drifting weather map, with rain only where
    the clouds are thick.
- Golden-hour bias: when time of day is animated, the cycle tends to
    clear before sunset.

## Prerequisites

- **Plan 2:** cold-weather rules, `temperatureAt` and the surface texture.
- **Plan 3:** rivers' `melt_factor`, which this plan drives from preset
    temperature.
- **Plan 5:** grass generation, which reads wind.
- **Plan 6:** layered tree wind, which reads the wind and gust uniforms.
- Check that each exists. If one is missing, carry out that plan first;
    each is self-contained.

## Current state

- `crates/vista_wasm/src/weather.rs`:
    - `WeatherProfile` holds per-kind values (cloud coverage, density,
        thickness, mist, haze, wind, gustiness, rain, snow, lightning,
        stratiform, towering, base darkness, ragged base, rain shafts);
    - `profile(kind)`, `next_kind`, and `WeatherSystem` with `advance`
        and blending between two states;
    - plan 2 added temperature phase rules and a cold bias.
- `WeatherKind` (7 states), `WeatherOptions`, `WeatherEffects`, and
    `getWeather()` returning `WeatherState`.
- Global scalars in the uniforms: wetness, snow cover, overcast and wind.
    Cloud coverage is one value for the whole sky
    (`shaders/atmosphere.wgsl` `cloud_shape(p, weather)`).
- Sun: `SunOptions` holds only azimuth, elevation and intensity. There
    is no time of day.
- The sky model is single scattering, shared by every shader through
    `common.wgsl`, with `AtmosphereOptions` (Rayleigh and Mie strength,
    haze distance).

## Design

### 1. The preset table

- **`WeatherPreset`** has every field optional in the API. After
    resolution it is complete:
    - **Clouds:** `cloudCoverage`, `cloudDensity`, `cloudThickness`,
        `stratiform`, `towering`, `baseDarkness`, `raggedBase`,
        `rainShafts`, and the cirrus amount.
    - **Air:** `humidity` (0 to 1), `turbidity` (2 to 10), `mistDensity`,
        and `temperatureOffsetCelsius`.
    - **Wind:** `windMetresPerSecond` and `gustiness`.
    - **Precipitation:** `rain`, `snow` and `lightningPerMinute`.
    - **Regional structure:** `coverageSpread`, `precipitationSpread`,
        `cellSizeKm` and `cellularity` (0 means smooth fields, 1 means
        discrete storm cells).
    - **Cycle:** `next` (successor weights), `minDurationSeconds`,
        `maxDurationSeconds`, and `climate` (`minCelsius` and
        `maxCelsius`, where the preset suits).
    - **`extends`:** the name of a preset to inherit unset fields from.
- **Built-in presets**, in `weather/presets.rs` (turn `weather.rs` into a
    `weather/` module):
    - `clear`, `fewClouds`, `partlyCloudy`, `brokenClouds`, `overcast`;
    - `mist`, `fog`;
    - `lightRain`, `rain`, `heavyRain`, `storm`;
    - `snow`, `blizzard`.
- Carry over the existing seven profiles' values exactly for the names
    that already exist, so the existing states look the same, apart from
    the new effects. Tune the new ones:
    - `fewClouds`: coverage 0.2;
    - `brokenClouds`: coverage 0.7, spread 0.25;
    - `mist`: humidity 0.9, mist 0.35, coverage 0.3;
    - `lightRain`: rain 0.35;
    - `heavyRain`: rain 1.3, rain shafts 1;
    - `blizzard`: snow 1.4, wind 16 m/s, gustiness 0.8, humidity 0.8.
- **Plan 2's cold rules move into the presets:**
    - `snow` and `blizzard` have `climate.maxCelsius: 1`;
    - `rain` family successors are re-weighted towards `snow` when the
        camera is below 0 °C;
    - precipitation phase by temperature stays in the resolved state.
- **Custom presets:** `WeatherOptions.presets` maps a name to a preset.
    - A new name adds a preset.
    - A built-in name overrides only the fields given.
    - `extends` chains are resolved with cycle detection. An error names
        the cycle.
    - Validate every numeric field's range. An unknown field is rejected
        with a message listing the valid ones: use
        `#[serde(deny_unknown_fields)]`.
- **`state`** accepts any resolved preset name.
- **`WeatherKind`** in TypeScript widens to the built-in names plus
    `string & {}` for custom names. It stays a string at runtime. The
    `"weatherChanged"` event and `RenderStats.weather` report the
    dominant preset name at the camera.

### 2. Blending

- `WeatherSystem` keeps weights over presets: the current preset, and the
    next one while a transition runs.
- The resolved state is the weighted average of every numeric field.
    Every consumer reads only the resolved state (plus the regional
    field): sky, clouds, mist, precipitation, wind, water, trees, grass,
    surfaces, light and rivers' melt.
- Remove every place that still reads per-kind constants directly. Test
    this with a grep in review, and with the blend test below.
- `WeatherEffects` toggles still gate each consumer, as today.

### 3. Regional weather map

- The CPU owns a regional field on a 128 x 128 grid, covering
    `regionSizeKm` (default 64 km), centred on the terrain centre, in
    `weather/regional.rs`. Its fields, each 0 to 1:
    - `coverage = clamp(mean + spread x warped_fbm(p - wind_drift(t)))`;
    - `precipitation`: the preset's rain and snow scaled by
        `smoothstep(0.55, 0.8, coverage)`, with its own spread, so it
        falls only under thick cloud;
    - `storminess`: `cellularity` x Worley cells, advected in the same
        way;
    - `humidity`: the preset's humidity ±0.1 x noise.
- Use plan 1's gradient noise (`terrain/noise.rs`). `wind_drift(t)` is
    the integrated wind vector at the cloud-layer speed. It is
    deterministic from the seed and the accumulated weather time.
- **Update:** recompute 16 rows per frame, which is a full refresh every
    8 frames. Upload into an `rgba16float` 128 x 128 texture at
    `group(1)`. Add the texture and its world mapping (origin and size)
    to the world uniforms.
- **Consumers:**
    - Cloud density in `cloud_shape` uses local coverage and storminess,
        from `textureSampleLevel` at `p.xz`, instead of the single
        `weather` scalar. The existing Worley towering cells become the
        storminess field.
    - Rain shafts use local precipitation.
    - Precipitation particles, lens drops, lightning, sounds and
        `getWeather()` use the camera's local values, computed on the CPU
        from the same field.
- **Light at the camera:** direct sun is dimmed by the cloud transmittance
    towards the sun above the camera. Evaluate this on the CPU from the
    field along the sun direction at cloud height (4 samples), so a cloud
    passing over the sun dims the scene consistently.
- `regional: false` gives uniform fields (spread 0), so everything
    behaves like one global state.
- Add `engine.weatherAt(x, z)` returning
    `{ coverage, precipitation, storminess, humidity, wetness, puddles, snowDepth }`
    for gameplay and audio.

### 4. Wet surfaces that dry

- Add a surface weather map: rgba8 at terrain resolution (capped at
    1024 x 1024; downsample larger terrains). Its channels:
    - r: wetness;
    - g: puddle water;
    - b: snow depth;
    - a: reserved.
- A compute pass (`shaders/surface_weather.wgsl`) runs once every 0.25 s,
    with the elapsed time:
    - **Wetness:** `+= precipitation(local) x rate x dt`. It dries at
        `evaporation x dt`, where
        `evaporation = base x (0.3 + sun_elevation_factor) x (0.5 + wind / 10) x temperature_factor x (1 - canopy_shade x 0.5)`.
        - Canopy shade comes from plan 5's cover texture: ground under
            trees stays damp longer.
    - **Puddles:** fill where wetness is above 0.6 on cells with slope
        under 3 degrees and concave curvature (from the height texture's
        Laplacian). They evaporate at 0.3x the wetness rate.
    - **Snow depth:** grows with snowfall when the local temperature is
        below 0.5 °C, and melts above 1 °C in proportion to degrees and
        sun. Plan 2's permanent snow still sets a floor.
- **Consumers:** terrain, trees, grass and river banks read the map. They
    replace the global wetness and snow-cover scalars; keep those only as
    `WeatherEffects` gates.
    - Wet terrain darkens (albedo x 0.6 to 1) and roughness falls.
    - Puddles are mirror-flat, reflect the sky with the existing water
        Fresnel, and show ripple rings while it rains, from the water
        texture.
    - Wet trees darken their bark.
- Weather time can be skipped: `engine.advanceWeather(seconds)` runs the
    simulation forward instantly, in 1 s steps, up to 86,400 s. This is
    useful for tools and for this plan's verification.

### 5. Humidity haze

- In `common.wgsl`'s sky model, turbidity scales the Mie scattering
    coefficient, from 2 (very clean alpine air) to 10 (thick tropical
    haze). Humidity:
    - whitens the Mie colour (aerosols swell with water);
    - raises the Mie anisotropy g from 0.76 to 0.85;
    - slightly shortens the haze distance.
- Blend these from the resolved state.
- Keep `AtmosphereOptions` as multipliers on top: `mieStrength` scales
    the preset turbidity, and `hazeDistanceMetres` scales the preset's
    haze. Existing scenes with the weather off keep their look.
- The result: hazy, white-blue tropical skies at high humidity and
    turbidity, and deep, crisp blue at low values.

### 6. Overcast dims the light

- From the camera-local coverage and the sun transmittance (section 3):
    - Scale direct sunlight by the transmittance.
    - Soften shadows: tree and terrain shadow strength x
        `(1 - 0.85 x overcast)`, and the terrain shadow penumbra widens.
    - Sky irradiance blends towards a flat, grey overcast irradiance
        (the cloud base lit from above) by coverage.
    - Indirect and bounce light (the existing ambient terms) fade by up
        to 60 % at full coverage.
    - Sun glitter on water fades by transmittance.
- Put these in `FrameUniforms`, appended as `vec4`s.

### 7. Wind on trees, grass and sea

- One resolved wind (speed, direction and gustiness) drives everything.
    The CPU computes a gust front function shared with the shaders: the
    gust at x is `gust(dot(x, dir) - gust_speed x t)`, a 1D noise with
    parameters passed as uniforms. Trees (plan 6), grass (plan 5), cloud
    drift, rain slant and water all use it, so a gust visibly crosses the
    landscape.
- **Sea state** from wind speed U, in m/s:
    - The significant wave height is `H = 0.0246 x U^2`, limited by the
        fetch (terrain extent over sea, at most 20 km). It scales the
        Gerstner amplitudes relative to the configured `WaveOptions`, so
        `WaveOptions` stays a multiplier.
    - Whitecap coverage is `3.84e-6 x U^3.41`, zero below 4 m/s and
        capped at 0.3. It drives foam on crests.
    - Wave direction turns towards the wind over about 60 s.
    - Add blown spray streaks above 15 m/s.

### 8. Time of day with golden-hour clearing

- Add `setTimeOfDay(options)` with `TimeOfDayOptions`:
    - `enabled` (default false);
    - `hours` (0 to 24, default 12);
    - `dayLengthMinutes`: real minutes for a full day, 1 to 1440,
        default 24;
    - `latitudeDegrees` (-89 to 89, default 45);
    - `dayOfYear` (1 to 366, default 172).
- When enabled, the sun's azimuth and elevation follow the NOAA solar
    position equations, and time advances with the engine's smoothed
    frame time. `setSun` still sets intensity. While time of day is
    enabled, its azimuth and elevation are ignored; document this.
- `getTimeOfDay()` returns `{ hours, sunAzimuthDegrees, sunElevationDegrees, sunriseHours, sunsetHours }`.
- **Golden-hour bias** (only when time of day is enabled and `autoCycle`
    is on):
    - Between sunset - 4 h and sunset - 1.5 h, while a precipitating
        preset is active, choose successors along a clearing chain:
        storm → rain → brokenClouds → fewClouds (or snow → brokenClouds
        in the cold).
    - Shorten each step so the chain completes by about sunset - 1 h.
    - From sunset - 1.5 h to sunset, the probability of starting
        precipitation drops by 70 %.
    - Evaporation is low at a low sun, so the ground is still wet and
        reflective at sunset.
    - Keep it deterministic from the seed.

## Public API

Additive (the widening of `WeatherKind` is backwards compatible for
inputs):

```ts
export type BuiltInWeatherKind =
  | "clear" | "fewClouds" | "partlyCloudy" | "brokenClouds" | "overcast"
  | "mist" | "fog" | "lightRain" | "rain" | "heavyRain" | "storm"
  | "snow" | "blizzard";
export type WeatherKind = BuiltInWeatherKind | (string & {});

export interface WeatherPreset {
  extends?: WeatherKind;
  cloudCoverage?: number;
  cloudDensity?: number;
  cloudThickness?: number;
  stratiform?: number;
  towering?: number;
  baseDarkness?: number;
  raggedBase?: number;
  rainShafts?: number;
  cirrus?: number;
  humidity?: number;
  turbidity?: number;
  mistDensity?: number;
  temperatureOffsetCelsius?: number;
  windMetresPerSecond?: number;
  gustiness?: number;
  rain?: number;
  snow?: number;
  lightningPerMinute?: number;
  coverageSpread?: number;
  precipitationSpread?: number;
  cellSizeKm?: number;
  cellularity?: number;
  next?: Record<string, number>;
  minDurationSeconds?: number;
  maxDurationSeconds?: number;
  climate?: { minCelsius?: number; maxCelsius?: number };
}

export interface WeatherOptions {
  // ...existing...
  presets?: Record<string, WeatherPreset>;
  /** Weather varies across the map. Defaults to true. */
  regional?: boolean;
  /** Size of the regional weather map in km, 16 to 256. Defaults to 64. */
  regionSizeKm?: number;
}

export interface LocalWeather {
  coverage: number;
  precipitation: number;
  storminess: number;
  humidity: number;
  wetness: number;
  puddles: number;
  snowDepth: number;
}

export interface TimeOfDayOptions {
  enabled?: boolean;
  hours?: number;
  dayLengthMinutes?: number;
  latitudeDegrees?: number;
  dayOfYear?: number;
}

export interface TimeOfDay {
  hours: number;
  sunAzimuthDegrees: number;
  sunElevationDegrees: number;
  sunriseHours: number | null;
  sunsetHours: number | null;
}

export interface VistaEngine {
  // ...existing...
  getWeatherPresets(): Record<string, Required<Omit<WeatherPreset, "extends">>>;
  weatherAt(x: number, z: number): LocalWeather | null;
  advanceWeather(seconds: number): void;
  setTimeOfDay(options: TimeOfDayOptions): void;
  getTimeOfDay(): TimeOfDay;
}
```

- Mirror these in Rust, with serde camelCase, defaults and validation.
- `sunriseHours` and `sunsetHours` are null during polar day or night.
- Keep every existing `WeatherOptions` field, with the same meaning.
    `stateDurationSeconds` becomes the default duration for presets that
    set none.

## Steps

1. Capture the reference shots. Add rain, storm, snow and fog shots at a
    fixed seed. Record GPU times.
2. Create the `weather/` module and the preset table, carrying over the
    existing values. Add blending, custom presets, `extends` and
    validation, with tests. Existing weather shots must be
    indistinguishable at this step.
3. Add the regional field, its upload and the cloud and shaft consumers.
    Add camera-local values, sun transmittance and `weatherAt`.
4. Add the surface weather map compute pass and all its consumers, plus
    `advanceWeather`.
5. Add humidity haze in the sky model.
6. Add overcast light dimming.
7. Add the shared gust front, sea state and spray.
8. Add time of day and the golden-hour bias.
9. Demo, in the weather section:
    - the preset select lists every resolved preset;
    - an "Edit preset" disclosure with sliders for the selected preset's
        main fields: coverage, humidity, turbidity, wind, rain, snow and
        spreads. It writes a custom override through
        `WeatherOptions.presets`, with "Reset preset" to remove the
        override;
    - a "Regional weather" checkbox;
    - a "Skip ahead 10 minutes" button (`advanceWeather(600)`);
    - a time-of-day group: enable, an hour slider with readout, day
        length, and latitude;
    - the stats panel shows the local coverage, precipitation and
        wetness.
10. Docs:
    - rewrite `docs/weather.md` around presets, with a custom-preset
        example (a "tropical downpour" extending `heavyRain`);
    - `docs/sky-atmosphere-and-weather.md` (haze, light, time of day);
    - `docs/water.md` (sea state);
    - `docs/options-reference.md`;
    - `docs/architecture.md` (weather data flow, textures, uniforms);
    - `CHANGELOG.md`.
11. Verify, then commit and push.

## Tests

- **Blend:** halfway through a transition, every resolved field is the
    mean of the two presets.
- **Presets:**
    - overriding one field of `rain` changes only that field;
    - `extends` inherits correctly;
    - a cycle (a → b → a) is rejected with a message naming it;
    - an unknown field is rejected;
    - out-of-range values are rejected.
- **Compatibility:** with the weather off, `AtmosphereOptions` produce
    the same uniforms as before. The existing seven names resolve to the
    old profile values.
- **Regional field:**
    - deterministic;
    - advection: the field at `x + drift` at time t equals the field at x
        at time 0;
    - `regional: false` gives spread 0;
    - precipitation is 0 wherever coverage is below 0.55.
- **Surface map** (a CPU reference of the compute pass):
    - wetness rises under rain and dries faster with sun and wind than
        without;
    - puddles only on concave cells with slope under 3 degrees;
    - snow depth accumulates below 0.5 °C and melts above 1 °C.
- **Solar position:** within 0.5 degrees of NOAA reference values for 6
    date, time and latitude cases (hard-code the expected values in the
    test).
- **Golden hour:** a seeded 200-day simulation with time of day and
    auto-cycle. The fraction of days clear (coverage under 0.5, no
    precipitation) at sunset - 0.5 h is at least 30 % higher than with
    the bias disabled (internal flag in tests only).
- **Sea state:** wave height rises monotonically with wind; there are no
    whitecaps below 4 m/s.
- **API:**
    - `advanceWeather` rejects negative, `NaN` and over-limit values;
    - `weatherAt` returns null with no terrain;
    - `setTimeOfDay` validates ranges;
    - TypeScript type tests for the new members, and for `WeatherKind`
        accepting custom strings.

## Verification

- **Humidity:** `clear` with humidity 0.95 and turbidity 8 against
    humidity 0.15 and turbidity 2 (as a custom preset override). Tropical
    haze against crisp alpine blue.
- **Regional storm:** `storm` with `cellSizeKm: 6`, taken where
    `weatherAt` reports low coverage at the camera. The storm cell is
    visible in the distance, with shafts beneath it, while the camera is
    in sun.
- **Drying:** rain for 10 minutes (`advanceWeather`), then `clear`:
    capture at +0, +20 and +60 minutes. Wet, then drying patchily, with
    puddles last to go.
- **Wind:** `storm` at the coast. Waves, whitecaps and spray, and trees
    bending the same way.
- **Overcast:** `overcast` against `clear`. Soft, dim light, faint
    shadows.
- **Golden hour:** time of day at sunset - 0.5 h after a storm. A
    clearing sky, wet ground and a low golden sun.
- The reference shots render with no errors. With the weather off, they
    are unchanged.
- **GPU:** clouds +0.4 ms at most; the surface compute pass at most
    0.1 ms (amortised); everything else negligible. Rain scene total at
    most 14 ms at 1080p on a mid-range GPU (scaled estimate).

## Budgets

- WASM growth: at most +15 KB gzipped.
- CPU: the regional field at most 0.5 ms per frame (16 rows), plus the
    camera-local evaluation at most 0.1 ms.
- GPU: as above.

## Out of scope

Mid-level cloud layers and cloud-base shape (plan 8 adds alto fields to
presets), and precipitation audio playback.
