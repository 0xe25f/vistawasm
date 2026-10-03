# Export and Snapshots

VistaWASM has no save-file format of its own — every export helper below
turns already-in-memory engine state into a plain `Blob`, `string`, or
`Uint8Array` you hand to your own download/upload/storage code. None of
these calls touch the network or the filesystem themselves.

## Heightmap export (raw data)

```ts
const bytes = engine.exportHeightmap(); // Uint8Array<ArrayBuffer>
```

Returns the active terrain's heights as little-endian `float32` samples,
row-major, `width * height * 4` bytes — the same layout
`loadRawHeightmap()` accepts with `sampleFormat: "float32"`, so a round
trip through export/re-import is lossless. Throws if there is no active
terrain, or (see
[`docs/events-errors-and-lifecycle.md`](events-errors-and-lifecycle.md#reentrancy))
while terrain generation is already in flight.

Use `readHeightmapFloats(bytes)` (exported from the package root) to turn
the raw bytes into a plain `Float32Array` for your own processing —
gameplay height queries
([`docs/game-development.md`](game-development.md#querying-terrain-height-for-gameplay)),
physics colliders
([`docs/game-development.md`](game-development.md#3-collision-and-physics)),
or feeding another engine's terrain renderer
([`docs/engine-integration.md`](engine-integration.md#pattern-b--vistawasm-generates-your-engine-renders)).

### Downloading the raw `.bin` for other software

```ts
import { downloadRawHeightmap } from "@vista-wasm/vista-wasm";

downloadRawHeightmap(handle.metadata, engine.exportHeightmap());
// -> downloads "vistawasm-heightmap-1024x768.f32le.bin"
```

The raw export has no header — it's exactly `width * height * 4` bytes,
nothing else — so `width`/`height` exist only in `TerrainMetadata`, not in
the file itself. Third-party heightmap importers usually need those
dimensions typed in separately, and some try to guess them by assuming the
sample count is a perfect square, which fails (with an error like "does not
appear to be a perfect square") for any non-square terrain, e.g. a DEM
import or a custom `width`/`height` that don't match. `downloadRawHeightmap`
encodes the real `width`/`height` into the filename so you always have the
correct numbers to hand to the importing tool, and some importers parse
that `WIDTHxHEIGHT` convention from the filename automatically. Prefer it
over building the `Blob`/filename yourself.

## Heightmap image export (PNG minimap)


```ts
import { renderHeightmapToCanvas, exportHeightmapImage } from "@vista-wasm/vista-wasm";

// Draw into an existing <canvas> — e.g. a live minimap:
renderHeightmapToCanvas(minimapCanvas, handle.metadata, engine.exportHeightmap());

// Or export straight to a downloadable Blob:
const blob = await exportHeightmapImage(handle.metadata, engine.exportHeightmap(), {
  colourMode: "hypsometric" // or "grayscale"
});
```

Both share `HeightmapImageOptions.colourMode`:

- `"hypsometric"` (default) — a colour ramp based on height relative to sea
  level (the same style of colouring a topographic map uses). Good for a
  human-readable minimap.
- `"grayscale"` — pixel luminance encodes height directly. Use this when
  *another* renderer expects a grayscale heightmap image (for example,
  Babylon.js's `CreateGroundFromHeightMap` — see
  [`docs/engine-integration.md`](engine-integration.md#babylonjs-built-in-heightmap-terrain)).
  Note this loses precision compared to the raw `float32` export (8 bits
  per channel unless you encode more yourself).

Any other `colourMode` throws `OPTIONS_INVALID`. `metadata.width` and
`metadata.height` must be whole numbers, and the bytes a `Uint8Array` of
exactly `width × height` float32 samples (`TypeError` otherwise).

`exportHeightmapImage()` additionally accepts `mimeType`/`quality`, passed
straight through to the underlying `canvas.toBlob()` call.

## 3-D model export (Wavefront OBJ)

```ts
import { exportTerrainObj, downloadText } from "@vista-wasm/vista-wasm";

const obj = exportTerrainObj(handle.metadata, engine.exportHeightmap(), {
  maxSamplesPerSide: 256
});
downloadText(obj, "terrain.obj", "model/obj");
```

Generates a triangulated Wavefront OBJ string centred on the terrain
origin, in the same metres-based coordinate space the engine uses
internally. Large terrain is downsampled to `maxSamplesPerSide` (default
`256`) per axis so the exported file stays a practical size for a "download
3-D model" button — raise it if you need a denser export mesh (at a real
file-size cost). It must be a whole number up to 2048, the largest terrain
(`OPTIONS_INVALID` otherwise), and `metadata.metresPerSample` a finite number.

## Canvas screenshot

```ts
const blob = await engine.exportSnapshot({ mimeType: "image/png" });
```

Captures whatever is currently drawn on the engine's own canvas via
`canvas.toBlob()` — a plain screenshot of the rendered frame, not a
re-render. Because it reads the canvas directly rather than calling into
the WASM engine, it works even while an async call like terrain generation
is in flight (unlike `exportHeightmap()`, which throws in that window) —
though the captured frame will simply be whatever was last rendered, which
may be a frame from before generation started if `renderOnce()`/`start()`
hasn't drawn a new one yet.

## Download helpers

```ts
import { downloadBlob, downloadText, downloadRawHeightmap } from "@vista-wasm/vista-wasm";

downloadBlob(pngBlob, "screenshot.png");
downloadText(objString, "terrain.obj", "model/obj");
downloadRawHeightmap(handle.metadata, engine.exportHeightmap());
```

`downloadBlob`/`downloadText`/`downloadRawHeightmap` all trigger a browser
download via a temporary `<a download>` click — no server round-trip. Use
`downloadBlob` for binary data (`Blob`, `ArrayBuffer`-backed),
`downloadText` for plain strings (it wraps the text in a `Blob` with the
given MIME type for you), and `downloadRawHeightmap` specifically for the
`exportHeightmap()` bytes (see above for why it's worth using over a plain
`downloadBlob` call).


## Map export

`engine.exportMap(kind, options?)` reads back any map the renderer
builds. Every map comes from the same data the renderer draws from, so
an export matches what you see. Maps are row-major from the terrain's
`-x, -z` corner, with channels interleaved.

```ts
const biomes = engine.exportMap("biome");
console.log(biomes.width, biomes.height, biomes.encoding.legend?.[3].name); // "innerForest"

// Resampled to 2048 x 2048: bicubic for heights.
const height = engine.exportMap("height", { size: [2048, 2048] });
```

| Kind | Type and channels | Meaning |
| --- | --- | --- |
| `height` | float32 x 1 | Metres above the datum, as `exportHeightmap()` gives them. |
| `biome` | uint8 x 1 | `BiomeKind` index. The legend names every biome, with the `biomes` debug view's colour. |
| `water` | uint8 x 1 | 0 none, 1 river, 2 lake, 3 ocean, 4 waterfall (with its plunge pool), 5 painted river, 6 painted lake. |
| `waterDepth` | float32 x 1 | Metres from the water surface to the ground; 0 on dry land. |
| `flow` | float32 x 1 | Upstream drainage area in km². |
| `discharge` | float32 x 1 | Mean discharge in m³/s. |
| `materials` | uint8 x 12 | Weights in `MAT_*` order, summing to 255: lush grass, dry grass, forest floor, sand, rock, snow, mud, volcanic, ice, tundra, gravel, scree. The legend names every channel. |
| `slope` | float32 x 1 | Degrees. |
| `normals` | float32 x 3 | World-space unit normals (x, y, z). |
| `occlusion` | uint8 x 1 | Ambient occlusion, 0 occluded to 255 open. |
| `temperature` | float32 x 1 | Mean annual °C. |
| `moisture` | uint8 x 1 | 0 arid to 255 saturated. |
| `treeDensity` | uint8 x 1 | Trees per hectare / 4 at the current density, up to 255. |
| `grassDensity` | uint8 x 1 | Share of the ground grass, ferns and undergrowth cover at the current density, 0 to 255. |
| `sourceHeight` | float32 x 1 | Metres above the datum before any carving: the heights as loaded, without rivers, lakes, the water mask or glacier ice. |

`encoding` says how to read the values: `units`, a `scale` from stored
values to those units (`treeDensity` has `scale: 4`), the `range` of a
float map, a `legend` for `biome`, `water` and `materials`,
`metresPerPixel`, `seaLevelMetres` and the terrain's `generator`.

Where the data comes from:

- `water` and `waterDepth` are the water the renderer draws: river
    ribbons, lakes and plunge pools where their surface is above the
    ground, streams narrower than a sample along their centreline, and the
    sea wherever the ground is below sea level. With water off, both are 0.
- `flow` is the drainage the soil and forests use: the rivers' own when
    they are carved, and a quick steepest-descent routing otherwise.
- `discharge` is the carved rivers' hydrology; with rivers off, it is
    what the river model would route over the ground as it is.
- `treeDensity` is the cover texture the tree lattice reads. It is 0 with
    flora off or with hand-placed trees.

**Resampling.** `size` is `[width, height]`, each a whole number from 2
to 2048, the largest terrain: resampling past it adds no detail, and the
map is built in WebAssembly memory beside the terrain, so it is held to
the same limit (see
[`security.md`](security.md#terrain-size)). The default is the terrain's own size, which copies every value
exactly. Continuous maps are bicubic, normals bilinear then renormalised,
materials bilinear then renormalised to sum to 255, and `biome` and
`water` take the nearest sample, so they never gain a value the map does
not have. Corners map to corners. A map over 1 GiB is rejected.

Exports run only when you call them; they add nothing to a frame. They
throw while terrain is generating.

### Encoding maps as files

```ts
import { encodePng, encodeRaw, downloadBlob } from "@vista-wasm/vista-wasm";

const png = await encodePng(engine.exportMap("biome")); // a palette PNG
downloadBlob(png as Blob, "biome.png");

const heights = engine.exportMap("height");
const png16 = await encodePng(heights, { bitDepth: 16 });
console.log((png16 as { range?: [number, number] }).range); // the metres black and white stand for

const { bytes, range } = encodeRaw(heights, { type: "uint16" });
```

`encodePng()` writes the PNG itself, compressing with the browser's
`CompressionStream`:

- `biome` and `water` as palette images in their legend colours;
- `normals` as RGB, `(n + 1) / 2 x 255`;
- `materials` as three RGBA files, resolving to an array:
    `splat0` (lush grass, dry grass, forest floor, sand), `splat1` (rock,
    snow, mud, volcanic) and `splat2` (ice, tundra, gravel, scree);
- every other map as grey. Byte maps at 8 bits are written as they are.
    Otherwise values are scaled over `range` (by default the map's lowest
    and highest value) to 0 to 255 or 0 to 65535; the range is stored in
    a `vistawasm:range` text chunk and returned as the Blob's `range`.

`smaller: true` filters each row against the one above, which makes
smoother maps smaller. `encodeRaw()` returns little-endian Float32 as it
is, or Uint16 scaled over a range, which it returns.

## Tree export

```ts
import { treesToCsv, downloadText } from "@vista-wasm/vista-wasm";

const trees = engine.exportTrees({ region: { minX: -500, minZ: -500, maxX: 500, maxZ: 500 } });
downloadText(treesToCsv(trees), "trees.csv", "text/csv");
```

Each `TreeRecord` is `{ x, y, z, species, variant, scale, rotation, tint,
dryness }`, plus `handPlaced: true` for trees from `setTreeInstances()`.

- The list is exact: every tree of the lattice at the current density,
    including the trees the renderer streams in near the camera, which are
    generated on the CPU with the same hash. Hand-placed trees replace
    procedural ones, as they do on screen.
- `y` is where the trunk stands: the lowest ground under its roots, less
    5 % of the root radius, on the ground at full detail. Trees you placed
    with `ground: false` keep their own `y`.
- `variant` is which of the species' grown shapes it is drawn with.
- More than `maxCount` trees (default 2,000,000, at most 10,000,000)
    throws `OPTIONS_INVALID`: pass a `region` or raise the cap.

`treesToCsv()` writes a header row and one row a tree, keeping full
precision. `treesToJson()` writes a JSON array.

## Bundles

```ts
import { downloadBundle } from "@vista-wasm/vista-wasm";

await downloadBundle(engine, "my-world.zip", { previews: true });
```

`exportBundle(engine, options?)` returns one zip holding:

- `manifest.json`;
- `height.f32` (exact, at the terrain's own size, even with `size`);
- `source-height.f32`: the heights before any carving, at the terrain's
    own size;
- `biome.png`, `water.png`, `flow.f32`, `discharge.f32`;
- `splat0.png`, `splat1.png`, `splat2.png`;
- `slope.png` (8-bit, 0 to 90 degrees), `normals.png`, `occlusion.png`,
    `temperature.f32`, `moisture.png`;
- `treeDensity.png`, `grassDensity.png`;
- `trees.csv`, when the map holds at most `maxTrees` trees (or always,
    with `trees: true`, which throws over the cap);
- `preview-height.png` (hypsometric) and `preview-biome.png`, with
    `previews: true`;
- the painted maps, when set: `painted-biome.png`, `water-mask.png`,
    `tree-mask.png` and `grass-mask.png`, at the size they were painted.

`loadBundle()` reads it back exactly: see
[Importing maps](import.md#bundles).

`.f32` files are little-endian Float32. PNGs are stored in the zip as
they are, and the rest deflated. A bundle over 4 GiB is rejected, since
the zip has no ZIP64 records. `downloadBundle(engine, filename,
options?)` exports and downloads it.

`manifest.json` is versioned, so later releases can read older bundles.
Bundles are version 2; version 1 bundles (without the source heights and
painted maps) still load:

```json
{
  "format": "vistawasm-bundle",
  "version": 2,
  "generator": "vistawasm-fractal-0.2.0",
  "terrain": {
    "width": 512,
    "height": 512,
    "metresPerSample": 12,
    "seaLevelMetres": 0,
    "minHeightMetres": -140.2,
    "maxHeightMetres": 1893.4,
    "landform": "continental"
  },
  "options": { "flora": { "density": 1 }, "terrain": { "seed": 1234 } },
  "files": [
    { "path": "height.f32", "kind": "height", "type": "float32", "width": 512, "height": 512, "channels": 1, "encoding": { "units": "m", "range": [-140.2, 1893.4] } },
    { "path": "source-height.f32", "kind": "sourceHeight", "type": "float32", "width": 512, "height": 512, "channels": 1, "encoding": { "units": "m", "range": [-140.2, 1880.1] } },
    { "path": "biome.png", "kind": "biome", "type": "uint8", "width": 512, "height": 512, "channels": 1, "bitDepth": 8, "encoding": { "legend": [{ "index": 0, "name": "grassyMeadows", "colour": [0.55, 0.8, 0.3] }] } },
    { "path": "splat1.png", "kind": "materials", "type": "uint8", "width": 512, "height": 512, "channels": 4, "firstChannel": 4, "encoding": { "units": "weight" } },
    { "path": "slope.png", "kind": "slope", "type": "float32", "width": 512, "height": 512, "channels": 1, "bitDepth": 8, "range": [0, 90], "encoding": { "units": "degrees" } },
    { "path": "trees.csv", "kind": "trees", "type": "csv", "count": 48210 },
    { "path": "painted-biome.png", "kind": "paintedBiome", "type": "uint8", "width": 512, "height": 512, "channels": 1, "borderSamples": 3 },
    { "path": "tree-mask.png", "kind": "treeMask", "type": "uint8", "width": 256, "height": 256, "channels": 1 }
  ]
}
```

- `options` is `engine.getOptionsSnapshot()`: the options in effect, and
    under `terrain` the `FractalTerrainOptions` a generated terrain came
    from.
- Each file lists its map's `kind`, `type`, size, `channels` and
    `encoding` (the `exportMap()` encoding, legends included). A splat file
    says which material it starts at (`firstChannel`); a PNG scaled from
    floats gives the `range` its grey levels span.

### Options snapshot

```ts
const snapshot = engine.getOptionsSnapshot();
const copy = await createVistaEngine(otherCanvas, snapshot);
if (snapshot.terrain) {
  await copy.generateFractal(snapshot.terrain);
}
```

`getOptionsSnapshot()` returns a plain-JSON copy of the options in
effect: those the engine was created with, replaced part by part by
every setter it has accepted since (`setCamera()`, `setSun()`,
`setWater()`, `setFlora()`, `setGrass()`, `setWeather()`,
`setRenderQuality()`, `resize()` and the rest). A part never set is at
its default, so passing the snapshot to `createVistaEngine()` gives the
same engine. `terrain` holds the options of the last `generateFractal()`,
and is absent after loading a heightmap; `landform` holds the landform a
loaded heightmap was given. Hand-placed trees and painted maps are not
options; a bundle keeps them in `trees.csv` and its painted map files,
and `getPaintedMaps()` returns the painted maps.

## Putting it together

The demo's Export section exports any map as an 8 or 16-bit PNG or raw
Float32 or Uint16 at its own size or up to 2048 x 2048, the trees as
CSV, and a bundle, with a status line giving each file's size. The demo
and the vanilla, React, Vue, and Svelte examples also wire up the
four original export buttons (screenshot, heightmap PNG, OBJ, raw heightmap)
against a single `refreshExportData()` call made after every
`generateFractal()`/DEM load — see `demo/src/main.js` for the full
wiring, including keeping a live minimap in sync with the current terrain.
