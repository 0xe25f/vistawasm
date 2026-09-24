# Designing Worlds With VistaWASM

This guide is about the creative/parameter side of VistaWASM: what each
terrain control actually does, how to combine them, and worked recipes you
can start from. It assumes you already have `generateFractal()` wired up —
see [`docs/getting-started.md`](getting-started.md) for that.

For the exact type/default/validation of every field mentioned here, see
[`docs/options-reference.md`](options-reference.md). For deeper,
system-specific detail beyond what this guide covers, see
[`docs/terrain-data.md`](terrain-data.md) (generation/DEM data model),
[`docs/vegetation.md`](vegetation.md) (trees/grass), and
[`docs/sky-atmosphere-and-weather.md`](sky-atmosphere-and-weather.md)/
[`docs/water.md`](water.md) (sky, weather, water).

## 1. The generation pipeline, in order

Understanding the order VistaWASM applies controls in makes it much easier
to predict what a parameter change will do:

1. **Landform** (`landform`) sets the big picture: how much of the map is
    land, how large continents and ranges are, how high the ranges stand,
    how hard rain and ice carve them, and the talus angle. Continents and
    ranges are laid out first, then carved by a stream-power model into
    valleys and ridge spurs. See
    [`docs/terrain-data.md`](terrain-data.md#landforms).
2. **Detail** (`noise.kind`, `octaves`, `gain`, `lacunarity`, `warp`) adds
    small-scale relief to the carved land, in metres. It is strongest on
    steep ground in the ranges and fades out on level ground, so plains and
    valley floors stay smooth.
3. **Shape** (`shape.island`, `shape.terrace`, `shape.basin`,
    `shape.canyon`, `shape.crater`) is applied on top, in units of the
    landform's relief.
4. **Scaling**: heights are stretched about sea level by `verticalScale`,
    then offset by `baseHeightMetres`.
5. **Erosion** (`erosion.quality`, `erosion.hydraulicIterations`, ...), if
    requested, runs last at full resolution: rain cuts gullies and builds
    fans, and thermal erosion leaves scree below cliffs. On browser builds
    it runs as GPU compute passes; if that fails for any reason it falls
    back to the CPU reference, so it always produces a result.
6. **Materials and normals** are derived from the *final*, eroded heights
    (slope and height above sea level), so erosion should always be tuned
    before you judge how grass, rock, snow and mud placement looks.

Because shape is applied before erosion, a `shape.canyon` gash will still
get carved further by hydraulic erosion; a `shape.crater` rim will still
accumulate thermal scree at its edges. This is usually what you want (it's
why erosion exists), but it means erosion parameters interact with shape
parameters — always look at the eroded result, not the pre-erosion preview.

Start with a landform. Reach for the noise and shape controls to adjust
its surface, not to build a whole world from noise.

## 2. Noise kinds

`noise.kind` picks the flavour of the detail layer. It no longer decides
where mountains are; the landform does.

| `noise.kind` | What it produces | Good for |
| --- | --- | --- |
| `simplex` | Smooth, rolling detail. | Soft, weathered slopes. |
| `ridged` | Crisp ridged detail on the ranges, smooth detail elsewhere. | Mountain ranges, alpine terrain. |
| `hybrid` | Half-ridged detail on the ranges. | Foothills — mountains fading into rolling terrain. |
| `island` | Like `simplex`, plus an automatic radial falloff toward the terrain edge (even without setting `shape.island`). | Quick islands/atolls without hand-tuning falloff. |
| `canyon` | Like `simplex`, plus an automatic canyon gash mask. | Quick canyon terrain; combine with `ridged`-like erosion for realism. |
| `cratered` | Like `simplex`, plus one deterministic (seeded) crater mask. | Volcanic calderas, impact sites, quarry pits. |
| `classic` | Billowy detail with a slight step, like the first releases. | A softer, older look. |

`shape.island` / `shape.canyon` / `shape.crater` (below) apply the *same*
masks as the `island`/`canyon`/`cratered` noise kinds, but as an independent,
tunable-strength layer on top of *any* noise kind — e.g. `ridged` noise with
`shape.island: 0.6` gives a mountainous island, which the `island` noise kind
alone cannot.

## 3. Shape controls

All `shape.*` values are roughly `0..1` strength dials (some tolerate
slightly over 1 for an exaggerated effect) applied in terrain-space, centred
on the terrain's own centre:

- **`island`** — radial falloff toward the edges. `0.3` gives a soft
  coastline fade; `0.8`+ gives a hard island silhouette with flat ocean at
  the edges. Combine with `WaterOptions.enabled` and a `seaLevelMetres` above the
  lowest heights to actually see the coastline.
- **`terrace`** — quantises height into flat steps. Small values (`0.05`)
  give subtle rock-strata banding on slopes; large values (`0.3`+) give
  pronounced rice-paddy/quarry terracing. Works best combined with `ridged`
  or `hybrid` noise so the terraces have interesting slopes to interrupt.
- **`basin`** — depresses the terrain toward the centre (the opposite of
  `island`). Use for crater lakes, calderas, or a central sea surrounded by
  higher terrain at the edges.
- **`canyon`** — carves a linear gash. Increase `erosion.hydraulicIterations`
  afterward to soften and naturalise the carved walls.
- **`crater`** — one deterministic circular depression, seeded from your
  terrain seed (so it moves if you change the seed). Good for a single
  focal-point feature; it is not a crater *field* — for many craters you
  would need to layer your own height edits externally (not currently
  supported by a single `generateFractal()` call).

## 4. Erosion tuning

The landform already carves valleys and ridge spurs before erosion runs.
Erosion adds what only full-resolution water and gravity make: gullies,
alluvial fans where valleys open out, aggraded valley floors, and scree
below cliffs.

- **`quality`** is the main dial. Unset iteration counts follow it:
  `"preview"` 60 hydraulic and 30 thermal iterations, `"balanced"` 120 and
  60, `"high"` 200 and 100, `"offline"` 400 and 200. Requested counts are
  capped at 120, 240, 400 and 5000. Use `"preview"` while scrubbing and
  `"high"` for the final map.
- **Hydraulic** (`hydraulicIterations`) simulates rain flowing over the
  terrain, cutting into slopes and depositing where water slows.
- **Thermal** (`thermalIterations`) moves material downhill wherever the
  slope exceeds `talusAngleDegrees`, and adds a slow soil creep.
- `rainAmount`, `evaporation` and `sedimentCapacity` control how hard
  hydraulic erosion works. Unset, rain follows the landform (dry for
  `mesaDesert`, wet for `fjords`). Raise `sedimentCapacity` first if
  gullies are too faint.
- `talusAngleDegrees` defaults to the landform's value. Lower it (e.g.
  `28`) for softer, rounder terrain; raise it (e.g. `45`) to keep sharp
  cliffs.

