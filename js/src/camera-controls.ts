import type { CameraOptions } from "./types.js";

/**
 * Options for {@link attachFlyCameraControls}.
 */
export interface FlyCameraControlsOptions {
  /**
   * Starting camera position in terrain metres.
   */
  initialPosition?: [number, number, number];

  /**
   * Starting yaw in degrees. Zero looks along the positive Z axis.
   */
  initialYawDegrees?: number;

  /**
   * Starting pitch in degrees. Positive values look upward.
   */
  initialPitchDegrees?: number;

  /**
   * Field of view passed through to every camera update.
   */
  fieldOfViewDegrees?: number;

  /**
   * Near plane passed through to every camera update.
   */
  nearMetres?: number;

  /**
   * Far plane passed through to every camera update.
   */
  farMetres?: number;

  /**
   * Starting move speed in metres per second.
   */
  moveSpeedMetresPerSecond?: number;

  /**
   * Minimum field of view reachable with the scroll wheel, in degrees.
   */
  minFieldOfViewDegrees?: number;

  /**
   * Maximum field of view reachable with the scroll wheel, in degrees.
   */
  maxFieldOfViewDegrees?: number;

  /**
   * Zoom sensitivity in degrees of field of view per scroll wheel unit.
   */
  zoomSensitivity?: number;

  /**
   * Vertical pan sensitivity in metres per pixel of pointer movement while
   * the middle mouse button is held.
   */
  verticalPanSensitivity?: number;

  /**
   * Mouse look sensitivity in degrees per pixel of pointer movement.
   */
  lookSensitivity?: number;

  /**
   * Keyboard look speed in degrees per second when using the arrow keys.
   */
  keyboardLookDegreesPerSecond?: number;

  /**
   * Called every time the controller applies a new camera. Useful for
   * driving a minimap marker without polling {@link FlyCameraControls.getCamera}.
   */
  onCameraChange?: (camera: CameraOptions) => void;
}

/**
 * Handle returned by {@link attachFlyCameraControls}.
 */
export interface FlyCameraControls {
  /**
   * Return the camera options currently being applied to the engine.
   */
  getCamera(): CameraOptions;

  /**
   * Return the current move speed in metres per second.
   */
  getMoveSpeed(): number;

  /**
   * Return the current field of view in degrees.
   */
  getFieldOfView(): number;

  /**
   * Teleport the controller to a new position without changing look
   * direction.
   */
  setPosition(position: [number, number, number]): void;

  /**
   * Remove every event listener and stop the internal animation loop.
   */
  dispose(): void;
}

/**
 * Minimal engine shape required to drive the camera. Matches
 * `VistaEngine.setCamera`.
 */
export interface FlyCameraEngine {
  setCamera(camera: CameraOptions): void;
}

const DEFAULT_MOVE_SPEED = 60;
const DEFAULT_MIN_FIELD_OF_VIEW_DEGREES = 20;
const DEFAULT_MAX_FIELD_OF_VIEW_DEGREES = 100;
const DEFAULT_ZOOM_SENSITIVITY = 0.05;
const DEFAULT_VERTICAL_PAN_SENSITIVITY = 1.5;
const DEFAULT_LOOK_SENSITIVITY = 0.12;
const DEFAULT_KEYBOARD_LOOK_DEGREES_PER_SECOND = 90;
const MAX_PITCH_DEGREES = 89;

const FORWARD_KEYS = new Set(["KeyW", "ArrowUp"]);
const BACKWARD_KEYS = new Set(["KeyS", "ArrowDown"]);
const LEFT_KEYS = new Set(["KeyA"]);
const RIGHT_KEYS = new Set(["KeyD"]);
const UP_KEYS = new Set(["Space", "KeyE"]);
const DOWN_KEYS = new Set(["ShiftLeft", "ShiftRight", "KeyQ"]);
const LOOK_LEFT_KEYS = new Set(["ArrowLeft"]);
const LOOK_RIGHT_KEYS = new Set(["ArrowRight"]);

/**
 * Attach WASD-plus-mouse fly camera controls to a VistaWASM engine.
 *
 * The controller keeps its own position, yaw, and pitch state and calls
 * `engine.setCamera()` from an internal animation loop. Call the returned
 * `dispose()` function when the canvas is removed or controls should stop.
 *
 * Controls:
 * - Drag with the primary (left) pointer button to look around.
 * - `W`/`A`/`S`/`D` to move, or up and down arrows to move forwards and backwards.
 * - Left and right arrows to turn.
 * - `Space`/`E` to rise, `Shift`/`Q` to descend.
 * - Hold the middle mouse button and drag up or down to rise or descend.
 * - Scroll wheel to zoom in and out by adjusting the field of view.
 *
 * Keyboard movement is ignored while an `<input>`, `<textarea>`, `<select>`,
 * or `contenteditable` element has focus, so the controls do not fight with
 * a host application's own form fields.
 */
