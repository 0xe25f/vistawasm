// Pure helpers for the GPU profile page: order statistics over frame
// times and the JSON report. No DOM or WebGPU here, so vitest can load it
// in Node (`js/tests/profile-stats.test.ts`).

/** The report's `format`; bump `formatVersion` when its shape changes. */
export const reportFormat = "vistawasm-gpu-profile";
export const reportFormatVersion = 1;

function finiteValues(values) {
  if (!Array.isArray(values) || values.length === 0) {
    throw new TypeError("Expected a non-empty array of frame times in milliseconds.");
  }

  for (const value of values) {
    if (typeof value !== "number" || !Number.isFinite(value)) {
      throw new TypeError(`Expected every frame time to be a finite number, got ${String(value)}.`);
    }
  }

  return [...values].sort((a, b) => a - b);
}

/**
 * The `fraction` quantile (0 to 1) of `values`, interpolating linearly
 * between the two nearest ranks, so 120 frames give a stable 90th
 * percentile rather than jumping between neighbouring samples.
 */
export function percentile(values, fraction) {
  if (typeof fraction !== "number" || !(fraction >= 0 && fraction <= 1)) {
    throw new RangeError(`Expected a percentile fraction from 0 to 1, got ${String(fraction)}.`);
  }

  const sorted = finiteValues(values);
  const rank = fraction * (sorted.length - 1);
  const below = Math.floor(rank);
  const above = Math.min(below + 1, sorted.length - 1);

  return sorted[below] + (sorted[above] - sorted[below]) * (rank - below);
}

export function median(values) {
  return percentile(values, 0.5);
}

// Microseconds are finer than any GPU timer a browser exposes.
function round(value) {
  return Math.round(value * 1000) / 1000;
}

/** `{ median, p90 }` of `values`, in milliseconds to three decimals. */
export function summarise(values) {
  return { median: round(median(values)), p90: round(percentile(values, 0.9)) };
}

/**
 * Summarise each pass of `frames` (each a `gpuPassTimesMs` object, or
 * `null` where the frame had none) as `{ pass: { median, p90 } }`, in the
 * order the passes first appear. Frames without timings are skipped; a
 * pass missing from some frames is summarised over the frames that have it.
 */
export function summarisePasses(frames) {
  const byPass = new Map();

  for (const frame of frames) {
    if (!frame) {
      continue;
    }

    for (const [pass, ms] of Object.entries(frame)) {
      if (typeof ms !== "number" || !Number.isFinite(ms)) {
        continue;
      }

      if (!byPass.has(pass)) {
        byPass.set(pass, []);
      }

      byPass.get(pass).push(ms);
    }
  }

  const result = {};

  for (const [pass, values] of byPass) {
    result[pass] = summarise(values);
  }

  return result;
}

/**
 * One scene's result. `passFrames` holds each measured frame's
 * `gpuPassTimesMs` (or `null`), `gpuFrameMs` each frame's summed GPU time
 * (or `null`), and `wallFrameMs` each frame's time from submitting it to
 * the GPU finishing it. The whole frame comes from GPU timestamps when
 * every frame has them, and from the wall clock otherwise.
 */
export function sceneResult(scene, { passFrames, gpuFrameMs, wallFrameMs }) {
  const timed = gpuFrameMs.length > 0 && gpuFrameMs.every((ms) => typeof ms === "number" && Number.isFinite(ms));

  return {
    id: scene.id,
    name: scene.name,
    group: scene.group,
    budget: scene.budget ?? {},
    frames: wallFrameMs.length,
    frame: summarise(timed ? gpuFrameMs : wallFrameMs),
    frameSource: timed ? "gpu-timestamps" : "wall-clock",
    wallClock: summarise(wallFrameMs),
    passes: timed ? summarisePasses(passFrames) : {},
    error: null
  };
}

/** A scene that could not be measured, with the reason. */
export function sceneError(scene, message) {
  return {
    id: scene.id,
    name: scene.name,
    group: scene.group,
    budget: scene.budget ?? {},
    frames: 0,
    frame: null,
    frameSource: null,
    wallClock: null,
    passes: {},
    error: String(message)
  };
}

/**
 * Whether a summary is within a scene's budget, judged on the median:
 * `true`, `false`, or `null` where the scene has no such budget or no
 * timing to judge.
 */
