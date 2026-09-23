# Vegetation: Trees and Grass

VistaWASM grows vegetation procedurally. No model files or image textures
ship with the library: tree species are modelled from code and their bark
and leaf textures are generated on the GPU when the engine starts. Trees
and grass are two independent systems (`FloraOptions` and `GrassOptions`),
and both follow the biome map (see [`docs/biomes.md`](biomes.md)).

For the exact field list, see
[`docs/options-reference.md`](options-reference.md#floraoptions) and
[`docs/options-reference.md`](options-reference.md#grassoptions).

## Trees (`FloraOptions`)

### Species

Eight species are modelled in `render/tree_models.rs`:

| Species | Look | Grows in |
| --- | --- | --- |
| Oak | Short, flared trunk splitting into spreading limbs; broad, rounded crown of leaf clusters. | Meadows, thickets, temperate forests |
| Pine | Tall, bare, reddish trunk; whorls of up-curving branches with needle sprays in the upper half. | Forests, foothills, coasts, volcanic slopes |
| Spruce | Dense, narrow cone of drooping needle branches from near the ground. | Cold forests, mountains |
| Palm | Leaning, ringed trunk with an S-curve; arching, V-folded fronds and coconuts. | Warm beaches, jungle edges |
| Jungle | 30 m emergent with plank buttress roots, an umbrella crown of large glossy leaves, and hanging lianas. | Jungles |
| Cypress | Strongly flared, fluted base with "knees"; feathery foliage and draped Spanish moss. | Swamps |
| Acacia | Short forked trunk and a flat-topped umbrella canopy. | Savannah |
| Shrub | Low clump of leafy stems. | Thickets, understorey, most biomes |

Each species is built once from a fixed seed, so every run produces exactly
the same meshes. Shipping the generator costs a few kilobytes of code rather
than megabytes of model data, and there is no per-frame cost: meshes are
uploaded once and drawn instanced. Per-tree variety (size, rotation, colour,
and climate yellowing) comes from instance data.

Trunks and branches are tapered tubes that follow curved growth paths with
a twist-free frame, so bark texture never shears. Foliage is made of
alpha-tested cards (leaf clusters, needle sprays, fronds, moss) whose
normals point away from the crown centre, which gives canopies soft,
rounded lighting. Foliage uses wrap lighting and glows when back-lit by the
sun.

### Placement

`render/flora.rs::build_tree_instances` scatters candidates on a grid (a
coarser stride on very large terrain, so the cost does not grow unbounded)
and rejects any that are underwater, above `treeLineMetres`, on a river or
lake bed, or too steep. Each surviving candidate is kept with a probability
of `density` times the biome's tree cover, so inner forests and jungles are
dense while meadows and savannah stay open. Forests thin out over the last
150 m below the tree line rather than stopping at a hard edge. The biome
then picks the species, with cold climates switching temperate forests to
conifers.

Placement is fully deterministic: the same seed, terrain, and options
always produce the same forest. `maxInstances` is a hard cap applied after
placement.

### Tree quality

`treeQuality` changes how trees are drawn, never where they are:

- **`"mesh"`** (default) — full 3D meshes near the camera. Beyond
  `meshDistanceMetres` each tree switches to an impostor: a picture of the
  very same mesh, rendered once at start-up. The two cross-fade with a
  screen-space dither, so the switch has no visible pop and never leaves
  holes.
- **`"cross-quad"`** — impostors only, as two crossed quads per tree.
- **`"billboard"`** — impostors only, as one camera-facing quad. Cheapest.

Culling and level-of-detail selection run on the GPU every frame
(`shaders/tree_cull.wgsl`): each tree is frustum-tested, trees smaller than
about a pixel are dropped, and the rest are appended to per-species mesh
or impostor lists that feed indirect draws. The CPU never touches tree
instances after upload.

`speciesVariation` (0 to 1) controls size and colour variety between trees.
`windStrength` (0 to 1) drives the wind: a slow whole-tree lean, faster
branch sway, and independent leaf flutter, all modulated by gusts that
travel across the landscape so a forest never moves in lockstep.

## Grass (`GrassOptions`)

Grass is an independent ground-cover layer and defaults to
`enabled: false`.

### Placement

`render/grass.rs::build_grass_instances` uses the baked surface materials
as the acceptance probability: grass grows on lush and dry grass ground,
sparsely on forest floor, and never on rock, snow, sand, mud, or under
water. Each tuft takes its colour from the local climate, from lush green
in wet regions to tall, straw-coloured grass on the savannah.

### Grass style

- **`"billboard-blades"`** (default once enabled) — each tuft is three
  crossed, world-oriented quads 60 degrees apart, each cut into several
  tapered blades of different heights.
- **`"dense-blades"`** — the same geometry, intended for a higher `density`
  with a shorter `viewDistanceMetres`.

### View-distance fade

Blades are alpha-tested and write depth, so they sort correctly against
trees and each other. Tufts thin out with a screen-space dither over the
last 30% of `viewDistanceMetres` instead of popping.

## Wind and the shared clock

Trees, grass, water, clouds, and mist all animate from one real-time clock
in the frame uniforms, so wind in the trees, drifting clouds, and moving
water stay in step.

## Performance

- `RenderQualityOptions.floraDensityScale` multiplies both
  `FloraOptions.density` and `GrassOptions.density`; it is the cheapest
  lever for vegetation-heavy scenes.
- Lowering `meshDistanceMetres` moves more trees to impostors. Around
  150–250 m suits integrated GPUs; 400–600 m suits desktop GPUs.
- `treeQuality: "billboard"` with `GrassOptions.enabled: false` is the
  cheapest vegetation configuration.
- `RenderStats.floraInstances` and `RenderStats.grassInstances` report how
  many instances exist; use them to drive adaptive quality.
