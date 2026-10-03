# Events, Errors, and Lifecycle

## Lifecycle

`createVistaEngine(canvas, options)` is the only way to create an engine.
It loads the WASM module (once per page — cached after the first call
across every `createVistaEngine()` invocation), requests a WebGPU
adapter/device, and configures the canvas's surface. It rejects with a
`VistaWasmError` if any step fails (see [Error codes](#error-codes) below).

Once created, an engine exists until you call `dispose()`:

```ts
engine.stop();       // stop the internal render loop, if you called start()
engine.dispose();     // release every GPU resource — terminal, cannot be undone
```

`dispose()` is idempotent on the JavaScript side (repeated calls are safe)
but is genuinely terminal — there is no way to "revive" a disposed engine;
create a new one with `createVistaEngine()` instead. Always stop rendering
(your own loop, or call `engine.stop()`) before disposing, so no frame
renders against a disposed engine.

There is no public API to query "what state is the engine in right now" —
`EngineState` (`Created`/`InitialisingGpu`/`Ready`/`LoadingTerrain`/
`Rendering`/`DeviceLost`/`Disposed`) is an internal Rust concept used by
the engine's own tests, not exposed to JavaScript. Infer engine state from
the events below instead (a `"terrainLoaded"` event means terrain is ready;
a `"deviceLost"`/`"fatalError"` event means the engine needs to be disposed
and recreated).

## Events

Subscribe with `engine.on(eventName, listener)`, which returns an
unsubscribe function:

```ts
const unsubscribe = engine.on("stats", (stats) => updateHud(stats));
// later:
unsubscribe();
```

| Event | Payload | When |
| --- | --- | --- |
| `"ready"` | `undefined` | Once, immediately after `createVistaEngine()` succeeds. |
| `"progress"` | `{ phase: string; progress: number }` | During `generateFractal()`: phase `"fractal"` at 0 when it starts and at 1 when it resolves, and between them each generation phase from 0 to 1 as it runs: `"tectonics"`, `"drainage"`, `"detail"`, `"erosion"` (when requested), and `"finishing"`, with `"rivers"` reported within it. **Not** emitted by `loadDemFromArrayBuffer`/`loadDemFromUrl`/`loadRawHeightmap` — those only emit `"terrainLoaded"` (and `"warning"`, below). `loadBundle()` emits phase `"bundle"` at 0, 0.3, 0.7 and 1. |
| `"warning"` | `{ message: string; details?: unknown }` | Once per entry in `TerrainHandle.metadata.warnings`, after any successful terrain call, and once for each option key a call ignored (see [Unknown option keys](#unknown-option-keys)). Terrain warnings include GPU erosion failing or not fitting the GPU's buffer limits and running on the CPU instead, a raw buffer longer than its options need, and a terrain over 10,000 km², on which no trees, grass, reeds or boulders are placed. Warnings from `createVistaEngine()` are emitted on a later task, so listeners added as soon as it resolves hear them. |
| `"terrainLoaded"` | `TerrainHandle` | After every successful `generateFractal`/`loadDemFromArrayBuffer`/`loadDemFromUrl`/`loadRawHeightmap` call. |
| `"stats"` | `RenderStats` | After every rendered frame, whether `start()` or your own loop calls `renderOnce()` (see [`docs/game-development.md`](game-development.md#1-where-vistawasm-fits-in-a-game-loop)). `renderOnce()` also returns the same `RenderStats`. Not emitted when `renderOnce()` draws nothing: while terrain is generating, or while the GPU is still drawing two earlier frames (see [Frame pacing](#frame-pacing)). The `start()` loop builds a frame's `RenderStats` only while something listens for `"stats"` (or the weather changed), so a loop nothing watches makes no garbage each frame. |
| `"weatherChanged"` | `WeatherKind \| null` | When the dominant weather changes (halfway through a transition), and `null` when the weather system is switched off. Checked on every frame, whether `start()` or you call `renderOnce()`. See [`docs/weather.md`](weather.md#reading-the-weather). |
| `"fatalError"` | `Error` (a `VistaWasmError`) | Once, when the engine stops for good after an internal error (a WebAssembly trap), from whichever call met it; see [After an internal error](#after-an-internal-error). Also from the `start()` loop's internal catch, for any error other than a lost device. The loop stops itself before emitting this. |
| `"deviceLost"` | `Error` (a `VistaWasmError` with code `WEBGPU_DEVICE_LOST`) | Once, as soon as a call (a frame, a setter that rebuilds, or a terrain call) finds the browser has lost the GPU device, with the browser's reason in the message. Rendering stops first. VistaWASM does not recreate the device itself; dispose and create a new engine. |
| `"gpuError"` | `Error` (a `VistaWasmError` with code `GPU_ERROR`) | For each GPU validation or out-of-memory error while a terrain's resources were uploaded, or that no error scope of the engine's own caught, with the browser's message, after the next call once the GPU has reported it. The engine keeps running; one may be followed by a black or missing part of the picture. At most eight are kept between calls. |

`"deviceLost"` and `"gpuError"` fire whether `start()` or your own loop
calls `renderOnce()`; the call that meets a lost device also throws.

## Error codes

Engine errors are `VistaWasmError`s (`instanceof Error`, with a stable
`.code: VistaErrorCode` and a human-readable `.message`). Arguments of the
wrong type throw a plain `TypeError` before reaching the engine: an
options argument that is not an object, a `buffer` that is not an
`ArrayBuffer`, a mask whose `data` is not a `Uint8Array`, an unknown tree
species, debug view or event name, or a listener that is not a function.
Values of the right type but out of range throw a `VistaWasmError` with
code `OPTIONS_INVALID`, and the message gives the valid range.

The WASM loads from the package's own `dist/pkg/` folder, next to
`dist/index.js`, unless you pass `wasmUrl`, `moduleUrl` or `wasmModule`
to `initialiseVistaWasm()`; it never loads from anywhere else on its own.
Loading errors are never swallowed: they reject with `WASM_LOAD_FAILED`.

```ts
import { VistaWasmError } from "@vista-wasm/vista-wasm";

try {
  await engine.generateFractal(options);
} catch (error) {
  if (error instanceof VistaWasmError) {
    handleByCode(error.code);
  }
}
```

| Code | Typical cause |
| --- | --- |
| `WEBGPU_UNAVAILABLE` | The browser has no WebGPU support at all. Check with `detectVistaWasmSupport()` *before* calling `createVistaEngine()` so you can show a fallback instead of a failed creation. |
| `WEBGPU_DEVICE_REQUEST_FAILED` | WebGPU exists but the browser refused to grant a device (e.g. GPU driver blocklisting). |
| `WEBGPU_DEVICE_LOST` | The active device was lost after startup (driver reset, tab discarded/restored, GPU switch). Surfaced once through the `"deviceLost"` event, from whichever call first finds the device lost (a frame, a setter that rebuilds, or a terrain call), and thrown by that call. |
| `CANVAS_INVALID` | The supplied canvas is missing, not an `HTMLCanvasElement`, or its WebGPU surface configuration failed. |
| `OPTIONS_INVALID` | A public option failed validation (see [`docs/options-reference.md`](options-reference.md) for every field's constraints) — this is a caller bug, not a transient failure; fix the offending value rather than retrying. |
| `TERRAIN_GENERATION_FAILED` | `exportHeightmap()` was called before any terrain was generated or loaded. |
| `DEM_FETCH_FAILED` | `loadDemFromUrl()`'s underlying `fetch()` failed or returned a non-OK status (retryable, unlike `OPTIONS_INVALID`), or the response is longer than `maxBytes` (80 MiB, `MAX_DEM_BYTES`, by default and at most): refused unread when its `Content-Length` says so, and cancelled as soon as it passes the limit otherwise. |
| `DEM_FORMAT_UNSUPPORTED` | The GeoTIFF is outside VistaWASM's supported subset (compressed, multi-band, tiled, or an otherwise unsupported format) — see [`docs/terrain-data.md`](terrain-data.md#exactly-what-is-supported) — or longer than `MAX_DEM_BYTES`, refused before it is copied. |
| `DEM_METADATA_MISSING` | Required GeoTIFF tags were missing or contradictory. |
| `WASM_LOAD_FAILED` | `initialiseVistaWasm()` or the first `createVistaEngine()` could not fetch, compile or start the WASM module or its JavaScript glue. The message names the module URL and the browser's own error, which is kept in `.details`. A failed load is not cached: call again, for example with a corrected `wasmUrl` or `moduleUrl`. |
| `INVALID_DEM` | An imported image, PNG or bundle is malformed: a bad signature or CRC, a size over 2048 for a heightmap image, a painted map or a bundle (8192 for other images), data that inflates past its header, a damaged zip or manifest. The message says what is wrong — see [`docs/import.md`](import.md#untrusted-files). |
| `GPU_LIMIT_EXCEEDED` | A GPU job would need a buffer past the device's limits. The engine asks for the largest buffers the adapter offers. Only GPU erosion could exceed them, and at 16 bytes a sample it needs 64 MiB at 2048, the largest terrain, within every WebGPU device's limits; were it to need more, it would run on the CPU instead, with this error's message as a `"warning"`, so it is not thrown. Instance counts above the limits are clamped. |
| `GPU_ERROR` | The GPU reported a validation or out-of-memory error while the engine created its resources (`createVistaEngine()` rejects) or eroded a terrain on the GPU (the terrain call rejects, and the engine stays as it was). Errors while a terrain's resources are uploaded, and any the engine's own error scopes did not catch, arrive as `"gpuError"` events with this code instead: waiting for the GPU to confirm the upload would hold every terrain call back. |
| `ENGINE_DISPOSED` | A call was made on an engine after `dispose()`, or after an internal error stopped it (see [After an internal error](#after-an-internal-error)). Guard against this in your own code if you hold a reference to the engine outside the component/module that owns its lifecycle. |
| `INTERNAL_ERROR` | An unexpected internal fault, or any non-VistaWASM error the wrapper caught and normalised (the original error, when there is one, is kept in `.details`). |

## Failed calls

A call that fails leaves the engine as it was, and usable:

- A terrain call (`generateFractal()`, `loadDemFromArrayBuffer()`,
    `loadDemFromUrl()`, `loadRawHeightmap()`) that rejects keeps the
    terrain, options and painted maps it had before, and the engine is
    ready for the next call.
- `loadBundle()` checks the bundle's options before it changes anything,
    loads its heights, and only then applies its settings. If any step
    fails, the settings it had applied are put back and the terrain is
    the one it had before.
- Inputs that could exhaust memory are refused before anything is
    allocated for them: see [`docs/security.md`](security.md#limits).

## Unknown option keys

Every options object is copied once, with `structuredClone`, and checked
and used from that copy, so a getter cannot pass the check with one value
and reach the engine with another; functions are refused. A key the
engine does not know, such as a misspelt `seaLevelMeters`, is taken out,
and a `"warning"` names it and the closest valid key:

```text
setWater() option "seaLevelMeters" is not one the engine reads, so it was ignored. Did you mean "seaLevelMetres"? Unknown options become errors in the next major version.
```

Unknown keys are warnings in this release and become `OPTIONS_INVALID`
errors in the next major version. A raw heightmap buffer longer than its
options need is read in part, with a warning; it will be an error then
too.

## After an internal error

Release builds of the engine stop on an internal fault (a Rust panic
becomes a WebAssembly trap), which leaves the WebAssembly instance
unusable. The wrapper then marks the engine dead and emits
`"fatalError"` once, with an `INTERNAL_ERROR` whose message names the
trap. The call that met the trap throws that error, and an async call
whose promise the trap left unsettled rejects with it. Every later call
throws `ENGINE_DISPOSED`. Dispose of the engine (which does not call into
the dead instance) and create a new one. Internal errors are bugs: please
report them, with the message.

## Frame pacing

Browsers keep firing animation frames on schedule even when the GPU cannot
keep up. If every one of them submitted a frame, frames would queue up
behind the GPU without limit, and the picture would fall seconds behind
the camera while the frame rate still looked high. So `renderOnce()`
draws nothing while two earlier frames are still on the GPU: it returns
the previous `RenderStats` (with the same `frameIndex`) and emits no
`"stats"` event. The engine then draws exactly as fast as the GPU
finishes frames, and the picture is never more than two frames behind.

`start()` also caps the frame rate at `RenderQualityOptions.maxFrameRate`
(default 60; `0` for uncapped). It renders evenly spaced animation frames,
so a 60 cap on a 144 Hz display gives a steady 60. A steady 60 looks
smoother than a rate that swings between 50 and 144. If you call
`renderOnce()` from your own loop, the cap does not apply; pace it
yourself.

Animation (wind, water, clouds, weather) advances by a smoothed time
step, so one late frame does not make the scene jump. A gap longer than
0.2 s, such as a background tab, is treated as one normal frame.

To measure the real frame rate, time the gap between `"stats"` events
(see [`docs/render-quality-and-diagnostics.md`](render-quality-and-diagnostics.md#render-statistics-renderstats)).

## Reentrancy

`wasm-bindgen` forbids any call reaching a given exported object while an
`async fn(&mut self)` method on that same object has not yet resolved.
VistaWASM's async methods (`generateFractal`, `loadDemFromArrayBuffer`,
`loadDemFromUrl`, `loadRawHeightmap`) can now span multiple animation
frames (GPU erosion readback is a genuine multi-frame async gap), so this
is reachable in practice — a camera-control loop calling `setCamera()`, or
a `ResizeObserver` calling `resize()`, while terrain generation is still
awaiting erosion, would trip it if unguarded.

The TypeScript wrapper guards against this so you never need to think
about it directly:

- Every synchronous setter (`setCamera`, `setSun`, `setAtmosphere`,
  `setWater`, `setFlora`, `setGrass`, `setClouds`, `setMist`,
  `setWeather`, `setShadows`, `setSurface`, `setRenderQuality`,
  `setBiomes`, `setDebugView`, `resize`) silently **no-ops** while an
  async call is in flight, rather than throwing or queuing.
  `getWeather()` and `biomeAt()` return `undefined`, and
  `temperatureAt()` returns `null`.
- The replacement hooks (`setTreeModel`, `resetTreeModel`,
  `setTreeInstances`, `replaceTexture`, `resetTextures`, `setWaterMask`,
  `setBiomeMap`, `setVegetationMasks`) **throw** a
  `VistaWasmError` instead, because silently dropping a one-off asset
  change would leave the scene wrong. Await the terrain call first.
- `renderOnce()` returns the last real `RenderStats` during that window
  instead of calling into the busy engine.
- `exportHeightmap()` throws a clear, catchable `VistaWasmError` instead of
  silently reading stale data.
- `exportMap()` and `exportTrees()` throw an `INTERNAL_ERROR`
  `VistaWasmError` for the same reason. `getOptionsSnapshot()` works
  throughout, as it does not call into the engine.
- `exportSnapshot()` is unaffected — it reads the canvas directly, not the
  WASM engine (see [`docs/export-and-snapshots.md`](export-and-snapshots.md#canvas-screenshot)).
- Overlapping async calls (for example, calling `generateFractal()` again
  before a previous call resolves) queue sequentially rather than racing.
- `dispose()` marks the wrapper disposed immediately (so no new calls
  start) but defers the real teardown until any in-flight call settles.

This means a camera controller, resize handler, or settings panel can call
its `set*()` methods every frame/on every change without any special-casing
around terrain generation — see
[`docs/game-development.md`](game-development.md#5-performance-and-async-work)
for the practical implications, and
[`docs/architecture.md`](architecture.md#lifecycle) for the full mechanism
(the `pendingCall` mutex) if you are contributing to VistaWASM itself.
