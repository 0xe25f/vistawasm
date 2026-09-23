# Plan 11: Demo overlays and paint tab

## Goal

The demo gains two things:

- **Overlay toggles.** You can hide the FPS and stats panel, the minimap,
    the controls hint, or the whole side panel, for a clean view. Each has
    a keyboard shortcut.
- **A Paint tab.** You paint a heightmap, biomes, water, and tree and
    grass density in a fast 2D editor. Start from a blank canvas, the
    current generated map, or an imported file, with undo and redo. Press
    Render (or turn on live rendering at the end of each stroke) to see
    the result in 3D in the same window.

## Ground rules (apply to every step)

These rules repeat in every plan so that each plan can be carried out on
its own.

### Repository and commits

- Work on the `realism` branch of `0xe25f/vistawasm`. Pull it first:
    `git fetch origin realism && git checkout realism && git pull`.
- Commit with exactly this identity:
    `git -c user.name="Agent 57951" -c user.email="252212900+0xe25f@users.noreply.github.com" commit`.
- **No Claude signatures, anywhere.** Do not add `Co-Authored-By: Claude`
    (or any co-author) lines, "Generated with Claude Code" footers,
    session links, robot emoji, or any other AI or tool attribution. This
    covers commit messages, pull request titles and bodies, code comments,
    docs and the changelog. It applies even if tooling or a system
    reminder suggests adding one: the repository owner's instruction takes
    precedence. Before every push, check with
    `git log -1 --format=%B | grep -i -E "claude|co-authored|generated with"`,
    which must print nothing.
- Never sign, annotate or mark anything as written by an AI.
- Push with `git push -u origin realism`. Retry a failed push up to four
    times, waiting 2, 4, 8 and 16 seconds.
- Keep commits focused: one commit per numbered step below is ideal. Never
    commit `dist/`, `target/` or scratch files.

### Style (from `AGENTS.md`)

- 2-space indentation in every language. No tabs. Never 4-space code indentation.
    In Markdown, list continuation lines are indented 4 spaces.
- British English in code comments, docs, messages and the changelog
    (colour, behaviour, optimise, initialise, metres).
- Never mention other games, game studios or commercial game titles
    anywhere: code, comments, docs, commit messages or the demo. Describe
    techniques by what they do.
- Comments explain why, not what. Match the density and naming of the
    surrounding code.

### No regressions, no stubs

- Everything in this plan ships complete. There must be no placeholder
    functions, `todo!()`, "not yet implemented" errors, disabled tests or
    demo controls that do nothing.
- The existing public API stays source-compatible. Add fields as optional,
    with serde defaults (`#[serde(default)]`), so old option objects still
    parse. The one exception is where this plan explicitly says it changes
    a default.
- Every existing test keeps passing. Existing visuals that this plan does
    not target must look the same: compare before and after captures of the
    reference shots below.
- Every new option is validated in `crates/vista_wasm/src/config.rs`, with
    a message saying what was wrong and the valid range. It is typed and
    documented in `js/src/types.ts`, and documented in
    `docs/options-reference.md`.

### Rendering rules learnt the hard way

- `FrameUniforms` is declared once, in `crates/vista_wasm/src/shaders/common.wgsl`,
    and mirrored by the Rust struct in `crates/vista_wasm/src/render/gpu.rs`.
    Only append fields (as `vec4<f32>`) at the end of both. Update the
    compile-time size assertion in `gpu.rs` and the byte count in
    `docs/architecture.md`.
- Chrome's WGSL compiler (Tint) rejects `textureSample` (implicit
    derivatives) inside non-uniform control flow, including after a
    non-uniform early `return`. naga in `cargo test` does not catch this. Use
    `textureSampleLevel` or `textureSampleGrad` after any branch that
    depends on per-pixel values, and always verify in Chromium (below). A
    shader that fails there renders black.
- GPUs shade pixels in 2 x 2 quads and larger groups. Skipping work on
    scattered single pixels saves nothing. Skip work for whole regions or
    in separate, smaller passes.
