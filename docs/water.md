# Water

VistaWASM draws four kinds of water, all shaded by `shaders/water.wgsl`:

- **Ocean** — a camera-following grid at `seaLevelMetres` that reaches the
  horizon, displaced by a simulated Gerstner swell.
- **Rivers** — fed by rain, snowmelt and springs, running in channels
  shaped into the terrain, with a current that runs downstream.
- **Lakes** — filling basins to their spill height and overflowing into
  the rivers below them.
- **Waterfalls** — where a river drops over a step, with mist and a
  churned plunge pool.

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

Rivers come from the water draining the land. When a terrain is
installed, and whenever `rivers` or the water mask changes, the engine
routes water over the finished heights (after erosion, glaciers and any
painted water) on a grid at full terrain resolution up to 1024 samples
per side. The river build is reported as the `"rivers"` progress phase;
at 512 × 512 it takes about 100 to 250 ms in the browser.

### Sources

- **Rain.** Every sample adds runoff from its precipitation: 300 mm a
    year in the driest climates to 3000 mm in the wettest, from the
    climate's moisture. Runoff is accumulated downhill into a mean
    discharge in m³/s.
- **Snowmelt.** Snow fields and glaciers add `snowmelt` × snow × 0.6 m of
    water a year. Glacier snouts (where meltwater leaves the ice) and the
    lower edge of each snowy-peak field start streams of their own, even
    where too little water has gathered yet. Water runs on under glacier
    ice: nothing is drawn or carved on the ice itself.
- **Springs.** Where steep ground (over 20 degrees) meets gentle ground
    (under 8 degrees) in a hollow draining more than 0.05 km², a small
    spring (0.02 m³/s) starts a stream. Springs are placed by the terrain
    and at most one per 600 m. `springs: false` turns them off.
- A sample becomes a channel when its discharge reaches
    `minCatchmentKm2` × 0.03 m³/s per km², about the flow of that
    catchment in an average climate.

With biomes switched off, rain and snow follow the default climate.

### Lakes

Every basin is filled to its spill height. A basin with at least 24
samples, or deeper than 1.5 m, becomes a lake whose surface is its spill
height. Its outlet river starts at the lowest point of its rim, so rivers
never end in a dead end. Where evaporation (warmer and drier climates
evaporate more) takes all the inflow, the lake keeps its water with no
outlet: an endorheic lake, as in a desert basin. Every channel ends at the
sea, at a lake, or at the map edge.

### Channel form

The channels follow the valleys erosion carved; the river build only
shapes their beds and banks:

- **Width and depth** grow with discharge: width 2.7 √Q × `widthScale`
    and depth 0.35 Q⁰·⁴ metres, both from 0.6 to 400 m. A small island
    stream carries a few litres a second and is under a metre wide; a
    large lowland river is tens of metres wide.
- **The bed never rises downstream**, and is smoothed along the river
    except across waterfall steps.
- **Cross-section.** In steep valleys (over 6 %) the channel is a narrow
    V; on gentle ground (under 2 %) it is flat-bottomed, with a floodplain
    four widths wide on each side levelled towards the bank; in between
    it blends.
- **Meanders.** On lowland reaches flatter than 1.5 % and wider than 4 m,
    the river swings in Kinoshita curves, 11 widths long and up to 2.5
    widths wide times `meanders`. The sharpest tenth of the loops leave
    oxbow lakes beside the river.
- **Deltas.** A river wider than 8 m meeting the sea on ground flatter
    than 0.5 % splits into two or three distributaries fanning out at
    ±25 degrees over its last eight widths, with a low fan of silt, 0.5 m
    above the sea, deposited between them.
- **Waterfalls** form where the bed drops more than 3 m (or 1.5 widths)
    within two samples far more steeply than the reach around it, or runs
    steeper than 35 degrees. A long steep run becomes a staircase of steps
    of at most two samples each, with pools between them, as steep
    mountain streams are. Steps under 3 m become rapids. Each fall has a
    plunge pool 0.3 × its height + its width across and 0.15 × its
    height deep. `waterfalls: false` keeps the channel draped over the
    step instead.

Every change is recorded, so turning rivers off (or clearing the water
mask) restores the original heights exactly, and `exportHeightmap()`
returns the shaped terrain while rivers are on.

### Flowing water

- **Ribbons** are 1.3 channel widths wide, and fade out where the water
    is shallower than 25 cm, so banks meet the water with no edge.
