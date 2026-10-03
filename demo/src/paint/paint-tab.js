// The Paint tab's controls: starting a document, the brushes, history,
// the view's layers, and rendering, saving and opening paintings.
import {
  BIOME_COLOURS,
  BIOME_KINDS,
  biomeMapFromImage,
  decodePng,
  densityMaskFromImage,
  downloadBlob,
  waterMaskFromImage
} from "@vista-wasm/vista-wasm";
import { createDocument, documentFromEngine, setMapWidth } from "./document.js";
import { applyDab, brushRadius, createStroke, dabParts, dabRect, dabSpacing, dabsAlong, toolLayer, unionRect } from "./brushes.js";
import { History } from "./history.js";
import { createPaintView } from "./view.js";
import { createRenderQueue, exportLayerPng, framingCamera, openPainting, renderDocument, savePainting } from "./render.js";
import { historyForKey, isTypingTarget } from "../shortcuts.js";
import { MAX_TERRAIN_SIDE, checkFile, checkImage } from "../file-checks.js";

/** Brush work allowed in one animation frame; dabs left over wait for the next. */
const FRAME_BUDGET_MS = 8;

// Frames are planned this far inside the budget, as a slow phone's timing varies from dab to dab.
const BUDGET_MARGIN_MS = 2;

// "innerForest" reads "Inner forest".
const BIOME_NAMES = BIOME_KINDS.map((kind) => kind.replace(/[A-Z]/g, (letter) => ` ${letter.toLowerCase()}`).replace(/^./, (first) => first.toUpperCase()));

function element(id) {
  const found = document.querySelector(`#${id}`);

  if (!found) {
    throw new Error(`Missing #${id} paint control.`);
  }

  return found;
}

function message(error) {
  return error instanceof Error ? (error.code ? `${error.code}: ${error.message}` : error.message) : String(error);
}

/**
 * Wire the Paint tab. `getSun()` returns the Explore tab's sun as
 * `[azimuth, elevation]` in degrees. `onTerrain({ handle, doc, camera,
 * show })` runs after the tab loads terrain into the engine: `camera` is
 * a framing to use, or `null`, and `show` asks for the Explore tab.
 */