- Per-pixel `time * something` terms that differ between neighbouring
    pixels shimmer. Derive such values from uniforms, not per pixel.
- Hold 60 FPS. Budgets are given per plan in milliseconds of GPU time at
    1920 x 1080 on a mid-range GPU (roughly an Apple M-series base chip or
    a desktop RTX 3060). Dynamic resolution (`RenderQualityOptions.dynamicResolution`)
    is a safety net, not a budget.
- Tiny files: the gzipped `dist/pkg/vista_wasm_bg.wasm` is 237,820 bytes
    at the start of this work. Each plan states how much it may add. Check
    with `gzip -9 -c dist/pkg/vista_wasm_bg.wasm | wc -c`. Generate data
    procedurally at start-up instead of embedding it.

### Commands

Run all of these before every push. All must pass with no new warnings:

```sh
cargo fmt --all
cargo clippy -q --all-targets     # no warnings beyond the existing ones
cargo test -q
npm run build                     # wasm-pack plus tsc
npx tsc -p tsconfig.json --noEmit
npx vitest run
npm run lint:indent
```

### Visual verification in headless Chromium

`scripts/visual-check/` renders the engine with software WebGPU and saves
PNGs. Serve the repository root, then capture:

```sh
python3 -m http.server 8124 &
node scripts/visual-check/capture.mjs /tmp/shot.png '{"size":[960,600]}' 3
```

- The config JSON takes `size`, `engine` (`createVistaEngine` options),
    `terrain` (`generateFractal` options), `set` (`{ "setWeather": {...} }`
    calls), `camera`, and `shots` (a list of camera or `set` changes, one
    PNG each). See the header of `capture.mjs`.
- The script exits non-zero on any page error or console error. Treat
    that as a failed check.
- If Playwright is not resolvable, set `PLAYWRIGHT_MODULE` to its
    `index.mjs`, and `CHROMIUM_PATH` to a Chromium binary.
- Software rendering takes seconds per frame. Use small sizes (480 x 300)
    for iteration and 960 x 600 for final shots. Relative GPU times
    between passes are meaningful; absolute times are not.
- Open every PNG you produce and look at it. A check that was not looked
    at did not happen.
- Reference shots to capture before starting and again at the end, to
    prove nothing else changed:
    1. The default config: `{"size":[960,600]}`.
    2. Rain: `{"size":[960,600],"set":{"setWeather":{"enabled":true,"state":"rain","autoCycle":false,"transitionSeconds":0.1}}}`
        with 4 frames.
    3. A forest close-up:
        `{"size":[960,600],"shots":[{"biome":"innerForest","back":40,"height":6}]}`.

### Documentation and demo

- Update `CHANGELOG.md` under the `## [1.1.0]` heading (it is unreleased).
    Put new features under `### Added` and behaviour changes under
    `### Changed`.
- Update the relevant guide in `docs/` and `docs/options-reference.md`.
    Add a short, runnable example for each new public function.
- Every new option gets a working control in the demo (`demo/index.html`
    and `demo/src/main.js`), in the right collapsible section, with a
    visible value readout for sliders.

### Finishing

Report what changed, how it was tested (commands and shots), the size and
GPU-time deltas, and any risks. Keep the report short.

## Decisions already made

- Toggles:
    - the stats and FPS panel (with a keyboard shortcut);
    - the minimap;
    - the controls hint;
    - the whole side panel.
- Paint tools:
    - height brushes: raise, lower, smooth, flatten, noise and erode, with
        size, strength and falloff;
    - a biome brush with a palette;
    - water brushes (lake, river, erase) and vegetation brushes (tree and
        grass density);
    - start from anything: blank, the current map, or an import;
    - undo and redo;
    - import and export.
- Layout: a 2D paint tab, then Render (or live on stroke end) sends it to
    the 3D view in the same window.
- Paint canvas up to 1024 x 1024, smooth on phones.

## Prerequisites

- **Plan 10** (map import): `loadRawHeightmap` or `loadHeightmapImage`,
    `setBiomeMap`, `setVegetationMasks`, `loadBundle`,
    `biomeMapFromImage` and the PNG decoder.