Erosion always runs on the *whole* heightmap, so cost scales with
`size` × iteration counts. It runs on the GPU in browsers, where even
`"high"` is a small part of generation on a mid-range GPU.

## 5. Sea level, atmosphere, and mood

- Set `WaterOptions.seaLevelMetres` relative to the generated heightmap's
  `minHeightMetres`/`meanHeightMetres` (from `TerrainMetadata`, returned by
  every generate/load call) rather than a hardcoded constant — the same sea
  level constant will flood a `verticalScale: 2` mountain range and look
  wrong on a `verticalScale: 0.3` plain.
- `SunOptions.elevationDegrees` below ~10° combined with a low
  `AtmosphereOptions.exposure` gives dawn/dusk lighting; combine with a
  warmer `skyTint` for golden hour. Elevation above 60° with a blue-leaning
  `skyTint` gives flat midday light.
- `AtmosphereOptions.hazeDistanceMetres` is your primary "how far can you
  see clearly" dial — lower it (a few thousand metres) for foggy/misty
  moods or to hide LOD/pop-in on very large terrain; raise it (tens of
  thousands of metres) for crisp, clear-day vistas.
- `rayleighStrength`/`mieStrength` control the sky gradient and sun-glare
  size respectively — higher `mieStrength` gives a bigger, hazier sun disc
  (good for a dusty/humid look), higher `rayleighStrength` gives a more
  saturated blue-to-horizon gradient.