- **Speed** comes from Manning's equation, `v = R^(2/3) S^(1/2) / n`
    with n = 0.035 and R the depth, from 0.2 to 6 m/s, times
    `currentSpeed`. It moves the ripples downstream at the water's speed.
- **Ripples** are glassy under 0.5 m/s and small and choppy above 2 m/s.
- **Rapids** on slopes of 2 to 8 % (and below small steps) carry standing
    waves fixed in place, and whitewater that grows with speed above
    1.5 m/s.
- **Bends** run faster, with more foam, on the outer bank.
- **Clarity.** Shallow water shows the bed; fast rivers carry silt that
    clouds and browns them.
- **Snowmelt fullness.** Where the camera is warmer, rivers run faster
    and foamier and rise up to a fifth of their depth in their channels;
    where it is cold they run slow and low.

### Waterfalls

Each waterfall has:

- **a sheet** following the path of water leaving the lip, `x = v t` and
    `y = -g t² / 2`, 12 rows down and at least three across, pushed out to
    lie just over the rock where the face is not vertical;
- **streaks** falling at the impact speed, `√(2 g drop)`, breaking up into
    aerated white water towards the foot, thin at the edges, and lit
    through from behind when the sun is beyond it;
- **mist**: 16 to 64 camera-facing sprites rising and drifting downwind
    from the foot, soft where they meet the ground, and not drawn beyond
    1.5 km;
- **a plunge pool** of churned foam in rings spreading from the foot.

`engine.getWaterfalls()` lists every waterfall, where its water lands:

```ts
const [fall] = engine.getWaterfalls();

if (fall) {
  const [x, y, z] = fall.position;
  const back = fall.heightMetres * 3;
  engine.setCamera({
    position: [x + back, y + fall.heightMetres * 0.5, z],
    target: [x, y + fall.heightMetres * 0.4, z],
    fieldOfViewDegrees: 55
  });
}
```

### Frozen water

