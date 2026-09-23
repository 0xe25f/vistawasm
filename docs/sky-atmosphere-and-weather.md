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
surfaces also receive hemispherical sky light. Hills, trees, and clouds
cast sun shadows (see [`docs/shadows.md`](shadows.md)). Three controls:

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
    in long steps, and when a step lands inside a cloud the march
    bisects back to the cloud's edge and continues in fine steps, so
    silhouettes stay crisp even at full resolution. The
    start of each ray is offset by a stable dither rather than per-frame
    noise, so clouds do not crawl with grain. Cost scales with
    `raymarchSteps` (`8..=64`, default `32`); the loop runs at most four
    iterations per step, most of them cheap empty-sky skips.
  - Clouds are rendered at `resolutionScale` (default half) of the canvas
    resolution and upsampled.

### Cloud types

Five fields change what kind of cloud the volumetric layer holds, and the
weather system sets them for you when it drives the clouds:

- **`stratiform`** blends heaped cumulus into a low, flat sheet (stratus,
  or nimbostratus under rain).
- **`towering`** grows storm cells, about 6 km apart, into cumulonimbus
  towers that rise up to 2.6 times the layer thickness and spread into
  anvils near the top.
- **`baseDarkness`** makes cloud bases absorb more light, as rain-laden
  clouds do. It also greys the sky under the weather system.
- **`raggedBase`** roughens and lifts the underside and scatters loose
  scraps of cloud (scud) beneath it.
- **`rainShafts`** hangs curtains of rain or snow below the clouds. They
  slant downwind and are lit by the cloud base above them.

During a storm, lightning also lights the clouds around each strike from
inside. All five default to `0`, which gives the fair-weather cumulus.

### Cirrus

A thin, high cirrus layer (`cirrus`, default `0.35`, at
`cirrusHeightMetres`, default 9000 m) adds wind-stretched streaks above
the main clouds. It glows around the sun and costs three texture lookups
per sky pixel. Set `cirrus: 0` to remove it.

### Movement

Clouds drift with the wind (`windDirectionDegrees`, `speed`; `1` is about
15 m/s) and change shape as they go (`evolution`), animated from a real-time
clock. `seedOffset` changes the cloudscape itself.

Cirrus drifts in the same direction at its own speed, `cirrusSpeed`
(default `0.4`, in the same units). The weather system changes `speed`
with the wind but leaves `cirrusSpeed` alone, so high cloud keeps its slow,
distant drift in a gale.

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

`setAtmosphere()`, `setClouds()`, and `setMist()` replace the whole
object, so every required field is listed.

**Clear midday**

```ts
engine.setSun({ azimuthDegrees: 140, elevationDegrees: 55, intensity: 1.2 });
engine.setAtmosphere({ rayleighStrength: 1, mieStrength: 0.4, hazeDistanceMetres: 80000, exposure: 1.1, skyTint: [1, 1, 1] });
engine.setClouds({ style: "painted", coverage: 0.3, speed: 1, heightMetres: 1800, colour: [1, 1, 1], seedOffset: 9007 });
engine.setMist({ style: "off", density: 0, baseHeightMetres: 40, heightFalloffMetres: 120, colour: [0.82, 0.85, 0.88], riseAboveWater: false, seedOffset: 5303 });
```

**Foggy valley morning**

```ts
engine.setSun({ azimuthDegrees: 100, elevationDegrees: 12, intensity: 1 });
engine.setAtmosphere({ rayleighStrength: 1, mieStrength: 0.45, hazeDistanceMetres: 20000, exposure: 0.95, skyTint: [0.95, 0.92, 0.88] });
engine.setClouds({ style: "painted", coverage: 0.55, speed: 1, heightMetres: 1800, colour: [1, 1, 1], seedOffset: 9007 });
engine.setMist({ style: "volumetric", density: 0.7, baseHeightMetres: 20, heightFalloffMetres: 90, colour: [0.82, 0.85, 0.88], riseAboveWater: true, seedOffset: 5303, windSpeedMetresPerSecond: 1.5 });
```

**Dramatic overcast, hyper-realistic**

```ts
engine.setSun({ azimuthDegrees: 220, elevationDegrees: 25, intensity: 0.9 });
engine.setAtmosphere({ rayleighStrength: 1, mieStrength: 0.6, hazeDistanceMetres: 60000, exposure: 1, skyTint: [1, 1, 1] });
engine.setClouds({ style: "volumetric", coverage: 0.8, speed: 2, heightMetres: 1800, colour: [1, 1, 1], seedOffset: 9007, raymarchSteps: 48, thicknessMetres: 2500, density: 0.9, stratiform: 0.6, baseDarkness: 0.5 });
engine.setMist({ style: "flat", density: 0.2, baseHeightMetres: 0, heightFalloffMetres: 200, colour: [0.82, 0.85, 0.88], riseAboveWater: false, seedOffset: 5303 });
```

See [`docs/world-design-guide.md`](world-design-guide.md#5-sea-level-atmosphere-and-mood)
for more recipes covering terrain shape alongside sky/weather.
