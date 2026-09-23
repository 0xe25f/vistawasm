# Building Games With VistaWASM

VistaWASM is a terrain **engine**, not a game engine: it owns one WebGPU
canvas, a heightfield, a camera, and a handful of environment systems (sun,
atmosphere, water, flora). Everything else — game state, entities, UI,
audio, save games — belongs to your application. This guide covers the
patterns that come up repeatedly when VistaWASM is the terrain/world layer
underneath a game.

## 1. Where VistaWASM fits in a game loop

VistaWASM already runs its own `requestAnimationFrame` loop once you call
`engine.start()`. If your game has its own loop (for entity updates, physics,
input polling), you have two options:

**Option A — let VistaWASM drive rendering, hook in with events.**
Call `engine.start()` once and drive your own game logic from a separate
`requestAnimationFrame` loop, or from the `"stats"` event which fires once per
rendered frame:

```ts
engine.on("stats", (stats) => {
  updateGameState(stats.frameTimeMs);
});
engine.start();
```

This is the simplest option and is what the demo and every example use.

**Option B — drive rendering yourself.**
Never call `engine.start()`. Instead call `engine.renderOnce()` from your own
loop, after updating game state and the camera for that frame:

```ts
function tick(nowMs: number) {
  updateGameState(nowMs);
  engine.setCamera(computeCameraFromPlayerState());
  const stats = engine.renderOnce();
  handleStats(stats);
  requestAnimationFrame(tick);
}
requestAnimationFrame(tick);
```

Option B is preferable once you have a real player/vehicle controller, since
it guarantees the camera you set is the one used for that exact frame,
rather than racing against VistaWASM's own internal loop.

Do **not** run both `engine.start()` and your own `renderOnce()` loop at the
same time — you will render two frames' worth of work per animation frame
for no benefit.

## 2. Player movement and camera control

`attachFlyCameraControls()` (exported from the package root) is a complete,
reusable WASD + mouse-look + scroll-zoom + middle-drag-pan controller. It is
good for spectator/photo-mode cameras, level review tools, and free-fly
debugging, but most games want player-relative movement instead (e.g. a
third-person character, a vehicle, or a fixed-height first-person walker).
For those, don't use `attachFlyCameraControls()` — instead read its source
(`js/src/camera-controls.ts`) as a template and write your own controller
that:

1. Owns your game's player/vehicle position and orientation (not the
    camera directly).
2. Computes a `CameraOptions` from that state each frame (e.g. an
    over-the-shoulder offset behind the player, or an eye-height first-person
    position).
