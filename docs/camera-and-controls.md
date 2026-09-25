# Camera and Controls

VistaWASM's camera is a plain look-at/perspective projector with no physics,
no collision, and no built-in player controller. This document covers the
camera model itself and the bundled fly-camera controller. For *using* a
camera inside a real game (player-relative cameras, height queries), see
[`docs/game-development.md`](game-development.md#2-player-movement-and-camera-control).

## The camera model

`CameraOptions` (see
[`docs/options-reference.md`](options-reference.md#cameraoptions)) is a
plain look-at camera:

- `position` and `target` define the view direction; VistaWASM builds a
  right-handed look-at matrix from them using a **fixed world-up vector**
  `[0, 1, 0]`. There is no way to roll the camera around its own view axis
  today, and no way to look straight up or down without the look-at math
  becoming degenerate (extremely close to straight up/down, `position` to
  `target` becomes parallel to the up vector) — keep pitch within roughly
  ±89° for stable results, which is exactly what `attachFlyCameraControls()`
  already clamps to internally.
- `fieldOfViewDegrees` sets the vertical field of view; `nearMetres`/
  `farMetres` set the projection's clip planes.
- `rollDegrees`, `minimumHeightAboveTerrainMetres`, and `allowUnderground`
  are accepted by the type and stored on the engine, but **none of them are
  currently applied** — there is no camera roll, and nothing clamps the
  camera's height or prevents it going below the terrain. This is a real
  gap in the current implementation, not a subtlety of how to use the
  fields correctly. See [Terrain clamping is not built
  in](#terrain-clamping-is-not-built-in) below for the workaround.

Calling `engine.setCamera(camera)` is synchronous, cheap, and safe to call
every frame — it recomputes the view/projection matrices immediately. It is
silently skipped (not queued, not thrown) while an async call like
`generateFractal()` is in flight; see
[`docs/events-errors-and-lifecycle.md`](events-errors-and-lifecycle.md#reentrancy)
for why.

## `attachFlyCameraControls()`

A complete, reusable WASD + mouse-look + scroll-zoom + middle-drag-pan
controller, exported from the package root. Good for spectator/photo-mode
cameras, level-review tools, and free-fly debugging — most real games
should write their own player-relative controller instead (see
[`docs/game-development.md`](game-development.md#2-player-movement-and-camera-control)).

```ts
import { attachFlyCameraControls } from "@vista-wasm/vista-wasm";

const controls = attachFlyCameraControls(engine, canvas, {
  initialPosition: [0, 400, 900],
  initialYawDegrees: 180,
  initialPitchDegrees: -12,
  moveSpeedMetresPerSecond: 120,
  onCameraChange: (camera) => updateMinimapMarker(camera)
});

// Later:
controls.dispose();
```

### Controls

- Drag with the primary (left) pointer button to look around.
- `W`/`A`/`S`/`D` to move, or `↑`/`↓` to move forwards and backwards.
- `←`/`→` to turn.
- `Space`/`E` to rise, `Shift`/`Q` to descend.
- Hold the middle mouse button and drag up or down to rise or descend.
- Scroll wheel to zoom in/out by adjusting the field of view (not by moving
  the camera).

Keyboard movement is ignored while an `<input>`, `<textarea>`, `<select>`,
or `contenteditable` element has focus, so the controls do not fight with a
host application's own form fields — you do not need to disable the
controller while a settings panel is focused.

### `FlyCameraControlsOptions`

| Field | Default | Notes |
| --- | --- | --- |
| `initialPosition` | `[0, 200, 0]` | |
| `initialYawDegrees` | `0` | Zero looks along the positive Z axis. |
| `initialPitchDegrees` | `-10` | Positive looks upward. Internally clamped to ±89°. |
| `fieldOfViewDegrees` | `60` | Starting field of view; changes as the scroll wheel zooms. |
| `nearMetres` / `farMetres` | engine defaults | Passed through to every camera update. |
| `moveSpeedMetresPerSecond` | `60` | |
| `minFieldOfViewDegrees` / `maxFieldOfViewDegrees` | `20` / `100` | Scroll-zoom range. |
| `zoomSensitivity` | `0.05` | Degrees of field of view per scroll wheel unit. |
| `verticalPanSensitivity` | `1.5` | Metres per pixel while middle-dragging. |
| `lookSensitivity` | `0.12` | Degrees per pixel while primary-dragging. |
| `keyboardLookDegreesPerSecond` | `90` | Turning speed for the `←` and `→` keys. |
| `onCameraChange` | none | Called with the new camera on every animation frame while the controller is attached, whether or not it moved. Use it to keep a minimap marker, a HUD compass, or another renderer's camera in step. |

The returned `FlyCameraControls` handle exposes `getCamera()`,
`getMoveSpeed()`, `getFieldOfView()`, `setPosition(position)` (teleport
without changing look direction), `lookAt(target)` (turn to face a world
position without moving), and `dispose()` (removes every event
listener and stops the internal animation loop — always call this when the
canvas is removed or controls should stop, to avoid leaking event
listeners).

`attachFlyCameraControls()` always passes `allowUnderground: true`
internally, since VistaWASM does not enforce that field anyway (see above).

## Writing your own controller

For anything player-relative (third-person character, vehicle, fixed-height
walker), don't use `attachFlyCameraControls()` — read its source
(`js/src/camera-controls.ts`) as a template instead, and write a controller
that owns your game's player/vehicle state and computes a `CameraOptions`
from it each frame. The full pattern, including how this interacts with
async terrain generation, is in
[`docs/game-development.md`](game-development.md#2-player-movement-and-camera-control).

## Terrain clamping is not built in

Since `minimumHeightAboveTerrainMetres`/`allowUnderground` are not enforced,
a "keep the camera above the ground" clamp is something you compute
yourself, using an exported heightmap:

```ts
import { readHeightmapFloats } from "@vista-wasm/vista-wasm";

const handle = await engine.generateFractal(options);
const heights = readHeightmapFloats(engine.exportHeightmap());

function clampAboveTerrain(
  position: [number, number, number],
  minimumHeightAboveTerrainMetres: number
): [number, number, number] {
  const halfWidth = (handle.metadata.width - 1) * 0.5;
  const halfHeight = (handle.metadata.height - 1) * 0.5;
  const sampleX = Math.round(position[0] / handle.metadata.metresPerSample + halfWidth);
  const sampleZ = Math.round(position[2] / handle.metadata.metresPerSample + halfHeight);
  const clampedX = Math.min(handle.metadata.width - 1, Math.max(0, sampleX));
  const clampedZ = Math.min(handle.metadata.height - 1, Math.max(0, sampleZ));
  const groundHeight = heights[clampedZ * handle.metadata.width + clampedX];
  const minimumY = groundHeight + minimumHeightAboveTerrainMetres;

  return [position[0], Math.max(position[1], minimumY), position[2]];
}
```

Call this on your own computed camera position before passing it to
`engine.setCamera()`. See
[`docs/game-development.md`](game-development.md#querying-terrain-height-for-gameplay)
for the same technique applied to gameplay height queries generally
(bilinear sampling, re-exporting after terrain changes, and so on).
