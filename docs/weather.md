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
| `clouds` | Cloud coverage, density, and thickness; sky greying. Turns clouds on (volumetric) if they are off. |
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

## Performance

The weather system runs on the CPU once per frame and costs microseconds.
Rain and snow are drawn in the existing composite pass (four layers,
skipped entirely when nothing is falling), so clear weather costs nothing
extra.
