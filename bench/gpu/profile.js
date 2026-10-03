// The GPU profile page: renders every budgeted scene from
// `scripts/visual-check/scenes.mjs` and records per-pass GPU times, so
// budgets rest on real hardware rather than on software-rendering ratios.
//
// URL parameters, for slow (software) runs:
//   size=WxH       canvas size in pixels (default 1920x1080)
//   frames=N       measured frames per scene (default 120)
//   warm=N         warm-up frames per scene (default 40)
//   scenes=a,b     scene ids or groups (default: all)
import { createVistaEngine } from "@vista-wasm/vista-wasm";
import { version as libraryVersion } from "../node_modules/@vista-wasm/vista-wasm/package.json";
import heightmapUrl from "../../scripts/visual-check/fixed-512.f32.gz?url";
import * as scenes from "../../scripts/visual-check/scenes.mjs";
import { buildReport, isSoftwareAdapter, sceneError, sceneResult, withinBudget } from "./stats.mjs";

const $ = (selector) => document.querySelector(selector);
const status = $("#status");
const runButton = $("#run");
const copyButton = $("#copy");
const downloadButton = $("#download");
let report = null;
// Software renderers are far slower than any GPU, so their times are
// shown but never judged against the budgets.
let software = false;

// Exposed for `scripts/visual-check/profile-check.mjs`, which waits on it.
window.profile = { done: false, report: null, error: null };

function showError(message) {
  const error = $("#error");
  error.textContent = message;
  error.hidden = false;
  status.textContent = "Stopped.";
  window.profile.error = message;
  window.profile.done = true;
}

function integerParameter(parameters, name, fallback, min, max) {
  const text = parameters.get(name);

  if (text === null) {
    return fallback;
  }

  const value = Number(text);

  if (!Number.isInteger(value) || value < min || value > max) {
    throw new RangeError(`The "${name}" parameter must be a whole number from ${min} to ${max}, not "${text}".`);
  }

  return value;
}

function sizeParameter(parameters) {
  const text = parameters.get("size");

  if (text === null) {
    return [1920, 1080];
  }

  const match = /^(\d{2,4})x(\d{2,4})$/.exec(text);
  const [width, height] = match ? [Number(match[1]), Number(match[2])] : [0, 0];

  if (width < 64 || height < 64 || width > 7680 || height > 4320) {
    throw new RangeError(`The "size" parameter must be WIDTHxHEIGHT from 64x64 to 7680x4320, such as 1920x1080, not "${text}".`);
  }

  return [width, height];
}

function chosenScenes(parameters) {
  const text = parameters.get("scenes");

  if (text === null || text === "" || text === "all") {
    return scenes.budgetedScenes;
  }

  const wanted = new Set(text.split(",").map((part) => part.trim()).filter(Boolean));
  const known = new Set([
    ...scenes.budgetedGroups,
    ...scenes.budgetedScenes.map((scene) => scene.id)
  ]);
  const unknown = [...wanted].filter((name) => !known.has(name));

  if (unknown.length > 0) {
    throw new RangeError(
      `Unknown scene or group in "scenes": ${unknown.join(", ")}. Use "all", a group (${scenes.budgetedGroups.join(", ")}) or a scene id (${scenes.budgetedScenes.map((scene) => scene.id).join(", ")}).`
    );
  }

  return scenes.budgetedScenes.filter((scene) => wanted.has(scene.id) || wanted.has(scene.group));
}

// The largest canvas up to the one asked for that the screen can show at
// its full resolution, keeping the aspect ratio.
function fitToScreen([width, height]) {
  const ratio = window.devicePixelRatio || 1;
  const maxWidth = Math.floor(window.screen.width * ratio);
  const maxHeight = Math.floor(window.screen.height * ratio);
  const scale = Math.min(1, maxWidth / width, maxHeight / height);

  return {
    width: Math.floor(width * scale),
    height: Math.floor(height * scale),
    limitedByScreen: scale < 1
  };
}

function addEnvironment(term, value) {
  const list = $("#environment");
  const dt = document.createElement("dt");
  const dd = document.createElement("dd");
  dt.textContent = term;
  dd.textContent = value;
  list.append(dt, dd);
}

