# Security

This page describes what VistaWASM defends against, what it checks, the
limits it enforces, and what a host page needs to run it safely. To report
a vulnerability, see [`SECURITY.md`](../SECURITY.md).

## Threat model

VistaWASM treats **all input as untrusted**: every option object, every
number and string, heightmaps, GeoTIFF files, painted masks and biome
maps, imported bundles and images, custom tree meshes and tree
placements. A page may pass data straight from a user's upload or from
another site, so the library must stay safe whatever it is given.

The library promises that input from JavaScript:

- cannot make the engine panic. Release builds abort on a panic, which
    would stop the engine for good, so every path that input can reach
    returns a typed error instead;
- cannot make it allocate without bound or loop without end. Every size
    taken from input is checked against a documented limit, with
    overflow-free arithmetic, before anything is allocated for it,
    including the copy of the input itself. Every loop that follows
    input-shaped data (receiver chains, polylines, outlines) has a step
    bound, and walks down the drainage graph stop with an error if they
    ever meet a cycle;
- cannot read or write outside its own buffers. Offsets and indices in
    files and arrays are bounds checked, and the engine contains no
    `unsafe` Rust;
- cannot change the page's JavaScript objects. Objects used as maps
    (custom weather presets, bundle manifests) reject `__proto__`,
    `constructor` and `prototype` keys.

The library does not protect against a host page that is itself
compromised, against the browser or GPU driver, or against a device
running out of memory because the host asked for several large engines at
once.

## What is validated

Validation happens when options or data are set (`create`, every `set*`
call, `resize`, generation, loading and export), never per frame. Each
options object is copied once with `structuredClone` and checked and used
from that copy, so a getter cannot give the check one value and the
engine another, and functions are refused. A key no option has is taken
out with a warning that names the closest valid key (it becomes an error
in the next major version). Errors
are `VistaWasmError` objects with a stable `code`, such as
`OPTIONS_INVALID`, and a message that names the field, the valid range
and the value given, for example:

```text
Options are not valid: grass.viewDistanceMetres must be a number greater than 0 and at most 1000, but it is 5000.
```

- **Numbers** must be finite (no `NaN` or `Infinity`) and within the range
    documented for each option in [`options-reference.md`](options-reference.md).
    Whole numbers must be whole numbers.
- **Enum strings** (styles, presets, landforms, species, debug views,
    texture targets) must be one of the documented values; the error
    lists them.
- **Strings** such as weather preset names are at most 64 bytes long.
- **Arrays** must have the documented length: three numbers per vertex,
    three indices per triangle, nine numbers per tree, `width x height`
    bytes per mask. Indices must refer to existing vertices.
- **Files** (GeoTIFF, raw heightmaps) are parsed with bounds-checked
    reads. Their sizes are checked against the limits below before any
    sample buffer is allocated, and the strips of a GeoTIFF must fit
    inside the file. A raw heightmap is read from the caller's buffer a
    megabyte at a time, never copied whole into WebAssembly memory; a
    buffer shorter than its options need is refused, and a longer one is
    read in part with a warning. A GeoTIFF is refused before it is copied
    when it is longer than the largest the limits allow.
- **Samples** that are not finite, or lie beyond ±100 km once scaled,
    are treated as no data, with a warning in the terrain's metadata.
- **Positions** passed to queries such as `biomeAt()` or
    `getWaterSounds()` that are not finite return nothing.

## Limits

These limits are constants in `crates/vista_wasm/src/config.rs` and the
modules named.

