# Replacing Trees, Textures, and Placement

Every procedural part of the world can be replaced with your own assets.
Replacements survive terrain changes until you reset them.

| Hook | Replaces |
| --- | --- |
| `setTreeModel(species, model)` | One species' 3D model. Impostors and shadows follow it. |
| `resetTreeModel(species)` | Restores the procedural model. |
| `setFlora({ speciesRules })` | Which species grow in each biome, and how densely. |
| `setTreeInstances(trees)` | Procedural placement, with your own list of trees. |
| `replaceTexture(target, layer, rgba)` | One terrain material or tree texture. |
| `resetTextures()` | Restores every procedural texture. |
| `setSurface({ materialTints })` | Recolours terrain materials without new textures. |

## Tree models

There are eight species slots: `"oak"`, `"pine"`, `"spruce"`, `"palm"`,
`"jungle"`, `"cypress"`, `"acacia"`, and `"shrub"`. Replace any of them
with a mesh in metres, with the base of the trunk at the origin and +Y up.

```ts
engine.setTreeModel("oak", {
  positions,       // Float32Array, three numbers per vertex
  normals,         // Float32Array, three numbers per vertex
  uvs,             // Float32Array, two numbers per vertex
  indices,         // Uint32Array, three per triangle
  textureLayers,   // optional: flora texture layer per vertex
  wind             // optional: sway weight per vertex, 0 to 1
});
```

A model may have up to 65,536 vertices and 393,216 indices (131,072
triangles); a longer array is rejected before it is copied into the
engine. Every index must refer to a vertex, every position must lie
within 1,000 m of the base, and every number must be finite. Plain arrays
must hold numbers, and plain index arrays whole numbers from 0 to
4,294,967,295, which a `Uint32Array` would otherwise wrap round
(`TypeError`). The engine rebuilds the tree
buffers and re-bakes that species' impostors, so near meshes, distant
impostors, and shadows all use the new model. Your model stands in for
every variant and age class of its species, at every level of detail:
build it light enough to draw at 150 m.

### Wind on custom models

Procedural trees carry layered wind data in every vertex: the pivot of
the limb it hangs from, its branch level, its stiffness and a phase. The
trunk bends from its base, limbs bob about their pivots, and leaves
flutter. For your model the engine derives this data, and the
`setTreeModel` call is unchanged:

- every vertex pivots at the origin;
- bark below 30 % of the model's height is trunk (level 0), and bark
  above it is limb (level 1), which bobs around the origin as well as
  bending with the trunk;
- vertices on foliage layers (4 and above) flutter, most at their
  texture's top (`v` = 1 is the tip of a leaf card, `v` = 0 where it
  joins the twig);
- stiffness is `1 - wind`, from your optional `wind` weights, or else
  rises towards the ground: `1 - (height / model height)²`.

Nothing moves at the ground, and nothing moves further than 8 % of the
tree's height. Wind strength comes from `FloraOptions.windStrength`, or
from the weather when it is on.

`textureLayers` selects a layer of the flora texture array:

| Layer | Texture | Layer | Texture |
| --- | --- | --- | --- |
| 0 | Oak bark | 8 | Acacia's bipinnate leaflets |
| 1 | Pine bark | 9 | Hanging moss |
| 2 | Palm trunk | 10 | Fern fronds |
| 3 | Smooth tropical bark | 11 | Undergrowth |
| 4 | Serrated broadleaves | 12 | Lobed oak leaves |
| 5 | Glossy tropical leaves | 13 | Spruce needle sprays |
| 6 | Pine needle bundles | 14 | Cypress scale-leaf sprays |
| 7 | Palm frond | | |

Layers 4 and above are foliage: alpha-tested, two-sided, and lit as
leaves. The leaf layers (4 to 8 and 12 to 14) come with normal maps that
give each leaf its midrib. Replace a layer with
`replaceTexture("flora", layer, rgba)` to use your own bark or leaves; a
replaced leaf layer's normal map turns flat.

## Species rules

Rules replace the built-in species mix for a biome. Biomes without a rule
keep the built-in mix.

```ts
engine.setFlora({
  ...flora,
  speciesRules: [
    { biome: "grassyMeadows", species: [{ species: "acacia", weight: 1 }], density: 0.5 },
    { biome: "coastalBeach", species: [{ species: "palm", weight: 3 }, { species: "shrub", weight: 1 }] },
    { biome: "outerThicket", species: [] } // no trees
  ]
});
```

Weights are relative. `density` (0 to 4, default 1) multiplies the
biome's tree density.

## Your own tree placement

```ts
engine.setTreeInstances([
  { x: 120, y: 34.5, z: -60, species: "oak", scale: 1.2, rotation: 0.4 },
  { x: 128, y: 35.1, z: -52, species: "shrub" }
]);

engine.setTreeInstances(undefined); // back to procedural placement
```

Coordinates are world metres, with `y` at the base of the trunk. Optional
fields default to `scale: 1`, `rotation: 0`, `tint: 0.5`, and
`dryness: 0`. `scale` must be above 0 and at most 20, `tint` and
`dryness` from 0 to 1, and positions within 10,000 km of the centre.
Up to a million trees are accepted. They are packed into
one typed array and sent to the engine in a single call, then culled and
drawn on the GPU exactly like procedural trees.

Set `ground: true` to stand a tree on the terrain as it is drawn, as
procedural trees are. The engine then ignores `y`. On a slope, the tree
sinks to its downhill root point, so no side of the trunk floats. The
tree stays upright. Grounding runs on the GPU once each time the terrain
mesh recentres, not every frame.

```ts
engine.setTreeInstances([
  { x: 120, y: 0, z: -60, species: "pine", ground: true }
]);
```

## Textures

Each texture array layer is 512 × 512 RGBA8. Use `imageToRgba()` to
convert an image, canvas, or blob:

```ts
import { imageToRgba } from "@vista-wasm/vista-wasm";

const response = await fetch("./grass.png");
const rgba = await imageToRgba(await response.blob());
engine.replaceTexture("terrainAlbedo", 0, rgba);
```

| Target | Layers | Channels |
| --- | --- | --- |
| `"terrainAlbedo"` | 0 lush grass, 1 dry grass, 2 forest floor, 3 sand, 4 rock, 5 snow, 6 mud, 7 volcanic, 8 glacier ice, 9 tundra, 10 river gravel, 11 scree | rgb colour (sRGB), a height for blending |
| `"terrainNormal"` | Same order | rg tangent-space normal, b occlusion, a roughness |
| `"flora"` | See the table above | rgb colour (sRGB), a coverage |

Use square, seamlessly tiling images. The alpha channel of a terrain
albedo texture is its height: higher texels win where two materials
meet, which gives natural blends such as grass growing between stones.
Use opaque alpha (255) if you have no height map.

Mip levels are rebuilt after each replacement. `resetTextures()` restores
every procedural texture.

## Surface options

```ts
engine.setSurface({
  textures: true,        // false: flat colours, cheaper on low-end GPUs
  detailNormals: true,
  textureScale: 1.5,     // stretch textures over more ground
  materialTints: [
    [1, 1, 1], [1.1, 0.95, 0.8], [1, 1, 1], [1, 0.95, 0.85],
    [0.9, 0.9, 1], [1, 1, 1], [1, 1, 1], [1, 1, 1]
  ]
});
```

Tints multiply each material's colour (0 to 4 per channel), in the same
order as the terrain texture layers (0 to 11). List all 12, the first 11
to leave scree untinted, the first 10 to leave gravel and scree
untinted, or the first 8 to leave ice, tundra, gravel and scree
untinted. Boulders take the rock tint.