// The engine creates its own device; the page needs it to wait for each
// frame to finish, so it notes the device the engine configures the
// canvas with.
let engineDevice = null;

if (typeof GPUCanvasContext !== "undefined") {
  const configure = GPUCanvasContext.prototype.configure;
  GPUCanvasContext.prototype.configure = function (descriptor) {
    engineDevice = descriptor.device;
    return configure.call(this, descriptor);
  };
}

async function loadTerrain(engine, terrain) {
  if (terrain.heightmap) {
    const response = await fetch(heightmapUrl);

    if (!response.ok) {
      throw new Error(`Could not fetch the fixed heightmap from ${heightmapUrl} (HTTP ${response.status}).`);
    }

    let bytes = await response.arrayBuffer();
    const head = new Uint8Array(bytes, 0, Math.min(2, bytes.byteLength));

    // Some servers (Vite's among them) send a `.gz` file with
    // `Content-Encoding: gzip`, so the browser has already inflated it;
    // inflate only what still starts with the gzip magic bytes.
    if (head[0] === 0x1f && head[1] === 0x8b) {
      const stream = new Blob([bytes]).stream().pipeThrough(new DecompressionStream("gzip"));
      bytes = await new Response(stream).arrayBuffer();
    }

    return engine.loadRawHeightmap(bytes, scenes.heightmapOptions);
  }

  return engine.generateFractal({ ...scenes.fractalDefaults, ...terrain });
}

const nextFrame = () => new Promise((resolve) => requestAnimationFrame(resolve));

// Render `count` frames, one per display frame, waiting for the GPU to
// finish each. Returns the per-frame pass times, summed GPU times and
// wall-clock times from submit to finish.
async function renderFrames(engine, count, label) {
  const passFrames = [];
  const gpuFrameMs = [];
  const wallFrameMs = [];
  let lastIndex = null;
  let skipped = 0;

  while (wallFrameMs.length < count) {
    await nextFrame();
    const started = performance.now();
    const stats = engine.renderOnce();
    await engineDevice.queue.onSubmittedWorkDone();
    const wall = performance.now() - started;

    // The engine skips a frame while the GPU is still busy; with the wait
    // above that is rare, and such a frame has nothing new to record.
    if (stats.frameIndex === lastIndex) {
      skipped += 1;

      if (skipped > count * 4) {
        throw new Error("The engine skipped too many frames; is the GPU busy with another tab?");
      }

      continue;
    }

    lastIndex = stats.frameIndex;
    passFrames.push(stats.gpuPassTimesMs ?? null);
    gpuFrameMs.push(stats.gpuPassTimesMs ? stats.gpuFrameTimeMs ?? null : null);
    wallFrameMs.push(wall);

    if (wallFrameMs.length % 10 === 0) {
      status.textContent = `${label}: frame ${wallFrameMs.length} of ${count}.`;
    }
  }

  return { passFrames, gpuFrameMs, wallFrameMs };
}

function cell(row, text, className) {
  const td = document.createElement("td");
  td.textContent = text;

  if (className) {
    td.className = className;
  }

  row.append(td);
  return td;
}

function budgetCells(row, result, key) {
  const limit = result.budget[key];
  cell(row, typeof limit === "number" ? limit.toFixed(2) : "", "number");
  const within = software ? null : withinBudget(result, key);
  const verdict = software && typeof limit === "number"
    ? "Not judged (software)"
    : within === null ? "" : within ? "Yes" : "No, over budget";
  cell(row, verdict, within === false ? "over" : "");
}

