import { describe, expect, it } from "vitest";
import { PHASE_LABELS, PHASE_SPANS, createLoadingOverlay, generationFraction } from "../src/loading.js";

function fakeElement() {
  const classes = new Set();
  const listeners = {};
  return {
    classList: {
      add: (...names) => names.forEach((name) => classes.add(name)),
      remove: (...names) => names.forEach((name) => classes.delete(name)),
      contains: (name) => classes.has(name)
    },
    attributes: new Set(),
    addEventListener: (type, listener) => {
      listeners[type] = listener;
    },
    click: () => listeners.click?.(),
    focus: () => {},
    removeAttribute(name) {
      if (name === "value") {
        delete this.value;
      }
    },
    hidden: false,
    textContent: ""
  };
}

function fakeOverlay() {
  const parts = {
    "[data-loading-title]": fakeElement(),
    progress: fakeElement(),
    "[data-loading-detail]": fakeElement(),
    "[data-loading-dismiss]": fakeElement()
  };
  const root = fakeElement();
  root.querySelector = (selector) => parts[selector];
  return { root, parts, overlay: createLoadingOverlay(root) };
}

describe("generation progress", () => {
  it("runs the generation phases end to end from 0 to 1", () => {
    const order = ["tectonics", "drainage", "detail", "erosion", "finishing"];
    expect(generationFraction(order[0], 0)).toBe(0);
    expect(generationFraction("finishing", 1)).toBe(1);

    for (let index = 1; index < order.length; index += 1) {
      expect(PHASE_SPANS[order[index]][0]).toBe(PHASE_SPANS[order[index - 1]][1]);
    }
  });

  it("puts rivers inside finishing, before its halfway report", () => {
    const [start, end] = PHASE_SPANS.rivers;
    expect(start).toBeGreaterThan(generationFraction("finishing", 0));
    expect(end).toBeLessThan(generationFraction("finishing", 0.5));
  });

  it("clamps progress and ignores phases the bar does not show", () => {
    expect(generationFraction("detail", -1)).toBe(PHASE_SPANS.detail[0]);
    expect(generationFraction("detail", 2)).toBe(PHASE_SPANS.detail[1]);
    expect(generationFraction("detail", Number.NaN)).toBe(PHASE_SPANS.detail[0]);
    expect(generationFraction("fractal", 0.5)).toBeNull();
  });

  it("has words for every phase", () => {
    expect(Object.keys(PHASE_LABELS).sort()).toEqual(Object.keys(PHASE_SPANS).sort());
  });
});

describe("loading overlay", () => {
  it("shows an indeterminate bar, then moves it forward only", () => {
    const { root, parts, overlay } = fakeOverlay();
    parts.progress.value = 0.5;
    overlay.show("Generating terrain", "Seed 1");
    expect(overlay.visible).toBe(true);
    expect(parts["[data-loading-title]"].textContent).toBe("Generating terrain");
    expect(parts.progress.value).toBeUndefined();

    overlay.progress("erosion", 1);
    overlay.progress("finishing", 0);
    overlay.progress("rivers", 0);
    overlay.progress("finishing", 0.1);
    expect(parts.progress.value).toBeCloseTo(PHASE_SPANS.rivers[0]);
    expect(parts["[data-loading-detail]"].textContent).toMatch(/^Growing the forests/);

    overlay.hide();
    expect(root.classList.contains("is-visible")).toBe(false);
  });

  it("turns into a dismissable error only while visible", () => {
    const { parts, overlay } = fakeOverlay();
    expect(overlay.fail("hidden")).toBe(false);

    overlay.show("Starting VistaWASM");
    expect(overlay.fail("WEBGPU_UNAVAILABLE: no adapter")).toBe(true);
    expect(parts["[data-loading-detail]"].textContent).toBe("WEBGPU_UNAVAILABLE: no adapter");
    expect(parts["[data-loading-dismiss]"].hidden).toBe(false);

    parts["[data-loading-dismiss]"].click();
    expect(overlay.visible).toBe(false);
    expect(parts["[data-loading-dismiss]"].hidden).toBe(true);
  });
});
