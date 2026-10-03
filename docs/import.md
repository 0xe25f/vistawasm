# Importing maps

Build a world from your own maps: a heightmap image, a painted biome map,
a water and river mask, and tree and grass density masks. A bundle
exported with `exportBundle()` loads back exactly, recreating the same
scene.

Every import reads a `Blob` (such as a `File` from an `<input
type="file">`), an `ArrayBuffer` or a `Uint8Array`. PNGs are decoded in
full, at 8 or 16 bits a channel. Other formats (JPEG, WebP, AVIF) are
decoded by the browser at 8 bits, with a warning.

Maps of a different size from the terrain are resampled to fit, with a
`"warning"` event: biome maps take the nearest sample, and masks are
bilinear.

## Heightmap images

```ts
const file = input.files[0];
await engine.loadHeightmapImage(file, {
  metresPerSample: 10,
  minHeightMetres: -50,
  maxHeightMetres: 1800,
  seaLevelMetres: 0
});
```

- Black is `minHeightMetres` and white `maxHeightMetres`. A 16-bit PNG
    from `encodePng(map, { bitDepth: 16 })` stores its range in a
    `vistawasm:range` text chunk, so both can be left out.
- `channel` picks what is read: `"luminance"` (the default; Rec. 709
    weights on RGB), `"r"`, `"g"`, `"b"` or `"a"`.