function addResultRows(result) {
  const body = $("#results tbody");
  const row = document.createElement("tr");
  row.className = "scene";
  const heading = document.createElement("th");
  heading.scope = "row";
  row.append(heading);

  if (result.error) {
    heading.textContent = result.name;
    const td = cell(row, `Not measured: ${result.error}`, "over");
    td.colSpan = 4;
    body.append(row);
    return;
  }

  const source = result.frameSource === "gpu-timestamps" ? "GPU" : "wall clock";
  heading.textContent = `${result.name}: whole frame (${source})`;
  cell(row, result.frame.median.toFixed(2), "number");
  cell(row, result.frame.p90.toFixed(2), "number");
  budgetCells(row, result, "frame");
  body.append(row);

  for (const [pass, summary] of Object.entries(result.passes)) {
    if (summary.p90 === 0 && typeof result.budget[pass] !== "number") {
      continue;
    }

    const passRow = document.createElement("tr");
    passRow.className = "pass";
    const passHeading = document.createElement("th");
    passHeading.scope = "row";
    passHeading.textContent = pass;
    passRow.append(passHeading);
    cell(passRow, summary.median.toFixed(2), "number");
    cell(passRow, summary.p90.toFixed(2), "number");
    budgetCells(passRow, result, pass);
    body.append(passRow);
  }
}

function newCanvas(size) {
  const view = $("#view");
  const canvas = document.createElement("canvas");
  canvas.width = size.width;
  canvas.height = size.height;
  canvas.setAttribute("aria-label", "The scene being measured");
  view.replaceChildren(canvas);
  return canvas;
}

async function measureGroup(group, chosen, size, warm, frames, results) {
  const all = scenes.budgetedScenes.filter((scene) => scene.group === group);
  const first = all[0];
  let engine = null;

  try {
    status.textContent = `${group}: creating the engine and terrain.`;
    engineDevice = null;
    engine = await createVistaEngine(newCanvas(size), {
      render: { width: size.width, height: size.height, devicePixelRatio: 1 },
      ...first.engine
    });

    if (!engineDevice) {
      throw new Error("The page could not find the engine's GPU device, so it cannot wait for frames to finish.");
    }

    const handle = await loadTerrain(engine, first.terrain);
    engine.setCamera(scenes.camera);
    const heights = new Float32Array(engine.exportHeightmap().buffer.slice(0));
    const heightAt = scenes.heightSampler(heights, handle.metadata);

    for (const scene of all) {
      if (!chosen.includes(scene)) {
        // A skipped scene still sets what later scenes keep (the climate
        // or weather), just not its camera.
        const kept = Object.entries(scene.calls).filter(([method]) => method !== "ground" && method !== "setCamera");
        scenes.applyCalls(engine, Object.fromEntries(kept), heightAt);
        continue;
      }

      try {
        scenes.applyCalls(engine, scene.calls, heightAt);
        await renderFrames(engine, warm, `${scene.name}, warming up`);
        const measured = await renderFrames(engine, frames, scene.name);
        results.push(sceneResult(scene, measured));
      } catch (error) {
        results.push(sceneError(scene, error.message ?? error));
      }

      addResultRows(results[results.length - 1]);
    }
  } catch (error) {
    for (const scene of all.filter((candidate) => chosen.includes(candidate))) {
      if (!results.some((result) => result.id === scene.id)) {
        results.push(sceneError(scene, error.message ?? error));
        addResultRows(results[results.length - 1]);
      }
    }
  } finally {
    engine?.dispose();
  }
}

function showReport() {
  const text = JSON.stringify(report, null, 2);
  $("#json").textContent = text;
  copyButton.disabled = false;
  downloadButton.disabled = false;
  window.profile.report = report;
}

