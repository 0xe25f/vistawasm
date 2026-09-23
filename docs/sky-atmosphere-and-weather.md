# Sky, Atmosphere, and Weather

VistaWASM's sky, clouds, and fog are drawn by one full-screen composite
pass (`shaders/atmosphere.wgsl`) after the opaque scene, reading its depth
buffer. The sky model itself lives in `shaders/common.wgsl`, shared by every
shader. This document covers the sun, the sky, clouds, and ground mist.
To have them change together with rain, snow, and wind, use the weather
system in [`docs/weather.md`](weather.md). Shadows are covered in
[`docs/shadows.md`](shadows.md).

For the exact field list of every option mentioned here, see
[`docs/options-reference.md`](options-reference.md).

## Sun (`SunOptions`)

The sun is a single directional light. Its colour comes from the
atmosphere model (white at midday, golden and red near the horizon);
surfaces also receive hemispherical sky light, and clouds cast moving
shadows. There is no terrain self-shadowing pass. Three controls:

- `azimuthDegrees` — compass direction the light comes from.
- `elevationDegrees` — angle above the horizon. Low values (below ~10°)
  give raking, golden light; negative values put the sun below the
  horizon for a dusk look (there is no star field).
- `intensity` — brightness multiplier, applied to both direct lighting and
  the sun disc/glare in the sky dome.

## The sky dome (`AtmosphereOptions`)

The sky is a compact single-scattering model shared by every shader in
`shaders/common.wgsl`, so the visible sky, water reflections, fog, and the
ambient light on terrain and trees always agree:

- **Rayleigh scattering** (`rayleighStrength`) — wavelength-dependent
  scattering that makes the zenith deep blue and the horizon pale.
- **Mie scattering** (`mieStrength`) — grey aerosol haze with a strong
  forward lobe, which gives the glow around the sun. Higher values give a
  hazier, more humid look.
- **Sunlight** is attenuated through the same atmosphere, using the real
  air mass for the sun's elevation, so direct light and the sky both turn
  golden and then red as the sun sets, and fade after sunset.
- **Sun disc** — a bright disc with a soft corona, hidden by clouds in
  front of it.
- **`exposure`** — scales the linear scene before ACES filmic tone
  mapping.
- **`skyTint`** — an RGB multiplier on the sky light.

All lighting is computed in linear light in a floating-point target and
tone mapped once, so bright skies roll off naturally instead of clipping.

### Haze vs. mist

`AtmosphereOptions.hazeDistanceMetres` and `MistOptions` are two
independent controls:

| | Haze (`AtmosphereOptions`) | Mist (`MistOptions`) |
| --- | --- | --- |
| What it responds to | Distance, thinning with altitude (1.2 km scale height), plus blue Rayleigh scattering | Height above `baseHeightMetres` (with a steep falloff), plus optional proximity to sea level |
| Effect | Distant ranges fade into blue-grey aerial perspective | Fog pools in valleys; a peak can stay clear above a buried valley |
| Default | Always on (`60000` m) | Off (`style: "off"`) |

Both are integrated analytically along each pixel's true view ray in the
composite pass, using the depth buffer, so they apply identically to
terrain, trees, grass, and (in its own pass) water.

## Clouds (`CloudsOptions`)

Clouds are drawn by the composite pass (`shaders/atmosphere.wgsl`) after
the opaque scene, so they appear behind terrain and also in front of it
when a mountain rises into the cloud layer.

- **`"off"`** (default) — no clouds and no cost.
- **`"painted"`** — a single layer shaded with a cheap self-shadow taken
  from the weather map towards the sun. Suitable for low-end devices.