- **Plan 9** (export): `exportMap`, `exportBundle` and `encodePng`.
- **Plan 3**: `setWaterMask`.
- Check that each exists. If one is missing, carry out that plan first.

## Current state

- `demo/index.html`:
    - `main.shell` holds `section.viewer` (the `#vista` canvas,
        `#minimap`, `#status`, `#stats` and `.hint`) and
        `aside.controls` with collapsible `details.section` blocks;
    - their open state is remembered in `localStorage`
        (`SECTION_STORAGE_KEY` in `demo/src/main.js`).
- `demo/src/main.js` is plain JavaScript with no build step. It is
    served by Vite in development and copied with the package by
    `scripts/build-demo.mjs`. The import map points
    `@vista-wasm/vista-wasm` at `./dist/index.js`.
- Keyboard: WASD moves, Space and Shift rise and fall. `camera-controls`
    handles input on the canvas.
- Vitest runs `js/tests`. There are no demo tests yet.

## Design

### 1. Overlay toggles

- Add a "View" section at the top of the side panel, with checkboxes:
    - Stats and FPS (key `1`);
    - Minimap (`2`);
    - Controls hint (`3`);
    - Side panel (`0`).
- Shortcuts ignore key presses while focus is in an input, select or
    textarea. They must not clash with the camera keys (WASD, Space,
    Shift).
- **Hiding the side panel** makes the viewer full width. A small floating
    button labelled "Show controls" stays visible in a corner, as an
    accessible way back. Pressing `0` also restores the panel.
- Hidden elements get the `hidden` attribute, not only CSS, so screen
    readers skip them.
- The `stats` event handler skips building its text while the stats are
    hidden. That saves string work every frame.
- Remember the states in `localStorage`, wrapped in try/catch. Defaults:
    everything shown.
- The hint lists the shortcuts: "1 stats · 2 minimap · 3 hint · 0 panel".
- On phones (a viewport under 700 px wide), start with the side panel
    collapsed.

### 2. Tabs

- Add a tab bar above the viewer: "Explore" (the 3D view, the default)
    and "Paint". It follows the ARIA tabs pattern: `role="tablist"`,
    arrow-key navigation, and `aria-selected`.
- **Switching to Paint:**
    - Stop the engine loop (`engine.stop()`), so the GPU is idle while
        painting.
    - Show the paint workspace in place of the viewer.
    - Replace the side panel's content with the paint tools. Keep the
        explore controls in the DOM, hidden, so their state survives.
- **Switching back:** show the viewer and restart the loop.

### 3. Paint document (`demo/src/paint/document.js`)

- **Layers** are typed arrays of size N x N, where N is 256, 512 or 1024,
    chosen when starting:
    - `height`: `Float32Array` in metres;
    - `biome`: `Uint8Array`, 255 = not painted;
    - `water`: `Uint8Array`, with plan 3's `WaterMask` semantics;
    - `trees` and `grass`: `Uint8Array`, where 128 means unchanged.
- **Document settings:**
    - map width in km (sets `metresPerSample`);
    - sea level;
    - the height range, for display.
