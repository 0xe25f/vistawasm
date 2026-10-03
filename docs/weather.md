# Weather

The weather system drives clouds, mist, haze, light, wind, waves, rain,
snow, wet ground, puddles, settled snow and lightning from one place. It
is off by default, so every system keeps its own manual settings until you
turn it on.

```ts
engine.setWeather({ enabled: true, state: "rain" });
```

Every kind of weather is a **preset**: a set of numbers such as cloud
coverage, humidity, wind and rain. At any moment the engine blends the
current preset into the next one, field by field, and every system reads
that one blend, so a change of weather stays consistent from the sky to
the puddles.

## Built-in presets

| Preset | Look |
| --- | --- |
| `"clear"` | Blue sky, a few small clouds, light breeze. |
| `"fewClouds"` | A few fair-weather cumulus. |
| `"partlyCloudy"` | Scattered cumulus. The default. |
| `"brokenClouds"` | Mostly cloudy, with breaks of blue. |
| `"overcast"` | Full grey cover, soft dim light, hazier distance. |
| `"mist"` | Damp, milky air and a thin ground mist. |
| `"fog"` | Low cloud and thick ground fog, very little wind. |
| `"lightRain"` | Light, steady rain from a grey deck. |
| `"rain"` | Dark cloud, steady rain, wet ground and puddles, choppier water. |
| `"heavyRain"` | A downpour under a very dark deck. |
| `"storm"` | Storm cells with heavy rain, strong gusting wind, rough water, and lightning. |
| `"snow"` | Falling snow that settles on the ground and on trees. |
| `"blizzard"` | Heavy snow in a gale. |

The seven presets that existed before (`clear`, `partlyCloudy`,
`overcast`, `fog`, `rain`, `storm` and `snow`) keep their old values;
they differ only through the effects presets added (humidity haze,
regional weather, wet ground that dries, and so on).

`engine.getWeatherPresets()` returns every resolved preset, built in and
custom, with every field set:

```ts
const presets = engine.getWeatherPresets();
console.log(Object.keys(presets)); // ["clear", "fewClouds", ...]
console.log(presets.storm.windMetresPerSecond); // 17
```

## Preset fields

| Field | Range | Meaning |
| --- | --- | --- |
| `cloudCoverage` | 0 to 1 | Mean cloud coverage. |
| `cloudDensity` | 0 to 1 | Cloud density. |
| `cloudThickness` | 0.2 to 3 | Multiplier on the cloud layer's thickness. |
| `stratiform` | 0 to 1 | Cloud type, 0 cumulus to 1 flat sheet. |
| `towering` | 0 to 1 | Towering storm clouds. |
| `baseDarkness` | 0 to 1 | Darkening of cloud bases. |
| `raggedBase` | 0 to 1 | Ragged cloud bases and scud. |
| `rainShafts` | 0 to 1 | Curtains of rain or snow below the clouds. |
| `cirrus` | 0 to 1 | High cirrus. |
| `humidity` | 0 to 1 | Relative humidity: damp air whitens and thickens the haze. |
| `turbidity` | 2 to 10 | Aerosols, from clean alpine air (2) to thick tropical haze (10). |
| `hazeDistanceScale` | 0.05 to 2 | Multiplier on the haze distance. |
| `mistDensity` | 0 to 1 | Ground mist. |
| `temperatureOffsetCelsius` | -30 to 30 | Added to the climate's temperature. |
| `windMetresPerSecond` | 0 to 60 | Mean wind. |
| `gustiness` | 0 to 1 | Gust strength relative to the mean wind. |
| `rain` | 0 to 2 | Rain; above 1 a downpour. |
| `snow` | 0 to 2 | Snowfall. |
| `lightningPerMinute` | 0 to 60 | Lightning flashes. |
| `coverageSpread` | 0 to 1 | How far coverage varies across the regional map. |
| `precipitationSpread` | 0 to 1 | How far rain and snow vary under thick cloud. |
| `cellSizeKm` | 1 to 100 | Size of weather cells. |
| `cellularity` | 0 to 1 | 0 smooth fields, 1 discrete storm cells. |
| `baseVariation` | 0 to 0.2 | How far low-cloud bases vary in height, as a share of the layer's thickness. Default 0.07. |
| `baseLumpiness` | 0 to 1 | Cotton-wool lumps under low clouds. Default 0.6. |
| `altocumulus` | 0 to 1 | Rippled mid-level cloudlets, a "mackerel sky". |
| `altostratus` | 0 to 1 | A grey mid-level veil that turns the sun into a watery disc. |
| `altoHeightMetres` | 2000 to 7000 | Height of the mid-level layer. Default 4200. |
| `altoSpeed` | 0 to 4 | Mid-level drift, in units of 15 m/s. Default 1. |
| `next` | weights | The presets that may follow this one when cycling. |
| `minDurationSeconds`, `maxDurationSeconds` | 1 to 86400 | How long the preset lasts when cycling. Unset, `stateDurationSeconds` ±40 %. |
| `climate` | -60 to 60 °C | `minCelsius` and `maxCelsius`: the temperatures at the camera the preset suits. |

