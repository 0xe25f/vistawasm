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

1. **Noise** (`noise.kind`, `octaves`, `gain`, `lacunarity`, `warp`) produces
    a base height field in the range `[-1, 1]` per sample, built from layered
    value noise.
2. **Shape** (`shape.island`, `shape.terrace`, `shape.basin`, `shape.canyon`,
    `shape.crater`) is applied on top of the noise field, still in `[-1, 1]`,
    before it is scaled to metres.
3. **Scaling** — the shaped `[-1, 1]` field is multiplied by
    `verticalScale` and by an internal metres-per-unit factor, then offset by
    `baseHeightMetres`, to produce real height-in-metres values.
4. **Erosion** (`erosion.hydraulicIterations`, `erosion.thermalIterations`,
    ...), if requested, runs last, directly on the height-in-metres data. On
    browser builds it runs as GPU compute passes; if that fails for any
    reason it automatically falls back to an equivalent CPU pass, so it
    always produces a result.
5. **Materials and normals** are derived from the *final*, eroded heights
    (slope and height-above-sea-level), so erosion should always be tuned
    before you judge how grass/rock/snow/mud placement looks.

Because shape is applied before erosion, a `shape.canyon` gash will still
get carved further by hydraulic erosion; a `shape.crater` rim will still
accumulate thermal scree at its edges. This is usually what you want (it's
why erosion exists), but it means erosion parameters interact with shape
parameters — always look at the eroded result, not the pre-erosion preview.

## 2. Noise kinds

| `noise.kind` | What it produces | Good for |
| --- | --- | --- |
| `simplex` | Smooth, rolling, un-shaped noise. | Gentle hills, farmland, base layer for heavy custom shaping. |
| `ridged` | Inverted-absolute-value noise; sharp connected ridgelines. | Mountain ranges, alpine terrain. |
| `hybrid` | A 45/55 blend of `simplex` and `ridged`. | Foothills — mountains fading into rolling terrain. |
| `island` | Like `simplex`, plus an automatic radial falloff toward the terrain edge (even without setting `shape.island`). | Quick islands/atolls without hand-tuning falloff. |
| `canyon` | Like `simplex`, plus an automatic canyon gash mask. | Quick canyon terrain; combine with `ridged`-like erosion for realism. |
| `cratered` | Like `simplex`, plus one deterministic (seeded) crater mask. | Volcanic calderas, impact sites, quarry pits. |
| `classic` | A gentler, more compressed remap of the base noise. | Subtle, low-contrast terrain — good "default" for UI seed-scrubbing previews. |

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
  the edges. Combine with `Water.enabled` and a `seaLevelMetres` above the
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

Erosion is the single highest-impact control for making noise look like
real terrain rather than a height field. Two independent passes:

- **Hydraulic** (`hydraulicIterations`) simulates rain, transport, and
  deposition — it carves valleys and river channels and softens ridgelines.
  Start around `10`–`20` for a quick pass, `40`–`80` for pronounced valley
  carving. Diminishing returns set in past roughly `100` iterations for most
  terrain sizes.
- **Thermal** (`thermalIterations`) simulates scree/talus slumping — it
  softens slopes steeper than `talusAngleDegrees` by moving material
  downhill until slopes settle. Use it *after* hydraulic erosion has carved
  the major features, to naturalise the resulting cliffs. `8`–`15`
  iterations is usually enough; more just keeps flattening slopes.
- `rainAmount` / `evaporation` / `sedimentCapacity` control how aggressively
  hydraulic erosion moves material; the defaults are reasonable starting
  points, tune `rainAmount` up first if valleys aren't carving deep enough.
- `talusAngleDegrees` is the slope angle (from horizontal) above which
  thermal erosion considers a slope "too steep" and moves material — lower
  it (e.g. `28`) for softer, more rounded terrain; raise it (e.g. `45`+) to
  preserve sharp cliffs and rock faces.
- `quality` (`"preview" | "balanced" | "high" | "offline"`) caps the total
  iteration budget regardless of what you request, so a "preview" quality
  UI stays responsive while a user drags sliders, while switching to
  `"high"`/`"offline"` before a final export gives the full, requested
  iteration count.

