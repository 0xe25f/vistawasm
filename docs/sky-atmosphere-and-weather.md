# Sky, Atmosphere, and Weather

VistaWASM's sky is one analytic, texture-free shader
(`shaders/atmosphere.wgsl`) drawn as a fullscreen background pass before
terrain, flora, grass, and water. This document covers everything that
shader (and the terrain/flora/grass/water shaders it shares state with)
renders: the sun, the sky dome itself, clouds, and ground mist.

For the exact field list of every option mentioned here, see
[`docs/options-reference.md`](options-reference.md).

## Sun (`SunOptions`)

The sun is a single directional light with no shadows (VistaWASM has no
shadow-mapping pass today — every surface is lit purely by
`max(dot(normal, sunDirection), 0) * intensity` plus a small constant
ambient term). Three controls:

- `azimuthDegrees` — compass direction the light comes from.
- `elevationDegrees` — angle above the horizon. Low values (below ~10°)
  give long shadows-equivalent raking light and warm colour from the sky
  dome's horizon gradient; negative values put the sun below the horizon
  entirely (a dusk/night look, though there is no separate night sky/star
  rendering).
- `intensity` — brightness multiplier, applied to both direct lighting and
  the sun disc/glare in the sky dome.

## The sky dome (`AtmosphereOptions`)

The sky is a practical approximation of real-world scattering, not a
physically integrated simulation:

- **Rayleigh term** (`rayleighStrength`) — a blue gradient that deepens
  towards the zenith and pales towards the horizon.
- **Mie term** (`mieStrength`) — a forward-scattering haze/glow around the
  sun, using a Henyey-Greenstein phase approximation. Higher values give a
  bigger, hazier sun disc — good for a dusty/humid look.
- **Sun disc** — a bright disc where the view ray closely aligns with the
  sun direction, coloured and scaled by `SunOptions.intensity`.
- **Horizon haze** — blends the sky towards `skyTint` near the horizon,
  approximating aerial perspective where no terrain is in view.
- **`exposure`** — an overall brightness multiplier applied after every
  other term, including clouds and the sun disc.
- **`skyTint`** — an RGB multiplier applied to the whole sky/haze/cloud
  result. Push it warm for golden hour, cool/blue for a crisp midday look.

### Haze vs. mist — read this before using either

`AtmosphereOptions.hazeDistanceMetres` and `MistOptions` are two
**independent** controls that are easy to confuse:

| | Haze (`AtmosphereOptions`) | Mist (`MistOptions`) |
| --- | --- | --- |
| What it responds to | Distance from the camera only | Height above `baseHeightMetres`, plus optional proximity to sea level |
| Effect | Fades everything to sky colour equally at a given distance, regardless of height | Pools near the ground/valleys; a mountain peak can stay clear while the valley beneath it is buried in fog, even at the same camera distance |
| Applies to | Terrain only | Terrain, flora, grass, and water — see below |
| Default | Always on (`60000` m) | Off (`style: "off"`) |

Use haze for "how far can I see" (and to hide LOD pop-in on very large
terrain — lower it for that). Use mist for "how much of the low ground is
buried in fog", independent of view distance.

## Clouds (`CloudsOptions`)

Clouds have two styles, both implemented inside the *same* atmosphere
shader/pipeline (no separate pass) — the fullscreen pass already
reconstructs a per-pixel view ray, and terrain drawn afterwards already
occludes distant sky/cloud pixels behind mountains via the depth buffer, so
no extra pipeline or depth-awareness is needed:

- **`"off"`** (default) — no cloud layer at all, and the shader skips the
  cloud branch entirely (one cheap check per pixel), so leaving clouds off
  costs effectively nothing.
- **`"painted"`** — a single 2-D noise sample at `heightMetres`, turned into
  a soft-edged cloud/clear mask by a `coverage`-controlled threshold. Cheap:
  one noise sample per pixel. The recommended default once you enable
  clouds at all.
- **`"volumetric"`** — a raymarched density band around `heightMetres`,
  composited with a proper `1 - transmittance` opacity accumulation (so
  results stay well-behaved across the whole `coverage` range instead of
  saturating to a flat white overcast at moderate coverage). Brightens
  toward the sun for a cheap single-scatter look. Real GPU cost,
  proportional to `raymarchSteps` (hard-clamped to `8..=64` — see
  [`docs/options-reference.md`](options-reference.md#cloudsoptions)).
  Reserve this for a `RenderQualityOptions.preset` of `"high"`/`"offline"`
  rather than `"preview"`/`"balanced"`.

`coverage` runs `0` (clear) to `1` (overcast); the middle of that range
(roughly `0.3`–`0.5`) gives the most visually interesting patchy sky — very
low or very high values trend towards "empty" or "solid" respectively, which
is realistic but less visually varied. `speed` controls drift rate (clouds
drift using the same shared per-frame clock the water/wind animations use,
scaled by `speed`, not wall-clock time). `seedOffset` makes the noise
pattern itself reproducible for a given seed — only the *drift* animates.

Cloud shadows on the terrain are not implemented — clouds affect the sky
only, not terrain lighting.

## Mist and ground fog (`MistOptions`)

Mist is a height-based ground fog, applied consistently in every shader
that can be affected by it: `clipmap_render.wgsl` (terrain),
`flora_instances.wgsl` (trees), `grass_instances.wgsl` (grass), and
`water.wgsl` (water) all read the same `mist_params`/`mist_colour` uniform
fields, so a tree or a patch of grass standing in a misty valley is tinted
the same as the ground beneath it, rather than popping out unaffected the
way it would if mist only touched the terrain.

- **`"off"`** (default) — no ground fog.
- **`"flat"`** — a static height-falloff blend: mist is thickest at
  `baseHeightMetres` and thins out over `heightFalloffMetres`, then fades
  further with distance from the camera so it does not extend to the far
  clip plane at full strength.
- **`"volumetric"`** — the same falloff, modulated by drifting noise (using
  the same shared per-frame clock as clouds/water/wind), so mist visibly
  moves rather than sitting static.

`riseAboveWater` adds an extra mist contribution near
`WaterOptions.seaLevelMetres`, independent of `baseHeightMetres` — this is
what gives the "mist rising off the lake" look, and only has an effect
while `WaterOptions.enabled` is true.

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
mist: { style: "volumetric", density: 0.7, baseHeightMetres: 20, heightFalloffMetres: 90, riseAboveWater: true }
```

**Dramatic overcast, hyper-realistic**
```ts
sun: { azimuthDegrees: 220, elevationDegrees: 25, intensity: 0.9 },
atmosphere: { mieStrength: 0.6, exposure: 1.0 },
clouds: { style: "volumetric", coverage: 0.8, raymarchSteps: 48 },
mist: { style: "flat", density: 0.2, baseHeightMetres: 0, heightFalloffMetres: 200, riseAboveWater: false }
```

See [`docs/world-design-guide.md`](world-design-guide.md#5-sea-level-atmosphere-and-mood)
for more recipes covering terrain shape alongside sky/weather.
