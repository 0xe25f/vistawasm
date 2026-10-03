# Vegetation: Trees and Grass

VistaWASM grows vegetation procedurally. No model files or image textures
ship with the library: trees are grown from code and their bark and leaf
textures are generated on the GPU when the engine starts. Trees
and grass are two independent systems (`FloraOptions` and `GrassOptions`),
and both follow the biome map (see [`docs/biomes.md`](biomes.md)). To use
your own tree models, species mixes, placement, or textures, see
[`docs/hooks.md`](hooks.md).

For the exact field list, see
[`docs/options-reference.md`](options-reference.md#floraoptions) and
[`docs/options-reference.md`](options-reference.md#grassoptions).

## Trees (`FloraOptions`)

### Species

Eight species are grown in `render/tree_growth.rs`:

| Species | Look | Grows in |
| --- | --- | --- |
| Oak | Short trunk forking low into heavy, crooked limbs; a broad, flattened crown of lobed leaves with wide sky gaps. | Meadows, thickets, temperate forests |
| Pine | Tall, self-pruned reddish trunk; an irregular, flat-topped crown of needle bundles in the top third. | Forests, foothills, coasts, volcanic slopes |
| Spruce | A narrow cone of whorled branches, drooping more lower down, clothed in flat needle sprays. | Cold forests, mountains |
| Palm | A curved, ringed trunk with 12 to 18 arching fronds of pinnate leaflets. | Warm beaches, jungle edges |
| Jungle | A 30 m emergent: a tall, straight trunk on plank buttresses under an umbrella crown of large glossy leaves. | Jungles |
| Cypress | A narrow column of scale-leaf sprays over a flared base, with knees rising round it on wet ground. | Swamps |
| Acacia | A short trunk forking at shallow angles into a wide, flat top of tiny bipinnate leaflets. | Savannah |
| Shrub | Many stems in a dense dome of small serrated leaves; in tundra and above the trees, a low, wide, prostrate dwarf. | Thickets, understorey, most biomes |

Trees are grown in WASM from fixed seeds, so every run produces exactly
the same trees. Shipping the generator costs a few
kilobytes of code rather than megabytes of model data, and there is no
per-frame cost: meshes are uploaded once and drawn instanced.

### Growth

Trees grow by space colonisation (Runions, Lane and Prusinkiewicz,
2007). Attraction points are scattered through the species' crown
envelope (a flattened ellipsoid, a cone, an umbrella, a column or a
dome), often in a few lobes, so crowns are irregular. Each point pulls
the nearest branch node within reach; nodes grow a step towards the
points pulling them, bent by the species' tropism (up for most, out and
down for a spruce's lower branches); points a branch reaches are used
up. Branches fill the crown without crossing, fork where points pull a
node two ways, and leave sky gaps between the lobes. Palms are built,
not grown.

- **Radii** follow the pipe model: at every fork `r_parent^2.5` is the
  sum of the children's `r^2.5`, and along a branch each node adds the
  pipes of the foliage it carries, so limbs taper to their twigs. The
  trunk at breast height takes the species' radius from one table in
  `render/flora.rs`, which also sets how wide its roots reach when it is
  grounded.
- **Root flare.** Below 0.6 m the trunk widens to 1.6 times its radius,
  and the mesh reaches 0.4 m below the ground, so a grounded tree never
  shows a gap on a slope. Rainforest emergents stand on five plank
  buttresses.
- **Branches** are generalised cylinders along the node paths, with 6
  to 10 sides by radius and parallel-transport frames (no twisting).
  Each child starts inside its parent, so joints close, and bark is
  mapped with whole repeats around and square texels along.
- **Leaves** are clump cards, 1 to 3 at each node near a twig tip (along
  the whole branch on conifers), 0.4 to 1.2 m by species, facing roughly
  outwards and randomly rolled, with needle clumps as two crossed quads.
  Their normals bend 65 % towards the direction from the crown centre, so
  a crown shades as a volume while its clumps still read. Each clump's
  colour varies a little.
- **Leaf textures**, generated on the GPU, hold clusters of 5 to 30
  leaves with real silhouettes per species (lobed oak, serrated
  broadleaf, pine needle bundles, spruce needle sprays, palm leaflets,
  glossy tropical leaves, cypress scale sprays, acacia leaflets), each
  leaf its own colour, with normal maps that give each leaf its midrib.
  Their mips keep the alpha-tested coverage of the full-size texture, so
  distant trees do not thin into skeletons.
- **Bark** is realistic in albedo: most bark 0.15 to 0.35, smooth
  tropical bark pale grey, with moss and lichen on the shaded side in
  wet climates.

### Variants, ages and lean

Each species grows `FloraOptions.variantsPerSpecies` variants (1 to 4,
default 4), each in four age classes:

- **young**: a narrower, lower crown with fewer branches (and, for
  shrubs, the dwarf prostrate form of tundra and alpine ground);
- **mature**;
- **old**: broader and more open, with heavier, drooping limbs;
- **krummholz**: stunted trees near the tree line and on exposed
  ridges grow with their windward branches dead and their crown streaming
  leeward, turned into the prevailing wind.

The first variant of every species grows before the first frame (about
100 ms in the browser); the rest grow over the frames after it, one mesh
a frame, and join together. The generators pick each tree's variant,
age class and lean from its lattice hash: seedlings and the understorey
are young, and trees by water are young or mature. Trees lean up to 6
degrees: towards the water within 8 m of a stream, otherwise downhill
and with the prevailing wind, more on steeper ground; on steep slopes
the lowest fifth of the trunk bends downhill and grows back upright, as
trees on creeping soil do. The lean pivots at the base, so roots stay
grounded. Size, rotation, tint (hue by about 4 %, saturation 10 % and
brightness 8 %) and climate yellowing still come from each tree's own
data.

### Levels of detail

- **Full meshes** within 50 m (or a third of the mesh distance).
- **Light meshes** to 150 m: twigs and branches of the third order and
  above are dropped, tubes have half the sides, and half the leaf cards
  stand in for all at 1.4 times the size (needle clumps as one quad,
  not two crossed), at under 45 % of the triangles.
- **Impostors** beyond 150 m, or `meshDistanceMetres` when nearer: each
  variant is baked into nine views (eight around and one from 40 degrees
  above) with colour and normals. Each tree draws whichever of the two
  views nearest the camera's direction its own random threshold picks,
  so as the camera circles, a forest turns over tree by tree instead of
  every crown hatching with a per-pixel blend.

Across the last tenth of each band a tree is drawn both ways with
complementary dithers, so nothing pops. The triangle budget
(`maxTreeTriangles`) counts each level's own triangles, so richer near
meshes cost fewer meshes, not a slower frame.

### Density

`FloraOptions.density` runs from `0` to `4`. `1` is the old maximum. `4`
closes the canopy wherever the land supports a forest.

| `density` | Trees per hectare, where the land is fully suited |
| --- | --- |
| `0` | none |
| `0.35` (default) | 25 |
| `1` | 70 |
| `2` | 220 |
| `3` | 480 |
| `4` | 900 |

Above `1`, the extra trees are understorey: saplings a quarter to half
full size beneath the canopy trees, which stay about as many as at `1`.
A dense jungle is a closed canopy over young trees, not a wall of giant
trunks. Thinning drops the understorey first, so distant views keep the
canopy.

The table is interpolated linearly between rows. Forest cover, the grove
noise and each species' suitability then scale it down, so an open
meadow at `4` is still open. The default island has about 15,600 trees at
`0.35` and 43,500 at `1`, within 10 % of the counts before the lattice.

### Placement

Trees grow only where their species would grow. Placement has two
halves.

1. **The cover texture.** Once per terrain, the CPU bakes a cover texture
    (`render/flora.rs::bake_cover`), one texel per heightmap sample at
    most 2048 × 2048. Red holds the forest share (square-root coded, up
    to 4 times the table density, so dense stands saturate late); green
    and blue hold the two likeliest species; alpha holds the second
    species' share and a stunted flag. The share is
    `forest cover x rule density x grove x suitability`, plus seedlings
    round shade-tolerant parents. Groves are a 180 m noise from 0.35 to
    1, so forests gather in stands with clearings between them.
2. **The lattice.** Candidate trees sit on one world lattice with a 3 m
    pitch, each jittered by up to 0.45 of a cell by a `pcg3d` hash of its
    cell, so trees never line up. A candidate becomes a tree when its
    hash rank falls below `share x density x clump`. The clump factor is
    a 12 m noise to the fourth power (1 on average): young trees gather
    round their parents, with thinner ground between. About one tree in
    nine is young, at 0.45 to 0.8 of full size.

The same hash runs in Rust and in WGSL (`shaders/lattice.wgsl`), in
integer and exactly rounded arithmetic, so the CPU and the GPU place
every tree at the same spot. A Rust port of the WGSL arithmetic is
tested against the Rust hash on 10,000 points.

The biome's species mix proposes the species; each one's suitability
weights the choice. Suitability is a product of four responses, each
from 0 to 1:

- **Temperature:** a triangle between the species' least, best and most
  mean annual temperature.
- **Moisture:** effective moisture rises to the species' best, stays
  there until halfway to its most, then falls as the ground turns
  waterlogged. Effective moisture is the surface moisture (which already
  holds the extra water beside rivers and lakes), plus up to 0.25 where
  water gathers from a large drainage area (from the river network, or
  a D8 flow accumulation over the heightmap when rivers are off), plus
  up to 0.12 on slopes
  facing away from the sun, plus `0.35 x water affinity x riparian`.
- **Slope:** 1 up to 75 % of the species' steepest slope, then falling to
  0 at it, measured on the fine per-sample normal. Above 30 degrees a
  shrub understorey joins every wooded biome, so steep slopes stay
  green without trees on rock or cliffs.
- **Exposure:** ridges standing above their 300 m neighbourhood lose
  trees, less so for species that tolerate wind.

| Species | °C (least, best, most) | Moisture (least, best, most) | Steepest slope | Water affinity | Shade tolerance | Exposure tolerance |
| --- | --- | --- | --- | --- | --- | --- |
| Oak | -2, 11, 28 | 0.3, 0.8, 1.7 | 38° | 0.2 | 0.5 | 0.4 |
| Pine | -4, 8, 30 | 0.15, 0.5, 1.5 | 45° | -0.2 | 0.2 | 0.7 |
| Spruce | -8, 2, 20 | 0.25, 0.85, 1.8 | 45° | 0.2 | 0.8 | 0.6 |
| Palm | 8, 24, 40 | 0.2, 0.75, 1.8 | 25° | 0.3 | 0.3 | 0.5 |
| Jungle | 8, 24, 40 | 0.4, 1.0, 2.0 | 40° | 0.4 | 0.7 | 0.2 |
| Cypress | 2, 16, 34 | 0.5, 1.1, 2.2 | 15° | 1.0 | 0.5 | 0.3 |
| Acacia | 8, 24, 40 | 0.0, 0.3, 0.9 | 30° | -0.6 | 0.1 | 0.6 |
| Shrub | -4, 10, 40 | 0.1, 0.5, 1.6 | 50° | 0.1 | 0.6 | 0.9 |

Shaded slopes therefore hold more trees, and shade-tolerant species
become likelier on them. Which side is shaded follows the sun: slopes
facing away from `SunOptions.azimuthDegrees` (north by default). Cypress
crowds riverbanks, while acacia keeps back from them. Where suitability
falls below 0.3, at forest edges and on poor ground, trees grow smaller
and shrubs become likelier. In savannah and grassy meadows, forest cover
rises by up to `1.5 x riparian` beside rivers, so gallery woods line
them.

Nothing grows:

- in water, or within a channel's half width + 1.5 m of any drawn river
  or stream centreline (including streams narrower than a sample, and
  their loops), or within half width + 1 m of a plunge pool;
- on gravel, on the river's gravel bars, or below the bank top;
- on sand (except palms), bare rock, lasting snow, glacier or volcanic
  heat;
- on the snowy peaks, or on the skirt beyond the terrain.

Trees thin to none over the last 200 m below `treeLineMetres`. Over the
last 150 m, and on exposed ridges, they are stunted: up to half size, and
flagged for wind-shaped crowns. In the alpine band below the snow line
only dwarf shrubs and stunted pines and spruces grow, at 15 % of the
normal density, thinning to none at the band's top. Whichever of the tree
line and the band is lower wins, so no full-size tree stands in the band.

Placement is fully deterministic: the same seed, terrain, and options
always produce the same forest. `maxInstances` caps the trees generated:
the far set first, then the streamed tiles share what is left. Over the
cap, the far trees with the lowest hash ranks are kept. The ranks are
spread evenly over the map, so the cap thins the whole forest rather than
dropping its last rows.

### Near and far

Up to 50,000 trees (the default density, and `1`, on a typical island),
the CPU places every tree once per terrain and every one is drawn at its
own size, at every distance. A closed canopy over a whole island is
millions of trees, too many to keep or draw. So past 50,000 each tree's
hash rank splits the lattice in two.

- **The far set.** Ranks below `50,000 / trees` of each point's
  probability, but never under 12 %. The CPU builds it once per terrain
  and keeps it for the whole map, so distant hills always hold trees.
- **Streamed tiles.** The rest. A compute pass
  (`shaders/tree_generate.wgsl`) fills 64 m tiles round the camera out to
  where only the far set is left, nearest first, at most 8 tiles a frame.
  Tiles are generated once and kept while the camera stays near.

The two halves never place the same point twice. Within
`vegetationDetailMetres` every tree is drawn. Further out, the culling
pass keeps a share `(R / d)²` of them, by rank, and fades the last 5 %
in by size, so trees never pop. The trees kept grow to fill the gaps:
up to 1.8 times as tall, and wider still, so the crown area per hectare
stays within 7 % of the full forest at every distance.

When trees are streamed, between 0.8 and 1 times `canopyDistanceMetres`
they give way one by one to the **canopy layer**: a shell drawn in the
terrain pass, lifted by the local tree height, and coloured from the
species in the cover texture. Its crowns hide the share of ground a
forest hides from that view: `1 - exp(-crowns x (1 + side / top x
tan(view angle)))`, with crowns the crown area per square metre of
ground. Sparse woods read as sparse and a jungle as solid, and seen at a
slant, as over a ridge, the crowns close up as real forest does. The
shell fades in crown by crown over the same band, so there is no ring
with neither trees nor canopy. Both appear in screen reflections.

### Forest floor

Under trees the ground turns to forest floor: leaf litter and needles,
moss where it is damp, and dappled light beyond the shadow distance. The
floor follows the canopy, not the biome: its weight is capped at the
canopy cover above it, so bare slopes in a forest biome stay grassy and
the floor stops at the forest edge. Mist gathers a little more (up to
20 %) under closed canopy.

With `GrassOptions.forestFloor` on (the default), ferns and low
undergrowth replace grass tufts under dense canopy, where grass would not
get the light.

### Grounding

Every tree stands on the ground as it is drawn, at every distance.

- On the CPU, each tree's height is the terrain mesh at full detail at
  its jittered position.
- The terrain mesh is coarser away from the camera. Each time the mesh
  recentres, a small GPU compute pass (`shaders/grounding.wgsl`) moves
  every tree to the mesh as drawn, in whichever level of detail holds
  it. It costs nothing on other frames.
- On a slope, a tree sinks to the lowest of the ground at its trunk and
  four root points 2.5 trunk radii out, less 5 % of that radius. The
  downhill side of the trunk then meets the ground and the uphill side
  is buried slightly, as real root flares are. Trees stay upright.

Streamed trees and tufts are grounded by the generators when they fill a
tile (`shaders/ground.wgsl`, the same code the terrain uses), and live
tiles are re-grounded when the mesh recentres. Nothing is grounded every
frame. Grass tufts skip the root points. Hand-placed trees are grounded
when you pass `ground: true` (see [`docs/hooks.md`](hooks.md)).

## Grass (`GrassOptions`)

Grass is an independent ground-cover layer. It is on by default, at
`density: 0.5`: a natural meadow near the camera costs about 0.7 ms a
frame at 1080p on a mid-range GPU, and the default scene's whole frame
stays near 3.3 ms. Pass `enabled: false` to turn it off.

### Density and placement

`GrassOptions.density` runs from `0` to `4`. It says how a meadow looks
near the camera, not a count:

| `density` | Meaning |
| --- | --- |
| `0` | No grass. |
| `0.5` (default) | A natural, dense meadow: tufts on 45 % of the points of a 0.35 m lattice, covering at least 70 % of the ground within the full-cover radius. |
| `1` | A lush, taller meadow: 65 % of the points, 15 % taller. |
| `4` | The maximum: long, dense grass on every point, 40 % taller. |

Values between interpolate. This replaces the old counts, where `0.5`
and `1` gave a few hundred scattered tufts on a meadow: realism wins over
matching the old numbers.

```ts
engine.setGrass({
  enabled: true,
  style: "billboard-blades",
  density: 0.5,
  viewDistanceMetres: 220,
  seedOffset: 7331,
  maxInstances: 1000000
});
```

Tufts are generated on the GPU (`shaders/grass_generate.wgsl`) in 16 m
tiles round the camera, from the same hash as trees. Within
`RenderQualityOptions.grassDetailMetres` (25, 45, 70 or 120 m by preset)
the meadow is at full density. Further out, tufts thin by rank, and the
ones kept widen and grow so the cover stays the same. `maxInstances`
bounds the slots the tiles are held in: each slot is sized for what
thinning keeps at its distance, and if they would not fit, the
full-density radius comes in instead (to 10 m at least), with the widened
tufts beyond it keeping the cover. So with the default options the
meadow is fully covered (at least 70 % of the ground at density 0.5) to
at least 60 m at density 0.5, and 40 m at density 4, within the grass
budgets.

From 1.5 times the full-density radius (at least 0.3 and at most 0.9
times `viewDistanceMetres`: 66 m at the default 220 m) to
`viewDistanceMetres`, tufts hand over to the ground itself. They drop
out evenly by rank, and grass-covered terrain takes on their look by
exactly the share they give up, so tufts and ground together always look
like the full meadow: there is no ring where the tufts end, from any
camera height. How much ground the tufts hide depends on the view: from
above only their cover, at a slant their sides as well, so seen from
above the ground takes a larger share sooner. The ground takes the
tufts' mean colour and light (at a slant mostly the lighter upper parts
of the blades), the glow of blades lit from behind when looking into the
sun, and a fine fuzz of blade shadows. There, blades are finer than a
pixel, so the ground shows the meadow for a fraction of the cost, and it
still reads as grass at 200 m without tufts.

Tiles are handed out only once the generator can fill them: its pipeline
is built in the background, so for the first frames nothing is handed
out, rather than the tiles round the camera being recorded as filled and
left empty.

The baked surface materials are the acceptance probability: grass grows
on lush and dry grass ground and tundra, and never on rock, snow, sand,
mud, glacier, gravel bars, in channels or under water. It is read
between texels, so a meadow shades smoothly across material borders.
Grass thins on steep ground: all of it up to 30 degrees, none at 50, and
a smooth fade between, tuft by tuft, so steep meadows show no bands
along the contours. Each tuft takes its colour from the local climate,
from lush green in wet regions to tall, straw-coloured grass on the
savannah. Reeds along rivers and lakes are still placed on the CPU
(`render/grass.rs::build_reed_instances`). Tufts stand on the drawn
terrain, as trees do (see [Grounding](#grounding)).

### Grass style

- **`"billboard-blades"`** (default once enabled) — within 15 m each tuft
  is two crossed, world-oriented quads 90 degrees apart, each cut into
  seven tapered blades of different heights. Beyond 15 m, where parallax
  is slight, a tuft is one card turned to the camera, which halves its
  vertices. Reeds keep three quads.
- **`"dense-blades"`** — the same geometry, intended for a higher `density`
  with a shorter `viewDistanceMetres`.

Grass is lit per vertex: sun, shadows, sky light and the glow of
back-lit blades. A blade is far smaller than the shadow map's texels, and
a meadow is many layers deep on screen, so per-pixel light would cost
many times as much for no visible gain.

### View-distance fade

Blades are alpha-tested and write depth, so they sort correctly against
trees and each other. By `viewDistanceMetres` every tuft has handed over
to the ground, and reeds thin out with a screen-space dither over the
last 30 % of it instead of popping.

## Painted density masks

`engine.setVegetationMasks({ trees, grass })` scales where trees and
grass grow with your own masks: 0 is none, 128 unchanged and 255 twice
as dense, never denser than density 4. A tree mask paints clearings and
thickets that show at every distance; a grass mask thins or thickens
the tufts and the ground's distant grass sheen alike. Trees keep their
physical limits: a mask of 255 on a cliff still grows nothing there.
See [Importing maps](import.md#vegetation-density-masks).

## Boulders

Boulders and talus lie below rock outcrops (`SurfaceOptions.boulders`,
on by default; see [Rock and soil](terrain-data.md#rock-and-soil) for
where the soil model puts them). They stream like grass:

- **Placement.** Every potential boulder is a point on its own lattice,
  2 m apart, with its own hash salt, in 32 m tiles within
  `boulderDistanceMetres` (300 m by default). A point holds a boulder
  where its rank is below the talus field's probability: 0.35 of the
  scree there, and half as much again at an outcrop's foot. Sizes follow
  a power law from 0.3 to 3 m (the median is about half a metre), larger
  the further down the cone. None lies in water, within a drawn channel's
  half width plus its clearance (at least 1 m, even beside streams
  narrower than a sample), on sand, snow, glacier or a river bed, on the
  crag itself, or off the map.
- **Meshes.** Six fractured blocks built at start-up: an icosphere of
  162 vertices, displaced by a noise with sharpened peaks, squashed, and
  cut flat on one or two sides. Each has three levels of detail (320, 80
  and 20 triangles) sharing its vertices, chosen by how large it looks.
  Their stone is the rock texture projected on the world planes, with
  the terrain's joints, crevice shade and lichen on the upper faces.
- **Grounding.** Each boulder is stood on the drawn terrain mesh as it
  is generated, sunk by 25 to 40 % of its height and leaning with the
  slope, and grounded again only when the mesh recentres.
- **Drawing.** A cull pass keeps those in view, at most 20,000, in one
  indirect draw per variant and level. Within 150 m they cast into the
  tree shadow map. From 0.8 of the boulder distance they drop out by
  rank, shrinking to nothing, so none pops; the scree texture carries the
  look on. Detail pressure and the 20,000 budget bring the distance in,
  as they do the vegetation's radii.
- **Room.** Trees keep their trunks half a metre clear of a boulder's
  footprint, and grass does not grow under one. On the soil side of a
  rock outcrop's lip turf grows up to 2.2 times as dense, so it overhangs
  the rock rather than fading into it.

Boulder pipelines are created only once boulders are on and tiles with
boulders are in view, after the first frame, and the boulder pass is
timed as `gpuPassTimesMs.boulders`.

## Wind and the shared clock

Trees, grass, water, clouds, and mist all animate from one real-time clock
in the frame uniforms, so wind in the trees, drifting clouds, and moving
water stay in step.

Trees move in three layers, from data every vertex carries:

- **The trunk bends** from its base, by the wind squared times its
  height squared, at 0.2 to 0.5 Hz, with gusts rolling downwind across
  the forest.
- **Limbs bob** about where they leave the trunk, up and down and a
  little downwind, freest at their twigs, at 0.8 to 1.5 Hz, each on its
  own phase.
- **Leaves flutter** about their twigs at 3 to 6 Hz: broadleaves most,
  needles least.

Nothing moves at the ground, and nothing moves further than 8 % of the
tree's height. Impostors and tree shadows bend with the trunk. The wind
is `FloraOptions.windStrength` and the weather's direction, or, with the
weather on, its wind. Custom models get wind data derived for them (see
[`docs/hooks.md`](hooks.md#wind-on-custom-models)).

## Budgets

`RenderQualityOptions` holds six vegetation budgets. `preset` fills in
any you leave unset.

| Field | `"preview"` | `"balanced"` | `"high"` | `"offline"` |
| --- | --- | --- | --- | --- |
| `vegetationDetailMetres` | 120 | 250 | 400 | 600 |
| `canopyDistanceMetres` | 1500 | 2500 | 4000 | 8000 |
| `maxTreeInstances` | 40,000 | 120,000 | 250,000 | 1,000,000 |
| `maxGrassInstances` | 150,000 | 400,000 | 800,000 | 2,000,000 |
| `maxTreeTriangles` | 1,000,000 | 2,500,000 | 5,000,000 | no limit |
| `grassDetailMetres` | 25 | 45 | 70 | 120 |

Each frame the CPU estimates how many trees and tufts will be drawn, from
the cover texture over the live tiles and the frustum. Over budget, the
near radius shrinks by 10 % a frame, never below 60 m for trees and 10 m
for grass. After 2 s with headroom it grows back one step at a time.

A full tree mesh is up to 10,000 triangles (5,600 for an average mature
tree), a light mesh up to 4,000 and an impostor two, so in a dense
forest the meshes, not the count, bound the trees pass. While trees are
streamed, at most a twentieth of `maxTreeInstances` are drawn as full
meshes (6,000 at `"balanced"`): the mesh distance comes in until that
many stand in view, nearest first. It never reaches past
`meshDistanceMetres`.

**The triangle budget.** The culling pass's draw counts are read back a
frame or two late, never stalling a frame, and multiplied by each
species' triangle count. Over `maxTreeTriangles`, the mesh distance
comes in, so the furthest meshes become impostors first and no tree is
dropped. Shadow casters keep to a quarter of the budget: their reach
from the camera comes in while they fill it, and the caster list holds
no more. `RenderStats.treeTriangles` reports the total.

**Understorey.** Saplings under a closed canopy are half-shaded and seen
through foliage, so beyond 15 m they are impostor cards; nearer they are
meshes. They cast shadows only within 60 m.

**Light.** Leaves take their sunlight, shadows and sky light from their
cards' corners, and bark and impostors their sky light and the sun
through clouds and past hills, so a crown many cards deep on screen is
not lit again for every layer. Trees' own shadows stay per pixel on bark
and impostors.

**Detail before resolution.** With streamed vegetation, dynamic
resolution lowers the render scale only to 0.85 while frames are late.
Then detail pressure rises by 0.25 per window, up to 1: the near radii
shrink by up to half and more trees become impostors. Only then may the
scale fall further, to `minRenderScale`. Recovery runs the other way.
The budgets hold the frame at render scale 1; dynamic resolution is a
safety net for weak devices.

`RenderStats.floraInstances` and `RenderStats.grassInstances` report the
estimated drawn counts.

### A jungle at full density

```ts
engine.setBiomes({ meanTemperatureCelsius: 27, moistureBias: 0.7 });
engine.setFlora({
  enabled: true,
  density: 4,
  treeLineMetres: 1800,
  seedOffset: 3001,
  maxInstances: 1000000,
  treeQuality: "mesh"
});
engine.setGrass({
  enabled: true,
  style: "billboard-blades",
  density: 4,
  viewDistanceMetres: 220,
  seedOffset: 7331,
  maxInstances: 2000000,
  forestFloor: true
});
engine.setRenderQuality({
  preset: "balanced",
  vegetationDetailMetres: 250,
  canopyDistanceMetres: 3000
});
```

## Performance

- The budgets above are the main levers. A smaller
  `vegetationDetailMetres` draws fewer full-density trees and tufts; a
  smaller `canopyDistanceMetres` hands more of the view to the cheap
  canopy layer. Both apply once the forest is streamed (over 50,000
  trees); grass always streams.
- `RenderQualityOptions.floraDensityScale` multiplies both
  `FloraOptions.density` and `GrassOptions.density`.
- Lowering `meshDistanceMetres` below 150 m moves more trees to
  impostors sooner. Around 80–120 m suits integrated GPUs; the default
  suits desktop GPUs.
- `FloraOptions.variantsPerSpecies` below 4 saves about 10 MB of
  impostors and 150,000 mesh vertices per variant dropped, at the cost
  of more alike neighbours.
- `treeQuality: "billboard"` with `GrassOptions.enabled: false` is the
  cheapest vegetation configuration.
- `gpuPassTimesMs.generation` reports the tile generation pass. It is
  near zero once the tiles round the camera are filled.
- Trees cast shadows within `ShadowOptions.trees.distanceMetres` of the
  camera. Each shadow costs two triangles, but a smaller distance or
  shadow map (`resolution`) helps on weak GPUs (see
  [`docs/shadows.md`](shadows.md)).
