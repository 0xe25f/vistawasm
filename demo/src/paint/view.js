// The 2D view of a paint document: a canvas one pixel a sample, shaded
// as hillshade times hypsometric colour with the painted layers tinted
// over it, and scaled and moved with a CSS transform so zooming and
// panning never redraw it. Pointer input becomes stroke points in sample
// coordinates.

/** The zoom range, in screen pixels a sample. */
export const ZOOM_LIMITS = [0.25, 8];

const LAKE = [40, 95, 200];
const RIVER = [60, 150, 230];
const MORE_TREES = [15, 95, 25];
const FEWER_TREES = [150, 100, 55];
const MORE_GRASS = [160, 215, 60];
const LESS_GRASS = [205, 185, 130];
// Hypsometric stops above the sea, as a fraction of the land's range.
const LAND = [
  [0, [96, 140, 78]],
  [0.25, [150, 162, 96]],
  [0.5, [158, 128, 88]],
  [0.78, [146, 140, 134]],
  [1, [246, 246, 246]]
];
const SHALLOW = [74, 138, 178];
const DEEP = [22, 52, 112];
const STEPS = 1024;

function mix(from, to, t) {
  return from + (to - from) * t;
}

/** The colour of each of `STEPS` heights across `range`, with the sea below `sea`. */
function colourTable(range, sea) {
  const table = new Uint8Array(STEPS * 3);
  const [low, high] = range;

  for (let step = 0; step < STEPS; step += 1) {
    const height = low + ((high - low) * step) / (STEPS - 1);
    let colour;

    if (height < sea) {
      const t = Math.min(1, (sea - height) / Math.max(1, sea - low));
      colour = SHALLOW.map((value, k) => mix(value, DEEP[k], t));
    } else {
      const t = Math.min(1, (height - sea) / Math.max(1, high - sea));
      const upper = LAND.findIndex(([at]) => at >= t);
      const [a, from] = LAND[Math.max(0, upper - 1)];
      const [b, to] = LAND[Math.max(0, upper)];
      colour = from.map((value, k) => mix(value, to[k], b > a ? (t - a) / (b - a) : 0));
    }

    table.set(colour.map(Math.round), step * 3);
  }

  return table;
}

/**
 * Shade `rect` of `doc` into `pixels`, a `Uint32Array` over the whole
 * document's RGBA bytes.
 * `options` holds `sun` (a unit vector: x east, y up, z south),
 * `overlays` (`{ biome, water, trees, grass }` booleans), `palette` (RGB
 * bytes a biome) and `table` (from `colourTable`).
 */
