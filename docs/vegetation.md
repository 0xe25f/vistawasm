# Vegetation: Trees and Grass

VistaWASM places vegetation procedurally — there are no image textures, no
imported tree assets, and no manual placement API. Trees and grass are two
independent systems (`FloraOptions` and `GrassOptions`) that share the same
placement approach but are enabled, styled, and tuned separately.

For the exact field list, see
[`docs/options-reference.md`](options-reference.md#floraoptions) and
[`docs/options-reference.md`](options-reference.md#grassoptions).

## Trees (`FloraOptions`)

### Placement

`render/flora.rs::build_flora_instances` scatters candidate positions across
a grid over the terrain (a coarser stride on very large terrain, so
placement cost does not grow unbounded with terrain size), then rejects a
candidate if any of the following are true:

- It is at or below `WaterOptions`' effective water line.
- Its elevation is above `treeLineMetres`.
- The local slope (measured from neighbouring heightmap samples) is too
  steep for a tree to plausibly stand.

Surviving candidates are then kept or dropped based on `density` (a per-cell
deterministic hash, not a raw percentage of all candidates, so scrubbing
`density` doesn't reshuffle which trees exist — it fades them in and out in
place). Each kept tree gets a deterministic scale and colour-tint variation
from the same seeded hash, so **the same seed and options always produce the
same forest** — this is relied on by the engine's own tests and is safe to
rely on in your own code (for example, to reproduce a specific look from a
saved seed).

`maxInstances` is a hard cap applied after placement (evenly thinning the
candidate list if it's exceeded), and is also clamped to the active
device's practical limits — a caller cannot request an unbounded instance
count.

### Tree quality

`treeQuality` controls *rendering fidelity only* — it never changes where
trees are placed, only how each one is drawn:

- **`"billboard"`** (default) — a single quad that always rotates to face
  the camera around the vertical axis (a "cylindrical" billboard). Cheapest
  option, and the original VistaWASM tree rendering. From any single
  viewing angle it looks like a normal tree silhouette; because it's always
  camera-facing, it never shows a "flat cardboard" edge-on profile, but a
  whole forest of them can look repetitive since every tree uses the same
  base silhouette.
- **`"cross-quad"`** — two *static*, world-oriented quads per tree, 90
  degrees apart, each with a random per-tree rotation offset (so a whole
  forest is not axis-aligned identically). Unlike the billboard, these
  quads do **not** rotate to face the camera — as you move around a tree,
  you genuinely see it from a different angle, giving real parallax and
  volume instead of a flat cutout. Costs roughly double the billboard's
  vertex count.
- **`"mesh"`** — accepted by the API, but currently renders identically to
  `"cross-quad"`. A true instanced 3-D tree mesh (real trunk/canopy
  geometry with lighting) is tracked as future work — see
  `docs/environment-upgrade-plan.md` ("Trees (Mesh tier)", deliberately the
  last phase of that plan). This is a deliberate, documented simplification,
  not a bug: shipping a genuine mesh tier needs its own LOD-fade design so
  it does not become a triangle-count problem on densely forested terrain,
  and that work was intentionally deferred rather than rushed.

`speciesVariation` (0 to 1) perturbs each tree's canopy silhouette (width,
height, and shape) using a per-tree deterministic hash, so a forest does not
look like one canopy shape copy-pasted everywhere. It has no effect on
`"billboard"`'s trunk shape, only the canopy mask.

`windStrength` (0 to 1) sways the canopy — never the trunk — in time with
the same animation clock that drives the water plane's ripples, so wind and
water motion read as part of the same weather rather than two unrelated
animations.

## Grass (`GrassOptions`)

Grass is a separate, independent ground-cover layer, not a variant of
flora. Unlike every other environmental option, it defaults fully
`enabled: false` — it is a genuinely new visual element with no prior
equivalent, so existing scenes are completely unaffected until you opt in.

### Placement

`render/grass.rs::build_grass_instances` reuses the terrain's own baked
material weights (`MaterialWeights.grass`, the same data that tints the
terrain surface itself green/rock/snow/mud — see
[`docs/terrain-data.md`](terrain-data.md)) as the *acceptance probability*
for a grass candidate, instead of recomputing its own slope/height
thresholds. This has two effects worth knowing:

- Grass naturally avoids rock, snow, wet mud, and underwater terrain,
  because that is exactly what the cached `grass` material weight already
  encodes — no separate tuning needed.
- Grass placement is influenced by whatever affects the terrain's material
  masks: slope, height relative to the snow line, and height relative to
  sea level. If you change `AtmosphereOptions`/terrain shape in a way that
  shifts where the terrain paints itself as "grassy", grass placement shifts
  to match.

Like flora, placement is fully deterministic for a given seed and grid
candidate, and `maxInstances` is clamped the same way.

### Grass style

- **`"billboard-blades"`** (default once enabled) — each tuft is three
  crossed, world-oriented quads 60 degrees apart (never camera-facing, for
  the same "real parallax up close" reason as tree `"cross-quad"`), with a
  per-tuft random rotation.
- **`"dense-blades"`** — the same geometry, intended to be paired with a
  higher `density` and a *shorter* `viewDistanceMetres` than the default.
  Grass is small on screen at range, so hyper-realistic density is best
  spent close to the camera where it is actually visible, rather than
  wasted on distant blades that would alias into noise anyway.

### View-distance fade

Grass fades out smoothly (alpha blend) over the last ~20% of
`viewDistanceMetres` rather than popping, and — unlike trees — is not
expected to render all the way to the terrain's horizon. If grass looks
like it "runs out" in a visible ring around the camera, that is
`viewDistanceMetres` doing its job, not a bug; raise it if you want a
larger radius (at a real performance cost — grass instance count and
`viewDistanceMetres` both drive fill-rate directly).

## Wind, sway, and the shared animation clock

Both tree canopy sway and grass blade sway read the same per-frame clock
that also drives the water plane's ripple animation
(`water_params.z`, an internal frame-counter-based clock — see
[`docs/architecture.md`](architecture.md#rendering)). This is a deliberate
choice: wind, grass, and water motion always stay in phase with each other
rather than each system inventing its own independent timer, which would
otherwise look like three unrelated animations layered on top of each
other. There is no separate "wind direction/gust" option today — sway
direction is derived per-instance from a deterministic hash of each
plant's own position, not a single global wind vector.

## Performance

- `RenderQualityOptions.floraDensityScale` is a single global multiplier
  applied to **both** `FloraOptions.density` and `GrassOptions.density` — it
  is the cheapest lever if vegetation fill-rate is your bottleneck (see
  [`docs/render-quality-and-diagnostics.md`](render-quality-and-diagnostics.md)).
- `RenderStats.floraInstances` and `RenderStats.grassInstances` (from the
  `"stats"` event) tell you exactly how many instances are actually being
  drawn each frame — use these to build an adaptive quality system rather
  than guessing from `density` alone, since the real count also depends on
  terrain size, `maxInstances`, and (for grass) how much of the terrain the
  material masks classify as grassy.
- `"cross-quad"` trees and grass both cost roughly double a plain billboard's
  vertex count per instance; if you need every last bit of headroom on a
  huge, densely forested terrain, `treeQuality: "billboard"` with
  `GrassOptions.enabled: false` is the cheapest possible vegetation
  configuration.