Erosion always runs on the *whole* heightmap, so cost scales with
`size` × iteration counts. If you're iterating on shape/noise parameters in
an editor UI, prefer a small `size` (e.g. `512`) with erosion disabled while
scrubbing, then switch to the real size with erosion enabled for the final
generation — exactly what the demo's control panel encourages by keeping
regeneration on every input change cheap.

## 5. Sea level, atmosphere, and mood

- Set `waterOptions.seaLevelMetres` relative to the generated heightmap's
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
  `AtmosphereOptions.hazeDistanceMetres` is a uniform, distance-only blend
  to sky colour: it does not care about height, so it fades a mountain
  peak and the valley beneath it equally at the same distance.
  `MistOptions` is a height-based ground fog: it pools near
  `baseHeightMetres` and thins out over `heightFalloffMetres`, so a
  mountain peak can stand clear above a misty valley even at the same
  distance from the camera. Use haze for "how far can I see"; use mist for
  "how much of the low ground is buried in fog". `MistOptions.style`
  starts at `"flat"` (a static falloff) or `"volumetric"` (the same
  falloff modulated by drifting noise, for mist that visibly moves);
  `riseAboveWater` adds extra mist near `WaterOptions.seaLevelMetres`
  regardless of `baseHeightMetres`, for a "mist rising off the lake" look.
- `CloudsOptions.style` adds an optional cloud layer to the sky dome:
  `"painted"` is a single, cheap noise sample at `heightMetres` (the
  default once enabled); `"volumetric"` raymarches a thin band around that
  altitude for real depth and sun-facing shading, at a real GPU cost —
  reserve it for a `RenderQualityOptions.preset` of `"high"` or
  `"offline"` rather than `"preview"`/`"balanced"`. `coverage` runs from
  `0` (clear) to `1` (overcast); values in the middle (`0.3`–`0.5`) give
  the most visually interesting patchy sky.

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
and colour variety; `windStrength` (0 to 1) drives gusting sway.

`GrassOptions` is a separate, independent ground-cover layer — unlike the
other environmental options it defaults fully `enabled: false`, since it
is a brand-new visual element with no prior equivalent. Once enabled, it
reuses the terrain's own baked material weights for placement (so grass
naturally avoids rock, snow, mud, and underwater terrain without any
extra tuning) and fades out smoothly over the last portion of
`viewDistanceMetres` rather than popping — grass is not expected to
render all the way to the terrain's horizon the way trees do. `style:
"dense-blades"` raises instance density and is intended to be paired with
a shorter `viewDistanceMetres`, since grass is small on screen at range
and hyper-realistic density is best spent where it is actually visible.

## 7. Recipes

Concrete starting points — copy, then adjust to taste. All assume
`Water.enabled: true` and reasonable defaults for anything not listed.

**Alpine valley**
```ts
noise: { kind: "ridged", octaves: 8, gain: 0.5, lacunarity: 2.1 },
shape: { terrace: 0.05 },
erosion: { hydraulicIterations: 40, thermalIterations: 15, talusAngleDegrees: 35 },
verticalScale: 1.4
```

**Archipelago**
```ts
noise: { kind: "island", octaves: 7, gain: 0.5, lacunarity: 2 },
shape: { island: 0.75 },
erosion: { hydraulicIterations: 15, thermalIterations: 8 },
verticalScale: 0.8,
// set seaLevelMetres just above the generated minHeightMetres
```

**Desert canyon**
```ts
noise: { kind: "canyon", octaves: 6, gain: 0.55, lacunarity: 2 },
shape: { canyon: 0.7, terrace: 0.12 },
erosion: { hydraulicIterations: 60, thermalIterations: 10, talusAngleDegrees: 45 },
verticalScale: 1.0
// flora density near 0 — deserts have sparse vegetation
```

**Volcanic crater island**
```ts
noise: { kind: "cratered", octaves: 7, gain: 0.5, lacunarity: 2 },
shape: { island: 0.6, crater: 0.5, basin: 0.2 },
erosion: { hydraulicIterations: 25, thermalIterations: 20, talusAngleDegrees: 40 },
verticalScale: 1.6
```

**Rolling farmland**
```ts
noise: { kind: "simplex", octaves: 5, gain: 0.5, lacunarity: 2 },
shape: {},
erosion: { hydraulicIterations: 10, thermalIterations: 5 },
verticalScale: 0.35
```

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
