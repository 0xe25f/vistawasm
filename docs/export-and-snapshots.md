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
file-size cost).

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


## Putting it together

The demo and every framework example wire up all four export buttons
(screenshot, heightmap PNG, OBJ, raw heightmap) against a single
`refreshExportData()` call made after every `generateFractal()`/DEM load —
see `demo/src/main.ts` for the full wiring, including keeping a live
minimap in sync with the current terrain.
