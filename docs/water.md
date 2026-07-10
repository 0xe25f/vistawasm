# Water

VistaWASM's water is one flat, animated plane (`render/water.rs::build_water_plane`
+ `shaders/water.wgsl`), sized to cover the active terrain's footprint at a
fixed sea level. There is no wave simulation, no rivers, no waterfalls, and
no flow-following geometry — all visual detail (ripples, fresnel blend,
specular highlight) comes from the shader, not from mesh geometry or a
texture.

For the exact field list, see
[`docs/options-reference.md`](options-reference.md#wateroptions).

## Sea level

`WaterOptions.seaLevelMetres` positions the plane in world space. Set it
relative to the *generated* terrain's real height range from
`TerrainMetadata`, not a hardcoded constant:

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

The same hardcoded sea level constant will flood a `verticalScale: 2`
mountain range and look wrong on a `verticalScale: 0.3` plain, since
`TerrainMetadata`'s height range scales with `verticalScale`. This also
matters for [`FractalTerrainOptions.seaLevelMetres`](options-reference.md#fractalterrainoptions),
a *separate* field that seeds the initial `TerrainMetadata.seaLevelMetres`
(used by flora/grass placement's underwater check) — the two are
independent, and it is normal to set both to the same value.

## Visual controls

- `waveScale` — procedural ripple amplitude. `0` gives a flat, mirror-like
  plane; higher values give choppier water and a more disturbed normal for
  the fresnel/specular terms.
- `reflectivity` — how strongly the water blends towards the sky colour at
  grazing angles (a fresnel term), versus showing its own deep/shallow
  colour gradient.
- `shorelineSoftnessMetres` — blend distance near the shoreline. There is
  no separate foam/wave-breaking effect; this only softens the colour
  transition.

Ripple animation reads the same per-frame clock shared with wind sway on
flora/grass and cloud/mist drift (`water_params.z`, an internal
frame-counter-based clock, not wall-clock time — see
[`docs/architecture.md`](architecture.md#rendering)), so all of VistaWASM's
animated systems stay in phase with each other.

## Interaction with mist

When `MistOptions.riseAboveWater` is enabled, extra ground mist appears
near `WaterOptions.seaLevelMetres` regardless of the mist's own
`baseHeightMetres` — see
[`docs/sky-atmosphere-and-weather.md`](sky-atmosphere-and-weather.md#mist-and-ground-fog)
for the full mist system. This only takes effect while `WaterOptions.enabled`
is `true`.

## What water does not do

- No buoyancy, swimming, or gameplay interaction of any kind — VistaWASM
  has no physics. If your game needs to know whether a position is
  underwater, compare your own gameplay height query (see
  [`docs/game-development.md`](game-development.md#querying-terrain-height-for-gameplay))
  against `WaterOptions.seaLevelMetres` yourself.
- No rivers or flowing water — only one flat plane per terrain, at one sea
  level.
- No reflections of scene geometry (trees, terrain relief) — the "sky
  reflection" is the analytic sky-dome colour blended in via the fresnel
  term, not a real reflection pass.
- Disabling water (`enabled: false`) removes the plane entirely — it is not
  drawn transparent-and-invisible, it is simply not uploaded to the GPU
  that frame (see `EngineCore::refresh_water` in
  [`docs/architecture.md`](architecture.md#rendering)).