export function attachFlyCameraControls(
  engine: FlyCameraEngine,
  canvas: HTMLCanvasElement,
  options: FlyCameraControlsOptions = {}
): FlyCameraControls {
  const position: [number, number, number] = options.initialPosition
    ? [...options.initialPosition]
    : [0, 200, 0];
  let yawDegrees = options.initialYawDegrees ?? 0;
  let pitchDegrees = options.initialPitchDegrees ?? -10;
  const moveSpeed = options.moveSpeedMetresPerSecond ?? DEFAULT_MOVE_SPEED;
  let fieldOfViewDegrees = options.fieldOfViewDegrees ?? 60;
  const minFieldOfViewDegrees = options.minFieldOfViewDegrees ?? DEFAULT_MIN_FIELD_OF_VIEW_DEGREES;
  const maxFieldOfViewDegrees = options.maxFieldOfViewDegrees ?? DEFAULT_MAX_FIELD_OF_VIEW_DEGREES;
  const zoomSensitivity = options.zoomSensitivity ?? DEFAULT_ZOOM_SENSITIVITY;
  const verticalPanSensitivity =
    options.verticalPanSensitivity ?? DEFAULT_VERTICAL_PAN_SENSITIVITY;
  const lookSensitivity = options.lookSensitivity ?? DEFAULT_LOOK_SENSITIVITY;
  const keyboardLookDegreesPerSecond =
    options.keyboardLookDegreesPerSecond ?? DEFAULT_KEYBOARD_LOOK_DEGREES_PER_SECOND;
  const nearMetres = options.nearMetres;
  const farMetres = options.farMetres;

  const pressedKeys = new Set<string>();
  let lookDragging = false;
  let panDragging = false;
  let disposed = false;
  let animationFrameId: number | null = null;
  let lastFrameTimeMs: number | null = null;
  let currentCamera = buildCamera();

  function buildCamera(): CameraOptions {
    const yawRadians = (yawDegrees * Math.PI) / 180;
    const pitchRadians = (pitchDegrees * Math.PI) / 180;
    const forward = computeForwardVector(yawRadians, pitchRadians);

    return {
      position: [...position],
      target: [
        position[0] + forward[0],
        position[1] + forward[1],
        position[2] + forward[2]
      ],
      fieldOfViewDegrees,
      nearMetres,
      farMetres,
      allowUnderground: true
    };
  }

  function apply(): void {
    currentCamera = buildCamera();
    engine.setCamera(currentCamera);
    options.onCameraChange?.(currentCamera);
  }

  function isFormFieldFocused(): boolean {
    const active = document.activeElement;

    if (!active) {
      return false;
    }

    const tag = active.tagName;
    return (
      tag === "INPUT" ||
      tag === "TEXTAREA" ||
      tag === "SELECT" ||
      (active instanceof HTMLElement && active.isContentEditable)
    );
  }

  function onKeyDown(event: KeyboardEvent): void {
    if (isFormFieldFocused()) {
      return;
    }

    pressedKeys.add(event.code);
  }

  function onKeyUp(event: KeyboardEvent): void {
    pressedKeys.delete(event.code);
  }

  function onWindowBlur(): void {
    pressedKeys.clear();
    lookDragging = false;
    panDragging = false;
  }

  function onPointerDown(event: PointerEvent): void {
    if (event.button === 0) {
      lookDragging = true;
      canvas.setPointerCapture(event.pointerId);
      return;
    }

    if (event.button === 1) {
      event.preventDefault();
      panDragging = true;
      canvas.setPointerCapture(event.pointerId);
    }
  }

  function onPointerMove(event: PointerEvent): void {
    if (lookDragging) {
      yawDegrees -= event.movementX * lookSensitivity;
      pitchDegrees = clamp(
        pitchDegrees - event.movementY * lookSensitivity,
        -MAX_PITCH_DEGREES,
        MAX_PITCH_DEGREES
      );
    }

    if (panDragging) {
      position[1] -= event.movementY * verticalPanSensitivity;
    }
  }

  function onPointerUp(event: PointerEvent): void {
    if (event.button === 0) {
      lookDragging = false;
    }

    if (event.button === 1) {
      panDragging = false;
    }

    if (!lookDragging && !panDragging && canvas.hasPointerCapture(event.pointerId)) {
      canvas.releasePointerCapture(event.pointerId);
    }
  }

  function onWheel(event: WheelEvent): void {
    event.preventDefault();
    fieldOfViewDegrees = clamp(
      fieldOfViewDegrees + event.deltaY * zoomSensitivity,
      minFieldOfViewDegrees,
      maxFieldOfViewDegrees
    );
  }

  function onContextMenu(event: Event): void {
    event.preventDefault();
  }

  function tick(nowMs: number): void {
    if (disposed) {
      return;
    }

    const deltaSeconds = lastFrameTimeMs === null ? 0 : (nowMs - lastFrameTimeMs) / 1000;
    lastFrameTimeMs = nowMs;

    updateFromKeyboard(Math.min(deltaSeconds, 0.25));
    apply();

    animationFrameId = window.requestAnimationFrame(tick);
  }

  function updateFromKeyboard(deltaSeconds: number): void {
    if (deltaSeconds <= 0) {
      return;
    }

    const yawRadians = (yawDegrees * Math.PI) / 180;
    const forward = computeForwardVector(yawRadians, 0);
    const right = computeRightVector(yawRadians);
    const distance = moveSpeed * deltaSeconds;

    let moveX = 0;
    let moveY = 0;
    let moveZ = 0;

    for (const key of pressedKeys) {
      if (FORWARD_KEYS.has(key)) {
        moveX += forward[0];
        moveZ += forward[2];
      }

      if (BACKWARD_KEYS.has(key)) {
        moveX -= forward[0];
        moveZ -= forward[2];
      }

      if (RIGHT_KEYS.has(key)) {
        moveX += right[0];
        moveZ += right[2];
      }

      if (LEFT_KEYS.has(key)) {
        moveX -= right[0];
        moveZ -= right[2];
      }

      if (UP_KEYS.has(key)) {
        moveY += 1;
      }

      if (DOWN_KEYS.has(key)) {
        moveY -= 1;
      }

      if (LOOK_LEFT_KEYS.has(key)) {
        yawDegrees += keyboardLookDegreesPerSecond * deltaSeconds;
      }

      if (LOOK_RIGHT_KEYS.has(key)) {
        yawDegrees -= keyboardLookDegreesPerSecond * deltaSeconds;
      }
    }

    const horizontalLength = Math.hypot(moveX, moveZ);

    if (horizontalLength > 0) {
      position[0] += (moveX / horizontalLength) * distance;
      position[2] += (moveZ / horizontalLength) * distance;
    }

    if (moveY !== 0) {
      position[1] += Math.sign(moveY) * distance;
    }
  }

  function onPointerCancel(event: PointerEvent): void {
    lookDragging = false;
    panDragging = false;

    if (canvas.hasPointerCapture(event.pointerId)) {
      canvas.releasePointerCapture(event.pointerId);
    }
  }

  canvas.addEventListener("pointerdown", onPointerDown);
  canvas.addEventListener("pointermove", onPointerMove);
  canvas.addEventListener("pointerup", onPointerUp);
  canvas.addEventListener("pointercancel", onPointerCancel);
  canvas.addEventListener("wheel", onWheel, { passive: false });
  canvas.addEventListener("contextmenu", onContextMenu);
  window.addEventListener("keydown", onKeyDown);
  window.addEventListener("keyup", onKeyUp);
  window.addEventListener("blur", onWindowBlur);

  apply();
  animationFrameId = window.requestAnimationFrame(tick);

  return {
    getCamera(): CameraOptions {
      return currentCamera;
    },
    getMoveSpeed(): number {
      return moveSpeed;
    },
    getFieldOfView(): number {
      return fieldOfViewDegrees;
    },
    setPosition(next: [number, number, number]): void {
      position[0] = next[0];
      position[1] = next[1];
      position[2] = next[2];
      apply();
    },
    dispose(): void {
      if (disposed) {
        return;
      }

      disposed = true;

      if (animationFrameId !== null) {
        window.cancelAnimationFrame(animationFrameId);
      }

      canvas.removeEventListener("pointerdown", onPointerDown);
      canvas.removeEventListener("pointermove", onPointerMove);
      canvas.removeEventListener("pointerup", onPointerUp);
      canvas.removeEventListener("pointercancel", onPointerCancel);
      canvas.removeEventListener("wheel", onWheel);
      canvas.removeEventListener("contextmenu", onContextMenu);
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("keyup", onKeyUp);
      window.removeEventListener("blur", onWindowBlur);
    }
  };
}

/**
 * Clamp a value to an inclusive range.
 */
function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), max);
}

/**
 * Return the unit forward vector for a yaw/pitch pair, in radians.
 *
 * Matches the `side = cross(forward, up)` convention used by the Rust
 * camera projector, so strafing lines up with what the renderer draws.
 */
export function computeForwardVector(
  yawRadians: number,
  pitchRadians: number
): [number, number, number] {
  return [
    Math.cos(pitchRadians) * Math.sin(yawRadians),
    Math.sin(pitchRadians),
    Math.cos(pitchRadians) * Math.cos(yawRadians)
  ];
}

/**
 * Return the unit rightward vector for a yaw, in radians, using
 * `cross(forward, worldUp)`.
 */
export function computeRightVector(yawRadians: number): [number, number, number] {
  const forward = computeForwardVector(yawRadians, 0);
  const upX = 0;
  const upY = 1;
  const upZ = 0;

  const rightX = forward[1] * upZ - forward[2] * upY;
  const rightY = forward[2] * upX - forward[0] * upZ;
  const rightZ = forward[0] * upY - forward[1] * upX;
  const length = Math.hypot(rightX, rightY, rightZ) || 1;

  return [rightX / length, rightY / length, rightZ / length];
}
