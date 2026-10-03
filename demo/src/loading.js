// The loading overlay over the 3D view: a title, a progress bar and a
// line of detail. The engine reports progress within each generation
// phase, so `generationFraction` lays the phases end to end on one bar.

/**
 * Each phase's share of the bar, as `[start, end]`. Measured on a
 * 512 x 512 map with erosion: finishing, which builds the flora and the
 * terrain mesh, takes over half the time, and rivers are built within it.
 * A bundle is read before its terrain is finished.
 */
export const PHASE_SPANS = {
  bundle: [0, 0.3],
  tectonics: [0, 0.03],
  drainage: [0.03, 0.07],
  detail: [0.07, 0.15],
  erosion: [0.15, 0.32],
  finishing: [0.32, 1],
  rivers: [0.44, 0.62]
};

/** What each phase is doing, in words. */
export const PHASE_LABELS = {
  bundle: "Reading the bundle",
  tectonics: "Raising the land",
  drainage: "Tracing where water drains",
  detail: "Adding fine detail",
  erosion: "Eroding the terrain",
  finishing: "Growing the forests and building the mesh",
  rivers: "Carving rivers and lakes"
};

/** Where `progress` (0 to 1) through `phase` sits on the bar, or `null` for a phase the bar does not show. */
export function generationFraction(phase, progress) {
  const span = PHASE_SPANS[phase];

  if (!span) {
    return null;
  }

  const clamped = Math.min(1, Math.max(0, Number(progress) || 0));
  return span[0] + (span[1] - span[0]) * clamped;
}

/**
 * Drive the overlay in `root`, which holds `[data-loading-title]`, a
 * `<progress>`, `[data-loading-detail]` and a `[data-loading-dismiss]`
 * button for errors.
 */
export function createLoadingOverlay(root) {
  const title = root.querySelector("[data-loading-title]");
  const bar = root.querySelector("progress");
  const detail = root.querySelector("[data-loading-detail]");
  const dismiss = root.querySelector("[data-loading-dismiss]");
  let fraction = 0;

  function hide() {
    root.classList.remove("is-visible", "is-first", "is-error");
    dismiss.hidden = true;
  }

  dismiss.addEventListener("click", hide);

  return {
    get visible() {
      return root.classList.contains("is-visible");
    },

    /** Show `heading` with an empty bar that moves until the first progress arrives. */
    show(heading, text = "") {
      root.classList.remove("is-error");
      root.classList.add("is-visible");
      title.textContent = heading;
      detail.textContent = text;
      dismiss.hidden = true;
      fraction = 0;
      // A `<progress>` without a value is indeterminate.
      bar.removeAttribute("value");
    },

    /** Move the bar for `progress` through `phase`. It never moves back. */
    progress(phase, progress) {
      const next = generationFraction(phase, progress);

      if (next === null) {
        return;
      }

      fraction = Math.max(fraction, next);
      bar.value = fraction;
      detail.textContent = `${PHASE_LABELS[phase]}… ${Math.round(fraction * 100)} %`;
    },

    hide,

    /** Turn a visible overlay into an error message with a Close button. Returns whether it was visible. */
    fail(message) {
      if (!root.classList.contains("is-visible")) {
        return false;
      }

      root.classList.add("is-error");
      title.textContent = "Something went wrong";
      detail.textContent = message;
      dismiss.hidden = false;
      dismiss.focus();
      return true;
    }
  };
}