### Mid-level clouds in the presets

Every preset keeps the default bases (`baseVariation` 0.07,
`baseLumpiness` 0.6) and mid-level layer (4200 m, speed 1). The amounts
differ:

| Preset | `altocumulus` | `altostratus` |
| --- | --- | --- |
| `"clear"` | 0 | 0 |
| `"fewClouds"` | 0.15 | 0 |
| `"partlyCloudy"` | 0.2 | 0 |
| `"brokenClouds"` | 0.45 | 0.1 |
| `"overcast"` | 0.1 | 0.65 |
| `"mist"`, `"fog"` | 0 | 0.3 |
| `"lightRain"` | 0 | 0.8 |
| `"rain"`, `"heavyRain"` | 0 | 0.95 |
| `"storm"` | 0.1 | 0.5 |
| `"snow"` | 0 | 0.85 |
| `"blizzard"` | 0 | 0.9 |

They blend between presets like every other field. Where the weather
varies across the map, its coverage raises or lowers the mid-level layer
by up to 30 %, so the veil thins towards clearer air. To keep the old
look of a preset, set both amounts to 0:

```ts
engine.setWeather({
  enabled: true,
  state: "overcast",
  presets: { overcast: { altocumulus: 0, altostratus: 0 } }
});
```

See [Mid-level clouds](sky-atmosphere-and-weather.md#mid-level-clouds)
for how the layer looks and what it costs.

## Your own presets

`WeatherOptions.presets` adds presets and changes built-in ones:

- a new name adds a preset. It takes every field it does not set from the
    preset it `extends`, or from `"partlyCloudy"`;
- a built-in name changes only the fields given;
- `extends` chains are followed. A cycle (`a` extends `b`, which extends
    `a`) is rejected with an error that names it;
- every number is checked against its range, and an unknown field is
    rejected with a message listing the valid ones.

A tropical downpour, from `heavyRain`:

```ts
engine.setWeather({
  enabled: true,
  state: "tropicalDownpour",
  presets: {
    tropicalDownpour: {
      extends: "heavyRain",
      humidity: 1,
      turbidity: 7,
      temperatureOffsetCelsius: 8,
      windMetresPerSecond: 6,
      cellSizeKm: 4,
      cellularity: 0.9,
      coverageSpread: 0.35,
      next: { rain: 0.6, brokenClouds: 0.4 }
    },
    // Rain on this map is a little hazier than usual.
    rain: { turbidity: 4.5 }
  }
});
```

`state` accepts any preset name, built in or custom. In TypeScript,
`WeatherKind` accepts the built-in names and any other string.

## Changing weather over time

```ts
engine.setWeather({
  enabled: true,
  autoCycle: true,
  stateDurationSeconds: 300,
  transitionSeconds: 45,
  allowSnow: false,
  seedOffset: 42
});
```

Changing `state` blends to the new weather over `transitionSeconds`
(default `30`). Nothing jumps: cloud drift, mist drift, gusts and water
currents are integrated over time.

With `autoCycle`, each preset lasts between its `minDurationSeconds` and
`maxDurationSeconds` (or `stateDurationSeconds` ±40 %) and then moves on to
one of its `next` presets, chosen by weight: clear skies cloud over,
overcast turns to rain, storms ease back to rain. The sequence is
deterministic for a given `seedOffset`.

`engine.advanceWeather(seconds)` runs the weather forward at once, in
one-second steps, up to a day (86,400 seconds): the cycle, the time of
day, and the wet and snowy ground. It is useful for tools and tests:

```ts
engine.setWeather({ enabled: true, state: "rain" });
engine.advanceWeather(600); // ten minutes of rain
engine.setWeather({ enabled: true, state: "clear" });
engine.advanceWeather(1200); // twenty minutes of drying
```

## Weather across the map

The weather varies across the land, so a storm can build over the hills
while the coast is in sunshine. A regional map, `regionSizeKm` across
(default 64 km) and centred on the terrain, holds four fields:

- **coverage**: the preset's mean plus `coverageSpread` times warped
    noise;
- **precipitation**: the preset's rain and snow, only where coverage is
    above about 0.55 and fully above 0.8, varied by `precipitationSpread`;
- **storminess**: `cellularity` times storm cells about `cellSizeKm`
    across, which build cloud at their cores and clear the air between
    them, and grow the towering clouds;
- **humidity**: the preset's humidity ±0.1.

The map drifts with the winds aloft (about twice the surface wind), so
the weather moves across the land. Clouds and rain curtains read the map
on the GPU; the rain and snow around the camera, lens drops, lightning,
the light, and `getWeather()` use the camera's own place on it.
`regional: false` makes every field the same everywhere.

`engine.weatherAt(x, z)` reports the weather anywhere, for gameplay and
audio. It returns `null` with no terrain. Beyond the terrain the ground
values are 0.

```ts
const here = engine.weatherAt(camera.x, camera.z);

if (here && here.precipitation > 0.5) {
  rainSound.volume = Math.min(here.precipitation, 1);
}
```

| Field | Meaning |
| --- | --- |
| `coverage` | Cloud coverage overhead, 0 to 1. |
| `precipitation` | Rain and snow falling, together (above 1 in a downpour). |
| `storminess` | How much of a storm cell is overhead, 0 to 1. |
| `humidity` | Relative humidity, 0 to 1. |
| `wetness` | Wetness of the ground, 0 to 1. |
| `puddles` | Puddle water, 0 to 1. |
| `snowDepth` | Settled snow, 0 to 1, and at least the snow that lies all year. |

## Wet ground that dries

The ground keeps its own wetness, puddle water and snow depth everywhere
on the terrain, and they follow the rain where it actually fell:

- rain wets the ground over a couple of minutes. It dries faster in sun,
    wind and warmth, on slopes that shed the water, and more slowly under
    trees and in the evening, and it dries in patches;
- puddles fill once the ground is soaked, on flat ground (under 3°) in
    hollows, and evaporate at a third of the ground's rate, so they are
    last to go. They are mirror-flat, reflect the sky, and show rings while
    it rains;
- snow builds up below 0.5 °C and melts above 1 °C, faster the warmer and
    sunnier it is, and its melt water wets the ground. Snow that lies all
    year stays.

Wet ground darkens and turns glossy, and wet bark darkens. Twenty minutes
after a shower most open ground is drying; an hour later only the damp
shade under trees and the puddles are left.

## Weather in the cold

The weather follows the climate under the camera
(`engine.temperatureAt()`, plus the preset's `temperatureOffsetCelsius`;
see [`docs/biomes.md`](biomes.md#climate-temperature)):

- **Snow, not rain.** Rain falls as snow below 0.5 °C, and as sleet (rain
    and snow together) between 0.5 and 2.5 °C. The preset keeps its name,
    so `getWeather()` still reports `"rain"`, but its `rain` and `snow`
    values show what is falling, and the snow settles.
- **A colder cycle.** `snow` and `blizzard` suit only climates up to 1 °C
    (`climate.maxCelsius`). Below 0 °C, cycling turns the rain presets into
    snow (a blizzard in a gale) and makes clear spells half as likely
    again. Cold climates always allow snow, whatever `allowSnow` says.
- **Blowing snow.** When settled or permanent snow covers at least half
    the ground under the camera and the wind is above 8 m/s, low streaks of
    snow race along with the wind up to about 2 m above the ground.
- **Crisp air.** Below 0 °C the air is clearer: haze reaches 40 % further
    and the Mie haze around the sun is 30 % weaker. This applies whether
    the weather system is on or not.
- **Rivers.** Warm spells swell the snowmelt in rivers and cold snaps
    shrink it, through the preset's temperature offset.

```ts
engine.setBiomes({ meanTemperatureCelsius: -18 });
engine.setWeather({ enabled: true, state: "blizzard" });
```

## Time of day and golden hour

With [`setTimeOfDay`](sky-atmosphere-and-weather.md#time-of-day) and
`autoCycle` on, the weather tends to clear before sunset:

- between four and one and a half hours before sunset, rain or snow
    starts a clearing along a chain (storm, rain, broken cloud, a few
    clouds; in the cold, through snow), each step shortened so it is done
    by about an hour before sunset;
- in the last hour and a half before sunset, rain and snow are 70 % less
    likely to start;
- a low sun dries the ground slowly, so it is still wet and reflective at
    sunset.

Over 200 simulated days the number of clear evenings (under half cloud
cover, nothing falling, half an hour before sunset) roughly doubles.

## Choosing what the weather drives

Every effect is on by default. Switch one off and that system keeps its
manual settings.

```ts
engine.setWeather({
  enabled: true,
  state: "storm",
  effects: {
    water: false,     // keep my own waves and currents
    lightning: false  // no flashes
  }
});
```

| Effect | Drives |
| --- | --- |
| `clouds` | Cloud coverage, density, thickness, type and cirrus; the regional map; sky greying; direct and indirect light under cloud and shadow softness. Turns clouds on (volumetric) if they are off. |
| `mist` | Ground mist density (turns volumetric mist on when needed), haze distance, turbidity and humidity haze. |
| `wind` | Tree and grass sway and the gust front, cloud and mist drift direction and speed. |
| `water` | Sea state from the wind: wave height, direction, whitecaps, spray, foam, and current. |
| `precipitation` | Falling rain and snow. |
| `ground` | Wet ground and puddles, wet bark, settled snow. |
| `lightning` | Flashes during storms. |

`windScale` (0 to 4) and `precipitationScale` (0 to 2) scale every
preset. `windDirectionDegrees` sets the prevailing wind; gusts wander
around it.

## One wind

One resolved wind (speed, direction and gustiness) drives everything.
Gusts are a front that sweeps downwind across the land: the trees, the
grass, the rain's slant, the drift of the clouds and dark gust patches on
the water all share it, so you can watch a gust cross the landscape.

The sea state follows the wind (see [`docs/water.md`](water.md#sea-state)):
waves grow with the wind and with the open water upwind, whitecaps break
from 4 m/s, and spray streaks the sea from 15 m/s.

## Reading the weather

```ts
const state = engine.getWeather(); // undefined when the weather is off
console.log(state?.rain, state?.cloudCoverage, state?.wetness);

engine.on("weatherChanged", (kind) => {
  label.textContent = kind ?? "off";
});
```

`getWeather()` reports the blend at the camera: the local coverage, rain,
snow, humidity and storminess from the regional map, the ground's
wetness, puddles and snow under the camera, and the share of sunlight the
clouds let through (`sunTransmittance`). `RenderStats.weather` holds the
dominant preset each frame, and the `"weatherChanged"` event fires when
it changes, halfway through a transition.

## Heavy rain, snow, and the view

Rain falls in short streaks, about 20 cm long (one drop blurred over a
frame), slanted by the wind. Snow falls as round flakes 2 to 6 cm across
that sway and drift with the wind. A storm is heavier than ordinary rain:
more drops, longer streaks, and a grey veil that cuts visibility to about
3 km. `precipitationScale` above 1 turns any rain into a downpour the
same way.

Under rain the sky is a full overcast deck: brightest overhead and darker
towards the horizon, with no direct sun, so the distant sea darkens under
the rain rather than glowing, and there are no sharp shadows or sun glints.

## Raindrops on the lens

```ts
engine.setWeather({ enabled: true, state: "rain", lensDrops: true });
```

With `lensDrops`, raindrops land on the camera lens while it rains at the
camera. Each drop refracts the scene behind it, flipped and magnified,
with a darker rim and a glint. It is off by default, since it suits a
filmed or "camera" look rather than a first-person eye. It adds one
full-screen pass, and only while drops are on the lens. Snow and sleet
add no drops.

Three options shape the drops:

| Option | Default | Meaning |
| --- | --- | --- |
| `lensDropCount` | `60` | Drops on the lens at once in full rain, 0 to 512. Light rain has fewer. |
| `lensDropMinSize` | `0.008` | Smallest drop diameter, as a fraction of the canvas height. |
| `lensDropMaxSize` | `0.05` | Largest drop diameter, as a fraction of the canvas height. |

Sizes are fractions of the canvas height, so drops look the same at
1080p, at 4K and on a phone, whatever the device pixel ratio. Most drops
are small, as real drop populations are. Behaviour follows size, as it
does on glass:

- drops in the smaller 60 % of the size range are beads: they stay put
    and evaporate, shrinking away over 4 to 9 seconds;
- larger drops run down the screen, faster the larger they are, with a
    slight sideways wander. A running drop swallows the beads it touches
    and grows (never past `lensDropMaxSize`), and leaves a trail of tiny
    beads behind it.

When the rain stops, no new drops land, and the ones on the lens run off
or evaporate within about ten seconds.

```ts
// A downpour on a wide lens: many drops, some large enough to run.
engine.setWeather({
  enabled: true,
  state: "storm",
  lensDrops: true,
  lensDropCount: 250,
  lensDropMinSize: 0.01,
  lensDropMaxSize: 0.08
});
```

The drops are simulated on the CPU (`lens_drops.rs`), at most 512 of
them, and sorted into a grid of screen tiles each frame; each pixel tests
only the drops of its own tile, and every drop is listed in every tile
it reaches, so drops are always drawn whole. The simulation and binning
take well under 0.1 ms a frame.

## Performance

The weather runs on the CPU once per frame: the blend, the camera's
place on the regional map and the light through the clouds cost a few
microseconds, and refreshing 16 of the regional map's 128 rows costs
about 0.3 ms in a storm and 0.2 ms in fair weather (measured in headless
Chromium on a 4-core cloud machine; a desktop is faster). Every other
texel of a row is evaluated and the ones between are averaged, since
the map's features span kilometres. The wet ground steps four times a second
in a small compute pass (well under 0.1 ms amortised on a mid-range GPU)
and a 64 x 64 copy on the CPU for `weatherAt`.

Rain and snow are not simulated across the world: they are drawn in thin
layers around the camera, from 1.5 m to 48 m away, in the passes that
already run (six layers per pixel, skipped entirely when nothing is
falling), so their cost does not depend on how much of the world is
raining, and clear weather costs nothing extra. Rain curtains seen in the
distance add a 20-sample march below the cloud base, from 1.2 km outwards,
in the reduced-resolution cloud pass, only while `rainShafts` is above
zero.
The mid-level layer adds a 2D layer to the same pass, as cirrus does,
with at most five 2D noise lookups per cloud pixel (two for altostratus
alone), plus one per lit cumulus sample and one per shaded ground pixel
for its shadows, and one of the regional map with each where the weather
varies. At amounts of 0
it costs nothing.
Storm towers make the cloud slab up to 2.6 times as tall, so storms cost
more than fair weather; lower `raymarchSteps` or `resolutionScale` if a
storm is too slow on a weak GPU.
Blowing snow is drawn in the same layers as falling snow and only while
it is blowing.
