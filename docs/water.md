# Water

VistaWASM draws three kinds of water, all shaded by `shaders/water.wgsl`:

- **Ocean** — a camera-following grid at `seaLevelMetres` that reaches the
  horizon, displaced by a simulated Gerstner swell.
- **Rivers** — ribbons traced along the terrain's own drainage network and
  carved into the terrain, with an animated current that runs downstream.
- **Lakes** — flat water filling closed basins in the terrain.

For the exact field list, see
[`docs/options-reference.md`](options-reference.md#wateroptions).

## Sea level

`WaterOptions.seaLevelMetres` positions the ocean. Set it relative to the
generated terrain's real height range from `TerrainMetadata`, not a
hardcoded constant:

```ts
const handle = await engine.generateFractal(options);

engine.setWater({
  enabled: true,
  seaLevelMetres: handle.metadata.minHeightMetres + 5,
  waveScale: 0.8,
  reflectivity: 0.35,
  shorelineSoftnessMetres: 6
});
```

[`FractalTerrainOptions.seaLevelMetres`](options-reference.md#fractalterrainoptions)
is a separate field that seeds `TerrainMetadata.seaLevelMetres`, which
biomes, rivers, and vegetation use. It is normal to set both to the same
value.

## Waves (`waves`)

The swell is a sum of eight Gerstner waves spread around
`directionDegrees`. Secondary waves are shorter and keep roughly the same
steepness as the dominant swell, which gives a natural, non-repeating sea.

- `amplitudeMetres` and `wavelengthMetres` set the dominant swell.
- `steepness` sharpens crests; high values produce choppy water and
  whitecaps where crests fold.
- `directionalSpread` goes from a clean, parallel swell (`0`) to a
  confused, storm-like sea (`1`).
- `speed` scales animation; `1` uses deep-water dispersion, so long waves
  travel faster than short ones.
- `enabled: false` keeps the surface flat and leaves only ripples.

Waves shoal as the water gets shallow: they shrink towards the shore and
break into rolling bands of surf foam. Each wave also fades out wherever
the grid is too coarse to represent it, so distant water never aliases.
The whole simulation is analytic and runs in the vertex and fragment
shaders; it costs nothing on the CPU.

```ts
engine.setWater({
  enabled: true,
  seaLevelMetres: 0,
  waveScale: 1.2,
  reflectivity: 0.4,
  shorelineSoftnessMetres: 6,
  waves: { amplitudeMetres: 3, wavelengthMetres: 90, steepness: 0.8, directionalSpread: 0.8 },
  currentSpeed: 1.2,
  foam: 1
});
```

## Currents

`currentDirectionDegrees` and `currentSpeed` move small ripples and foam
across open water and lakes. Rivers carry their own current (below).

## Rivers and lakes (`rivers`)

When a terrain is installed (and whenever `rivers` changes), the engine:

1. Fills closed depressions with a priority-flood. Filled basins with real
    depth become lakes; tiny pits stay dry.
2. Routes flow downhill and accumulates upstream catchment area.
3. Traces every channel whose catchment exceeds `minCatchmentKm2` into a
    smoothed polyline running to the sea or into a larger river.
4. Carves the channel into the heightmap, so the river sits in a real bed
    with sloping banks, and marks the bed as sand and mud for the biome map.
5. Builds a ribbon whose width grows with catchment (`widthScale`) and
    whose vertices carry the flow direction and speed. Steep reaches run
    fast and turn white with rapids; flat lowland reaches drift slowly.

The water surface never runs uphill. Carving is fully reversible: turning
rivers off restores the original heights exactly, and `exportHeightmap()`
returns the carved terrain while rivers are on.

Rivers are extracted on a grid of at most 512 × 512 samples, so the cost is
bounded (tens of milliseconds) regardless of terrain size.

## Shading

- **Depth colour.** The shader reads the real water depth from the
  terrain heights. Shallow water shows the sea bed through
  `shallowColour`; it fades to `deepColour` by `clarityMetres`.
- **Reflections.** Schlick Fresnel reflects the same analytic sky as the
  sky pass, including cloud reflections, plus a GGX sun glitter.
  `reflectivity` scales reflection strength.
- **Subsurface light** glows through thin wave crests facing the sun.
- **Foam** appears on folding crests, along shorelines, and in rapids,
  scaled by `foam`.
- **Cloud shadows** and fog apply to water like everything else.

## Interaction with mist

When `MistOptions.riseAboveWater` is enabled, extra mist appears near
`seaLevelMetres`. This only takes effect while `WaterOptions.enabled` is
`true`.

## Interaction with weather

While the weather system drives water (`WeatherOptions.effects.water`,
on by default), wind raises and steepens the swell, turns it and the
current downwind, and adds foam in strong wind. Rain also rings the
surface with raindrop ripples. Switch the effect off to keep your own wave
settings. See [`docs/weather.md`](weather.md).

## What water does not do

- No buoyancy or gameplay interaction. Compare your own height query (see
  [`docs/game-development.md`](game-development.md#querying-terrain-height-for-gameplay))
  against `seaLevelMetres` yourself.
- No reflections of scene geometry; reflections show the sky and clouds.
- Rivers do not change sea level or flood terrain.