export function withinBudget(result, key) {
  const limit = result.budget?.[key];

  if (typeof limit !== "number") {
    return null;
  }

  const summary = key === "frame" ? result.frame : result.passes[key];

  if (!summary || (key === "frame" && result.frameSource !== "gpu-timestamps")) {
    return null;
  }

  return summary.median <= limit;
}

/**
 * Whether `GPUAdapter.info` names a software renderer (SwiftShader,
 * llvmpipe, WARP), whose times say nothing about the budgets.
 */
export function isSoftwareAdapter(info) {
  const text = [info?.vendor, info?.architecture, info?.device, info?.description]
    .filter((value) => typeof value === "string")
    .join(" ");

  return /swiftshader|llvmpipe|lavapipe|\bwarp\b|software/i.test(text);
}

function adapterInfo(info) {
  const text = (value) => (typeof value === "string" ? value : "");

  return {
    vendor: text(info?.vendor),
    architecture: text(info?.architecture),
    device: text(info?.device),
    description: text(info?.description),
    software: isSoftwareAdapter(info)
  };
}

/**
 * The JSON report: what ran where, and every scene's result. `info` is
 * `GPUAdapter.info` (or `null`), `canvas` is `{ width, height,
 * limitedByScreen }`.
 */
export function buildReport({
  date,
  userAgent,
  info,
  timestampQueries,
  canvas,
  libraryVersion,
  warmFrames,
  frames,
  scenes
}) {
  return {
    format: reportFormat,
    formatVersion: reportFormatVersion,
    date,
    library: { name: "@vista-wasm/vista-wasm", version: String(libraryVersion) },
    browser: { userAgent: String(userAgent) },
    adapter: adapterInfo(info),
    timestampQueries: Boolean(timestampQueries),
    canvas: {
      width: canvas.width,
      height: canvas.height,
      limitedByScreen: Boolean(canvas.limitedByScreen)
    },
    settings: {
      renderScale: 1,
      dynamicResolution: false,
      devicePixelRatio: 1,
      warmFrames,
      frames
    },
    scenes
  };
}

function budgetText(result, software) {
  const entries = Object.entries(result.budget);

  if (entries.length === 0) {
    return { budget: "", verdict: "" };
  }

  const budget = entries.map(([key, ms]) => `${key} ${ms}`).join(", ");

  if (software) {
    return { budget, verdict: "not judged (software)" };
  }

  const verdicts = entries.map(([key]) => withinBudget(result, key));
  const verdict = verdicts.includes(false)
    ? "**over**"
    : verdicts.every((within) => within === true) ? "within" : "not judged";

  return { budget, verdict };
}

/**
 * A report as Markdown for `docs/performance.md`: one line naming the
 * run, then a table row per scene with the whole frame, its budget and
 * the four costliest passes (by median).
 */
export function reportToMarkdown(report) {
  const adapter = [report.adapter.vendor, report.adapter.architecture, report.adapter.description]
    .filter(Boolean)
    .join(", ") || "adapter not reported";
  const lines = [
    `${report.date.slice(0, 10)}: ${adapter}; ${report.browser.userAgent}; ` +
      `${report.canvas.width} x ${report.canvas.height}; ` +
      `${report.settings.warmFrames} warm-up and ${report.settings.frames} measured frames; ` +
      `timestamps ${report.timestampQueries ? "on" : "off"}; VistaWASM ${report.library.version}.`,
    "",
    "| Scene | Frame median (ms) | Frame p90 (ms) | Budget (ms) | Median within budget | Costliest passes, median (ms) |",
    "| --- | --- | --- | --- | --- | --- |"
  ];

  for (const result of report.scenes) {
    if (result.error) {
      lines.push(`| ${result.id} | not measured: ${result.error} | | | | |`);
      continue;
    }

    const { budget, verdict } = budgetText(result, report.adapter.software);
    const passes = Object.entries(result.passes)
      .filter(([, summary]) => summary.median > 0)
      .sort(([, a], [, b]) => b.median - a.median)
      .slice(0, 4)
      .map(([pass, summary]) => `${pass} ${summary.median.toFixed(2)}`)
      .join(", ");
    const source = result.frameSource === "gpu-timestamps" ? "" : " (wall clock)";
    lines.push(
      `| ${result.id} | ${result.frame.median.toFixed(2)}${source} | ${result.frame.p90.toFixed(2)} | ${budget} | ${verdict} | ${passes} |`
    );
  }

  return `${lines.join("\n")}\n`;
}
