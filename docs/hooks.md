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

A model may have up to 65,536 vertices. Every index must refer to a
vertex, and every number must be finite. The engine rebuilds the tree
buffers and re-bakes that species' impostor, so near meshes, distant
impostors, and shadows all use the new model.

`textureLayers` selects a layer of the flora texture array:

| Layer | Texture | Layer | Texture |
| --- | --- | --- | --- |
| 0 | Oak bark | 5 | Tropical leaves |
| 1 | Pine bark | 6 | Needles |
| 2 | Palm trunk | 7 | Palm frond |
| 3 | Smooth bark | 8 | Fine leaflets |
| 4 | Broad leaves | 9 | Hanging moss |

Layers 4 and above are foliage: alpha-tested, two-sided, and lit as
leaves. Replace a layer with `replaceTexture("flora", layer, rgba)` to
use your own bark or leaves.

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
`dryness: 0`. Up to a million trees are accepted. They are packed into
one typed array and sent to the engine in a single call, then culled and
drawn on the GPU exactly like procedural trees.

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
| `"terrainAlbedo"` | 0 lush grass, 1 dry grass, 2 forest floor, 3 sand, 4 rock, 5 snow, 6 mud, 7 volcanic | rgb colour (sRGB), a height for blending |
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
order as the terrain texture layers.
