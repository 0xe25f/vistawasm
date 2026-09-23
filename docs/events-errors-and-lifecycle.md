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
| `"progress"` | `{ phase: string; progress: number }` | Twice per `generateFractal()` call: `progress: 0` when it starts, `progress: 1` when it resolves. **Not** emitted by `loadDemFromArrayBuffer`/`loadDemFromUrl`/`loadRawHeightmap` — those only emit `"terrainLoaded"` (and `"warning"`, below). |
| `"warning"` | `{ message: string; details?: unknown }` | Once per entry in `TerrainHandle.metadata.warnings`, after a successful `loadDemFromArrayBuffer`/`loadDemFromUrl`/`loadRawHeightmap`. **Not currently emitted after `generateFractal()`**, even though its returned handle has the same `warnings` field (fractal generation practically never produces warnings today). |
| `"terrainLoaded"` | `TerrainHandle` | After every successful `generateFractal`/`loadDemFromArrayBuffer`/`loadDemFromUrl`/`loadRawHeightmap` call. |
| `"stats"` | `RenderStats` | After every rendered frame, whether `start()` or your own loop calls `renderOnce()` (see [`docs/game-development.md`](game-development.md#1-where-vistawasm-fits-in-a-game-loop)). `renderOnce()` also returns the same `RenderStats`. Not emitted while terrain is generating, when `renderOnce()` returns the previous stats without drawing. |
| `"weatherChanged"` | `WeatherKind \| null` | When the dominant weather changes (halfway through a transition), and `null` when the weather system is switched off. Checked on every `renderOnce()`, whether called by `start()` or by you. See [`docs/weather.md`](weather.md#reading-the-weather). |
| `"fatalError"` | `Error` (a `VistaWasmError`) | From the `start()` loop's internal catch, for any error other than a lost device. The loop stops itself before emitting this. |
| `"deviceLost"` | `Error` (a `VistaWasmError` with code `WEBGPU_DEVICE_LOST`) | From the `start()` loop's internal catch, specifically for a lost GPU device. The loop stops itself before emitting this — VistaWASM does not attempt automatic device recreation; dispose and create a new engine. |

Note that `"fatalError"`/`"deviceLost"` are **only** raised through the
`start()` loop. If you drive rendering yourself via `renderOnce()`, catch
errors from that call directly instead — those events will not fire.

## Error codes

Engine errors are `VistaWasmError`s (`instanceof Error`, with a stable
`.code: VistaErrorCode` and a human-readable `.message`). The replacement
hooks (`setTreeModel()`, `setTreeInstances()`, `replaceTexture()`) throw a
plain `TypeError` for arguments of the wrong type or an unknown species,
before reaching the engine.

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
| `WEBGPU_DEVICE_LOST` | The active device was lost after startup (driver reset, tab discarded/restored, GPU switch). Only ever surfaced via the `"deviceLost"` event from the `start()` loop, or as a thrown error from your own `renderOnce()` call if you drive rendering yourself. |
| `CANVAS_INVALID` | The supplied canvas is missing, not an `HTMLCanvasElement`, or its WebGPU surface configuration failed. |
| `OPTIONS_INVALID` | A public option failed validation (see [`docs/options-reference.md`](options-reference.md) for every field's constraints) — this is a caller bug, not a transient failure; fix the offending value rather than retrying. |
| `TERRAIN_GENERATION_FAILED` | `exportHeightmap()` was called before any terrain was generated or loaded. |
| `DEM_FETCH_FAILED` | `loadDemFromUrl()`'s underlying `fetch()` failed or returned a non-OK status. Retryable (network issue), unlike `OPTIONS_INVALID`. |
| `DEM_FORMAT_UNSUPPORTED` | The GeoTIFF is outside VistaWASM's supported subset (compressed, multi-band, tiled, or an otherwise unsupported format) — see [`docs/terrain-data.md`](terrain-data.md#exactly-what-is-supported). |
| `DEM_METADATA_MISSING` | Required GeoTIFF tags were missing or contradictory. |
| `GPU_LIMIT_EXCEEDED` | Reserved. The current release does not raise it: instance counts above the device's limits are clamped instead. |
| `ENGINE_DISPOSED` | A call was made on an engine after `dispose()`. Guard against this in your own code if you hold a reference to the engine outside the component/module that owns its lifecycle. |
| `INTERNAL_ERROR` | An unexpected internal fault, or any non-VistaWASM error the wrapper caught and normalised (the original error, when there is one, is kept in `.details`). |

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
  `getWeather()` and `biomeAt()` return `undefined`.
- The replacement hooks (`setTreeModel`, `resetTreeModel`,
  `setTreeInstances`, `replaceTexture`, `resetTextures`) **throw** a
  `VistaWasmError` instead, because silently dropping a one-off asset
  change would leave the scene wrong. Await the terrain call first.
- `renderOnce()` returns the last real `RenderStats` during that window
  instead of calling into the busy engine.
- `exportHeightmap()` throws a clear, catchable `VistaWasmError` instead of
  silently reading stale data.
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
