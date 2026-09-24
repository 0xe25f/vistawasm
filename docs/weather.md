# Weather

The weather system drives clouds, mist, haze, wind, waves, rain, snow,
wet ground, settled snow, and lightning from one place. It is off by
default, so every system keeps its own manual settings until you turn it
on.

```ts
engine.setWeather({ enabled: true, state: "rain" });
```

## States

| State | Look |
| --- | --- |
| `"clear"` | Blue sky, a few small clouds, light breeze. |
| `"partlyCloudy"` | Scattered cumulus. The default. |
| `"overcast"` | Full grey cover, flat light, hazier distance. |
| `"fog"` | Low cloud and thick ground fog, very little wind. |
| `"rain"` | Dark cloud, steady rain, wet ground and puddles, choppier water. |
| `"storm"` | Heavy rain, strong gusting wind, rough water, and lightning. |
| `"snow"` | Falling snow that settles on the ground and on trees. |

Changing `state` blends to the new weather over `transitionSeconds`
(default `30`). Nothing jumps: cloud drift, mist drift, and water currents
are integrated over time, so a change of wind speed changes how fast they
move, not where they are.

The ground responds more slowly than the sky. Puddles take about a minute
and a half to form and four minutes to dry. Snow settles over a minute or
two and melts over about five.

## Cloud types

Each state has its own kind of cloud, and transitions blend between them
as smoothly as everything else.

| State | Clouds |
| --- | --- |
| `"clear"`, `"partlyCloudy"` | Fair-weather cumulus: white heaps with flat bases. |
| `"overcast"` | A smooth grey sheet (stratus). |
| `"fog"` | Low, flat cloud merging with the ground fog. |
| `"rain"` | A low, heavy, dark sheet (nimbostratus) with a ragged underside, loose scraps of cloud beneath, and grey curtains of rain hanging below it. |
| `"storm"` | Towering storm clouds (cumulonimbus) with anvil tops and very dark bases, gaps of sky between cells, rain shafts, and lightning that lights the clouds from inside. |
| `"snow"` | A grey sheet with softer, paler snow curtains. |

The weather does this through five `CloudsOptions` fields, which you can
also set yourself with the weather off: `stratiform`, `towering`,
`baseDarkness`, `raggedBase`, and `rainShafts`. For example, a distant
summer thunderstorm on an otherwise fair day:

```ts
engine.setClouds({ ...clouds, coverage: 0.5, towering: 0.8, baseDarkness: 0.6, rainShafts: 0.7 });
```

Switch off `effects.clouds` to keep your own cloud settings while the
weather drives everything else.

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

With `autoCycle`, each state lasts roughly `stateDurationSeconds` (±40 %)
and then moves on to a plausible neighbour: clear skies cloud over,
overcast turns to rain, storms ease back to rain. The sequence is
deterministic for a given `seedOffset`.

## Weather in the cold

The weather follows the climate under the camera
(`engine.temperatureAt()`; see [`docs/biomes.md`](biomes.md#climate-temperature)):

- **Snow, not rain.** Rain falls as snow below 0.5 °C, and as sleet (rain
    and snow together) between 0.5 and 2.5 °C. The state keeps its name,
    so `getWeather()` still reports `"rain"`, but its `rain` and `snow`
    values show what is falling, and the snow settles.
- **A colder cycle.** Below 0 °C, cycling turns rain and storms into
    snow and makes clear spells half as likely again. Cold climates always
    allow snow, whatever `allowSnow` says.
- **Blowing snow.** When settled or permanent snow covers at least half
    the ground under the camera and the wind is above 8 m/s, low streaks of
    snow race along with the wind up to about 2 m above the ground.
- **Crisp air.** Below 0 °C the air is clearer: haze reaches 40 % further
    and the Mie haze around the sun is 30 % weaker. This applies whether
    the weather system is on or not.

```ts
engine.setBiomes({ meanTemperatureCelsius: -18 });
// Snow falls, and a gale lifts it off the ice.
engine.setWeather({ enabled: true, state: "storm" });
```

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
| `clouds` | Cloud coverage, density, thickness, and type (see [Cloud types](#cloud-types)); sky greying. Turns clouds on (volumetric) if they are off. |
| `mist` | Ground mist density (turns volumetric mist on when needed) and haze distance. |
| `wind` | Tree and grass sway, cloud and mist drift direction and speed. |
| `water` | Wave height, steepness, direction, foam, and current. |
| `precipitation` | Falling rain and snow. |
| `ground` | Wet, darker ground with puddles; settled snow. |
| `lightning` | Flashes during storms. |

`windScale` (0 to 4) and `precipitationScale` (0 to 2) scale every state.
`windDirectionDegrees` sets the prevailing wind; gusts wander around it.

## Reading the weather

```ts
const state = engine.getWeather(); // undefined when the weather is off
console.log(state?.rain, state?.windSpeedMetresPerSecond);

engine.on("weatherChanged", (kind) => {
  label.textContent = kind ?? "off";
});
```

`RenderStats.weather` holds the dominant state each frame, and the
`"weatherChanged"` event fires when it changes. It changes halfway
through a transition.

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

With `lensDrops`, raindrops land on the camera lens while it rains. Each
drop refracts the scene behind it, flipped and magnified, with a darker
rim and a glint. It is off by default, since it suits a filmed or
"camera" look rather than a first-person eye. It adds one full-screen
pass, and only while drops are on the lens.

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

The weather system runs on the CPU once per frame and costs microseconds.
Rain and snow are not simulated across the world: they are drawn in thin
layers around the camera, from 1.5 m to 48 m away, in the passes that
already run (six layers per pixel, skipped entirely when nothing is
falling), so their cost does not depend on how much of the world is
raining, and clear weather costs nothing extra. Rain curtains seen in the
distance add a 20-sample march below the cloud base, from 1.2 km outwards,
in the reduced-resolution cloud pass, only while `rainShafts` is above
zero.
Storm towers make the cloud slab up to 2.6 times as tall, so storms cost
more than fair weather; lower `raymarchSteps` or `resolutionScale` if a
storm is too slow on a weak GPU.
Blowing snow is drawn in the same layers as falling snow and only while
it is blowing.