Lakes, rivers and waterfalls follow the climate (see
[`docs/biomes.md`](biomes.md#climate-temperature)):

- **Lakes** freeze where the temperature at their outlet is below 0 °C,
    fully at -2 °C, shallow margins first. Lake ice is drawn as great
    smooth sheets meeting at pressure cracks, under snow that deepens as
    it gets colder and is blown thin in patches of clear, dark ice. No
    ripples, flow or foam show through it.
- **Rivers** freeze below -5 °C, fully at -7 °C: snow-dusted ice with open
    dark leads over the fastest water (over 2 m/s).
- **Waterfalls** below -8 °C become icefalls: still, blue-white ice ribbed
    down the fall line, with no mist, no churned pool and no sound.

A map with no water below 0 °C pays nothing for any of this.

```ts
engine.setBiomes({ meanTemperatureCelsius: -18 });
```

### Wet banks and reeds

Within 6 m of a river, lake or waterfall the ground darkens by up to
35 %, turns glossy, and gentle banks turn to mud. Within 12 m grass grows
denser and greener (when grass is on), and beside lakes, oxbows and
rivers slower than 0.6 m/s, in temperate and warm climates, reeds 1.4 to
2.2 m tall grow in the same wind.

### Painted water (`setWaterMask`)

`engine.setWaterMask(mask)` paints rivers and lakes into the terrain,
which carves and draws them like its own. Each byte of `mask.data` is one
sample, row-major, north row first:

- `0`: no water;
- `1` to `127`: a river brush, `1` painting a river 1 m wide and `127` one
    60 m wide (or wider, where its discharge asks for it);
- `128` to `255`: a lake or pond.

Painted lakes are flattened into basins below their rim, `max(1.5 m,
0.05 × √area)` deep, and fill to their lowest rim point, where their
outlet joins the drainage. Painted river strokes are thinned to
centrelines, run downhill from their higher end (towards the nearer sea
or lake when both ends are level), are cut with the same channel form,
and join the natural network; natural streams end where they reach
painted water. Painted water always wins over generated water.

A mask of another size is resampled to the terrain (the nearest value
decides between no water, river and lake; river strength is
interpolated), with a `"warning"` event. The mask stays through
`setWater()` and is cleared when new terrain loads. `null` removes it and
restores the terrain exactly.

```ts
const width = 512;
const height = 512;
const data = new Uint8Array(width * height);

// A 20 m wide river across the middle, and a round pond.
for (let x = 100; x < 400; x += 1) {
  data[256 * width + x] = 40;
}

for (let y = 0; y < height; y += 1) {
  for (let x = 0; x < width; x += 1) {
    if (Math.hypot(x - 380, y - 140) < 25) {
      data[y * width + x] = 255;
    }
  }
}

engine.setWaterMask({ width, height, data });
// Later: remove it and restore the terrain.
engine.setWaterMask(null);
```

A wrong type throws a `TypeError`; a wrong size or data length throws a
`VistaWasmError` with the code `OPTIONS_INVALID`.

### Sound hooks

`engine.getWaterSounds(x, y, z)` returns the loudest river, waterfall,
lake shore and surf near a position, for your own audio. Each is
`{ distanceMetres, loudness, position }`, or `null` when there is none
within 400 m (rivers and lake shores), 1500 m (waterfalls) or 600 m
(surf). Loudness runs from 0 to 1, from the source's strength over its
distance squared: river strength is speed × width, waterfall strength
discharge × drop, surf strength the wave height. Frozen water is silent.
A query reads only the grid cells around the position and costs a few
microseconds, so it can run every frame; VistaWASM plays no audio itself.

```ts
const audio = new AudioContext();
const river = new Audio("river-loop.ogg");
river.loop = true;
const gain = audio.createGain();
audio.createMediaElementSource(river).connect(gain).connect(audio.destination);
await river.play();

// `controls` from attachFlyCameraControls(), or your own camera.
engine.on("stats", () => {
  const [x, y, z] = controls.getCamera().position;
  const sounds = engine.getWaterSounds(x, y, z);
  gain.gain.setTargetAtTime(sounds.river?.loudness ?? 0, audio.currentTime, 0.2);
});
```

## Shading

- **Depth colour.** The shader reads the real water depth from the
  terrain heights. Shallow water shows the sea bed through
  `shallowColour`; it fades to `deepColour` by `clarityMetres`.
- **Reflections.** Schlick Fresnel reflects the same analytic sky as the
  sky pass, including cloud reflections, plus a GGX sun glitter.
  `reflectivity` scales reflection strength.
- **Subsurface light** glows through thin wave crests facing the sun.
- **Foam** appears on folding crests, along shorelines, in rapids, on the
  outer bank of bends and in plunge pools, scaled by `foam`.
- **Cloud shadows** and fog apply to water like everything else.

## Sea ice

Cold seas freeze. The concentration of ice follows the sea's mean
temperature: open water above -1.5 °C, full pack ice below -7.5 °C, and
loose floes in between. Over the terrain the temperature comes from the
climate (see [`docs/biomes.md`](biomes.md#climate-temperature)); beyond
it, from the sea-level mean of `BiomeOptions.meanTemperatureCelsius` and
`temperatureBias`. Where the climate is colder than -10 °C, fast ice is
frozen solid to the shore for 200 m out.

- **Floes** come in three sizes: big floes about 600 m across, broken by
    a 120 m scale into bays, cracks and loose pieces, with 25 m cakes in
    the gaps. Their edges are rounded and irregular. Beyond 1.5 to 3 km
    the cakes fade out, and beyond 4 to 8 km the middle scale, so distant
    ice does not shimmer. The pack drifts with the weather's wind at 2 %
    of its speed.
- **Leads** are long, narrow cracks of open water, 5 to 40 m wide, that
    meander across the sea. They grow fewer and narrower as the pack
    closes up.
- **Pressure ridges** run along some floe boundaries as thin, bright,
    raised lines.
- **Slush.** In a close pack, grey brash and grease ice fills the gaps
    between floes, matt and without glint.
- **Shading.** Floes are snow-white with a blue shadow side, lit like
    snow on the ground, with bevelled rims. About one in five carries only
    thin, patchy snow over blue-grey ice. The water in the leads is dark.
- **Calm.** Waves, ripples, and foam die down as the concentration rises.
    Where the ice is solid, the water beneath is not shaded at all.

Sea ice forms on the ocean; lakes, rivers and waterfalls freeze by their
own rules (see [Frozen water](#frozen-water)). A map whose sea never
freezes pays nothing for sea ice.

```ts
engine.setBiomes({ meanTemperatureCelsius: -12 });
```

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
- Rivers do not change sea level or flood terrain: discharge is a yearly
  mean, with no floods or droughts.
- No audio playback; `getWaterSounds()` tells your own audio where the
  water is.