- Images are 2 to 2048 pixels a side, the largest terrain, and need not
    be square. The limit is set by WebAssembly's memory, not by a flaw in
    VistaWASM (see [`security.md`](security.md#terrain-size)); the size
    is read from the header, so a larger image is refused before it is
    decoded. Painted maps are held to the same 2048, as they are
    resampled to the terrain.
- The heights load through `loadRawHeightmap()`, so rivers, biomes and
    everything else follow as they do for any heightmap.

`decodePng(bytes)` is exported too. It returns `{ width, height,
channels, bitDepth, data, text, palette?, transparent? }`, with 16-bit
samples in a `Uint16Array`.

## Painted biome maps

```ts
import { biomeMapFromImage } from "@vista-wasm/vista-wasm";

const { map, unmatchedFraction } = await biomeMapFromImage(file, { borderSamples: 4 });
engine.setBiomeMap(map);
```

A painted biome is absolute: it stays where you painted it.

- **Natural borders.** The engine warps each border with a seeded,
    two-octave noise (7 and 23 samples across), by up to `borderSamples`
    samples (0 to 8, default 3). Borders interlock irregularly instead of
    following your pixels, and interiors keep their biome. Ground textures
    then blend across the border over `borderSamples` samples.
- **Materials follow the biome.** Painted forest gets forest floor and
    trees, painted savannah dry grass, painted swamp mud. Rock, snow and
    sand still follow the local slope, height and climate.
- **Climate extras follow too.** Painted `iceArctic` gets glacier ice
    and permanent snow, and its glaciers are smoothed, whatever the
    climate. Painted volcanic biomes get their heat and glow; other painted
    biomes lose any volcanic heat.
- **One physical exception.** Ocean painted above sea level cannot hold
    the sea. Those samples are classified as usual, and a warning gives
    the count. Paint lakes and rivers with the water mask instead.
- **Trees need somewhere to stand.** A painted forest on a 60-degree
    cliff still has no trunks there: trees keep their physical limits.
- Pixels with value 255 (or fully transparent pixels in an image) are
    not painted, and the engine classifies them itself.

`setBiomeMap(null)` clears the map. The map stays through option changes
and clears when new terrain loads.

### Colours

`biomeMapFromImage()` reads:

- a palette PNG in the exported biome map's colours, such as the
    `biome.png` in a bundle, by its indices;
- any other image by colour: each pixel takes the nearest legend colour
    in CIELAB. Pixels more than ΔE 25 from every colour still take the
    nearest biome, but count towards `unmatchedFraction`, and a warning
    is raised when that is over 1 %.

| Index | Biome | Colour (sRGB) |
| --- | --- | --- |
| 0 | `grassyMeadows` | `#8ccc4d` (140, 204, 77) |
| 1 | `outerThicket` | `#669e38` (102, 158, 56) |
| 2 | `outerForest` | `#33802e` (51, 128, 46) |
| 3 | `innerForest` | `#14521a` (20, 82, 26) |
| 4 | `mountainFoothills` | `#998c66` (153, 140, 102) |
| 5 | `mountainProper` | `#8c8c94` (140, 140, 148) |
| 6 | `outerVolcanic` | `#59332e` (89, 51, 46) |
| 7 | `calderaVolcanic` | `#e6400d` (230, 64, 13) |
| 8 | `savannahExpanse` | `#d9b859` (217, 184, 89) |
| 9 | `coastalBeach` | `#f2e09e` (242, 224, 158) |
| 10 | `coastalRocky` | `#80736b` (128, 115, 107) |
| 11 | `outerJungle` | `#26b359` (38, 179, 89) |
| 12 | `innerJungle` | `#057333` (5, 115, 51) |
| 13 | `swampWetlands` | `#4d6140` (77, 97, 64) |
| 14 | `ocean` | `#1a408c` (26, 64, 140) |
| 15 | `alpineTransition` | `#99809e` (153, 128, 158) |
| 16 | `lowerSnowyPeaks` | `#9ec7f2` (158, 199, 242) |
| 17 | `upperSnowyPeaks` | `#f7fcff` (247, 252, 255) |
| 18 | `iceArctic` | `#bfebff` (191, 235, 255) |

`BIOME_KINDS` and `BIOME_COLOURS` hold the same table. To paint in your
own colours, pass a legend:

```ts
const { map } = await biomeMapFromImage(file, {
  legend: [
    { colour: [0, 128, 0], biome: "innerForest" },
    { colour: [255, 220, 0], biome: "savannahExpanse" },
    { colour: [255, 255, 255], biome: "iceArctic" }
  ]
});
```

## Water masks

```ts
import { waterMaskFromImage } from "@vista-wasm/vista-wasm";

engine.setWaterMask(await waterMaskFromImage(file));
```

By default the image uses the exported water map's colours:

| Paint | Colour (sRGB) | Becomes |
| --- | --- | --- |
| Lake blue | `#1a59bf` (26, 89, 191) or teal `#269999` (38, 153, 153) | a lake |
| River blue | `#338ce6` (51, 140, 230) | a river |
| River cyan | `#4dccd9` (77, 204, 217) | a river |
| Black, ocean blue `#0d2673`, or any other colour | | dry |

The hue picks the kind of water. A river's brightness sets its strength:
the full colour is strength 127 (60 m wide), half as bright about 64, and
so on down to 1 (1 m wide). Transparent pixels are dry.

`{ mode: "grey" }` reads each pixel's luminance as a `WaterMask` value
instead: 0 none, 1 to 127 a river of that strength, 128 to 255 a lake.

## Vegetation density masks

```ts
import { densityMaskFromImage } from "@vista-wasm/vista-wasm";

engine.setVegetationMasks({
  trees: await densityMaskFromImage(treeFile),
  grass: await densityMaskFromImage(grassFile)
});
```

Each byte scales the vegetation: 0 is none, 128 unchanged and 255 twice
as dense, linear between. Nothing grows denser than at density 4, so at
high densities the doubling is capped.

- The tree mask scales the cover texture the forest is placed from, so
    clearings and thickets show at every distance.
- The grass mask scales the tufts and the ground's distant grass sheen
    alike, so the two stay matched.
- An omitted key keeps its mask; `null` clears it.
- `densityMaskFromImage()` reads luminance by default, or `channel`.

The masks are applied when you set them, never per frame. They stay
through option changes and clear when new terrain loads.

`engine.getPaintedMaps()` returns the biome map, water mask and density
masks in effect, as you set them.

## Loading several images at once

```ts
import { loadTerrainFromImages } from "@vista-wasm/vista-wasm";

await loadTerrainFromImages(engine, {
  height: heightFile,
  biome: biomeFile,
  water: waterFile,
  trees: treeFile,
  grass: grassFile
}, { metresPerSample: 10 });
```

Every image is decoded before the terrain changes, so a bad file leaves
the scene as it was. Warnings arrive as the engine's `"warning"` events.

## Bundles

```ts
import { exportBundle, loadBundle } from "@vista-wasm/vista-wasm";

const bundle = await exportBundle(engine);
// ... later, or on another machine:
await loadBundle(otherEngine, bundle);
```

`loadBundle(engine, source, options?)` reads a bundle from
`exportBundle()`:

1. It reads the zip, checking every file's CRC.
2. It applies the manifest's options through the engine's setters,
    unless `options.applySettings` is `false`.
3. It loads `source-height.f32`: the heights before any carving.
4. It applies the painted maps: water mask, biome map, and tree and
    grass masks.

The other maps (biomes, water, flow and the rest) are not read: the
engine recomputes them from the source heights, the options and the
painted maps. That makes the round trip exact. Exporting the loaded
scene gives the same maps, byte for byte. `"progress"` events report
phase `"bundle"`.

### Format, version 2

Version 2 adds these files to the version 1 layout (see
[Export and Snapshots](./export-and-snapshots.md#bundles)):

- `source-height.f32`: little-endian Float32 heights before rivers,
    lakes, the water mask and glacier ice, also from
    `exportMap("sourceHeight")`;
- `painted-biome.png`: the painted biome map, a palette PNG in the
    legend colours, with index 255 (not painted) transparent. Its manifest
    entry has kind `"paintedBiome"` and gives `borderSamples`;
- `water-mask.png`, `tree-mask.png` and `grass-mask.png`: the masks as
    8-bit grey PNGs, their bytes as they are (kinds `"waterMask"`,
    `"treeMask"` and `"grassMask"`).

Painted files are written only when set, at the size they were painted.
`terrain.landform` records the landform, which sets the bedrock's beds.
`height.f32` stays, as the final heights, for other tools.

Version 1 bundles still load, from their final heights, which are carved
again, so rivers and glaciers can differ a little. A warning says so.
Export the scene again to get a version 2 bundle.

### Untrusted files

The readers are built for files from anywhere:

- Heightmap images, painted maps and bundles over 2048 pixels a side,
    and any other image over 8192, are refused from their headers before
    anything is allocated.
- Inflating stops as soon as the data passes the size the header
    implies, so a small file cannot expand into a huge one. The output is
    allocated only once the data has proved as long as the header says,
    so a header alone cannot claim memory.
- A bundle may hold at most 64 files, each at most 300 MiB inflated, and
    at most 1 GiB together (`BUNDLE_LIMITS`). `manifest.json` is at most
    1 MiB, checked before it is inflated, and may not hold `__proto__`,
    `constructor` or `prototype` keys.
- A `biomeMapFromImage()` legend holds at most 256 colours
    (`MAX_LEGEND_COLOURS`).
- Manifest paths are only keys into the zip, never file paths, and
    unknown files are ignored.
- Every malformed file throws a `VistaWasmError` with code `INVALID_DEM`
    and a message saying what is wrong, such as "PNG chunk CRC mismatch
    in "IDAT"".

## An end-to-end example

```ts
import {
  createVistaEngine,
  exportBundle,
  loadBundle,
  loadTerrainFromImages
} from "@vista-wasm/vista-wasm";

const engine = await createVistaEngine(canvas);
engine.on("warning", ({ message }) => status.textContent = message);

// A 16-bit heightmap with a painted biome map and a clearing in the forest.
await loadTerrainFromImages(engine, {
  height: await (await fetch("height.png")).blob(),
  biome: await (await fetch("biomes.png")).blob(),
  trees: await (await fetch("clearing.png")).blob()
}, { metresPerSample: 12 });
engine.start();

// Save the whole scene, and load it again into a fresh engine.
const bundle = await exportBundle(engine);
const copy = await createVistaEngine(otherCanvas);
await loadBundle(copy, bundle);
copy.start();
```