- **Haze vs. mist** — these are two independent controls, easy to confuse.
  `AtmosphereOptions.hazeDistanceMetres` sets distance haze (aerial
  perspective). It thins gently with altitude (a 1.2 km scale height), so
  it reads as distance, not as a fog layer.
  `MistOptions` is a height-based ground fog: it pools near
  `baseHeightMetres` and thins out over `heightFalloffMetres`, so a
  mountain peak can stand clear above a misty valley even at the same
  distance from the camera. Use haze for "how far can I see"; use mist for
  "how much of the low ground is buried in fog". `MistOptions.style`
  starts at `"flat"` (a static falloff) or `"volumetric"` (the same
  falloff modulated by drifting noise, for mist that visibly moves);
  `riseAboveWater` adds extra mist near `WaterOptions.seaLevelMetres`
  regardless of `baseHeightMetres`, for a "mist rising off the lake" look.
- `CloudsOptions.style` adds an optional cloud layer: `"painted"` is a
  single, cheap noise layer at `heightMetres`; `"volumetric"` raymarches
  real 3D clouds with self-shadowing, silver linings, and moving shadows,
  at a real GPU cost. `coverage` runs from `0` (clear) to `1` (overcast);
  values in the middle (`0.3`–`0.5`) give the most interesting patchy
  sky. `cirrus` adds thin, high streaks above them. The cloud-type
  options set the character of the sky: `stratiform` for a grey sheet,
  `towering` for distant thunderstorms, `baseDarkness` and `raggedBase`
  for heavy rain cloud, and `rainShafts` for curtains of rain (see
  [`docs/sky-atmosphere-and-weather.md`](sky-atmosphere-and-weather.md#cloud-types)).
- For a sky that changes by itself, turn on the weather system
  (`setWeather({ enabled: true, autoCycle: true })`). It drives clouds,
  mist, wind, waves, rain, snow, wet ground, and lightning together, and
  any of those can be left under your own control (see
  [`docs/weather.md`](weather.md)).
- Low sun and strong shadows sell a landscape's shape: mountains shadow
  valleys and trees shadow the ground (see [`docs/shadows.md`](shadows.md)).
  Soften them (`softness`) or lighten them (`strength`) for a hazier,
  gentler mood.

## 6. Vegetation

See [`docs/vegetation.md`](vegetation.md) for the full placement algorithm
and every quality tier in detail; this section is the short, practical
version.

`FloraOptions.density` and `treeLineMetres` are the two levers that matter
most: `density` scales instance count, `treeLineMetres` sets the height
above which trees stop spawning (use your terrain's real height range from
`TerrainMetadata` to set this sensibly — a `treeLineMetres` above your
terrain's `maxHeightMetres` means no visible tree line at all). Flora
automatically avoids underwater, river, and very steep terrain, so you rarely need
to hand-tune placement beyond these two values plus `RenderQualityOptions.
floraDensityScale` for a performance-driven global multiplier.

`FloraOptions.treeQuality` is a separate, purely visual dial: `"mesh"`
(the default) draws full 3D species models near the camera and impostors
in the distance; `"cross-quad"` and `"billboard"` draw impostors only and
are cheaper. Which species grow where is decided by the biome map — to
change the character of a forest, shift the climate with
`setBiomes({ temperatureBias, moistureBias })` (see
[`docs/biomes.md`](biomes.md)). `speciesVariation` (0 to 1) controls size
and colour variety; `windStrength` (0 to 1) drives gusting sway. To choose
the species yourself, give a biome a `speciesRules` entry; to use your own
tree models, placement, or textures, see [`docs/hooks.md`](hooks.md).

`GrassOptions` is a separate, independent ground-cover layer — unlike the
other environmental options it defaults fully `enabled: false`, since it
is a brand-new visual element with no prior equivalent. Once enabled, it
reuses the terrain's own baked material weights for placement (so grass
naturally avoids rock, snow, mud, and underwater terrain without any
extra tuning) and fades out smoothly over the last portion of
`viewDistanceMetres` rather than popping — grass is not expected to
render all the way to the terrain's horizon the way trees do. `style:
"dense-blades"` currently draws the same tufts as `"billboard-blades"`;
for thicker grass, raise `density` and shorten `viewDistanceMetres`, since
grass is small on screen at range and density is best spent where it is
actually visible.

## 7. Recipes

Concrete starting points — copy, then adjust to taste. Each is a complete
`generateFractal()` call; pick your own `seed`, `size`, and
`horizontalScaleMetres`. Water is on by default, so set
`WaterOptions.seaLevelMetres` from the returned metadata (see
[section 5](#5-sea-level-atmosphere-and-mood)).

**Alpine valleys**

```ts
await engine.generateFractal({
  seed: 1, size: 1024, horizontalScaleMetres: 10, verticalScale: 1,
  noise: { kind: "ridged", octaves: 7, gain: 0.5, lacunarity: 2 },
  landform: "alpine",
  erosion: { quality: "high" }
});
```

**Archipelago**

```ts
const archipelago = await engine.generateFractal({
  seed: 2, size: 1024, horizontalScaleMetres: 10, verticalScale: 1,
  noise: { kind: "simplex", octaves: 6, gain: 0.5, lacunarity: 2 },
  landform: "archipelago",
  erosion: { quality: "high" }
});
// The coast is generated at seaLevelMetres, so the water plane matches it.
engine.setWater({
  enabled: true,
  seaLevelMetres: archipelago.metadata.seaLevelMetres,
  waveScale: 0.8,
  reflectivity: 0.35,
  shorelineSoftnessMetres: 6
});
```

**Desert mesas and canyons**

```ts
await engine.generateFractal({
  seed: 3, size: 1024, horizontalScaleMetres: 10, verticalScale: 1,
  noise: { kind: "ridged", octaves: 6, gain: 0.5, lacunarity: 2 },
  landform: "mesaDesert",
  shape: { canyon: 0.4 },
  erosion: { quality: "high" }
});
// Deserts have sparse vegetation: also push the climate hot and dry.
engine.setBiomes({ temperatureBias: 0.8, moistureBias: -0.8 });
```

**Volcanic island**

```ts
await engine.generateFractal({
  seed: 4, size: 1024, horizontalScaleMetres: 10, verticalScale: 1,
  noise: { kind: "hybrid", octaves: 7, gain: 0.5, lacunarity: 2 },
  landform: "volcanicIsland",
  erosion: { quality: "high", talusAngleDegrees: 40 }
});
```

**Fjord coast**

```ts
await engine.generateFractal({
  seed: 5, size: 1024, horizontalScaleMetres: 10, verticalScale: 1,
  noise: { kind: "ridged", octaves: 7, gain: 0.5, lacunarity: 2 },
  landform: "fjords",
  erosion: { quality: "high" }
});
```

**Rolling farmland**

```ts
await engine.generateFractal({
  seed: 6, size: 1024, horizontalScaleMetres: 10, verticalScale: 1,
  noise: { kind: "simplex", octaves: 5, gain: 0.5, lacunarity: 2 },
  landform: "rollingHills",
  erosion: { quality: "balanced" }
});
```

See [`docs/terrain-data.md`](terrain-data.md#erosion) for the erosion
model and its budgets.

## 8. Real-world terrain (DEM import)

For a specific real location rather than a procedural one, load a DEM
instead of generating fractal noise — see the README's "Loading A DEM"
section for the API. A few things that affect how a real DEM *looks* once
loaded, versus a fractal terrain:

- Real DEMs typically have a much larger absolute height range and a
  non-zero base elevation (a DEM of a mountain range might start at
  `1500m`), so set `seaLevelMetres`/atmosphere `hazeDistanceMetres` relative
  to the DEM's own reported `TerrainMetadata`, not the constants you used
  for fractal terrain.
- `verticalScale` on DEM loads is a multiplier on real elevation — usually
  leave it at `1` (true scale) unless you deliberately want an exaggerated
  or flattened look (e.g. `1.5` to make subtle real-world relief read more
  dramatically at a distance, common in flight-sim-style visualisations).
- DEM-derived terrain has no `shape`/`erosion` controls applied by default —
  the height data is real. You can still enable `erosion` on a loaded DEM if
  you want to visually soften resampling artefacts, but be aware it will
  also erode real, meaningful terrain features.
- `generateNormals`/`generateMaterialMasks` (in `DemLoadOptions`) should
  normally stay `true` — the render pipeline needs both regardless of
  terrain source.
