# Options Reference

This is a field-by-field reference for every public VistaWASM option, in
TypeScript's camelCase naming (the Rust engine uses the equivalent
`snake_case` internally; both are the same value once decoded). Types shown
are the TypeScript shapes from `js/src/types.ts`. Every option is validated
by the Rust engine — an invalid value throws a `VistaWasmError` with code
`OPTIONS_INVALID` rather than being silently clamped, except where noted.

For narrative, "why would I change this" guidance, see
[`docs/world-design-guide.md`](world-design-guide.md) (terrain/sky/weather)
and [`docs/vegetation.md`](vegetation.md) (trees/grass). This document is the
precise reference; those documents are the tour.

## Contents

- [`VistaEngineOptions`](#vistaengineoptions-engine-creation)
- [`RenderSizeOptions`](#rendersizeoptions)
- [`CameraOptions`](#cameraoptions)
- [`SunOptions`](#sunoptions)
- [`AtmosphereOptions`](#atmosphereoptions)
- [`CloudsOptions`](#cloudsoptions)
- [`MistOptions`](#mistoptions)
- [`WaterOptions`](#wateroptions)
- [`FloraOptions`](#floraoptions)
- [`GrassOptions`](#grassoptions)
- [`BiomeOptions`](#biomeoptions)
- [`WeatherOptions`](#weatheroptions)
- [`ShadowOptions`](#shadowoptions)
- [`SurfaceOptions`](#surfaceoptions)
- [`RenderQualityOptions`](#renderqualityoptions)
- [`DebugView`](#debugview)
- [`FractalTerrainOptions`](#fractalterrainoptions)
- [`DemLoadOptions`](#demloadoptions)
- [`RawHeightmapOptions`](#rawheightmapoptions)
- [`ExportHeightmapOptions` / `SnapshotOptions`](#export-options)
- [Read-only shapes](#read-only-shapes-returned-by-the-engine) (`TerrainMetadata`, `TerrainHandle`, `RenderStats`)

## `VistaEngineOptions` (engine creation)

Passed to `createVistaEngine(canvas, options)`. Every field is optional;
omitted fields use the defaults below.

| Field | Type | Default |
| --- | --- | --- |
| `render` | `RenderSizeOptions` | `{ width: 1, height: 1, devicePixelRatio: 1 }` |
| `camera` | `CameraOptions` | see [`CameraOptions`](#cameraoptions) |
| `sun` | `SunOptions` | see [`SunOptions`](#sunoptions) |
| `atmosphere` | `AtmosphereOptions` | see [`AtmosphereOptions`](#atmosphereoptions) |
| `water` | `WaterOptions` | see [`WaterOptions`](#wateroptions) |
| `flora` | `FloraOptions` | see [`FloraOptions`](#floraoptions) |
| `grass` | `GrassOptions` | see [`GrassOptions`](#grassoptions) (disabled by default) |
| `clouds` | `CloudsOptions` | see [`CloudsOptions`](#cloudsoptions) (off by default) |
| `mist` | `MistOptions` | see [`MistOptions`](#mistoptions) (off by default) |
| `quality` | `RenderQualityOptions` | see [`RenderQualityOptions`](#renderqualityoptions) |
| `biomes` | `BiomeOptions` | see [`BiomeOptions`](#biomeoptions) (climate biomes on by default) |
| `weather` | `WeatherOptions` | see [`WeatherOptions`](#weatheroptions) (off by default) |
| `shadows` | `ShadowOptions` | see [`ShadowOptions`](#shadowoptions) (all on by default) |
| `surface` | `SurfaceOptions` | see [`SurfaceOptions`](#surfaceoptions) |

Always pass a real `render.width`/`render.height` matching your canvas's
actual CSS size — the `{ width: 1, height: 1 }` default exists only so the
type is safely constructible, not as a usable render size.

## `RenderSizeOptions`

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `width` | `number` | `1` | CSS pixels. Must be at least 1. |
| `height` | `number` | `1` | CSS pixels. Must be at least 1. |
| `devicePixelRatio` | `number?` | `1` | Must be finite, `> 0`, and `<= 8`. |

Also used by `engine.resize(width, height, devicePixelRatio?)`.

## `CameraOptions`

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `position` | `[number, number, number]` | `[0, 120, 300]` | World position in terrain metres. |
| `target` | `[number, number, number]` | `[0, 0, 0]` | Look-at point in terrain metres. |
| `rollDegrees` | `number?` | `0` | Accepted and stored, but **not currently applied** to the view matrix — the camera always uses a fixed world-up vector. Reserved for future use. |
| `fieldOfViewDegrees` | `number` | `55` | Must be between `1` and `160`. |
| `nearMetres` | `number?` | `0.5` | Must be positive and less than `farMetres`. |
| `farMetres` | `number?` | `120000` | Must be positive and greater than `nearMetres`. |
| `minimumHeightAboveTerrainMetres` | `number?` | `2` | Accepted and stored, but **not currently enforced** by the engine — nothing clamps the camera's height against the terrain today. If you need a ground clamp, compute it yourself against an exported heightmap (see [`docs/camera-and-controls.md`](camera-and-controls.md#terrain-clamping-is-not-built-in)). |
| `allowUnderground` | `boolean?` | `false` | Accepted and stored, but **not currently enforced** — see the note above. `attachFlyCameraControls()` always behaves as if this were `true` internally. |

See [`docs/camera-and-controls.md`](camera-and-controls.md) for the
projector model and the bundled fly-camera controller.

## `SunOptions`

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `azimuthDegrees` | `number` | `132` | Compass direction the sun shines from. Must be finite. |
| `elevationDegrees` | `number` | `18` | Angle above the horizon. Negative values put the sun below the horizon. Must be finite. |
| `intensity` | `number` | `1.2` | Light brightness multiplier. Must be `> 0`. |

## `AtmosphereOptions`

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `rayleighStrength` | `number` | `1.0` | Sky gradient saturation. |
| `mieStrength` | `number` | `0.45` | Sun glare/haze size. |
| `hazeDistanceMetres` | `number` | `60000` | Distance haze (aerial perspective), thinning gently with altitude. Must be `> 0`. See the haze-vs-mist distinction in [`docs/sky-atmosphere-and-weather.md`](sky-atmosphere-and-weather.md). |
| `exposure` | `number` | `1.1` | Overall brightness multiplier. Must be `> 0`. |
| `skyTint` | `[number, number, number]` | `[1, 1, 1]` | RGB multiplier applied to the whole sky/haze/cloud result. |

## `CloudsOptions`

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `style` | `"off" \| "painted" \| "volumetric"` | `"off"` | See [`docs/sky-atmosphere-and-weather.md`](sky-atmosphere-and-weather.md#clouds-cloudsoptions). |
| `coverage` | `number` | `0.45` | `0` clear to `1` overcast. Must be `>= 0`. |
| `speed` | `number` | `1.0` | Wind speed multiplier; `1` is roughly 15 m/s, `0` freezes the clouds. |
| `heightMetres` | `number` | `1800` | Altitude of the cloud base. |
| `colour` | `[number, number, number]` | `[1, 1, 1]` | Base cloud tint. |
| `seedOffset` | `number \| bigint` | `9007` | Deterministic cloud noise seed. |
| `raymarchSteps` | `number?` | `32` | `"volumetric"` style only. Must be within `8..=64`: an out-of-range value throws `OPTIONS_INVALID` rather than being silently clamped, since this value bounds a real shader loop. |
| `windDirectionDegrees` | `number?` | `70` | Direction the wind carries clouds towards (0 = +Z, 90 = +X). |
| `evolution` | `number?` | `0.35` | How quickly cloud shapes billow and change while drifting, `0` (rigid) to `1`. |
| `thicknessMetres` | `number?` | `1600` | Vertical thickness of the cloud layer. Must be `> 0`. |
| `density` | `number?` | `0.6` | Optical density, `0` (wispy) to `1` (dense cumulus). |
| `castShadows` | `boolean?` | `true` | Moving cloud shadows on terrain, trees, grass, and water. Also requires `ShadowOptions.clouds.enabled`. |
| `resolutionScale` | `number?` | `0.5` | Cloud render resolution relative to the canvas, `0.25` to `1`. Lower is faster. |
| `temporal` | `boolean?` | `false` | Reuse distant clouds between frames: each frame raymarches a quarter of the sky's cloud pixels and reprojects the rest. Cuts the cost of sky clouds by about three quarters. Volumetric only; off automatically while the camera is within 300 m of the cloud layer. |
| `cirrus` | `number?` | `0.35` | Thin, high cirrus above the main clouds, `0` (none) to `1`. Needs a cloud style other than `"off"`. |
| `cirrusHeightMetres` | `number?` | `9000` | Cirrus altitude, `1000` to `20000`. Always kept above the main cloud layer. |
| `cirrusSpeed` | `number?` | `0.4` | Cirrus drift speed, `0` to `10`, in units of 15 m/s. Separate from `speed`, and not changed by the weather. |
| `stratiform` | `number?` | `0` | Cloud type: `0` heaped cumulus to `1` a flat sheet (stratus, nimbostratus). Volumetric only. |
| `towering` | `number?` | `0` | Towering storm clouds (cumulonimbus) with anvils, `0` to `1`. Towers rise up to 2.6 × `thicknessMetres`. Volumetric only. |
| `baseDarkness` | `number?` | `0` | Darker, rain-laden bases, `0` to `1`. |
| `raggedBase` | `number?` | `0` | Ragged bases with loose scraps of cloud (scud) beneath, `0` to `1`. |
| `rainShafts` | `number?` | `0` | Curtains of rain or snow below the clouds, visible from a distance, `0` to `1`. |

When the weather system drives clouds, it sets the five cloud-type fields
for you (see [`docs/weather.md`](weather.md#cloud-types)).

## `MistOptions`

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `style` | `"off" \| "flat" \| "volumetric"` | `"off"` | See [`docs/sky-atmosphere-and-weather.md`](sky-atmosphere-and-weather.md#mist-and-ground-fog-mistoptions). |
| `density` | `number` | `0.5` | `0` to `1`. Must be `>= 0`. |
| `baseHeightMetres` | `number` | `40` | Altitude mist is thickest at. Must be finite. |
| `heightFalloffMetres` | `number` | `120` | How quickly mist thins with altitude. Must be `>= 0`. |
| `colour` | `[number, number, number]` | `[0.82, 0.85, 0.88]` | Mist tint. |
| `riseAboveWater` | `boolean` | `true` | Adds extra mist near `WaterOptions.seaLevelMetres`, independent of `baseHeightMetres`. |
| `seedOffset` | `number \| bigint` | `5303` | Deterministic mist noise seed (`"volumetric"` style only). |
| `windDirectionDegrees` | `number?` | `70` | Direction fog banks drift towards. |
| `windSpeedMetresPerSecond` | `number?` | `2.5` | Fog bank drift speed (`"volumetric"` style). |
| `sunScattering` | `number?` | `0.6` | How strongly mist glows when looking towards the sun, `0` to `1`. |

## `WaterOptions`

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `enabled` | `boolean` | `true` | |
| `seaLevelMetres` | `number` | `0` | Set relative to `TerrainMetadata.minHeightMetres`/`meanHeightMetres`, not a hardcoded constant — see [`docs/water.md`](water.md). Must be finite. |
| `waveScale` | `number` | `0.8` | Small-scale ripple strength. Large swell is `waves`. Must be `>= 0`. |
| `reflectivity` | `number` | `0.35` | Sky reflection strength on top of physical Fresnel. Must be `>= 0`. |
| `shorelineSoftnessMetres` | `number` | `6` | Shoreline blend distance. |
| `waves` | `WaveOptions?` | see below | Gerstner swell simulation. |
| `rivers` | `RiverOptions?` | see below | Rivers and lakes from the terrain drainage network. Changing these re-carves the terrain. |
| `currentDirectionDegrees` | `number?` | `60` | Direction of the open-water surface current. |
| `currentSpeed` | `number?` | `0.35` | Surface current in m/s; moves ripples and foam. `0` for still water. |
| `shallowColour` | `[number, number, number]?` | `[0.1, 0.52, 0.5]` | sRGB colour of shallow water. Components `0` to `4`. |
| `deepColour` | `[number, number, number]?` | `[0.015, 0.09, 0.16]` | sRGB colour of deep water. |
| `clarityMetres` | `number?` | `6` | Depth at which the sea bed stops being visible. Must be `> 0`. |
| `foam` | `number?` | `0.7` | Foam on crests, shorelines, and rapids, `0` to `1`. |

### `WaveOptions`

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `enabled` | `boolean?` | `true` | `false` keeps the surface flat (ripples only). |
| `amplitudeMetres` | `number?` | `0.9` | Trough-to-crest height of the dominant swell, `0` to `30`. |
| `wavelengthMetres` | `number?` | `38` | Dominant wavelength, `> 0` and at most `2000`. |
| `directionDegrees` | `number?` | `35` | Direction the swell travels towards. |
| `steepness` | `number?` | `0.55` | `0` rolling to `1` sharp, choppy crests. |
| `speed` | `number?` | `1` | Animation speed multiplier (`1` = deep-water dispersion). |
| `directionalSpread` | `number?` | `0.55` | `0` parallel swell to `1` confused, storm-like sea. |

### `RiverOptions`

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `enabled` | `boolean?` | `true` | Rivers and lakes on or off. |
| `minCatchmentKm2` | `number?` | `0.15` | Upstream area before a channel becomes a river. Smaller draws more, thinner streams. Must be `> 0`. |
| `widthScale` | `number?` | `1` | Multiplier on the automatic width. Must be `> 0`. |
| `currentSpeed` | `number?` | `1` | River current speed multiplier. |

## `FloraOptions`

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `enabled` | `boolean` | `true` | |
| `density` | `number` | `0.35` | `0` to `1`. Must be `>= 0`. Scaled by `RenderQualityOptions.floraDensityScale`. |
| `treeLineMetres` | `number` | `1800` | Altitude above which trees stop spawning. |
| `seedOffset` | `number \| bigint` | `3001` | Deterministic placement seed. |
| `maxInstances` | `number` | `500000` | Upper bound on instance count, also clamped to the active device's limits. |
| `treeQuality` | `"billboard" \| "cross-quad" \| "mesh"` | `"mesh"` | See [`docs/vegetation.md`](vegetation.md#tree-quality). |
| `speciesVariation` | `number?` | `0.6` | `0` to `1`. Per-tree size and colour variety. Must be `>= 0`. |
| `windStrength` | `number?` | `0.3` | `0` to `1`. Wind sway strength. Must be `>= 0`. |
| `meshDistanceMetres` | `number?` | `420` | Distance at which `"mesh"` trees cross-fade to impostors. Must be `> 0`. |
| `speciesRules` | `FloraRule[]?` | `[]` | Per-biome species mix and density, replacing the built-in mix. At most 64 rules of 8 species each. See [`docs/hooks.md`](hooks.md#species-rules). |

Species and density follow the biome under each tree, and placement avoids
underwater, river, and steep terrain; see
[`docs/vegetation.md`](vegetation.md#placement).

## `GrassOptions`

Unlike every other environmental option, this defaults fully **disabled** —
grass is a new visual element with no prior equivalent, so existing scenes
are unaffected until a host opts in.

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `enabled` | `boolean` | `false` | |
| `style` | `"billboard-blades" \| "dense-blades"` | `"billboard-blades"` | See [`docs/vegetation.md`](vegetation.md#grass-grassoptions). |
| `density` | `number` | `0.5` | `0` to `1`. Must be `>= 0`. Scaled by `RenderQualityOptions.floraDensityScale`. |
| `viewDistanceMetres` | `number` | `220` | Distance from the camera at which grass fully fades out. Must be `> 0`. |
| `seedOffset` | `number \| bigint` | `7331` | Deterministic placement seed. |
| `maxInstances` | `number` | `200000` | Upper bound on instance count, also clamped to the active device's limits. |

## `BiomeOptions`

Passed at creation (`biomes`) or later with `engine.setBiomes()`. Every
field is optional. See [`docs/biomes.md`](biomes.md).

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `enabled` | `boolean?` | `true` | `false` uses height and slope only (a temperate world). |
| `seedOffset` | `number \| bigint?` | `1733` | Seed for the climate fields. |
| `temperatureBias` | `number?` | `0` | `-1` colder to `1` hotter. Must be finite. |
| `moistureBias` | `number?` | `0` | `-1` drier to `1` wetter. Must be finite. |
| `climateScaleMetres` | `number?` | `7000` | Typical climate region size. Must be `> 0`. |
| `volcanism` | `number?` | `0.35` | `0` to `1`. Volcanic regions around high peaks. |
| `beachHeightMetres` | `number?` | `5` | Height above sea level below which flat ground becomes beach. |
| `snowLineMetres` | `number?` | 80% of sea-to-peak, at least 400 m above sea | Snow line; lowered further in cold climates. |
| `meanTemperatureCelsius` | `number?` | unset | Mean annual temperature at sea level in °C, `-30` to `35`. Drives every biome, cooling 6.5 °C per 1000 m. Below about -2 °C lowlands freeze into ice sheets. Unset keeps the classic climate, with no ice. |

`engine.biomeAt(x, z)` returns the biome name at a world position (or
`undefined` outside the terrain): `"grassyMeadows"`, `"outerThicket"`,
`"outerForest"`, `"innerForest"`, `"mountainFoothills"`, `"mountainProper"`,
`"outerVolcanic"`, `"calderaVolcanic"`, `"savannahExpanse"`,
`"coastalBeach"`, `"coastalRocky"`, `"outerJungle"`, `"innerJungle"`,
`"swampWetlands"`, `"ocean"`, `"alpineTransition"`, `"lowerSnowyPeaks"`,
`"upperSnowyPeaks"`, or `"iceArctic"`.

`engine.temperatureAt(x, z)` returns the mean annual temperature in °C at
a world position, or `null` when there is no terrain or the position is
outside it. It throws a `TypeError` unless both arguments are finite
numbers.

## `WeatherOptions`

Passed to `engine.setWeather()`. Every field is optional. See
[`docs/weather.md`](weather.md).

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `enabled` | `boolean` | `false` | When `false`, every system keeps its manual settings. |
| `state` | `WeatherKind` | `"partlyCloudy"` | `"clear"`, `"partlyCloudy"`, `"overcast"`, `"fog"`, `"rain"`, `"storm"`, or `"snow"`. |
| `autoCycle` | `boolean` | `false` | Move on to new weather over time. |
| `stateDurationSeconds` | `number` | `240` | Average length of each state when cycling. Must be `> 0`. |
| `transitionSeconds` | `number` | `30` | Blend time between states. Must be `>= 0`. |
| `allowSnow` | `boolean` | `false` | Whether cycling may choose snow. Below 0 °C under the camera, snow is always allowed. |
| `seedOffset` | `number \| bigint` | `4111` | Seed for the cycle sequence and gusts. |
| `windDirectionDegrees` | `number` | `70` | Prevailing wind direction. |
| `windScale` | `number` | `1` | `0` to `4`. |
| `precipitationScale` | `number` | `1` | `0` to `2`. Above `1`, rain becomes a downpour: more and longer streaks and a grey veil that cuts visibility. |
| `lensDrops` | `boolean` | `false` | Raindrops land on the lens and run down the screen while it rains. Adds one full-screen pass. |
| `effects` | `WeatherEffects` | all `true` | `clouds`, `mist`, `wind`, `water`, `precipitation`, `ground`, `lightning`. |

`engine.getWeather()` returns the blended `WeatherState` (`from`, `to`,
`blend`, `cloudCoverage`, `cloudDensity`, `mistDensity`,
`windSpeedMetresPerSecond`, `windDirectionDegrees`, `rain`, `snow`,
`wetness`, `snowCover`, `lightning`, and the cloud type: `stratiform`,
`towering`, `baseDarkness`, `raggedBase`, `rainShafts`), or `undefined`
when the weather is off.

## `ShadowOptions`

Passed to `engine.setShadows()`. Every field is optional. See
[`docs/shadows.md`](shadows.md).

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `terrain.enabled` | `boolean` | `true` | Hills and mountains cast shadows. |
| `terrain.strength` | `number` | `0.9` | `0` to `1`. |
| `terrain.softness` | `number` | `0.35` | Penumbra width, `0` to `1`. |
| `trees.enabled` | `boolean` | `true` | Trees cast shadows. |
| `trees.distanceMetres` | `number` | `260` | `10` to `4000`. |
| `trees.resolution` | `512 \| 1024 \| 2048 \| 4096` | `2048` | Shadow map size. |
| `trees.strength` | `number` | `0.8` | `0` to `1`. |
| `trees.softness` | `number` | `0.5` | Filter width, `0` to `1`. |
| `clouds.enabled` | `boolean` | `true` | Also requires `CloudsOptions.castShadows`. |
| `clouds.strength` | `number` | `0.78` | `0` to `1`. |

## `SurfaceOptions`

Passed to `engine.setSurface()`. Every field is optional.

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `textures` | `boolean` | `true` | `false` shades each material as a flat colour. |
| `detailNormals` | `boolean` | `true` | Detail normal maps. |
| `textureScale` | `number` | `1` | `0.05` to `20`. Larger stretches textures over more ground. |
| `materialTints` | `[r, g, b][10]` | all `[1, 1, 1]` | Colour multipliers (`0` to `4`) for lush grass, dry grass, forest floor, sand, rock, snow, mud, volcanic, ice, tundra. A list of the first 8 is also accepted; ice and tundra then stay untinted. |

## `RenderQualityOptions`

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `preset` | `"preview" \| "balanced" \| "high" \| "offline"` | `"balanced"` | Fills in any distance below that is left unset (see [`docs/render-quality-and-diagnostics.md`](render-quality-and-diagnostics.md#distances-and-presets-renderqualityoptions)). Does not affect erosion; that is `ErosionOptions.quality`. |
| `maxClipmapLevels` | `number?` | `7` | Theoretical LOD-level budget used only by native/test builds without a GPU; browser builds report the real uploaded mesh's stats regardless of this value (see [`docs/render-quality-and-diagnostics.md`](render-quality-and-diagnostics.md)). |
| `floraDensityScale` | `number?` | `1.0` | Global multiplier applied on top of both `FloraOptions.density` and `GrassOptions.density`. The cheapest performance lever for vegetation-heavy scenes. |
| `renderDistanceMetres` | `number?` | from `preset` | At least `100`. Terrain, trees, and water beyond it are not shaded, hidden by horizon-coloured fog that is complete at this distance. |
| `renderFadeMetres` | `number?` | a third of the render distance | `0` or more, capped at the render distance. Length of the fog band before the render distance; `0` is a hard edge. |
| `detailDistanceMetres` | `number?` | from `preset` | At least `100`. Past it, terrain takes one far-scale texture sample per material instead of up to eight. |
| `cloudDistanceMetres` | `number?` | from `preset` | At least `100`. Clouds and rain curtains are raymarched only this far. |
| `cloudFadeMetres` | `number?` | 30 % of the cloud distance | `0` or more, capped at the cloud distance. Length of the band before the cloud distance over which clouds thin out. |
| `maxFrameRate` | `number?` | `60` | `0` or more. Frame-rate cap for `start()`, with evenly spaced frames. `0` renders on every animation frame. |
| `renderScale` | `number?` | `1` | `0.25` to `1`. Fraction of the canvas resolution the scene is rendered at; a final pass upscales and sharpens it. The highest scale dynamic resolution uses. |
| `dynamicResolution` | `boolean?` | `true` | Lower the render scale when frames arrive late, and raise it when there is time to spare, to hold `maxFrameRate` (or 60 when uncapped). |
| `minRenderScale` | `number?` | `0.5` | `0.25` to `1`. Lowest scale dynamic resolution may use; capped at `renderScale`. |

## `DebugView`

A plain string, not an object: `"none" | "height" | "slope" | "normals" |
"lod" | "flow" | "materials" | "no-data" | "biomes"`. Passed to
`engine.setDebugView()`. `"height"`, `"slope"`, `"normals"`, `"materials"`,
and `"biomes"` are implemented; the rest currently render like `"none"` —
see [`docs/render-quality-and-diagnostics.md`](render-quality-and-diagnostics.md).

## `FractalTerrainOptions`

Passed to `engine.generateFractal(options)`.

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `seed` | `number \| bigint` | — (required) | Deterministic seed for the terrain. Flora, grass, clouds, mist, biomes, and weather have their own `seedOffset`. |
| `size` | `512 \| 1024 \| 2048 \| 4096 \| 8192 \| number` | — (required) | Square terrain side length in samples. |
| `horizontalScaleMetres` | `number` | — (required) | Metres between adjacent samples. |
| `verticalScale` | `number` | — (required) | Stretches heights about sea level. Must be greater than 0. |
| `baseHeightMetres` | `number?` | `0` | Offset added after scaling; raises or sinks the whole map, coast included. |
| `seaLevelMetres` | `number?` | `0` | Where the generated coast sits, and the initial `TerrainMetadata.seaLevelMetres`. Independent from `WaterOptions.seaLevelMetres`, which controls the rendered water plane. |
| `landform` | `LandformKind?` | `"continental"` | The character of the land. See below and [`docs/terrain-data.md`](terrain-data.md#landforms). Unknown names are rejected with a list of the valid ones. |
| `edges` | `"coast" \| "open"` | `"coast"` | `"coast"` keeps the land inside the map, ringed by sea along a natural coastline. `"open"` lets land run to the map edge, for tiling several maps. See [`docs/terrain-data.md`](terrain-data.md#map-edges). Unknown values are rejected with a list of the valid ones. |
| `noise` | `NoiseOptions` | see below | The detail layer. |
| `shape` | `TerrainShapeOptions?` | none | |
| `erosion` | `ErosionOptions?` | none (disabled) | |

`size` must be a power of two from 16 to 8192, and `horizontalScaleMetres`
must be greater than 0.

### `LandformKind`

| Value | Land | Ranges | Notes |
| --- | --- | --- | --- |
| `"continental"` | 70 % | up to 1400 m on 40 % of the land | Mixed plains, hills and ranges. |
| `"alpine"` | 95 % | up to 2600 m on 85 % of the land | Glacial valleys and lakes. |
| `"rollingHills"` | 90 % | none | Lowlands up to 250 m; nothing steeper than 30 degrees. |
| `"archipelago"` | 35 % | up to 600 m | Many islands of varied size. |
| `"mesaDesert"` | 97 % | up to 420 m | Terraced plateaus, low rain. |
| `"fjords"` | 75 % | up to 1600 m | Flooded U-shaped glacial valleys. |
| `"volcanicIsland"` | 30 % | a 1500 m cone | Crater lake, radial gullies and a reef shelf. |

Relief shrinks on maps too small to hold a full range: ranges stand at most
a quarter of their wavelength, which is at most 0.6 times the map's width.

### `NoiseOptions`

Controls the detail layer added to the carved land. Detail is strongest on
steep ground in the ranges and fades out on level ground.

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `kind` | `"simplex" \| "ridged" \| "hybrid" \| "island" \| "canyon" \| "cratered" \| "classic"` | `"ridged"` | Flavour of the detail. `island`, `canyon` and `cratered` also apply their shape masks. See [`docs/world-design-guide.md`](world-design-guide.md#2-noise-kinds). |
| `octaves` | `number` | `7` | At most this many detail octaves, 1 to 16. Octaves finer than 2.5 samples are skipped. |
| `gain` | `number` | `0.5` | Amplitude multiplier between octaves, 0 to 1. |
| `lacunarity` | `number` | `2.0` | Frequency multiplier between octaves; greater than 1. |
| `warp` | `number?` | `0` | Domain warp of the detail, 0 to 4. |

### `TerrainShapeOptions`

All fields optional, roughly `0..1` strength dials (see
[`docs/world-design-guide.md`](world-design-guide.md#3-shape-controls)):
`island`, `terrace`, `basin`, `canyon`, `crater`.

### `ErosionOptions`

Passing `erosion` at all enables erosion; omit it entirely to skip erosion.
Unset fields take the landform's defaults, and unset iteration counts
follow `quality`.

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `hydraulicIterations` | `number?` | from `quality` | Virtual-pipe water iterations, 0 to 5000, capped by `quality`. |
| `thermalIterations` | `number?` | from `quality` | Talus and soil-creep iterations, 0 to 5000, capped by `quality`. |
| `rainAmount` | `number?` | `0.02` × landform rain | Rain per iteration, 0 to 1. |
| `evaporation` | `number?` | `0.5` | Water loss per iteration, 0 to 1. |
| `sedimentCapacity` | `number?` | `0.04` | How much sediment water can carry, 0 to 1. |
| `talusAngleDegrees` | `number?` | the landform's (30 to 44) | Slope above which thermal erosion moves material, 1 to 89. |
| `quality` | `"preview" \| "balanced" \| "high" \| "offline"?` | `"preview"` | Default iterations 60/30, 120/60, 200/100 or 400/200 (hydraulic/thermal); caps 120, 240, 400 or 5000. |

Erosion runs as GPU compute passes on browser builds (CPU fallback on any
GPU error); native/test builds always use the CPU path — see
[`docs/terrain-data.md`](terrain-data.md#erosion).

## `DemLoadOptions`

Passed to `engine.loadDemFromArrayBuffer(buffer, options)` /
`engine.loadDemFromUrl(url, options)`.

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `verticalScale` | `number?` | `1` | Height multiplier applied after decode. |
| `generateNormals` | `boolean?` | `true` | |
| `generateMaterialMasks` | `boolean?` | `true` | |

See [`docs/terrain-data.md`](terrain-data.md#dem-import-geotiff) for the supported
GeoTIFF subset and what decode warnings mean.

## `RawHeightmapOptions`

Passed to `engine.loadRawHeightmap(buffer, options)`.

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `width` | `number` | — (required) | Samples. |
| `height` | `number` | — (required) | Samples. |
| `sampleFormat` | `"uint16" \| "int16" \| "float32"` | — (required) | |
| `byteOrder` | `"little-endian" \| "big-endian"?` | `"little-endian"` | |
| `metresPerSample` | `number` | — (required) | |
| `heightScaleMetres` | `number` | — (required) | |
| `noDataValue` | `number?` | none | Samples equal to this value are marked no-data. |
| `seaLevelMetres` | `number?` | `0` | |

## Export options

### `ExportHeightmapOptions`

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `format` | `"float32-le"?` | `"float32-le"` | Currently the only supported export format. |

### `SnapshotOptions`

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `mimeType` | `string?` | `"image/png"` | Passed to `canvas.toBlob()`. |
| `quality` | `number?` | browser default | Only meaningful for lossy formats. |

See [`docs/export-and-snapshots.md`](export-and-snapshots.md) for the full
set of export helpers, including the ones with their own option types
(`HeightmapImageOptions`, `TerrainObjExportOptions`).

## Read-only shapes returned by the engine

These are never passed *in* — the engine returns them.

### `TerrainMetadata` (on `TerrainHandle.metadata`)

| Field | Type | Notes |
| --- | --- | --- |
| `width`, `height` | `number` | Heightmap dimensions in samples. |
| `metresPerSample` | `number` | |
| `verticalScale` | `number` | |
| `seaLevelMetres` | `number` | |
| `minHeightMetres`, `maxHeightMetres`, `meanHeightMetres` | `number` | Real height range of the loaded/generated terrain — use these, not hardcoded constants, when placing a camera or setting `WaterOptions.seaLevelMetres`. |
| `source` | `string` | `"fractal"`, `"raw-heightmap"`, or `"geotiff"`. |
| `generatorVersion` | `string` | |
| `geospatial` | `GeospatialMetadata \| null` | Present for GeoTIFF sources with recognised tags. |
| `warnings` | `string[]` | Non-fatal decode/generation warnings — see [`docs/events-errors-and-lifecycle.md`](events-errors-and-lifecycle.md#events). |

### `RenderStats` (from `engine.renderOnce()` and the `"stats"` event)

| Field | Type | Notes |
| --- | --- | --- |
| `frameIndex` | `number` | Monotonic. |
| `frameTimeMs` | `number` | CPU time to record and submit the frame, measured by the JS wrapper. Does not include GPU time; see [`docs/render-quality-and-diagnostics.md`](render-quality-and-diagnostics.md#render-statistics-renderstats). |
| `gpuFrameTimeMs` | `number \| null` | Sum of `gpuPassTimesMs`, or `null` without timestamp queries. |
| `terrainTriangles` | `number` | Real uploaded mesh triangle count on browser builds; a theoretical estimate on native/test builds. |
| `floraInstances` | `number` | |
| `grassInstances` | `number` | |
| `clipmapLevels` | `number` | Number of exponential LOD bands the terrain mesh's half-span currently spans. |
| `activeGpuMemoryBytes` | `number \| null` | Not currently populated. |
| `weather` | `WeatherKind \| null` | The dominant weather, or `null` when the weather system is off. |
| `renderScale` | `number` | Fraction of the canvas resolution the scene was rendered at. |
| `gpuPassTimesMs` | `GpuPassTimes \| null` | GPU milliseconds per pass (`terrain`, `trees`, `grass`, `clouds`, `skyAndFog`, `water`, `shadows`, `treeCulling`, `present`), when the browser supports timestamp queries. A few frames behind. |

See [`docs/render-quality-and-diagnostics.md`](render-quality-and-diagnostics.md)
for how to use these for a performance HUD.
