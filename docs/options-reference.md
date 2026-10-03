# Options Reference

This is a field-by-field reference for every public VistaWASM option, in
TypeScript's camelCase naming (the Rust engine uses the equivalent
`snake_case` internally; both are the same value once decoded). Types shown
are the TypeScript shapes from `js/src/types.ts`. Every option is validated
by the Rust engine — an invalid value throws a `VistaWasmError` with code
`OPTIONS_INVALID` rather than being silently clamped, except where noted.

The TypeScript wrapper checks every options object first, before it
reaches the engine:

- Anything but an object (`null`, a number, a string, an array) throws a
    `TypeError` saying which call expected an options object.
- No key at any depth may be `__proto__`, `constructor` or `prototype`
    (`OPTIONS_INVALID`), so options parsed from JSON cannot change object
    prototypes when they are copied.
- Options may nest at most 8 levels deep (`MAX_OPTIONS_DEPTH`), which
    also rejects an object that contains itself. Typed arrays inside
    options are not walked.

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
- [`WeatherOptions`](#weatheroptions) and [`WeatherPreset`](#weatherpreset)
- [`TimeOfDayOptions`](#timeofdayoptions)
- [`ShadowOptions`](#shadowoptions)
- [`SurfaceOptions`](#surfaceoptions)
- [`RenderQualityOptions`](#renderqualityoptions)
- [`DebugView`](#debugview)
- [`FractalTerrainOptions`](#fractalterrainoptions)
- [`DemLoadOptions`](#demloadoptions)
- [`DemFetchOptions`](#demfetchoptions)
- [`RawHeightmapOptions`](#rawheightmapoptions)
- [`ExportHeightmapOptions` / `SnapshotOptions`](#export-options)
- [Map, tree and bundle export](#map-tree-and-bundle-export) (`ExportMapOptions`, `ExportTreesOptions`, `PngOptions`, `RawOptions`, `BundleOptions`)
- [Map import](#map-import) (`HeightmapImageImportOptions`, `BiomeMap`, `DensityMask`, `BiomeMapFromImageOptions`, `WaterMaskFromImageOptions`, `DensityMaskFromImageOptions`, `BundleLoadOptions`)
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
| `grass` | `GrassOptions` | see [`GrassOptions`](#grassoptions) (on by default) |
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
| `width` | `number` | `1` | CSS pixels, `1` to `8192`. |
| `height` | `number` | `1` | CSS pixels, `1` to `8192`. |
| `devicePixelRatio` | `number?` | `1` | Must be finite, `> 0`, and `<= 8`. |

Also used by `engine.resize(width, height, devicePixelRatio?)`, which
takes `width` and `height` from 0 to 8192 CSS pixels (0 still gives a
1-pixel canvas, as a hidden element measures 0) and `devicePixelRatio`
over 0 and at most 8, defaulting to `window.devicePixelRatio`. It checks
them before the canvas changes: `NaN`, `Infinity`, negative or larger
values throw `OPTIONS_INVALID`, where a non-finite ratio used to become 1.
The drawing buffer is at most 8192 device pixels a side; beyond that it
is clamped and the browser scales it up.

## `CameraOptions`

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `position` | `[number, number, number]` | `[0, 120, 300]` | World position in terrain metres, each within `±10,000,000`. |
| `target` | `[number, number, number]` | `[0, 0, 0]` | Look-at point in terrain metres, each within `±10,000,000`. |
| `rollDegrees` | `number?` | `0` | Must be finite. Accepted and stored, but **not currently applied** to the view matrix — the camera always uses a fixed world-up vector. Reserved for future use. |
| `fieldOfViewDegrees` | `number` | `55` | Must be between `1` and `160`. |
| `nearMetres` | `number?` | `0.5` | Must be positive, at most `1e9`, and less than `farMetres`. |
| `farMetres` | `number?` | `120000` | Must be positive, at most `1e9`, and greater than `nearMetres`. |
| `minimumHeightAboveTerrainMetres` | `number?` | `2` | `0` to `10,000,000`. Accepted and stored, but **not currently enforced** by the engine — nothing clamps the camera's height against the terrain today. If you need a ground clamp, compute it yourself against an exported heightmap (see [`docs/camera-and-controls.md`](camera-and-controls.md#terrain-clamping-is-not-built-in)). |
| `allowUnderground` | `boolean?` | `false` | Accepted and stored, but **not currently enforced** — see the note above. `attachFlyCameraControls()` always behaves as if this were `true` internally. |

See [`docs/camera-and-controls.md`](camera-and-controls.md) for the
projector model and the bundled fly-camera controller.

## `SunOptions`

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `azimuthDegrees` | `number` | `132` | Compass direction the sun shines from. Must be finite. |
| `elevationDegrees` | `number` | `18` | Angle above the horizon, `-90` to `90`. Negative values put the sun below the horizon. |
| `intensity` | `number` | `1.2` | Light brightness multiplier, `> 0` and at most `100`. |

While the time of day is enabled (see [`TimeOfDayOptions`](#timeofdayoptions)),
it sets `azimuthDegrees` and `elevationDegrees`, and `setSun` sets only
`intensity`.

## `AtmosphereOptions`

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `rayleighStrength` | `number` | `1.0` | Sky gradient saturation, `0` to `100`. |
| `mieStrength` | `number` | `0.45` | Sun glare/haze size, `0` to `100`. |
| `hazeDistanceMetres` | `number` | `60000` | Distance haze (aerial perspective), thinning gently with altitude. `> 0` and at most `1e9`. See the haze-vs-mist distinction in [`docs/sky-atmosphere-and-weather.md`](sky-atmosphere-and-weather.md). |
| `exposure` | `number` | `1.1` | Overall brightness multiplier, `> 0` and at most `100`. |
| `skyTint` | `[number, number, number]` | `[1, 1, 1]` | RGB multiplier applied to the whole sky/haze/cloud result, each `0` to `4`. |

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
| `baseVariation` | `number?` | `0.07` | How far low-cloud bases vary in height, from cloud to cloud and within each cloud, `0` (one flat level) to `0.2` of `thicknessMetres`. The lowest bases rest at `heightMetres`. Fades out as `stratiform` rises. Volumetric only. |
| `baseLumpiness` | `number?` | `0.6` | Soft, cotton-wool lumps under low clouds, `0` (wispy undersides) to `1`. Volumetric only. |
| `altocumulus` | `number?` | `0` | Rippled mid-level cloudlets (a "mackerel sky"), `0` (none) to `1`. Needs a cloud style other than `"off"`. |
| `altostratus` | `number?` | `0` | A grey mid-level veil that turns the sun into a watery disc, `0` (none) to `1`. At `1` it lets through about 8 % of the sunlight overhead. Needs a cloud style other than `"off"`. |
| `altoHeightMetres` | `number?` | `4200` | Height of the mid-level layer, `2000` to `7000`. Kept at least 300 m above the top of the low clouds and, where there is room, 500 m below the cirrus. |
| `altoSpeed` | `number?` | `1` | Mid-level drift speed, `0` to `4`, in units of 15 m/s, along the cloud wind veered 20 degrees. |

When the weather system drives clouds, it sets the five cloud-type fields,
the two base fields and the four mid-level fields for you (see
[`docs/weather.md`](weather.md#preset-fields)).

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
| `seaLevelMetres` | `number` | `0` | Set relative to `TerrainMetadata.minHeightMetres`/`meanHeightMetres`, not a hardcoded constant — see [`docs/water.md`](water.md). Within `±100,000`. |
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
| `eddies` | `number?` | `1` | Eddies and vortices in flowing rivers, `0` (off) to `1`: slow, reversed water and turning vortices on the inner bank below bends, beside joins, below falls and behind stones, and small boils in fast water. Changing it does not rebuild rivers. |
| `refraction` | `number?` | `1` | Seeing into shallow water, `0` (off) to `1`: the bed under water less than 2 m deep bends with the ripples (read from the scene behind the water), and sunlight the ripples focus plays over it as caustics, drifting with the flow and gone under cloud. Changing it does not rebuild rivers. |
| `reflections` | `"screen" \| "sky"?` | `"screen"` | `"screen"` reflects the terrain, trees and banks on screen, falling back to the sky where a reflected ray leaves the screen; `"sky"` reflects the sky and clouds only, and costs less. |

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
| `currentSpeed` | `number?` | `1` | River current speed multiplier. Must be `>= 0`. |
| `snowmelt` | `number?` | `1` | How strongly snow fields and glaciers feed streams, `0` (none) to `2`. Streams also start at glacier snouts and at the lower edge of snowy peaks. |
| `springs` | `boolean?` | `true` | Small springs at the foot of steep slopes. |
| `meanders` | `number?` | `0.6` | How strongly lowland bends migrate by bank erosion, `0` (rivers follow their valley without meandering) to `1`. |
| `meanderMaturity` | `number?` | `0.5` | How long meanders have been developing, `0` (young, gentle bends) to `1` (mature loops, neck cut-offs and oxbow lakes). |
| `braiding` | `number?` | `1` | Braided threads between gravel bars on steep, wide, unconfined valley floors, `0` (never) to `1` (wherever slope and discharge call for them). |
| `waterfalls` | `boolean?` | `true` | Waterfalls, with mist and plunge pools, where rivers cross steps and cliffs. |
| `inflow` | `"auto" \| "none" \| RiverInflow[]?` | `"auto"` | Water from beyond the map. `"auto"` places one inflow at the lowest valley mouth on an open edge, an eighth of the map from the sea, sized from a basin ten times the map's land area; `"none"` adds nothing; or up to 8 `{ position: [x, z], dischargeCubicMetresPerSecond }` (`0` to `100000`), each snapped to the nearest land sample. Positions must be on the map. |
| `riparian` | `number?` | `1` | Bankside greening and plants, `0` (off) to `2`: moister ground, greener biomes and more trees within 25 to 400 m of rivers (by discharge) and 40 m of lakes, and the band of tufts, tall herbs and scrub lining every channel (`0` removes it, `2` doubles it). |

`minCatchmentKm2` becomes a discharge threshold of 0.03 m³/s per km².
`getInflows()` returns the inflows in use as `WaterInflow` objects:
`position` (world x, y, z on the land sample the inflow snapped to) and
`dischargeCubicMetresPerSecond`.
See [`docs/water.md`](water.md#rivers-and-lakes-rivers) for how rivers
form.

## `FloraOptions`

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `enabled` | `boolean` | `true` | |
| `density` | `number` | `0.35` | `0` to `4`. `1` is the old maximum; `4` is a closed canopy where the land supports it. Scaled by `RenderQualityOptions.floraDensityScale`. See [`docs/vegetation.md`](vegetation.md#density). |
| `treeLineMetres` | `number` | `1800` | Altitude above which trees stop spawning. |
| `seedOffset` | `number \| bigint` | `3001` | Deterministic placement seed. |
| `maxInstances` | `number` | `500000` | `0` to `4000000`. Upper bound on trees generated (the far set plus the streamed tiles), also clamped to the active device's limits. |
| `treeQuality` | `"billboard" \| "cross-quad" \| "mesh"` | `"mesh"` | See [`docs/vegetation.md`](vegetation.md#levels-of-detail). |
| `speciesVariation` | `number?` | `0.6` | `0` to `1`. Per-tree size and colour variety. |
| `windStrength` | `number?` | `0.3` | `0` to `1`. Wind sway strength. |
| `meshDistanceMetres` | `number?` | `420` | Distance at which `"mesh"` trees cross-fade to impostors, at most 150 m. Full meshes give way to lighter ones at 50 m, or a third of this if nearer. `> 0` and at most `5000`. See [`docs/vegetation.md`](vegetation.md#levels-of-detail). |
| `variantsPerSpecies` | `number?` | `4` | `1` to `4`. Distinct grown shapes per species. The first grows before the first frame and the rest over the frames after it. Fewer use less GPU memory: about 10 MB of impostors each. See [`docs/vegetation.md`](vegetation.md#variants-ages-and-lean). |
| `speciesRules` | `FloraRule[]?` | `[]` | Per-biome species mix and density, replacing the built-in mix. At most 64 rules of 8 species each. See [`docs/hooks.md`](hooks.md#species-rules). |

Species and density follow the biome under each tree, and placement avoids
underwater, river, and steep terrain; see
[`docs/vegetation.md`](vegetation.md#placement).

## `GrassOptions`

Grass is on by default, at density `0.5`: a natural meadow near the
camera, for well under a millisecond a frame. Pass `enabled: false` to
turn it off.

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `enabled` | `boolean` | `true` | |
| `style` | `"billboard-blades" \| "dense-blades"` | `"billboard-blades"` | See [`docs/vegetation.md`](vegetation.md#grass-grassoptions). |
| `density` | `number` | `0.5` | `0` to `4`: `0.5` is a natural, dense meadow covering at least 70 % of the ground near the camera, `1` a lush, taller meadow, `4` long, dense grass. Scaled by `RenderQualityOptions.floraDensityScale`. See [`docs/vegetation.md`](vegetation.md#density-and-placement). |
| `viewDistanceMetres` | `number` | `220` | Distance from the camera at which grass fully fades out. `> 0` and at most `1000`. |
| `seedOffset` | `number \| bigint` | `7331` | Deterministic placement seed. |
| `maxInstances` | `number` | `200000` | `0` to `4000000`. Upper bound on tufts and reeds generated, also clamped to the active device's limits. The streamed tiles' slots fit within it: where they would not, the full-density radius comes in and the widened tufts beyond it keep the cover (see `docs/vegetation.md`). |
| `forestFloor` | `boolean?` | `true` | Ferns and undergrowth replace grass under dense canopy. See [`docs/vegetation.md`](vegetation.md#forest-floor). |

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
| `state` | `WeatherKind` | `"partlyCloudy"` | Any preset: `"clear"`, `"fewClouds"`, `"partlyCloudy"`, `"brokenClouds"`, `"overcast"`, `"mist"`, `"fog"`, `"lightRain"`, `"rain"`, `"heavyRain"`, `"storm"`, `"snow"`, `"blizzard"`, or a name from `presets`. |
| `autoCycle` | `boolean` | `false` | Move on to new weather over time. |
| `stateDurationSeconds` | `number` | `240` | Average length of each preset when cycling, for presets that set no durations. `> 0` and at most `86400`. |
| `transitionSeconds` | `number` | `30` | Blend time between states, `0` to `86400`. |
| `allowSnow` | `boolean` | `false` | Whether cycling may choose snow. Below 0 °C under the camera, snow is always allowed. |
| `seedOffset` | `number \| bigint` | `4111` | Seed for the cycle sequence and gusts. |
| `windDirectionDegrees` | `number` | `70` | Prevailing wind direction. |
| `windScale` | `number` | `1` | `0` to `4`. |
| `precipitationScale` | `number` | `1` | `0` to `2`. Above `1`, rain becomes a downpour: more and longer streaks and a grey veil that cuts visibility. |
| `lensDrops` | `boolean` | `false` | Raindrops land on the lens and run down the screen while it rains. Adds one full-screen pass. |
| `lensDropCount` | `number` | `60` | Drops on the lens at once in full rain, an integer from `0` to `512`. It scales with the rain's intensity. |
| `lensDropMinSize` | `number` | `0.008` | Smallest drop diameter as a fraction of the canvas height, `0.002` to `0.2` (0.008 is about 9 px at 1080p). |
| `lensDropMaxSize` | `number` | `0.05` | Largest drop diameter as a fraction of the canvas height, `0.002` to `0.2`, and at least `lensDropMinSize`. |
| `effects` | `WeatherEffects` | all `true` | `clouds`, `mist`, `wind`, `water`, `precipitation`, `ground`, `lightning`. |
| `presets` | `Record<string, WeatherPreset>` | `{}` | Custom presets, and overrides of built-in ones (only the fields given). A plain object, not a `Map` or an object with another prototype (`TypeError`), of at most 64 presets (`MAX_WEATHER_PRESETS`) whose names are 1 to 64 characters long (`MAX_PRESET_NAME_LENGTH`). `state` must be a name of that length too. See [`WeatherPreset`](#weatherpreset). |
| `regional` | `boolean` | `true` | The weather varies across the map. When `false`, it is the same everywhere. |
| `regionSizeKm` | `number` | `64` | Size of the regional weather map, `16` to `256`, centred on the terrain. |

`engine.getWeather()` returns the blended `WeatherState` at the camera
(`from`, `to`, `blend`, `cloudCoverage`, `cloudDensity`, `mistDensity`,
`windSpeedMetresPerSecond`, `windDirectionDegrees`, `rain`, `snow`,
`wetness`, `snowCover`, `lightning`, the cloud type: `stratiform`,
`towering`, `baseDarkness`, `raggedBase`, `rainShafts`, and `humidity`,
`turbidity`, `storminess`, `sunTransmittance`, `gustiness` and
`puddles`), or `undefined` when the weather is off.

Other weather calls:

| Call | Returns | Notes |
| --- | --- | --- |
| `engine.getWeatherPresets()` | `Record<string, ResolvedWeatherPreset>` | Every preset, built in and custom, with every field of `WeatherPreset` but `extends` set. |
| `engine.weatherAt(x, z)` | `LocalWeather \| null` | `coverage`, `precipitation`, `storminess`, `humidity`, `wetness`, `puddles` and `snowDepth` at a world position; `null` with no terrain; ground values 0 beyond it. Throws a `TypeError` unless both arguments are finite. |
| `engine.advanceWeather(seconds)` | — | Runs the weather, the time of day and the wet ground forward at once, in one-second steps. `seconds` must be from `0` to `86400`. |

### `WeatherPreset`

Every field is optional. Unknown fields are rejected, with the valid
ones listed. Numbers outside their range are rejected with the range.
There may be at most 64 custom presets, and preset names (including
`weather.state`, `extends` and the names in `next`) are at most 64
bytes long.

| Field | Range | Notes |
| --- | --- | --- |
| `extends` | preset name | Inherit unset fields. Unset: a built-in name keeps its own values; a new name starts from `"partlyCloudy"`. Cycles are rejected. |
| `cloudCoverage` | `0` to `1` | Mean cloud coverage. |
| `cloudDensity` | `0` to `1` | Cloud density. |
| `cloudThickness` | `0.2` to `3` | Multiplier on `CloudsOptions.thicknessMetres`. |
| `stratiform`, `towering`, `baseDarkness`, `raggedBase`, `rainShafts`, `cirrus` | `0` to `1` | Cloud type (see [`docs/weather.md`](weather.md#preset-fields)). |
| `baseVariation` | `0` to `0.2` | Variation of low-cloud base height, as a share of the layer thickness. |
| `baseLumpiness`, `altocumulus`, `altostratus` | `0` to `1` | Lumps under low clouds, and the mid-level cloud amounts (see [`docs/weather.md`](weather.md#mid-level-clouds-in-the-presets)). |
| `altoHeightMetres` | `2000` to `7000` | Height of the mid-level layer. |
| `altoSpeed` | `0` to `4` | Mid-level drift speed, in units of 15 m/s. |
| `humidity` | `0` to `1` | Whitens and thickens the haze. |
| `turbidity` | `2` to `10` | Aerosols: clean alpine air to thick tropical haze. `3` leaves the sky as without weather. |
| `hazeDistanceScale` | `0.05` to `2` | Multiplier on `AtmosphereOptions.hazeDistanceMetres`. |
| `mistDensity` | `0` to `1` | Ground mist. |
| `temperatureOffsetCelsius` | `-30` to `30` | Added to the climate's temperature, for rain or snow, melting and rivers. |
| `windMetresPerSecond` | `0` to `60` | Mean wind, times `windScale`. |
| `gustiness` | `0` to `1` | Gust strength. |
| `rain`, `snow` | `0` to `2` | Precipitation, times `precipitationScale`; above 1 a downpour. |
| `lightningPerMinute` | `0` to `60` | Lightning. |
| `coverageSpread`, `precipitationSpread` | `0` to `1` | Variation across the regional map. |
| `cellSizeKm` | `1` to `100` | Weather cell size. |
| `cellularity` | `0` to `1` | 0 smooth fields, 1 discrete storm cells. |
| `next` | `Record<string, number>` | Successor weights, `0` or more, naming presets. |
| `minDurationSeconds`, `maxDurationSeconds` | `1` to `86400` | How long the preset lasts when cycling; min not above max. |
| `climate` | `{ minCelsius?, maxCelsius? }` | `-60` to `60` °C, min not above max: temperatures at the camera the preset suits when cycling. |

## `TimeOfDayOptions`

Passed to `engine.setTimeOfDay()`. Every field is optional; unset fields
take their defaults. See
[`docs/sky-atmosphere-and-weather.md`](sky-atmosphere-and-weather.md#time-of-day).

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `enabled` | `boolean` | `false` | The sun follows the time of day. |
| `hours` | `number` | `12` | Local time, `0` to `24`. |
| `dayLengthMinutes` | `number` | `24` | Real minutes for a full day, `1` to `1440`. |
| `latitudeDegrees` | `number` | `45` | `-89` to `89`, north positive. |
| `dayOfYear` | `number` | `172` | A whole number, `1` to `366`. |

`engine.getTimeOfDay()` returns `{ hours, sunAzimuthDegrees,
sunElevationDegrees, sunriseHours, sunsetHours }`; the azimuth is in the
convention of `SunOptions.azimuthDegrees`, and sunrise and sunset are
`null` during polar day or night.

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
| `materialTints` | `[r, g, b][12]` | all `[1, 1, 1]` | Colour multipliers (`0` to `4`) for lush grass, dry grass, forest floor, sand, rock, snow, mud, volcanic, ice, tundra, river gravel, scree. A list of the first 8, 10 or 11 is also accepted; the rest then stay untinted. Boulders take the rock tint. |
| `rockiness` | `number` | `1` | `0` to `2`. How readily bedrock shows through thin soil: `0` keeps deep soil everywhere, `2` halves the soil's depth. Changing it bakes the ground, trees and grass again. See [Rock and soil](terrain-data.md#rock-and-soil). |
| `boulders` | `boolean` | `true` | Fallen boulders and talus below rock outcrops, drawn in 3D. |
| `boulderDistanceMetres` | `number` | `300` | `50` to `1000`. Boulders within this distance are drawn; beyond it they shrink away, and the scree texture carries the look. Detail pressure brings it in, as it does vegetation. |

## `RenderQualityOptions`

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `preset` | `"preview" \| "balanced" \| "high" \| "offline"` | `"balanced"` | Fills in any distance below that is left unset (see [`docs/render-quality-and-diagnostics.md`](render-quality-and-diagnostics.md#distances-and-presets-renderqualityoptions)). Does not affect erosion; that is `ErosionOptions.quality`. |
| `maxClipmapLevels` | `number?` | `7` | `1` to `12`. Theoretical LOD-level budget used only by native/test builds without a GPU; browser builds report the real uploaded mesh's stats regardless of this value (see [`docs/render-quality-and-diagnostics.md`](render-quality-and-diagnostics.md)). |
| `floraDensityScale` | `number?` | `1.0` | `0` to `4`. Global multiplier applied on top of both `FloraOptions.density` and `GrassOptions.density`. The cheapest performance lever for vegetation-heavy scenes. |
| `renderDistanceMetres` | `number?` | from `preset` | `100` to `1e9`. Terrain, trees, and water beyond it are not shaded, hidden by horizon-coloured fog that is complete at this distance. |
| `renderFadeMetres` | `number?` | a third of the render distance | `0` or more, capped at the render distance. Length of the fog band before the render distance; `0` is a hard edge. |
| `detailDistanceMetres` | `number?` | from `preset` | At least `100`. Past it, terrain takes one far-scale texture sample per material instead of up to eight. |
| `cloudDistanceMetres` | `number?` | from `preset` | At least `100`. Clouds and rain curtains are raymarched only this far. |
| `cloudFadeMetres` | `number?` | 30 % of the cloud distance | `0` or more, capped at the cloud distance. Length of the band before the cloud distance over which clouds thin out. |
| `maxFrameRate` | `number?` | `60` | `0` to `1000`. Frame-rate cap for `start()`, with evenly spaced frames. `0` renders on every animation frame. |
| `renderScale` | `number?` | `1` | `0.25` to `1`. Fraction of the canvas resolution the scene is rendered at; a final pass upscales and sharpens it. The highest scale dynamic resolution uses. |
| `dynamicResolution` | `boolean?` | `true` | Lower the render scale when frames arrive late, and raise it when there is time to spare, to hold `maxFrameRate` (or 60 when uncapped). |
| `minRenderScale` | `number?` | `0.5` | `0.25` to `1`. Lowest scale dynamic resolution may use; capped at `renderScale`. |
| `vegetationDetailMetres` | `number?` | from `preset` | `50` to `5000`. Radius of full-density trees round the camera, once they are streamed; the budgets may shrink it. See [`docs/vegetation.md`](vegetation.md#budgets). |
| `canopyDistanceMetres` | `number?` | from `preset` | At least `50`. Distance where individual trees give way to the canopy layer, once the forest is too large to hold whole (over 50,000 trees). |
| `maxTreeInstances` | `number?` | from `preset` | `1000` to `4000000`. Most trees drawn per frame; a twentieth of it are full meshes at most, once trees are streamed. |
| `maxGrassInstances` | `number?` | from `preset` | `1000` to `4000000`. Most grass tufts drawn per frame. |
| `maxTreeTriangles` | `number?` | from `preset` | `100000` to `50000000`. Most tree triangles drawn per frame (meshes, impostors and shadow casters): over it, the furthest meshes become impostors first, and shadow casters keep to a quarter of it. `1000000`, `2500000` or `5000000`; no limit at `"offline"`. See [`docs/vegetation.md`](vegetation.md#budgets). |
| `grassDetailMetres` | `number?` | from `preset` | `10` to `300`. Radius of full-density grass round the camera: `25`, `45`, `70` or `120`, less if `GrassOptions.maxInstances` cannot hold it. Beyond it tufts thin out, widened to keep the cover, and from 1.5 times it (at least 0.3 times the grass view distance) hand over to the ground's grass sheen. |
| `splitTreeTiming` | `boolean?` | `false` | Time the trees pass as three passes (canopy meshes, understorey meshes and impostors), reported in `gpuPassTimesMs`. For profiling; the extra passes cost a little. |

## `DebugView`

A plain string, not an object: `"none" | "height" | "slope" | "normals" |
"lod" | "flow" | "materials" | "no-data" | "biomes"`. Passed to
`engine.setDebugView()`; any other value throws a `TypeError` listing these.
`"height"`, `"slope"`, `"normals"`, `"materials"`,
and `"biomes"` are implemented; the rest currently render like `"none"` —
see [`docs/render-quality-and-diagnostics.md`](render-quality-and-diagnostics.md).

## `FractalTerrainOptions`

Passed to `engine.generateFractal(options)`.

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `seed` | `number \| bigint` | — (required) | Deterministic seed for the terrain. Flora, grass, clouds, mist, biomes, and weather have their own `seedOffset`. |
| `size` | `512 \| 1024 \| 2048 \| number` | — (required) | Square terrain side length in samples. |
| `horizontalScaleMetres` | `number` | — (required) | Metres between adjacent samples, `> 0` and at most `10000`. |
| `verticalScale` | `number` | — (required) | Stretches heights about sea level, `> 0` and at most `100`. |
| `baseHeightMetres` | `number?` | `0` | Offset added after scaling; raises or sinks the whole map, coast included. |
| `seaLevelMetres` | `number?` | `0` | Where the generated coast sits, and the initial `TerrainMetadata.seaLevelMetres`. Independent from `WaterOptions.seaLevelMetres`, which controls the rendered water plane. |
| `landform` | `LandformKind?` | `"continental"` | The character of the land. See below and [`docs/terrain-data.md`](terrain-data.md#landforms). Unknown names are rejected with a list of the valid ones. |
| `edges` | `"coast" \| "open"` | `"coast"` | `"coast"` keeps the land inside the map, ringed by sea along a natural coastline. `"open"` lets land run to the map edge, for tiling several maps. See [`docs/terrain-data.md`](terrain-data.md#map-edges). Unknown values are rejected with a list of the valid ones. |
| `noise` | `NoiseOptions` | see below | The detail layer. |
| `shape` | `TerrainShapeOptions?` | none | |
| `erosion` | `ErosionOptions?` | none (disabled) | |

`size` must be a power of two from 16 to 2048 (`MAX_TERRAIN_SIDE`). The
limit is set by WebAssembly's memory, not by a flaw in VistaWASM: a
terrain, and the work of rebuilding and exporting it, must fit in the
4 GiB a WASM module can address (see [`security.md`](security.md#terrain-size)). `baseHeightMetres` and
`seaLevelMetres` must lie within `±100,000`.

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

Relief shrinks on maps too small to hold a full range. A range stands at
most a set fraction of its wavelength (which is itself at most 0.6 times
the map's width): 0.55 for `"alpine"` and `"fjords"`, 0.4 for
`"archipelago"` and `"volcanicIsland"`, 0.35 for `"continental"` and 0.3
for `"mesaDesert"`. On a 512 x 512 map at 12 m (6.1 km across), alpine
ranges reach 1,500 to 2,000 m, fjords 1,100 to 1,700 m, and continental
ranges 900 to 1,400 m.

### `NoiseOptions`

Controls the detail layer added to the carved land. Detail is strongest on
steep ground in the ranges and fades out on level ground.

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `kind` | `"simplex" \| "ridged" \| "hybrid" \| "island" \| "canyon" \| "cratered" \| "classic"` | `"ridged"` | Flavour of the detail. `island`, `canyon` and `cratered` also apply their shape masks. See [`docs/world-design-guide.md`](world-design-guide.md#2-noise-kinds). |
| `octaves` | `number` | `7` | At most this many detail octaves, 1 to 16. Octaves finer than 2.5 samples are skipped. |
| `gain` | `number` | `0.5` | Amplitude multiplier between octaves, 0 to 1. |
| `lacunarity` | `number` | `2.0` | Frequency multiplier between octaves; greater than 1 and at most 8. |
| `warp` | `number?` | `0` | Domain warp of the detail, 0 to 4. |

### `TerrainShapeOptions`

All fields optional, `0` to `1` strength dials (see
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
| `talusAngleDegrees` | `number?` | the landform's (30 to 47) | Slope above which thermal erosion moves material, 1 to 89. |
| `quality` | `"preview" \| "balanced" \| "high" \| "offline"?` | `"preview"` | Default iterations 60/30, 120/60, 200/100 or 400/200 (hydraulic/thermal); caps 120, 240, 400 or 5000. |

Erosion runs as GPU compute passes on browser builds (CPU fallback on any
GPU error); native/test builds always use the CPU path — see
[`docs/terrain-data.md`](terrain-data.md#erosion).

## `DemLoadOptions`

Passed to `engine.loadDemFromArrayBuffer(buffer, options)` /
`engine.loadDemFromUrl(url, options)`.

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `verticalScale` | `number?` | `1` | Height multiplier applied after decode. Must be finite. |
| `generateNormals` | `boolean?` | `true` | |
| `generateMaterialMasks` | `boolean?` | `true` | |

See [`docs/terrain-data.md`](terrain-data.md#dem-import-geotiff) for the supported
GeoTIFF subset and what decode warnings mean.

## `DemFetchOptions`

Passed to `engine.loadDemFromUrl(url, options)` and
`fetchDemBytes(url, options)`. It takes every `DemLoadOptions` field, and:

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `headers` | `HeadersInit?` | none | Passed to `fetch()`. |
| `signal` | `AbortSignal?` | none | Cancels the fetch. |
| `maxBytes` | `number?` | `MAX_DEM_BYTES` (80 MiB) | 1 to `MAX_DEM_BYTES`. A response whose `Content-Length` is larger is refused unread, and one that grows past it is cancelled (`DEM_FETCH_FAILED`). |

## `RawHeightmapOptions`

Passed to `engine.loadRawHeightmap(buffer, options)`.

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `width` | `number` | — (required) | Samples, `2` to `2048` (the largest terrain; see `FractalTerrainOptions.size`). |
| `height` | `number` | — (required) | Samples, `2` to `2048`. |
| `sampleFormat` | `"uint16" \| "int16" \| "float32"` | — (required) | |
| `byteOrder` | `"little-endian" \| "big-endian"?` | `"little-endian"` | |
| `metresPerSample` | `number` | — (required) | `> 0` and at most `10000`. |
| `heightScaleMetres` | `number` | — (required) | Must be finite. |
| `noDataValue` | `number?` | none | Samples equal to this value are marked no-data. Samples that are not finite, or lie beyond ±100,000 m once scaled, are marked no-data too, with a warning. |
| `seaLevelMetres` | `number?` | `0` | Within `±100,000`. |
| `landform` | `LandformKind?` | `"continental"` | The landform the heights were generated as, which sets the bedrock's beds. `loadBundle()` passes the exported one. |

## Export options

### `ExportHeightmapOptions`

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `format` | `"float32-le"?` | `"float32-le"` | Currently the only supported export format. |

### `SnapshotOptions`

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `mimeType` | `string?` | `"image/png"` | An image type such as `"image/webp"`, passed to `canvas.toBlob()`. Anything not of the form `image/…` throws `OPTIONS_INVALID`. |
| `quality` | `number?` | browser default | `0` to `1`. Only meaningful for lossy formats. |

See [`docs/export-and-snapshots.md`](export-and-snapshots.md) for the full
set of export helpers, including the ones with their own option types
(`HeightmapImageOptions`, `TerrainObjExportOptions`).

## Map, tree and bundle export

See [`docs/export-and-snapshots.md`](export-and-snapshots.md#map-export)
for every map kind and the bundle layout. Invalid values throw
`OPTIONS_INVALID` with the valid range, or `TypeError` for the wrong type.
`encodePng()` and `encodeRaw()` take a map only when its `width` and
`height` are whole numbers from 1 to 2048 (the largest map `exportMap()`
gives), it has 1 to 12 `channels`, and
its `data` is a `Float32Array`, `Uint8Array` or `Uint16Array` of exactly
`width × height × channels` values (`TypeError` otherwise). A biome or
water map's legend may hold at most 256 entries, each with an index from
0 to 255 and three finite colour components. `treesToCsv()` and
`treesToJson()` take only trees whose fields are numbers and whose
species is a `TreeSpecies`, so no CSV cell can hold a comma, a line
break or a formula.

### `ExportMapOptions` (for `engine.exportMap(kind, options)`)

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `size` | `[number, number]?` | the terrain's own size | Whole numbers from 2 to 2048, the largest terrain (set by WebAssembly's memory; see [`security.md`](security.md#terrain-size)). |

`kind` is a `MapKind`: `"height"`, `"biome"`, `"water"`, `"waterDepth"`,
`"flow"`, `"discharge"`, `"materials"`, `"slope"`, `"normals"`,
`"occlusion"`, `"temperature"`, `"moisture"`, `"treeDensity"`,
`"grassDensity"` or `"sourceHeight"` (the heights before any carving).
The result is an `ExportedMap`: `{ kind, width, height,
channels, type, data, encoding }`, where `encoding` is a `MapEncoding`
(`units`, `scale`, `range`, `legend`, `metresPerPixel`, `seaLevelMetres`,
`generator`).

### `ExportTreesOptions` (for `engine.exportTrees(options)`)

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `region` | `{ minX, minZ, maxX, maxZ }?` | the whole map | Finite metres, each minimum at most its maximum. |
| `maxCount` | `number?` | `2000000` | Whole number from 1 to 10,000,000. More trees than this throws `OPTIONS_INVALID`; pass a `region`. |

Returns `TreeRecord[]`: `{ x, y, z, species, variant, scale, rotation,
tint, dryness, handPlaced? }`.

### `PngOptions` (for `encodePng(map, options)`)

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `bitDepth` | `8 \| 16` | `8` | 16-bit only for single-channel maps. |
| `range` | `[number, number]?` | the map's lowest and highest value | Finite, low at most high. Byte maps at 8 bits are written as they are unless a range is given. |
| `smaller` | `boolean?` | `false` | Filter rows against the row above (PNG filter 2): smaller, slower. |

### `RawOptions` (for `encodeRaw(map, options)`)

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `type` | `"float32" \| "uint16"` | `"float32"` | Little-endian. |
| `range` | `[number, number]?` | the map's lowest and highest value | For `"uint16"`: the values mapped to 0 and 65535. |

### `BundleOptions` (for `exportBundle(engine, options)`)

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `size` | `[number, number]?` | the terrain's own size | Every map but `height.f32`, which stays exact. |
| `trees` | `boolean?` | included when under `maxTrees` | `true` throws when there are more; `false` leaves them out. |
| `previews` | `boolean?` | `false` | Adds `preview-height.png` and `preview-biome.png`. |
| `maxTrees` | `number?` | `2000000` | Whole number from 1 to 10,000,000. |

### `WaterMask` (for `engine.setWaterMask()`)

| Field | Type | Notes |
| --- | --- | --- |
| `width`, `height` | `number` | `2` to `2048`, the largest terrain. Resampled to the terrain's size, with a warning, when they differ. |
| `data` | `Uint8Array` | `width × height` bytes, row-major, north row first: `0` no water, `1` to `127` a river brush (1 m to 60 m wide), `128` to `255` a lake. |

A wrong type throws a `TypeError`; a wrong size or length a
`VistaWasmError` with the code `OPTIONS_INVALID`. `null` removes the mask.
See [`docs/water.md`](water.md#painted-water-setwatermask).

## Map import

See [`docs/import.md`](import.md) for the colour conventions, border
dithering and the bundle format. A wrong type throws a `TypeError`; a
value out of range a `VistaWasmError` with the code `OPTIONS_INVALID`
and the valid range; a malformed image or bundle one with the code
`INVALID_DEM`. Calls while terrain is generating throw
`INTERNAL_ERROR`.

### `HeightmapImageImportOptions` (for `engine.loadHeightmapImage(source, options)`)

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `metresPerSample` | `number` | — (required) | Finite, over 0. |
| `minHeightMetres` | `number?` | the PNG's `vistawasm:range` | Height of black. Finite. Required when the image has no range. |
| `maxHeightMetres` | `number?` | the PNG's `vistawasm:range` | Height of white. Finite, above `minHeightMetres`. |
| `seaLevelMetres` | `number?` | `0` | Finite. |
| `channel` | `"luminance" \| "r" \| "g" \| "b" \| "a"` | `"luminance"` | The channel read as height. |

The image is 2 to 2048 pixels a side, as it becomes the terrain
(WebAssembly's limit on terrain size); its header is checked before it is
decoded.

### `BiomeMap` (for `engine.setBiomeMap()`)

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `width`, `height` | `number` | — (required) | `2` to `2048`, the largest terrain. Resampled (nearest) to the terrain's size, with a warning, when they differ. |
| `data` | `Uint8Array` | — (required) | `width × height` bytes, row-major, north row first: a `BiomeKind` index (0 to 18) or `255` for not painted. |
| `borderSamples` | `number?` | `3` | Whole number from 0 to 8: how far borders are warped and blended. |

`null` clears the map.

### `DensityMask` (for `engine.setVegetationMasks({ trees, grass })`)

| Field | Type | Notes |
| --- | --- | --- |
| `width`, `height` | `number` | `2` to `2048`, the largest terrain. Resampled (bilinear) to the terrain's size, with a warning, when they differ. |
| `data` | `Uint8Array` | `width × height` bytes: `0` none, `128` unchanged, `255` twice as dense (never denser than density 4), linear between. |

An omitted key keeps its mask; `null` clears it.

### `BiomeMapFromImageOptions` (for `biomeMapFromImage(image, options)`)

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `legend` | `{ colour: [r, g, b], biome }[]?` | the exported biome colours | 1 to 256 entries (`MAX_LEGEND_COLOURS`). Each component a whole number from 0 to 255; `biome` a `BiomeKind`. |
| `borderSamples` | `number?` | `3` | Whole number from 0 to 8, copied to the map. |
| `onWarning` | `(message) => void?` | `console.warn` | Receives the unmatched-colour and 8-bit warnings. |

Resolves to `{ map, unmatchedFraction }`.

### `WaterMaskFromImageOptions` (for `waterMaskFromImage(image, options)`)

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `mode` | `"legend" \| "grey"` | `"legend"` | `"legend"` reads the exported water colours; `"grey"` reads luminance as `WaterMask` values. |
| `onWarning` | `(message) => void?` | `console.warn` | |

### `DensityMaskFromImageOptions` (for `densityMaskFromImage(image, options)`)

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `channel` | `"luminance" \| "r" \| "g" \| "b" \| "a"` | `"luminance"` | |
| `onWarning` | `(message) => void?` | `console.warn` | |

### `BundleLoadOptions` (for `loadBundle(engine, source, options)`)

| Field | Type | Default | Notes |
| --- | --- | --- | --- |
| `applySettings` | `boolean?` | `true` | Apply the bundle's options through the engine's setters. |

`loadTerrainFromImages(engine, images, options)` takes
`HeightmapImageImportOptions`, and `images` is `{ height, biome?,
water?, trees?, grass? }`.

## Read-only shapes returned by the engine

These are never passed *in* — the engine returns them.

### `WaterSounds` (from `engine.getWaterSounds(x, y, z)`)

| Field | Type | Notes |
| --- | --- | --- |
| `river` | `WaterSound \| null` | Running water within 400 m. |
| `waterfall` | `WaterSound \| null` | A waterfall within 1500 m. |
| `lakeShore` | `WaterSound \| null` | A lake shore within 400 m. |
| `surf` | `WaterSound \| null` | Waves on the coast within 600 m. |

Each `WaterSound` is `{ distanceMetres, loudness, position }`: the
distance in metres, a loudness from 0 to 1 (the source's strength over its
distance squared) and the source's world position `[x, y, z]`.

### `Waterfall` (from `engine.getWaterfalls()`)

| Field | Type | Notes |
| --- | --- | --- |
| `position` | `[number, number, number]` | Where the water lands, at the river level at the foot of the fall. |
| `heightMetres` | `number` | Height of the drop. |
| `widthMetres` | `number` | Width of the falling water. |
| `dischargeCubicMetresPerSecond` | `number` | Mean discharge. |

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
| `floraInstances` | `number` | Trees expected to be drawn this frame (every placed tree while none are streamed). |
| `grassInstances` | `number` | Tufts and reeds expected to be drawn this frame. |
| `clipmapLevels` | `number` | Number of exponential LOD bands the terrain mesh's half-span currently spans. |
| `activeGpuMemoryBytes` | `number \| null` | Not currently populated. |
| `weather` | `WeatherKind \| null` | The dominant preset at the camera, or `null` when the weather system is off. |
| `renderScale` | `number` | Fraction of the canvas resolution the scene was rendered at. |
| `treeTriangles` | `number` | Tree triangles drawn (meshes, impostors and shadow casters), read back from the GPU a frame or two late. |
| `treeGrowthMs` | `[number, number]` | Milliseconds spent growing trees in WASM: before the first frame, and after it. |
| `treeBakeMs` | `number` | GPU milliseconds the tree impostor bakes took, in all, when the browser supports timestamp queries. |
| `gpuPassTimesMs` | `GpuPassTimes \| null` | GPU milliseconds per pass (`terrain`, `trees`, `grass`, `boulders`, `clouds`, `skyAndFog`, `water`, `shadows`, `treeCulling`, `present`, `generation` for streamed vegetation and boulder tiles, and `surfaceWeather` for the wet ground, which runs four times a second), when the browser supports timestamp queries. A few frames behind. With `splitTreeTiming`, `treeMeshes`, `understorey` and `treeImpostors` break `trees` down. |

See [`docs/render-quality-and-diagnostics.md`](render-quality-and-diagnostics.md)
for how to use these for a performance HUD.