3. Calls `engine.setCamera(camera)` once per frame — cheap, synchronous, and
    safe to call every frame even during terrain generation (VistaWASM
    silently skips the call while an async operation like terrain generation
    is in flight, rather than throwing; see [Section 5](#5-performance-and-async-work)).

`CameraOptions.minimumHeightAboveTerrainMetres` and `allowUnderground` are
accepted by the API but **not currently enforced by the engine** — nothing
clamps the camera against the terrain today, for the player camera or
anything else. If you need a "don't let the camera clip into the ground"
clamp, compute it yourself from an exported heightmap (see the next
section) and adjust `position.y` before calling `setCamera()`. See
[`docs/camera-and-controls.md`](camera-and-controls.md#terrain-clamping-is-not-built-in)
for a worked example.

### Querying terrain height for gameplay

VistaWASM does not expose a live "sample height at (x, z)" API. For
gameplay code that needs ground height (placing a character, snapping props
to terrain, simple line-of-sight checks), export the heightmap once after
generation and query it yourself:

```ts
import { readHeightmapFloats } from "@vista-wasm/vista-wasm";

const handle = await engine.generateFractal(options);
const bytes = engine.exportHeightmap();
const heights = readHeightmapFloats(bytes); // Float32Array, row-major

function heightAt(sampleX: number, sampleZ: number): number {
  const x = Math.min(handle.metadata.width - 1, Math.max(0, Math.round(sampleX)));
  const z = Math.min(handle.metadata.height - 1, Math.max(0, Math.round(sampleZ)));
  return heights[z * handle.metadata.width + x];
}
```

Convert a world-space position to sample coordinates with
`metresPerSample` and the same terrain-centred convention VistaWASM uses
internally:

```ts
const halfWidth = (handle.metadata.width - 1) * 0.5;
const halfHeight = (handle.metadata.height - 1) * 0.5;
const sampleX = worldX / handle.metadata.metresPerSample + halfWidth;
const sampleZ = worldZ / handle.metadata.metresPerSample + halfHeight;
```

Re-export the heightmap any time terrain changes (new seed, reloaded DEM).
For bilinear-smoothed height queries (recommended for camera collision so it
doesn't step at sample boundaries), sample the four nearest cells and lerp.

## 3. Collision and physics

VistaWASM does not run physics and does not expose a collision API. Two
practical approaches, depending on how much of the terrain you need
colliders for:

- **Heightfield collider (most physics engines support this natively).**
  Export the heightmap once with `exportHeightmap()`, read it with
  `readHeightmapFloats()` (row-major, row 0 at −z), and give it to your
  physics engine's heightfield/terrain collider, reordering it if the
  engine expects column-major data (Rapier's `ColliderDesc.heightfield`,
  Ammo.js's `btHeightfieldTerrainShape`, Cannon-es's `Heightfield`). This
  is exact, fast, and — because it's the same data VistaWASM itself
  renders from — never drifts out of sync with what the player sees.
- **Baked triangle mesh collider.** Use `exportTerrainObj()` to get a
  downsampled Wavefront OBJ mesh and load it as a static triangle-mesh
  collider. Prefer this only if your physics engine cannot do heightfields,
  since a triangle mesh is far more expensive to collide against than a
  heightfield of the same resolution.

Either way, generate the terrain and its collider **once**, then treat it as
static for the session — VistaWASM does not currently support editing
individual height samples at runtime (only whole-terrain regeneration or
reload).

## 4. Multiple terrains, streaming worlds, and world size

One `VistaEngine` instance owns exactly one canvas, one terrain, and one set
of environment settings. There is no built-in concept of terrain "tiles" or
streaming multiple terrains in and out. If your game needs a world larger
than a single heightmap comfortably supports:

- Prefer a single large `generateFractal({ size: 2048 | 4096 | 8192, ... })`
  or a single large DEM. VistaWASM's terrain mesh already recentres on the
  camera with distance-based level of detail (see
  [`docs/architecture.md`](architecture.md#terrain-rendering-and-level-of-detail)) —
  a single call, high vertex budget near the camera, coarser far away — so
  you rarely need to manage LOD yourself even for large worlds.
- For genuinely unbounded/procedural worlds (larger than any single
  heightmap should be), you will need your own tiling scheme above
  VistaWASM: generate a new terrain (new seed, offset shape parameters) when
  the player approaches its edge, and accept a load hitch or a soft
  transition (e.g. fade/fog) at the seam. VistaWASM does not do this for you.

## 5. Performance and async work

- `generateFractal()`, `loadDemFromUrl()`, `loadDemFromArrayBuffer()`, and
  `loadRawHeightmap()` are the only async engine calls. They take from
  tens of milliseconds for a small terrain to much longer for a large one
  with many erosion iterations. Show loading UI while the returned promise
  is pending, and disable "Generate" buttons/inputs until it settles. (The
  `"progress"` event only reports the start and end of `generateFractal()`,
  so it cannot drive a progress bar.)
- While an async call is in flight, sync setters (`setCamera`, `setSun`,
  `setAtmosphere`, `setWater`, `setFlora`, `setGrass`, `setClouds`,
  `setMist`, `setWeather`, `setShadows`, `setSurface`, `setBiomes`,
  `setRenderQuality`, `setDebugView`, `resize`) are silently skipped
  rather than queued or thrown; the replacement hooks throw instead (see
  [`docs/events-errors-and-lifecycle.md`](events-errors-and-lifecycle.md#reentrancy)) — this is intentional (see the
  reentrancy note in [`docs/architecture.md`](architecture.md#lifecycle))
  and means your camera controller can keep calling `setCamera()` every frame without any
  special-casing around terrain generation. `renderOnce()` keeps returning
  the last real `RenderStats` during this window rather than blocking.
- Trade quality for frame time feature by feature (`RenderQualityOptions.preset`
  has no effect in this release).
  `RenderQualityOptions.floraDensityScale` is the cheapest lever if flora/grass billboard
  fill-rate is your bottleneck — it scales both `FloraOptions.density` and
  `GrassOptions.density` together. `CloudsOptions.style: "volumetric"` and
  `MistOptions.style: "volumetric"` are the next things to turn off or
  down first if you need frame time back; both have cheaper
  `"painted"`/`"flat"` equivalents that look nearly as good for far less
  cost (see [`docs/sky-atmosphere-and-weather.md`](sky-atmosphere-and-weather.md)).
  After those, lower `CloudsOptions.resolutionScale`, the tree shadow map
  (`ShadowOptions.trees.resolution` and `distanceMetres`), or switch tree
  shadows off; terrain shadows are nearly free, since they are only
  recalculated when the sun or terrain changes (see
  [`docs/shadows.md`](shadows.md)).
- Weather is a gameplay tool too: `getWeather()` reports rain, snow,
  wetness, and wind every frame, and `"weatherChanged"` fires on each
  change, so gameplay (slippery roads, sound, NPC shelter) can follow the
  sky (see [`docs/weather.md`](weather.md)).
- Time the gap between `"stats"` events, and watch
  `terrainTriangles`/`floraInstances`/`grassInstances`, to build your own
  performance HUD or adaptive quality logic (for example, drop
  `floraDensityScale` if frames stay slow for a few seconds).
  `RenderStats.frameTimeMs` only covers the CPU side of a frame; see
  [`docs/render-quality-and-diagnostics.md`](render-quality-and-diagnostics.md#render-statistics-renderstats).

## 6. Error handling in a shipped game

Treat these as distinct cases, not one generic "something broke":

- `WEBGPU_UNAVAILABLE` / `WEBGPU_DEVICE_REQUEST_FAILED` at `createVistaEngine()`
  time — the player's browser or GPU can't run VistaWASM at all. Detect this
  *before* showing any VistaWASM UI with `detectVistaWasmSupport()` (exported
  from the package root) and show a static fallback (screenshot, video, or a
  "your browser doesn't support this" message) instead of a blank canvas.
- `"deviceLost"` event — the GPU device was lost after startup (driver
  reset, browser tab discarded and restored, laptop GPU switch). Treat this
  as fatal for the current engine instance: stop your game loop, dispose the
  engine, and offer the player a "reload" action. VistaWASM does not attempt
  automatic device recreation.
- `"fatalError"` event / thrown `VistaWasmError` — check `error.code`
  against the `VistaErrorCode` union and branch your handling (e.g.
  `DEM_FETCH_FAILED` is retryable, `OPTIONS_INVALID` is a bug in your own
  parameter construction and should be fixed, not retried).

## 7. Suggested project layout

For a game (as opposed to a one-off demo), keep VistaWASM concerns in one
module so the rest of your game never touches `VistaEngine` directly:

```text
src/
  world/
    terrain-service.ts   // wraps createVistaEngine, generateFractal, height queries
    world-camera.ts       // your player/vehicle-relative camera controller
  game/
    player.ts
    physics.ts             // heightfield collider built from terrain-service
  ui/
    loading-screen.tsx     // driven by terrain-service's "progress" events
```

This keeps the rest of the game decoupled from VistaWASM's API shape, which
matters if you later swap in a different terrain source (a fixed DEM for a
specific level, for example) without touching gameplay code.