| Limit | Value | Why |
| --- | --- | --- |
| Render size (`create`, `resize`) | 1 to 8,192 CSS pixels a side, device pixel ratio above 0 and at most 8 | WebGPU's default largest texture. Device pixels beyond 8,192 are clamped, so large displays still work. |
| Generated terrain size | Power of two, 16 to 2,048 samples | WebAssembly's memory limit (see [Terrain size](#terrain-size)). |
| Loaded heightmap, GeoTIFF, heightmap image and bundle size | 2 to 2,048 samples a side | The same as generated terrain; checked before allocating, and for images and bundles before decoding. |
| Metres per sample | Above 0 and at most 10,000 | Keeps every world coordinate finite. A GeoTIFF outside it falls back to 1 m with a warning. |
| Loaded heights | Within ±100,000 m once scaled | Anything further is treated as no data. |
| Heights and sea levels in options | Within ±100,000 m | As above. |
| Camera position and target | Within ±10,000,000 m on each axis | Keeps grid and noise lookups within integer range. |
| Distances (render, detail, cloud, fades, haze, camera near and far) | At most 1,000,000,000 m | The engine's own "unlimited" distance. |
| `grass.viewDistanceMetres` | At most 1,000 m | Grass tiles are laid out to it. |
| `flora.meshDistanceMetres`, `quality.vegetationDetailMetres` | At most 5,000 m | Tree tiles within them are walked every frame. |
| Tree and grass instances | At most 4,000,000 | GPU buffer sizes. |
| Erosion iterations | At most 5,000 each | Bounds GPU and CPU work per generation. |
| Volumetric cloud raymarch steps | 8 to 64 | Bounds a shader loop. |
| Custom weather presets | At most 64, names at most 64 bytes; at most 256 entries in any object of values by name | Lookups are linear; bounds parsing time. |
| Explicit river inflows | At most 8 | |
| Lens drops | At most 512 | |
| `advanceWeather()` | 0 to 86,400 s | Bounds the work of one call. |
| Painted water, biome and density maps | 2 to 2,048 a side, exactly `width x height` bytes | Resampled to the terrain, so held to its limit (see [Terrain size](#terrain-size)). |
| Custom tree mesh | At most 65,536 vertices and 393,216 indices, every vertex within 1,000 m of the base | Mesh buffers and bounds stay finite. |
| Custom tree placements | At most 1,000,000 trees, scale above 0 and at most 20, tint and dryness 0 to 1 | Checked before the trees are unpacked. |
| Replaced textures | 512 x 512 RGBA8, 2,048 KiB | |
| Map export | 2 to 2,048 a side, at most 1 GiB | Built in WASM memory beside the terrain, so held to its limit. |
| Tree export | 1 to 10,000,000 trees, 2,000,000 by default | |
| Raw heightmap buffer (`loadRawHeightmap`) | At most 16,777,216 bytes (`MAX_RAW_HEIGHTMAP_BYTES`, 2048 x 2048 float32) | Checked in JavaScript and again in the engine before reading. |
| GeoTIFF buffer (`loadDemFromArrayBuffer`) | At most 83,886,080 bytes (`MAX_DEM_BYTES`: 2048 x 2048 float32 and 64 MiB of tags) | Checked before the file is copied. |
| Vegetation area | Trees, grass, reeds and boulders are placed over at most 10,000 km² (a 2048 map at 49 m); beyond it, none, with a warning | Their lattices and tile grids span the whole map, so their work and memory grow with its area, whatever its sample count. Tile grids are also capped at 64 Mi tiles. |
| Spatial buckets (water sounds, river segments) | At most 4 Mi cells; a vast map gets wider cells | Their cell counts no longer overflow. |
| Rivers | None on a map under 50 m across | Too small for the narrowest channel. |
| GPU buffers | The largest the adapter offers, never less than WebGPU's defaults (256 MiB, 128 MiB storage bindings) | GPU erosion needs 16 bytes a sample, 64 MiB at 2048, within the defaults; were it to need more, it would run on the CPU, with a warning. |

## Terrain size

A terrain is at most 2,048 samples a side: generated, loaded from a raw
heightmap or GeoTIFF, imported from an image, or read from a bundle.
The limit is set by WebAssembly's memory, not by a flaw in VistaWASM. A
WASM module can address at most 4 GiB, a terrain's heights, normals,
surface, drainage, rivers and vegetation must all fit in it, and the
memory a page has used is never given back while the engine lives. A
2048 x 2048 terrain peaks at 0.5 to 1.4 GiB. A 4096 x 4096 one peaks
at up to 3.7 GiB, too close to the 4 GiB limit to rebuild and export it
safely under every option. A larger map is refused at once with
`OPTIONS_INVALID` (or `INVALID_DEM` for an image or bundle), and nothing
is allocated for it. Peak memory for the largest jobs is measured with
`scripts/visual-check/memory-check.mjs`.

Rebuilding the world (when a painted map is set, or water or biome
options change) frees the old world before it builds the new one, so a
terrain rebuilds within the same memory as a load: a raw 2048 x 2048
load, a painted water mask and every map exported at full size peak at
1.3 GiB.

Painted maps, mask images and exports are held to the same 2,048: they
are resampled to the terrain, or from it, so detail past it would be lost,
and they are held in WASM memory beside it. Only images that never become
terrain data, such as texture images, and the PNG decoder itself, take up
to 8,192 a side.

## JavaScript-side limits

The JavaScript wrapper checks its own inputs (PNG images, bundles, typed
arrays passed to the engine) before they reach WebAssembly.

| Limit (where) | Value | Why |
| --- | --- | --- |
| `MAX_IMAGE_SIDE` (`js/src/codec.ts`, exported) | 8192 px a side | Largest image `decodePng()` and the image readers accept; at four 16-bit channels this decodes to 512 MiB at most. Checked from the IHDR before anything is allocated. |
| Inflated PNG data (`inflate()` in `codec.ts`) | exactly the size the IHDR implies (rows x (stride + 1), per Adam7 pass) | Decompression stops as soon as the output passes it, and the output is allocated only once the data has proved that long, so a header cannot claim memory and a small file cannot expand. |
| `BUNDLE_LIMITS.entries` (`map-import.ts`, exported) | 64 files | Zip entry count; checked from the end record before the directory is walked. |
| `BUNDLE_LIMITS.entryBytes` | 300 MiB (314,572,800 bytes) | Largest file a bundle entry may inflate to: far more than any map of a 2048 x 2048 terrain (its float32 heights are 16 MiB). Checked from the directory before inflating. |
| `BUNDLE_LIMITS.totalBytes` | 1 GiB (1,073,741,824 bytes) | Total the reader inflates for one bundle. |
| `BUNDLE_LIMITS.manifestBytes` | 1 MiB (1,048,576 bytes) | Largest `manifest.json` parsed as JSON; now checked against the directory's size before inflating, and again on the bytes. |
| Manifest keys | no `__proto__`, `constructor`, `prototype` at any depth (JSON reviver) | Manifest options are copied into engine setters; paths are only keys into the zip's `Map`, and unknown entries are ignored. |
| Bundle terrain size (`parseManifest`) | width and height whole numbers 2..2048 (`MAX_TERRAIN_SIDE`); heights exactly width x height x 4 bytes | Matches the engine's terrain limit. |
| `MAX_OPTIONS_DEPTH` (`errors.ts`, exported) | 8 levels | Options objects nest at most this deep (the deepest real option is four levels down); also rejects options that contain themselves, which the engine's deserialiser would recurse into without end. |
| `PROTOTYPE_KEYS` (`errors.ts`) | `__proto__`, `constructor`, `prototype` | Rejected as keys at any depth of any options object, including weather preset names and `next` weights. |
| `MAX_WEATHER_PRESETS` (`index.ts`, exported) | 64 presets per `setWeather()` | Bounds the preset table the engine resolves. |
| `MAX_PRESET_NAME_LENGTH` (`index.ts`, exported) | 64 UTF-16 code units | Preset names and `WeatherOptions.state`; also required non-empty. |
| `MAX_LEGEND_COLOURS` (`map-import.ts`, exported) | 256 colours | `biomeMapFromImage()` compares every distinct pixel colour with every legend colour. |
| `encodePng()` legend | 256 entries, index 0..255 | A PNG palette holds one colour per byte value; larger indices would allocate a palette past the PNG limit (and `Math.max(...legend)` would overflow the stack). |
| `encodePng()`/`encodeRaw()` maps | width/height 1..2048, channels 1..12, data length exactly width x height x channels; raw output <= 1 GiB | Checked before the product is compared or any output is allocated. |
| `MAX_TREE_MODEL_VERTICES` (`index.ts`) | 65,536 vertices | Mirrors the engine's `MAX_CUSTOM_TREE_VERTICES`; checked before the arrays are copied into WASM memory. |
| `setTreeInstances()` | At most 1,000,000 trees | Counted before the nine floats a tree are allocated. |
| `MAX_TERRAIN_SIDE` (`codec.ts`, exported) | 2,048 samples | Largest terrain side, as the engine allows; heightmap images and bundles are held to it from their headers. |
| `MAX_RAW_HEIGHTMAP_BYTES` (`index.ts`, exported) | 16,777,216 bytes | `loadRawHeightmap()` refuses a longer buffer, or one shorter than `width x height` samples, before the engine sees it. |
| `MAX_DEM_BYTES` (`index.ts`, exported) | 83,886,080 bytes | Largest GeoTIFF `loadDemFromArrayBuffer()` accepts. |
| `fetchDemBytes()` / `loadDemFromUrl()` `maxBytes` | 1 to `MAX_DEM_BYTES`, `MAX_DEM_BYTES` by default | A longer `Content-Length` is refused unread, and a body that grows past it is cancelled. |
| `imageSize()` (`map-import.ts`, exported) | 2048 px a side for heightmaps and painted maps, 8192 for other images | Sizes are read from headers and refused past the limit before anything decodes them; formats whose size cannot be read are refused. |
| `BUNDLE_LIMITS.decodedBytes` (`map-import.ts`, exported) | 1 GiB (1,073,741,824 bytes) | What a bundle's painted maps may decode to together, from their PNG headers; they are decoded one at a time, after the check. Four maps at 2048 decode to at most 128 MiB, so this is a second guard. |
| Tree model index values (plain arrays) | whole numbers 0..4,294,967,295 | A `Uint32Array` would wrap anything else round silently. |
| `replaceTexture()` | layer whole 0..255; texels exactly 512 x 512 x 4 = 1,048,576 bytes (`TEXTURE_LAYER_SIZE`) | The engine takes the layer as u32 (2^32 would wrap to 0); the length is checked before copying. |
| `MAX_CANVAS_SIDE` (`index.ts`) | 8,192 CSS px; `devicePixelRatio` in (0, 8] | `resize()` checks before the canvas is touched. |
| `imageToRgba()` size | whole 1..8192 | Output is size x size x 4 bytes. |
| `exportTerrainObj()` `maxSamplesPerSide` | whole number up to 2048, the largest terrain | Bounds the OBJ text it builds. |
| `exportSnapshot()` | `mimeType` matches `image/[\w.+-]{1,64}`; `quality` 0..1 | Only image types reach `canvas.toBlob()`. |
| CSV/JSON tree export | numeric fields must be numbers; species a `TreeSpecies` | No cell can hold a comma, a line break or a formula. |
| Loader | module from `new URL("./pkg/vista_wasm.js", import.meta.url)`, WASM from `new URL("./pkg/vista_wasm_bg.wasm", import.meta.url)` (or next to a glue the caller names) | Nothing loads from another origin unless the caller passes `wasmUrl`, `moduleUrl` or `wasmModule`; failures reject with `WASM_LOAD_FAILED` and are not cached. |

## Content Security Policy

VistaWASM runs under a strict Content Security Policy. The page needs:

```text
script-src 'self' 'wasm-unsafe-eval'
```

`'wasm-unsafe-eval'` allows the page to compile WebAssembly. It does not
allow `eval()` or `new Function()`, which VistaWASM never uses. Without
it, `WebAssembly.instantiateStreaming` fails and `createVistaEngine()`
rejects with the browser's error.

The WASM file is fetched from the package's own URL, so `connect-src`
(or `default-src`) must allow wherever you host it, usually `'self'`. If
you pass `wasmUrl` pointing at another origin, allow that origin in
`connect-src`; if you pass `moduleUrl` for the glue script, allow its
origin in `script-src`.

A complete policy for a page that also saves snapshots and exports:

```html
<meta
  http-equiv="Content-Security-Policy"
  content="default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; img-src 'self' blob: data:; style-src 'self'"
/>
```

## Cross-origin isolation

No cross-origin isolation headers are needed. VistaWASM uses no threads,
`SharedArrayBuffer` or shared memory, so it works without
`Cross-Origin-Opener-Policy` and `Cross-Origin-Embedder-Policy`. Setting
them does no harm.

## Dependencies

The published package has no runtime npm dependencies. The engine's Rust
dependencies are `wgpu`, `wasm-bindgen`, `js-sys`, `web-sys`,
`serde`, `serde-wasm-bindgen`, `thiserror`, `bytemuck` and their
dependencies.

Advisory status, checked on 2026-10-02:

| Check | Result |
| --- | --- |
| `cargo audit` (130 crates in `Cargo.lock`) | No advisories |
| `npm audit` (root) | 0 vulnerabilities |
| `npm audit` (`bench/`) | 0 vulnerabilities |

CI (`.github/workflows/checks.yml`) runs these audits on every push and
pull request, including Dependabot's weekly updates, and fails on any
advisory. The build checks the prebuilt `wasm-bindgen` it downloads
against SHA-256 checksums recorded in `scripts/wasm-bindgen-sha256.json`
and refuses any other archive.

There are no advisories without a released fix. If one appears, it is
recorded here with the reason it cannot be fixed yet and the parts of the
library it affects.

Only one development dependency runs an install script: `fsevents`, an
optional, macOS-only file watcher that Vite uses. It is never installed on
other systems and is not part of the published package.

Run the checks yourself with:

```sh
cargo audit
npm audit
(cd bench && npm audit)
```