export function createPaintTab({ engine, getSun, onTerrain }) {
  // Each control's id is "paint" and its name, capitalised.
  const controls = Object.fromEntries([
    "source", "size", "widthKm", "sea", "flatHeight", "flatHeightLabel", "blankOptions", "copyBiomes",
    "copyBiomesLabel", "importOptions", "importHeight", "importMin", "importMax", "importBiome",
    "importWater", "importTrees", "importGrass", "importBundle", "start", "docReadout", "tool",
    "brushSize", "strength", "falloff", "palette", "undo", "redo", "historyReadout", "showBiome",
    "showWater", "showTrees", "showGrass", "rangeLow", "rangeHigh", "zoomOut", "zoomIn", "fit",
    "zoomReadout", "exportLayer", "exportLayerButton", "render", "live", "save", "open",
    "renderReadout", "timingReadout", "cursor", "status"
  ].map((name) => [name, element(`paint${name[0].toUpperCase()}${name.slice(1)}`)]));

  const palette = Uint8Array.from(BIOME_COLOURS.flat());
  const state = {
    doc: null,
    history: null,
    // Bumped by every change, so a render knows whether it is current.
    version: 0,
    renderedVersion: -1,
    framed: false,
    frame: 0,
    stroke: null,
    brush: null,
    queue: [],
    queued: 0,
    ending: false,
    seed: 1,
    timing: null,
    // The dearest recent dab of each tool and size, and redrawn pixel, in ms.
    dabCosts: new Map(),
    pixelCost: 0,
    biome: BIOME_KINDS.indexOf("innerForest"),
    toolSizes: { river: 3 },
    tool: controls.tool.value,
    busy: false,
    // Set while this tab loads terrain, so its own loads don't mark the painting stale.
    loading: false,
    wantFraming: false
  };
  const view = createPaintView({
    surface: element("paintSurface"),
    canvas: element("paintCanvas"),
    cursor: element("brushCursor"),
    palette,
    onStroke: stroke,
    onHover: hover,
    onZoom: (scale) => {
      controls.zoomReadout.textContent = `Zoom ${scale.toFixed(2)}x`;
    }
  });
  const renders = createRenderQueue(renderNow);

  function setStatus(text) {
    controls.status.textContent = text;
  }

  function showError(error, readout = controls.docReadout) {
    const text = message(error);
    setStatus(text);
    readout.textContent = text;
  }

  function setBusy(busy) {
    state.busy = busy;
    controls.start.disabled = busy;
    controls.open.disabled = busy;
    syncButtons();
  }

  function syncButtons() {
    const ready = Boolean(state.doc) && !state.busy;
    controls.render.disabled = !ready;
    controls.save.disabled = !ready;
    controls.exportLayerButton.disabled = !state.doc;
    controls.undo.disabled = !state.history?.canUndo || Boolean(state.stroke);
    controls.redo.disabled = !state.history?.canRedo || Boolean(state.stroke);
  }

  function describeDocument() {
    const { doc } = state;
    controls.docReadout.textContent =
      `${doc.size} x ${doc.size} samples, ${doc.widthKm.toFixed(2)} km wide ` +
      `(${doc.metresPerSample.toFixed(1)} m a sample), sea level ${doc.seaLevelMetres} m.`;
  }

  function setDocument(doc) {
    state.doc = doc;
    state.history = new History(doc.size);
    state.version += 1;
    state.framed = false;
    [controls.rangeLow.value, controls.rangeHigh.value] = doc.displayRange.map(String);
    view.setSun(...getSun());
    view.setDocument(doc);
    describeDocument();
    syncButtons();
  }

  function readBrush() {
    const size = Number(controls.brushSize.value);
    return {
      tool: controls.tool.value,
      radius: brushRadius(size),
      strength: Number(controls.strength.value),
      falloff: controls.falloff.value,
      biome: state.biome
    };
  }

  function setBrushSize(size) {
    controls.brushSize.value = String(Math.round(Math.min(256, Math.max(1, size))));
    // Updates the slider's readout.
    controls.brushSize.dispatchEvent(new Event("input"));
  }

  function wireBrush() {
    controls.brushSize.addEventListener("input", () => {
      view.setBrushRadius(brushRadius(Number(controls.brushSize.value)));
    });
    controls.tool.addEventListener("change", () => {
      // Each tool keeps its own size, so rivers stay thin.
      state.toolSizes[state.tool] = Number(controls.brushSize.value);
      state.tool = controls.tool.value;
      setBrushSize(state.toolSizes[state.tool] ?? 32);
      controls.palette.hidden = state.tool !== "biome";
    });
    const buttons = [...BIOME_KINDS.map((_, index) => index), 255].map((index) => {
      const button = document.createElement("button");
      const swatch = document.createElement("span");
      button.type = "button";
      button.className = "secondary swatch";
      button.dataset.biome = String(index);
      button.setAttribute("aria-pressed", String(index === state.biome));
      swatch.setAttribute("aria-hidden", "true");

      if (index === 255) {
        swatch.classList.add("erase");
      } else {
        swatch.style.backgroundColor = `rgb(${BIOME_COLOURS[index].join(" ")})`;
      }

      button.append(swatch, index === 255 ? "Erase (unpainted)" : BIOME_NAMES[index]);
      button.addEventListener("click", () => {
        state.biome = index;

        for (const other of buttons) {
          other.setAttribute("aria-pressed", String(other === button));
        }
      });
      return button;
    });
    controls.palette.replaceChildren(...buttons);
    view.setBrushRadius(brushRadius(Number(controls.brushSize.value)));
  }

  function schedule() {
    state.frame ||= requestAnimationFrame(tick);
  }

  function queuePoints(points) {
    const { stroke: current, brush } = state;
    const spacing = dabSpacing(brush.radius);
    const parts = dabParts(brush.tool);

    // Four numbers a queued dab part: x, y, pressure, and the part (-1 for a whole dab).
    for (const point of points) {
      for (const [x, y] of dabsAlong(current, point.x, point.y, spacing)) {
        for (let part = 0; part < parts; part += 1) {
          state.queue.push(x, y, brush.pen ? point.pressure : 1, parts > 1 ? part : -1);
        }
      }
    }

    schedule();
  }

  function stroke({ type, points, pen }) {
    if (!state.doc) {
      return;
    }

    if (type === "start") {
      if (state.stroke) {
        finishQueued();
      }

      state.brush = { ...readBrush(), pen };
      const [first] = points;
      state.stroke = createStroke(state.doc, state.brush.tool, first.x, first.y, state.seed);
      state.seed += 1;
      state.queue = [];
      state.queued = 0;
      state.ending = false;
      state.timing = { dabs: 0, dabMs: 0, frames: [] };
      state.history.begin();
      renders.cancel();
      syncButtons();
      queuePoints(points);
    } else if (type === "move" && state.stroke) {
      queuePoints(points);
    } else if (type === "end" && state.stroke) {
      state.ending = true;
      schedule();
    } else if (type === "cancel" && state.stroke) {
      state.queue = [];
      state.queued = 0;
      state.stroke = null;
      state.ending = false;
      const restored = state.history.cancel(state.doc.layers);
      view.draw(restored.reduce((rect, tile) => unionRect(rect, tile.rect), null));
      syncButtons();
    }
  }

  /** The pixels a redraw of `rect` shades: it and a sample around it, for the hillshade. */
  function redrawArea(rect) {
    return rect ? (rect.x1 - rect.x0 + 2) * (rect.y1 - rect.y0 + 2) : 0;
  }

  /**
   * Apply queued dab parts while they and the redraw of what they change fit
   * the frame's budget, judged by the dearest recent dab and pixel;
   * return the rectangle they changed.
   */
  function runDabs(budgetMs) {
    const { doc, stroke: current, brush, history, queue } = state;
    const layer = toolLayer(brush.tool);
    const started = performance.now();
    // Costs are kept by dab part, as parts are what the loop schedules.
    const key = `${brush.tool}:${brush.radius}`;
    let dabCost = state.dabCosts.get(key) ?? 0;
    let dirty = null;

    while (state.queued < queue.length) {
      const x = queue[state.queued];
      const y = queue[state.queued + 1];
      const rect = dabRect(doc.size, x, y, brush.radius);
      const predicted = performance.now() - started + dabCost + redrawArea(unionRect(dirty, rect)) * state.pixelCost;

      if (dirty !== null && predicted > budgetMs) {
        break;
      }

      const dabStarted = performance.now();
      const pressure = queue[state.queued + 2];
      const part = queue[state.queued + 3];
      state.queued += 4;
      history.touch(doc.layers, layer, rect);
      applyDab(doc, current, { ...brush, x, y, strength: brush.strength * pressure }, part < 0 ? null : part);
      dirty = unionRect(dirty, rect);
      state.timing.dabs += part <= 0 ? 1 : 0;
      dabCost = Math.max(performance.now() - dabStarted, dabCost * 0.9);
    }

    state.dabCosts.set(key, dabCost);

    state.timing.dabMs += performance.now() - started;

    if (state.queued === queue.length) {
      state.queue = [];
      state.queued = 0;
    }

    return dirty;
  }

  function tick() {
    state.frame = 0;

    if (!state.stroke) {
      return;
    }

    const started = performance.now();
    const dirty = runDabs(FRAME_BUDGET_MS - BUDGET_MARGIN_MS);
    const dabsDone = performance.now();
    view.draw(dirty);
    const timing = state.timing;

    if (dirty) {
      state.pixelCost = Math.max((performance.now() - dabsDone) / redrawArea(dirty), state.pixelCost * 0.9);
      timing.frames.push([dabsDone - started, performance.now() - started]);
    }

    if (state.queued < state.queue.length) {
      schedule();
    } else if (state.ending) {
      endStroke();
    }
  }

  /** Apply every queued dab now and end the stroke, as a new stroke or a tab change needs. */
  function finishQueued() {
    if (state.stroke) {
      view.draw(runDabs(Infinity));
      endStroke();
    }
  }

  function endStroke() {
    const timing = state.timing;
    const recorded = state.history.end(state.doc.layers);
    state.stroke = null;
    state.ending = false;

    if (timing.dabs) {
      const sorted = (index) => timing.frames.map((frame) => frame[index]).sort((a, b) => a - b);
      const brush = sorted(0);
      const total = sorted(1);
      const p95 = (values) => values[Math.min(values.length - 1, Math.floor(values.length * 0.95))];
      controls.timingReadout.textContent =
        `Last stroke: ${timing.dabs} dabs over ${timing.frames.length} frames, ${(timing.dabMs / timing.dabs).toFixed(2)} ms a dab. ` +
        `Brush work a frame: ${p95(brush).toFixed(1)} ms at the 95th percentile, ${brush[brush.length - 1].toFixed(1)} ms at most; ` +
        `with the redraw: ${p95(total).toFixed(1)} ms and ${total[total.length - 1].toFixed(1)} ms.`;
    }

    if (recorded) {
      changed();
    }

    syncButtons();
  }

  function changed() {
    state.version += 1;
    const strokes = state.history.undoStack.length;
    controls.historyReadout.textContent = `${strokes} ${strokes === 1 ? "stroke" : "strokes"} to undo.`;

    if (controls.live.checked) {
      renders.later({}, (error) => showError(error, controls.renderReadout));
    }
  }

  function step(direction) {
    if (!state.doc || state.stroke) {
      return;
    }

    const restored = direction === "undo" ? state.history.undo(state.doc.layers) : state.history.redo(state.doc.layers);

    if (restored) {
      view.draw(restored.reduce((rect, tile) => unionRect(rect, tile.rect), null));
      changed();
      setStatus(direction === "undo" ? "Undid the last stroke." : "Redid the stroke.");
    }

    syncButtons();
  }

  function hover(point) {
    const { doc } = state;

    if (!doc || !point) {
      controls.cursor.textContent = "Move over the map to read it.";
      return;
    }

    const at = point.y * doc.size + point.x;
    const { height, biome, water, trees, grass } = doc.layers;
    const parts = [`x ${point.x}, y ${point.y}: ${height[at].toFixed(1)} m`];
    parts.push(biome[at] === 255 ? "biome not painted" : BIOME_NAMES[biome[at]]);

    if (water[at]) {
      parts.push(water[at] >= 128 ? "lake" : `river ${water[at]}`);
    }

    if (trees[at] !== 128) {
      parts.push(`trees ${trees[at]}`);
    }

    if (grass[at] !== 128) {
      parts.push(`grass ${grass[at]}`);
    }

    controls.cursor.textContent = parts.join(" · ");
  }

  async function renderNow() {
    const { doc, version } = state;
    const show = state.wantFraming;
    state.wantFraming = false;
    const started = performance.now();
    controls.renderReadout.textContent = "Rendering...";
    setStatus("Rendering the painting in 3D...");
    state.loading = true;

    try {
      const { handle, warnings } = await renderDocument(engine, doc);
      state.renderedVersion = doc === state.doc ? version : -1;
      const camera = show || !state.framed ? framingCamera(doc) : null;
      state.framed = true;
      onTerrain({ handle, doc, camera, show });
      const seconds = ((performance.now() - started) / 1000).toFixed(2);
      const text = `Rendered in ${seconds} s.${warnings.length ? ` Warning: ${warnings.join(" ")}` : ""}`;
      controls.renderReadout.textContent = text;
      setStatus(text);
    } finally {
      state.loading = false;
    }
  }

  function render() {
    finishQueued();
    state.wantFraming = true;
    return renders.now().catch((error) => showError(error, controls.renderReadout));
  }

  async function loadInto(label, work) {
    setBusy(true);
    setStatus(`Loading ${label}...`);
    state.loading = true;
    const warnings = [];
    const stop = engine.on("warning", ({ message: text }) => warnings.push(text));

    try {
      const { doc, handle } = await work();
      setDocument(doc);
      // The engine now shows exactly this document.
      state.renderedVersion = state.version;
      state.framed = true;
      onTerrain({ handle, doc, camera: framingCamera(doc), show: false });
      setStatus(`Loaded ${label}.${warnings.length ? ` Warning: ${warnings.join(" ")}` : ""}`);
    } finally {
      stop();
      state.loading = false;
      setBusy(false);
    }
  }

  async function importImages(size, copyBiomes, widthKm, seaLevelMetres) {
    const heightFile = await checkImage(controls.importHeight.files?.[0], "a heightmap image", MAX_TERRAIN_SIDE);
    const masks = await Promise.all([
      [controls.importBiome, "a biome map"],
      [controls.importWater, "a water mask"],
      [controls.importTrees, "a tree density mask"],
      [controls.importGrass, "a grass density mask"]
    ].map(([input, label]) => (input.files?.[0] ? checkImage(input.files[0], label, MAX_TERRAIN_SIDE) : null)));
    const [biomeFile, waterFile, treesFile, grassFile] = masks;
    // Every image is read before the scene changes, so a bad one leaves it as it was.
    const biome = biomeFile ? (await biomeMapFromImage(biomeFile)).map : undefined;
    const water = waterFile ? await waterMaskFromImage(waterFile, { mode: await waterMode(waterFile) }) : undefined;
    const trees = treesFile ? await densityMaskFromImage(treesFile) : undefined;
    const grass = grassFile ? await densityMaskFromImage(grassFile) : undefined;
    const handle = await engine.loadHeightmapImage(heightFile, {
      metresPerSample: (widthKm * 1000) / (size - 1),
      minHeightMetres: Number(controls.importMin.value),
      maxHeightMetres: Number(controls.importMax.value),
      seaLevelMetres
    });
    engine.setBiomeMap(biome ?? null);
    engine.setWaterMask(water ?? null);
    engine.setVegetationMasks({ trees: trees ?? null, grass: grass ?? null });
    const doc = documentFromEngine(engine, { size, copyBiomes });
    setMapWidth(doc, widthKm);
    return { doc, handle };
  }

  /** A grey PNG holds mask values as they are, as the Layers section exports them; anything else uses the legend. */
  async function waterMode(file) {
    if (file.type !== "image/png" && !/\.png$/i.test(file.name)) {
      return "legend";
    }

    const decoded = await decodePng(new Uint8Array(await file.arrayBuffer()));
    return decoded.channels === 1 && !decoded.palette ? "grey" : "legend";
  }

  async function start() {
    finishQueued();
    const size = Number(controls.size.value);
    const source = controls.source.value;
    const copyBiomes = controls.copyBiomes.checked;

    try {
      const widthKm = Number(controls.widthKm.value);
      const seaLevelMetres = Number(controls.sea.value);

      // Checked before anything loads, so a bad value leaves the scene as it was.
      if (source !== "current") {
        setMapWidth({ size }, widthKm);

        if (!(seaLevelMetres >= -200 && seaLevelMetres <= 400)) {
          throw new RangeError(`The sea level must be from -200 to 400 m, but it is ${controls.sea.value}.`);
        }
      }

      if (source === "blank") {
        setDocument(createDocument({ size, widthKm, seaLevelMetres, heightMetres: Number(controls.flatHeight.value) }));
        setStatus("Started a blank painting. Drag on the map to paint.");
      } else if (source === "current") {
        setDocument(documentFromEngine(engine, { size, copyBiomes }));
        setStatus("Started from the current map. Drag on the map to paint.");
      } else if (controls.importBundle.files?.[0]) {
        const file = checkFile(controls.importBundle.files[0], "zip", "a bundle");
        await loadInto("the bundle", async () => {
          const { doc, handle } = await openPainting(engine, file, size);
          return { doc: copyBiomes ? documentFromEngine(engine, { size: doc.size, copyBiomes }) : doc, handle };
        });
      } else {
        await loadInto("the images", () => importImages(size, copyBiomes, widthKm, seaLevelMetres));
      }
    } catch (error) {
      showError(error);
    }
  }

  function syncSource() {
    const source = controls.source.value;
    controls.blankOptions.hidden = source === "current";
    controls.flatHeightLabel.hidden = source !== "blank";
    controls.copyBiomesLabel.hidden = source === "blank";
    controls.importOptions.hidden = source !== "import";
  }

  function wireDocument() {
    controls.source.addEventListener("change", syncSource);
    syncSource();
    controls.start.addEventListener("click", () => {
      start();
    });
    // A 16-bit PNG from an export carries its height range.
    controls.importHeight.addEventListener("change", async () => {
      const file = controls.importHeight.files?.[0];

      try {
        if (file && (file.type === "image/png" || /\.png$/i.test(file.name))) {
          await checkImage(file, "a heightmap image", MAX_TERRAIN_SIDE);
          const range = JSON.parse((await decodePng(new Uint8Array(await file.arrayBuffer()))).text["vistawasm:range"] ?? "null");

          if (Array.isArray(range)) {
            [controls.importMin.value, controls.importMax.value] = range.map(String);
            controls.docReadout.textContent = `Height range ${range[0]} m to ${range[1]} m, from the PNG.`;
          }
        }
      } catch (error) {
        showError(error);
      }
    });
  }

  function wireHistory() {
    controls.undo.addEventListener("click", () => step("undo"));
    controls.redo.addEventListener("click", () => step("redo"));
  }

  function wireLayers() {
    const overlays = () => ({
      biome: controls.showBiome.checked,
      water: controls.showWater.checked,
      trees: controls.showTrees.checked,
      grass: controls.showGrass.checked
    });

    for (const box of [controls.showBiome, controls.showWater, controls.showTrees, controls.showGrass]) {
      box.addEventListener("change", () => view.setOverlays(overlays()));
    }

    for (const input of [controls.rangeLow, controls.rangeHigh]) {
      input.addEventListener("change", () => {
        const low = Number(controls.rangeLow.value);
        const high = Number(controls.rangeHigh.value);

        if (!state.doc) {
          return;
        }

        if (!(Number.isFinite(low) && Number.isFinite(high) && high - low >= 10)) {
          showError(new RangeError("The colour range needs a top at least 10 m above its bottom."), controls.zoomReadout);
          return;
        }

        // The range also sets how far raise and lower move a dab.
        state.doc.displayRange = [low, high];
        view.drawAll();
      });
    }

    controls.fit.addEventListener("click", () => view.fit());
    controls.zoomIn.addEventListener("click", () => view.zoomBy(1.25));
    controls.zoomOut.addEventListener("click", () => view.zoomBy(0.8));
    controls.exportLayerButton.addEventListener("click", async () => {
      const layer = controls.exportLayer.value;

      try {
        downloadBlob(await exportLayerPng(state.doc, layer), `vistawasm-paint-${layer}-${state.doc.size}.png`);
        setStatus(`Exported the ${layer} layer.`);
      } catch (error) {
        showError(error);
      }
    });
  }

  function wireRender() {
    controls.render.addEventListener("click", () => {
      render();
    });
    controls.live.addEventListener("change", () => {
      if (controls.live.checked && state.doc && state.renderedVersion !== state.version) {
        renders.later({}, (error) => showError(error, controls.renderReadout));
      }
    });
    controls.save.addEventListener("click", async () => {
      finishQueued();
      setBusy(true);

      try {
        // The bundle is written from what the engine shows, so bring it up to date first.
        if (state.renderedVersion !== state.version) {
          await renders.now();
        }

        setStatus("Saving the painting...");
        downloadBlob(await savePainting(engine), "vistawasm-painting.zip");
        setStatus("Saved the painting.");
      } catch (error) {
        showError(error, controls.renderReadout);
      } finally {
        setBusy(false);
      }
    });
    controls.open.addEventListener("change", async () => {
      const file = controls.open.files?.[0];
      controls.open.value = "";

      if (!file) {
        return;
      }

      finishQueued();

      try {
        checkFile(file, "zip", "a painting");
        await loadInto(file.name, () => openPainting(engine, file, Number(controls.size.value)));
      } catch (error) {
        showError(error, controls.renderReadout);
      }
    });
    // Terrain loaded from the Explore tab replaces what was rendered.
    engine.on("terrainLoaded", () => {
      if (!state.loading) {
        state.renderedVersion = -1;
      }
    });
  }

  wireBrush();
  wireDocument();
  wireHistory();
  wireLayers();
  wireRender();
  syncButtons();

  return {
    activate() {
      view.setSun(...getSun());
      view.drawAll();
    },
    deactivate() {
      finishQueued();
      view.setPanKey(false);
    },
    /** Paint shortcuts; returns whether the key was used. */
    keyDown(event) {
      const action = historyForKey(event);

      if (action) {
        event.preventDefault();
        step(action);
        return true;
      }

      if (isTypingTarget(event.target) || event.ctrlKey || event.metaKey || event.altKey) {
        return false;
      }

      // Space pans over the canvas; on a button it still presses the button.
      if (event.code === "Space" && (event.target === document.body || event.target.id === "paintSurface")) {
        event.preventDefault();
        view.setPanKey(true);
        return true;
      }

      if (event.key === "[" || event.key === "]") {
        const size = Number(controls.brushSize.value);
        setBrushSize(event.key === "]" ? Math.max(size + 1, size * 1.25) : Math.min(size - 1, size * 0.8));
        return true;
      }

      return false;
    },
    keyUp(event) {
      if (event.code === "Space") {
        view.setPanKey(false);
      }
    }
  };
}