- **`"volumetric"`** — a raymarch through a cloud slab between
  `heightMetres` and `heightMetres + thicknessMetres`:
  - A 2D weather map decides where clouds form (`coverage`) and how tall
    they grow.
  - Shapes come from baked 3D Perlin-Worley noise. Each cloud has a flat
    base and narrows into a rounded dome; denser weather grows taller
    towers.
  - Finer Worley detail erodes the edges into cauliflower billows at the
    top and soft wisps at the base. The detail fades out with distance,
    where it would otherwise alias into streaks.
  - A secondary march towards the sun gives self-shadowing. Lighting adds
    three orders of multiple scattering, which is what makes real cumulus
    glow white inside, plus sky light that leaves the bases darker. A
    two-lobe phase function gives silver linings towards the sun.
  - The march is adaptive. Steps grow with distance, empty sky is crossed
    in long steps, and when a step lands inside a cloud the march backs up
    and approaches the edge in fine steps, so silhouettes stay crisp. The
    start of each ray is offset by a stable dither rather than per-frame
    noise, so clouds do not crawl with grain. Cost scales with
    `raymarchSteps` (`8..=64`, default `32`); the loop runs at most four
    iterations per step, most of them cheap empty-sky skips.
  - Clouds are rendered at `resolutionScale` (default half) of the canvas
    resolution and upsampled.

### Cirrus

A thin, high cirrus layer (`cirrus`, default `0.35`, at
`cirrusHeightMetres`, default 9000 m) adds wind-stretched streaks above
the main clouds. It glows around the sun and costs three texture lookups
per sky pixel. Set `cirrus: 0` to remove it.

### Movement

Clouds drift with the wind (`windDirectionDegrees`, `speed`; `1` is about
15 m/s) and change shape as they go (`evolution`), animated from a real-time
clock. `seedOffset` changes the cloudscape itself.

### Cloud shadows

With `castShadows` (default `true`), clouds cast soft, moving shadows on
terrain, trees, grass, and water. Shadows are projected along the sun
direction from the same weather map that shapes the clouds, so they line
up with the clouds you can see.

## Mist and ground fog (`MistOptions`)

Mist is an exponential height fog, integrated along each view ray:

- **`"off"`** (default) — no ground fog.
- **`"flat"`** — a smooth layer, thickest at `baseHeightMetres` and thinning
  over `heightFalloffMetres`.
- **`"volumetric"`** — the same layer broken into fog banks by 3D noise
  sampled along the ray. Banks drift with `windDirectionDegrees` and
  `windSpeedMetresPerSecond`.

Mist is lit by the sky and the sun; `sunScattering` controls how strongly it
glows when you look towards the sun. `riseAboveWater` adds extra mist near
`WaterOptions.seaLevelMetres`, which gives the "mist rising off the sea"
look while water is enabled.

## Putting it together: a few starting points

**Clear midday**
```ts
sun: { azimuthDegrees: 140, elevationDegrees: 55, intensity: 1.2 },
atmosphere: { rayleighStrength: 1, mieStrength: 0.4, hazeDistanceMetres: 80000, exposure: 1.1, skyTint: [1, 1, 1] },
clouds: { style: "painted", coverage: 0.3 },
mist: { style: "off" }
```

**Foggy valley morning**
```ts
sun: { azimuthDegrees: 100, elevationDegrees: 12, intensity: 1.0 },
atmosphere: { hazeDistanceMetres: 20000, exposure: 0.95, skyTint: [0.95, 0.92, 0.88] },
clouds: { style: "painted", coverage: 0.55 },
mist: { style: "volumetric", density: 0.7, baseHeightMetres: 20, heightFalloffMetres: 90, riseAboveWater: true, windSpeedMetresPerSecond: 1.5 }
```

**Dramatic overcast, hyper-realistic**
```ts
sun: { azimuthDegrees: 220, elevationDegrees: 25, intensity: 0.9 },
atmosphere: { mieStrength: 0.6, exposure: 1.0 },
clouds: { style: "volumetric", coverage: 0.8, raymarchSteps: 48, thicknessMetres: 2500, density: 0.9, speed: 2 },
mist: { style: "flat", density: 0.2, baseHeightMetres: 0, heightFalloffMetres: 200, riseAboveWater: false }
```

See [`docs/world-design-guide.md`](world-design-guide.md#5-sea-level-atmosphere-and-mood)
for more recipes covering terrain shape alongside sky/weather.