- **Starting points:**
    - **Blank:** a flat height (choose 0 to 500 m), everything unpainted.
    - **Current map:** height from `exportMap("sourceHeight")`, resampled
        to N. An option, "Copy generated biomes", fills the biome layer
        from `exportMap("biome")`. The water and vegetation layers start
        at no change, unless the engine has painted masks set, which are
        then copied.
    - **Import:** any heightmap image (decoded with plan 10's decoder),
        and optionally a biome map, water mask and density masks, or a
        bundle (the paint layers are taken from its source height and
        painted inputs).

### 4. Brushes (`demo/src/paint/brushes.js`)

These are pure functions on layers and a dirty rectangle, so they can be
unit tested.

- **Common settings:**
    - size, 1 to 256 samples;
    - strength, 0 to 1;
    - falloff: smooth (cosine), linear or constant;
    - spacing: a dab every 25 % of the radius along the stroke;
    - pen pressure scales strength, when `pointerType === "pen"`.
- **Height brushes:**
    - **Raise and lower:** `± strength x falloff x 2 m` per dab, scaled by
        the map's height range.
    - **Smooth:** blend towards a 5 x 5 Gaussian of the neighbourhood.
    - **Flatten:** blend towards the height sampled at the stroke start.
    - **Noise:** add fbm (plan 1's gradient-noise port in JS, or a small
        local implementation) with amplitude `strength x 10 m`, at a
        wavelength tied to the brush size.
    - **Erode:** inside the brush, run 3 iterations of thermal erosion
        (a talus of 35 degrees) and a small droplet pass (30 droplets per
        dab, deterministic seeds). Valleys deepen and scree forms locally.
        Keep it within 3 ms per dab at size 128.
- **Biome brush:** a palette of every `BiomeKind`, with legend colours
    and names from plan 9's legend; "Erase" sets 255. Painting sets
    samples above the falloff threshold (0.5), because categories don't
    blend. The engine's border dithering makes the result natural.
- **Water brushes:**
    - Lake sets 200.
    - River sets the stroke strength mapped to 1 to 127. Rivers are thin:
        the default size is 3.
    - Erase sets 0.
- **Vegetation brushes:** tree and grass density, "more" towards 255,
    "less" towards 0, and "reset" towards 128, blended by falloff and
    strength.

### 5. History (`demo/src/paint/history.js`)

- Before each stroke's first write to a 64 x 64 tile of a layer, keep a
    copy of that tile. A stroke's undo record is the set of copied tiles;
    redo keeps the stroke's final tiles.
- Limits: 100 strokes, or 256 MB of tile copies, whichever comes first.
    The oldest records are dropped first.
- Shortcuts: Ctrl or Cmd + Z to undo, Ctrl or Cmd + Shift + Z (and
    Ctrl + Y) to redo. There are also buttons.

### 6. View (`demo/src/paint/view.js`)

- **Canvas:**
    - A 2D canvas shows the height layer as hillshade (from the sun
        direction set in Explore), multiplied by hypsometric colours.
    - Overlays can be toggled: biome colours at 45 % opacity, water in
        blue, and trees and grass as green or brown difference tints.
    - Only the dirty rectangle is recomputed after each dab, with
        `putImageData` on that rectangle.
    - Full redraws happen only on layer toggles and loads.
- **Navigation:** the wheel or pinch zooms (0.25x to 8x); Space-drag,
    middle-drag or two-finger drag pans.
- **Input:** Pointer Events, with `setPointerCapture`,
    `touch-action: none` on the canvas, and coalesced events
    (`getCoalescedEvents`) for smooth strokes.
- **Cursor:** a preview circle shows the brush size. A readout shows the
    height and biome under the cursor.
- **Performance target:** 60 FPS while painting a 1024 x 1024 document at
    size 128 on a mid-range phone. Profile and keep each dab under 4 ms,
    and the redraw of its dirty rectangle under 4 ms.

### 7. Render and live mode (`demo/src/paint/render.js`)

- **Render:**
    1. Call `engine.loadRawHeightmap(height.buffer, { width: N, height: N, sampleFormat: "float32", metresPerSample, heightScaleMetres: 1, seaLevelMetres })`.
    2. Then `setBiomeMap` (unless all 255, in which case pass `null`),
        `setWaterMask` (null if empty) and `setVegetationMasks`.
    3. Switch to Explore, and frame the camera: at 45 degrees, at 0.8x
        the map width, looking at the centre.
    - Progress and warnings appear in the status line.
- **"Render after each stroke" checkbox:** debounce 600 ms after a
    stroke ends, then render in the background:
    - the 3D view stays hidden, but the engine updates;
    - switching to Explore shows the result instantly;
    - painting stays responsive, because the engine's async load runs
        between frames.
    - If a render is running when another is due, queue only the latest.
- **Save and open:**
    - "Save painting (.zip)" writes a version 2 bundle through plan 9 and
        10's writer: the source height, the painted inputs, and the
        options snapshot.
    - "Open painting" reads one with `loadBundle`'s reader, into the
        paint layers.
    - "Export layer as PNG" writes the current layer with `encodePng`.

### 8. Structure and tests

- Create modules under `demo/src/paint/`: `document.js`, `brushes.js`,
    `history.js`, `view.js`, `render.js` and `paint-tab.js` (the UI
    wiring). Use plain JS, no build step, and ES modules, matching the
    demo.
- Add `demo/tests/*.test.js`, and make sure Vitest runs them: update the
    Vitest config or `package.json` `test` script so `npx vitest run`
    includes `demo/tests`.
- Add `scripts/visual-check/demo-capture.mjs`. It opens the demo in
    headless Chromium with the same WebGPU init script as
    `scripts/visual-check/index.html` (factor the script into a shared
    `scripts/visual-check/offscreen-canvas.js` used by both). It can:
    - click and drag on elements, and screenshot 2D UI with Playwright;
    - read back the 3D canvas through the offscreen texture.

## Steps

1. Capture before screenshots of the demo: full UI at 1280 x 800 and
    390 x 844.
2. Add the overlay toggles with shortcuts, persistence, the floating
    restore button and phone defaults.
3. Add the tabs.
4. Add the paint document, starting points and loaders.
5. Add the brushes, with tests.
6. Add history, with tests.
7. Add the view: rendering, navigation, pointer input and cursor.
8. Add render, live mode, save and open, and layer export.
9. Add `demo-capture.mjs` and the shared init script.
10. Docs:
    - a new `docs/demo.md` covering the demo's tabs, shortcuts and paint
        workflow, with screenshots under 60 KB each;
    - link it from `README.md` and `docs/README.md`;
    - `CHANGELOG.md` (Demo entries).
11. Verify, then commit and push.

## Tests

`demo/tests`:

- **Brushes:**
    - raise at strength 1 and size 10 raises the centre by the documented
        amount, falling off to 0 at the radius;
    - smooth reduces the variance of a noisy patch;
    - flatten converges to the start height;
    - erode on a synthetic cone lowers the peak and conserves mass within
        2 %;
    - the biome brush sets samples above the falloff threshold;
    - each brush touches only its dirty rectangle.
- **History:** a stroke, undo, redo round-trips byte-exactly. The memory
    cap drops the oldest records.
- **Document:** starting "Current map" resamples to N correctly (use a
    mock engine returning a synthetic map).
- **Shortcuts:** they are ignored while an input has focus (jsdom, or a
    tiny DOM shim; if jsdom isn't installed, test the pure key-handling
    function without the DOM).

## Verification

With `demo-capture.mjs`:

- **Toggles:** press `1`, `2`, `3` and `0` in turn, and screenshot each.
    The overlays are hidden, and the restore button is visible when the
    panel is hidden.
- **Paint tab:**
    1. Start from blank, 512 x 512, and 6 km wide.
    2. Draw a raised ridge with the raise brush.
    3. Paint a biome region (`innerForest`), a lake, and a river from the
        ridge to the lake.
    4. Paint a tree-density "more" patch.
    5. Screenshot the 2D view.
    6. Press Render, then read back the 3D view. The ridge, forest, lake
        and river should be visible.
- **Undo:** undo the lake, render, and read back. The lake is gone.
- **Save and open:** save the painting, reload, open it, and render. The
    result matches the previous render (compare the readback pixels: mean
    absolute difference under 1.0).
- **Phone:** repeat a short stroke at 390 x 844 with touch emulation.
- The full-UI screenshots show no layout breakage at either size. There's
    no horizontal page scroll at 390 px.

## Budgets

- The demo's paint code is at most 25 KB gzipped in total (it isn't part
    of the published package).
- There's no change to the library's size or per-frame cost.
- Painting holds 60 FPS on a mid-range phone at 1024 x 1024 with a
    128-sample brush.

## Out of scope

Painting directly on the 3D terrain, a split view, and canvases over
1024 x 1024 (not chosen).