async function run(settings) {
  runButton.disabled = true;
  copyButton.disabled = true;
  downloadButton.disabled = true;
  $("#results tbody").replaceChildren();
  $("#json").textContent = "Running.";
  const results = [];
  const started = performance.now();

  for (const group of scenes.budgetedGroups) {
    const chosenInGroup = settings.chosen.filter((scene) => scene.group === group);

    if (chosenInGroup.length > 0) {
      await measureGroup(group, settings.chosen, settings.size, settings.warm, settings.frames, results);
    }
  }

  const timed = results.some((result) => result.frameSource === "gpu-timestamps");

  if (!timed) {
    const notice = $("#timestamps");
    notice.textContent = "No frame carried GPU timestamps, so per-pass times are unavailable. Whole frames are timed from submitting each frame until the GPU finished it (wall clock), which also counts some CPU and scheduling time and cannot be held to the GPU budgets.";
    notice.hidden = false;
  }

  report = buildReport({
    date: new Date().toISOString(),
    userAgent: navigator.userAgent,
    info: settings.info,
    timestampQueries: timed,
    canvas: settings.size,
    libraryVersion,
    warmFrames: settings.warm,
    frames: settings.frames,
    scenes: results
  });
  showReport();
  // The engine is gone, so its last frame is no longer shown.
  $("#view").replaceChildren();
  const failed = results.filter((result) => result.error).length;
  const minutes = ((performance.now() - started) / 60000).toFixed(1);
  status.textContent = failed === 0
    ? `Done: ${results.length} ${results.length === 1 ? "scene" : "scenes"} measured in ${minutes} minutes.`
    : `Done in ${minutes} minutes, but ${failed} of ${results.length} scenes could not be measured; see the table.`;
  runButton.disabled = false;
  window.profile.done = true;
}

copyButton.addEventListener("click", async () => {
  try {
    await navigator.clipboard.writeText(JSON.stringify(report, null, 2));
    $("#copied").textContent = "Copied.";
  } catch (error) {
    $("#copied").textContent = `Could not copy (${error.message}); use Download JSON, or select the text below.`;
  }
});

downloadButton.addEventListener("click", () => {
  const blob = new Blob([JSON.stringify(report, null, 2)], { type: "application/json" });
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = `vistawasm-gpu-profile-${report.date.slice(0, 10)}.json`;
  link.click();
  // The download has started by the next task; the URL is no longer needed.
  setTimeout(() => URL.revokeObjectURL(url), 0);
});

async function start() {
  const parameters = new URLSearchParams(location.search);
  const requested = sizeParameter(parameters);
  const settings = {
    size: fitToScreen(requested),
    frames: integerParameter(parameters, "frames", 120, 1, 10000),
    warm: integerParameter(parameters, "warm", 40, 0, 10000),
    chosen: chosenScenes(parameters),
    info: null
  };

  if (!navigator.gpu) {
    throw new Error("This browser has no WebGPU (navigator.gpu is missing). Use a current Chrome, Edge, Safari or Firefox with WebGPU on.");
  }

  const adapter = await navigator.gpu.requestAdapter({ powerPreference: "high-performance" });

  if (!adapter) {
    throw new Error("WebGPU gave no adapter. Check that hardware acceleration is on in the browser settings.");
  }

  settings.info = adapter.info ?? null;
  const info = settings.info ?? {};
  addEnvironment("Library", `@vista-wasm/vista-wasm ${libraryVersion}`);
  addEnvironment("Browser", navigator.userAgent);
  addEnvironment(
    "Adapter",
    [info.vendor, info.architecture, info.device, info.description].filter(Boolean).join(", ") || "Not reported"
  );
  addEnvironment(
    "Canvas",
    `${settings.size.width} x ${settings.size.height}${settings.size.limitedByScreen ? ` (limited by the screen; ${requested[0]} x ${requested[1]} asked for)` : ""}`
  );
  addEnvironment(
    "Frames",
    `${settings.warm} warm-up, then ${settings.frames} measured, for each of ${settings.chosen.length} ${settings.chosen.length === 1 ? "scene" : "scenes"}`
  );

  software = isSoftwareAdapter(settings.info);

  if (software) {
    const notice = $("#software");
    notice.textContent = "This adapter is a software renderer. Its times show that every scene runs and how the passes compare, but they are far slower than any GPU and are not a budget check.";
    notice.hidden = false;
  }

  if (!adapter.features.has("timestamp-query")) {
    const notice = $("#timestamps");
    notice.textContent = "This browser does not offer WebGPU timestamp queries, so the page cannot time each pass. It times whole frames only, from submitting each frame until the GPU finished it (wall clock).";
    notice.hidden = false;
  }

  runButton.addEventListener("click", () => {
    run(settings).catch((error) => showError(`The profile stopped: ${error.message}`));
  });
}

start().catch((error) => {
  runButton.disabled = true;
  showError(error.message);
});