export function shade(doc, rect, pixels, options) {
  const { size, metresPerSample, seaLevelMetres, displayRange } = doc;
  const { height, biome, water, trees, grass } = doc.layers;
  const { sun, overlays, palette, table } = options;
  const [low, high] = displayRange;
  const toStep = (STEPS - 1) / Math.max(1e-6, high - low);
  const slope = 1 / (2 * metresPerSample);
  // Flat ground at full brightness, whatever the sun's height.
  const flat = 1 / Math.max(0.25, sun[1]);

  for (let y = rect.y0; y < rect.y1; y += 1) {
    let out = y * size + rect.x0;
    const up = (y > 0 ? y - 1 : 0) * size;
    const down = (y < size - 1 ? y + 1 : y) * size;

    for (let x = rect.x0; x < rect.x1; x += 1) {
      const at = y * size + x;
      const h = height[at];
      const nx = (height[y * size + (x > 0 ? x - 1 : 0)] - height[y * size + (x < size - 1 ? x + 1 : x)]) * slope;
      const nz = (height[up + x] - height[down + x]) * slope;
      const light = Math.max(0, (nx * sun[0] + sun[1] + nz * sun[2]) / Math.sqrt(nx * nx + 1 + nz * nz)) * flat;
      const lit = h < seaLevelMetres ? 0.85 + 0.15 * Math.min(light, 1) : 0.3 + 0.7 * Math.min(light, 1.4);
      const scaled = (h - low) * toStep + 0.5;
      const step = (scaled <= 0 ? 0 : scaled >= STEPS - 1 ? STEPS - 1 : scaled | 0) * 3;
      let r = table[step] * lit;
      let g = table[step + 1] * lit;
      let b = table[step + 2] * lit;

      if (overlays.biome && biome[at] !== 255) {
        const p = biome[at] * 3;
        r = mix(r, palette[p], 0.45);
        g = mix(g, palette[p + 1], 0.45);
        b = mix(b, palette[p + 2], 0.45);
      }

      if (overlays.trees && trees[at] !== 128) {
        const d = (trees[at] - 128) / 127;
        const [tr, tg, tb] = d > 0 ? MORE_TREES : FEWER_TREES;
        const t = Math.abs(d) * 0.55;
        r = mix(r, tr, t);
        g = mix(g, tg, t);
        b = mix(b, tb, t);
      }

      if (overlays.grass && grass[at] !== 128) {
        const d = (grass[at] - 128) / 127;
        const [gr, gg, gb] = d > 0 ? MORE_GRASS : LESS_GRASS;
        const t = Math.abs(d) * 0.45;
        r = mix(r, gr, t);
        g = mix(g, gg, t);
        b = mix(b, gb, t);
      }

      if (overlays.water && water[at] !== 0) {
        const lake = water[at] >= 128;
        const [wr, wg, wb] = lake ? LAKE : RIVER;
        const t = lake ? 0.8 : 0.55 + (0.35 * water[at]) / 127;
        r = mix(r, wr, t);
        g = mix(g, wg, t);
        b = mix(b, wb, t);
      }

      // Opaque RGBA packed little-endian, as every browser lays out ImageData.
      pixels[out] = 0xff000000 | ((b > 255 ? 255 : b) << 16) | ((g > 255 ? 255 : g) << 8) | (r > 255 ? 255 : r);
      out += 1;
    }
  }
}

/**
 * The view on `surface`, drawing into `canvas`, with `cursor` as the
 * brush outline. `palette` is RGB bytes a biome. The callbacks:
 * `onStroke({ type, points, pen })` with `type` `"start"`, `"move"`,
 * `"end"` or `"cancel"` and points `{ x, y, pressure }` in samples;
 * `onHover(point | null)`; `onZoom(scale)`.
 */
export function createPaintView({ surface, canvas, cursor, palette, onStroke, onHover, onZoom }) {
  const context = canvas.getContext("2d", { alpha: false });
  // The whole document's pixels, shaded in place and put back by region, so redraws allocate nothing.
  let frame = null;
  let framePixels = null;
  const pointers = new Map();
  const state = {
    doc: null,
    scale: 1,
    panX: 0,
    panY: 0,
    overlays: { biome: true, water: true, trees: true, grass: true },
    sun: [0.6, 0.5, 0.6],
    table: null,
    radius: 16,
    mode: "idle",
    panKey: false,
    painter: null,
    last: null,
    pinch: null,
    bounds: surface.getBoundingClientRect()
  };

  function applyTransform() {
    canvas.style.transform = `translate(${state.panX}px, ${state.panY}px) scale(${state.scale})`;
    canvas.classList.toggle("pixelated", state.scale >= 2);
  }

  function toSample(clientX, clientY) {
    return {
      x: (clientX - state.bounds.left - state.panX) / state.scale - 0.5,
      y: (clientY - state.bounds.top - state.panY) / state.scale - 0.5
    };
  }

  function moveCursor(clientX, clientY) {
    if (!state.doc || state.mode === "pan" || state.mode === "pinch") {
      cursor.hidden = true;
      return;
    }

    const diameter = Math.max(4, state.radius * 2 * state.scale);
    cursor.hidden = false;
    cursor.style.width = `${diameter}px`;
    cursor.style.height = `${diameter}px`;
    cursor.style.transform = `translate(${clientX - state.bounds.left - diameter / 2}px, ${clientY - state.bounds.top - diameter / 2}px)`;
  }

  function zoomAt(clientX, clientY, scale) {
    const next = Math.min(ZOOM_LIMITS[1], Math.max(ZOOM_LIMITS[0], scale));
    const x = clientX - state.bounds.left;
    const y = clientY - state.bounds.top;
    // Keep the sample under the pointer where it is.
    state.panX = x - ((x - state.panX) * next) / state.scale;
    state.panY = y - ((y - state.panY) * next) / state.scale;
    state.scale = next;
    applyTransform();
    onZoom(next);
  }

  function fit() {
    if (!state.doc) {
      return;
    }

    state.bounds = surface.getBoundingClientRect();
    const { width, height } = state.bounds;
    state.scale = Math.min(ZOOM_LIMITS[1], Math.max(ZOOM_LIMITS[0], (Math.min(width, height) * 0.94) / state.doc.size));
    state.panX = (width - state.doc.size * state.scale) / 2;
    state.panY = (height - state.doc.size * state.scale) / 2;
    applyTransform();
    onZoom(state.scale);
  }

  function draw(rect) {
    const { doc } = state;

    if (!doc || !rect) {
      return;
    }

    // Hillshade reads each sample's neighbours, so their pixels change too.
    const x0 = Math.max(0, rect.x0 - 1);
    const y0 = Math.max(0, rect.y0 - 1);
    const x1 = Math.min(doc.size, rect.x1 + 1);
    const y1 = Math.min(doc.size, rect.y1 + 1);
    shade(doc, { x0, y0, x1, y1 }, framePixels, {
      sun: state.sun,
      overlays: state.overlays,
      palette,
      table: state.table
    });
    context.putImageData(frame, 0, 0, x0, y0, x1 - x0, y1 - y0);
  }

  function drawAll() {
    if (state.doc) {
      state.table = colourTable(state.doc.displayRange, state.doc.seaLevelMetres);
      draw({ x0: 0, y0: 0, x1: state.doc.size, y1: state.doc.size });
    }
  }

  function pinchState() {
    const [a, b] = [...pointers.values()];
    return {
      distance: Math.max(1, Math.hypot(a.x - b.x, a.y - b.y)),
      x: (a.x + b.x) / 2,
      y: (a.y + b.y) / 2
    };
  }

  function point(event) {
    const { x, y } = toSample(event.clientX, event.clientY);
    return { x, y, pressure: event.pointerType === "pen" ? event.pressure : 1 };
  }

  function endPaint(type) {
    if (state.mode === "paint") {
      onStroke({ type, points: [] });
    }

    state.painter = null;
  }

  surface.addEventListener("pointerdown", (event) => {
    if (!state.doc) {
      return;
    }

    state.bounds = surface.getBoundingClientRect();
    surface.focus({ preventScroll: true });
    pointers.set(event.pointerId, { x: event.clientX, y: event.clientY });
    surface.setPointerCapture(event.pointerId);

    if (pointers.size === 2) {
      // A second finger turns a stroke into a pinch; the stroke is undone.
      endPaint("cancel");
      state.mode = "pinch";
      state.pinch = { ...pinchState(), scale: state.scale, panX: state.panX, panY: state.panY };
      cursor.hidden = true;
      return;
    }

    if (pointers.size > 2) {
      return;
    }

    if (event.button === 1 || (event.button === 0 && state.panKey)) {
      event.preventDefault();
      state.mode = "pan";
      state.last = { x: event.clientX, y: event.clientY };
      cursor.hidden = true;
      return;
    }

    if (event.button !== 0) {
      return;
    }

    state.mode = "paint";
    state.painter = event.pointerId;
    onStroke({ type: "start", points: [point(event)], pen: event.pointerType === "pen" });
  });

  surface.addEventListener("pointermove", (event) => {
    if (pointers.has(event.pointerId)) {
      pointers.set(event.pointerId, { x: event.clientX, y: event.clientY });
    }

    if (state.mode === "paint" && event.pointerId === state.painter) {
      // Coalesced events keep fast strokes smooth where the browser batches input.
      const events = event.getCoalescedEvents?.() ?? [];
      onStroke({ type: "move", points: (events.length ? events : [event]).map(point) });
    } else if (state.mode === "pan" && state.last) {
      state.panX += event.clientX - state.last.x;
      state.panY += event.clientY - state.last.y;
      state.last = { x: event.clientX, y: event.clientY };
      applyTransform();
    } else if (state.mode === "pinch" && pointers.size >= 2) {
      const now = pinchState();
      const { pinch } = state;
      const scale = Math.min(ZOOM_LIMITS[1], Math.max(ZOOM_LIMITS[0], (pinch.scale * now.distance) / pinch.distance));
      // The sample under the fingers' first midpoint follows their midpoint.
      const startX = pinch.x - state.bounds.left;
      const startY = pinch.y - state.bounds.top;
      state.scale = scale;
      state.panX = now.x - state.bounds.left - ((startX - pinch.panX) * scale) / pinch.scale;
      state.panY = now.y - state.bounds.top - ((startY - pinch.panY) * scale) / pinch.scale;
      applyTransform();
      onZoom(scale);
    }

    if (event.pointerType !== "touch") {
      moveCursor(event.clientX, event.clientY);
    }

    if (state.doc && state.mode !== "pinch") {
      const { x, y } = toSample(event.clientX, event.clientY);
      const inside = x > -0.5 && y > -0.5 && x < state.doc.size - 0.5 && y < state.doc.size - 0.5;
      onHover(inside ? { x: Math.round(x), y: Math.round(y) } : null);
    }
  });

  const release = (event) => {
    pointers.delete(event.pointerId);

    if (surface.hasPointerCapture(event.pointerId)) {
      surface.releasePointerCapture(event.pointerId);
    }

    if (state.mode === "paint" && event.pointerId === state.painter) {
      endPaint(event.type === "pointercancel" ? "cancel" : "end");
    }

    if (pointers.size === 0) {
      state.mode = "idle";
      state.last = null;
      state.pinch = null;
    } else if (state.mode === "pinch" && pointers.size === 1) {
      // Lifting one finger of a pinch pans with the other, never paints.
      state.mode = "pan";
      const [rest] = pointers.values();
      state.last = { x: rest.x, y: rest.y };
    }
  };

  surface.addEventListener("pointerup", release);
  surface.addEventListener("pointercancel", release);
  surface.addEventListener("pointerleave", (event) => {
    if (event.pointerType !== "touch" && state.mode !== "paint") {
      cursor.hidden = true;
      onHover(null);
    }
  });
  surface.addEventListener("wheel", (event) => {
    if (!state.doc) {
      return;
    }

    event.preventDefault();
    state.bounds = surface.getBoundingClientRect();
    const lines = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? 400 : 1;
    zoomAt(event.clientX, event.clientY, state.scale * Math.exp(-event.deltaY * lines * 0.0015));
  }, { passive: false });
  surface.addEventListener("contextmenu", (event) => event.preventDefault());
  new ResizeObserver(() => {
    state.bounds = surface.getBoundingClientRect();
  }).observe(surface);

  return {
    setDocument(doc) {
      state.doc = doc;
      frame = context.createImageData(doc.size, doc.size);
      framePixels = new Uint32Array(frame.data.buffer);
      canvas.width = doc.size;
      canvas.height = doc.size;
      canvas.style.width = `${doc.size}px`;
      canvas.style.height = `${doc.size}px`;
      drawAll();
      fit();
    },
    draw,
    drawAll,
    fit,
    /** Zoom by `factor` about the middle of the view. */
    zoomBy(factor) {
      state.bounds = surface.getBoundingClientRect();
      zoomAt(state.bounds.left + state.bounds.width / 2, state.bounds.top + state.bounds.height / 2, state.scale * factor);
    },
    setOverlays(overlays) {
      state.overlays = { ...overlays };
      drawAll();
    },
    /** Light the hillshade from the Explore tab's sun, in degrees. */
    setSun(azimuthDegrees, elevationDegrees) {
      const azimuth = (azimuthDegrees * Math.PI) / 180;
      const elevation = (Math.max(5, elevationDegrees) * Math.PI) / 180;
      state.sun = [Math.cos(azimuth) * Math.cos(elevation), Math.sin(elevation), Math.sin(azimuth) * Math.cos(elevation)];
    },
    setBrushRadius(radius) {
      state.radius = radius;
    },
    setPanKey(held) {
      state.panKey = held;
      surface.classList.toggle("panning", held);
    },
    get scale() {
      return state.scale;
    }
  };
}
