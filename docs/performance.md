# Performance

VistaWASM aims to hold 60 FPS. Its budgets are GPU time per frame at
1920 x 1080 on a mid-range GPU, roughly an Apple M-series base chip or a
desktop RTX 3060. Dynamic resolution (`RenderQualityOptions.dynamicResolution`)
is a safety net for slower GPUs, not part of the budget.

This page lists every budget beside the scene that checks it, explains
how the figures so far were estimated, and holds the measured results.
The GPU profile page in `bench/gpu/` measures the scenes on real
hardware.

## The GPU profile page

`bench/gpu/` renders every budgeted scene in turn at render scale 1,
with dynamic resolution off, and reads each pass's GPU time from
`RenderStats.gpuPassTimesMs`. For each scene it renders 40 warm-up frames
(streamed trees and grass fill their tiles over the first frames), then
records the median and the 90th percentile of every pass, and of the
whole frame, over 120 frames. It renders one frame per display frame and
waits for the GPU to finish each one, so frames never queue up.

The whole frame is the sum of the pass times, from the GPU's own
timestamps. Where the browser offers no timestamp queries, the page says
so and times whole frames only, from submitting each frame until the GPU
has finished it. That wall-clock time also counts some CPU and
scheduling time, so it is not held to the budgets.

The results appear in a table, with "Copy JSON" and "Download JSON"
buttons. The JSON names the browser (its user agent), the adapter (from
`GPUAdapter.info`), the canvas size and the library version, and holds
every scene's median and 90th percentile.

### Run it on your computer

Build the library, then start the bench server:

```bash
npm ci && npm run build
cd bench
npm ci
npm run gpu      # opens http://127.0.0.1:5174/gpu/
```

Press "Run the profile" and leave the tab in front until the status line
says it is done. All 18 scenes take a few minutes on a GPU.

For steady numbers:

- plug a laptop in and close other tabs and GPU-heavy apps;
- keep the window on the built-in display;
- run it twice and keep the second run.

The canvas is 1920 x 1080. On a screen that cannot show that many
pixels, the page uses the largest canvas with the same shape that fits,
and reports the size it used.

Chrome rounds timestamp queries to 100 microseconds unless
`chrome://flags/#enable-webgpu-developer-features` is on. The rounding
matters only for the smallest passes; turn the flag on for a run you
record.

### Run it on another device

WebGPU needs a secure context. `localhost` is one, but a computer's
address on the local network over plain HTTP is not, so a phone or
tablet cannot use the page from `npm run gpu` on another computer as it
stands. Either:

- run the page on the device itself, where it can run a Node server; or
- serve the bench over HTTPS: set `server.https` in
    `bench/vite.config.js` to a certificate the device trusts, and start
    it with `npx vite --host 0.0.0.0`; or
- in Chrome on the device, list the server's address under
    `chrome://flags/#unsafely-treat-insecure-origin-as-secure` for the
    run, and turn the flag off afterwards.

### URL parameters

The defaults are the budget conditions. The parameters exist for slow
runs, such as software rendering:

| Parameter | Default | Meaning |
| --- | --- | --- |
| `size=WxH` | `1920x1080` | Canvas size in pixels, from 64 x 64 to 7680 x 4320. |
| `frames=N` | `120` | Frames measured per scene. |
| `warm=N` | `40` | Warm-up frames per scene. |
| `scenes=a,b` | all | Scene ids or groups, from the table below. |

For example, `/gpu/?scenes=meadow-0.5,sky-rain-deck` measures the
meadow at density 0.5 and the rain deck only.

### Run it headless

`scripts/visual-check/profile-check.mjs` drives the page in headless
Chromium with software WebGPU (SwiftShader), saves the JSON and a
screenshot of the finished page, and exits non-zero on any console
error or scene that could not be measured:

```bash
cd bench && npx vite --host 127.0.0.1 --port 5174 &
node scripts/visual-check/profile-check.mjs /tmp/profile "size=480x270&warm=10&frames=20"
```

`--no-timestamps` hides timestamp queries from the page, to check the
whole-frame fallback. Software rendering takes seconds per frame, so
keep the size and frame counts small.

## Scenes and budgets

The scenes live in `scripts/visual-check/scenes.mjs`, which both the
profile page and `scripts/visual-check/fixed-scene.mjs` (the
software-rendering gate) use, so the two never drift. Budgets are
milliseconds of GPU time at 1920 x 1080 on a mid-range GPU.

| Scene id | Group | What it shows | Budget |
| --- | --- | --- | --- |
| `fixed-clear` | `fixed` | The fixed heightmap from above, clear weather: the default scene. | Whole frame 12 ms |
| `fixed-rain` | `fixed` | The same in rain, with 120 lens drops. | Whole frame 14 ms |
| `fixed-ice` | `fixed` | -18 °C, looking out over pack ice on the open sea. | Whole frame 12 ms |
| `fixed-clear-grass`, `fixed-rain-grass`, `fixed-ice-grass` | `fixed-grass` | The same three with the grass settings pinned. | As above |
| `jungle-closeUp` | `jungle` | Dense jungle (27 °C, moisture bias 0.7, trees at density 4, grass on), 6 m up inside the stand. | Whole frame 14 ms |
| `jungle-clearing` | `jungle` | The edge of the stand from 20 m. | None; for reference |
| `jungle-hillside` | `jungle` | The stand from a hillside 300 m up. | None; for reference |
| `meadow-0.5-4-m` | `meadow-0.5` | The rolling-hills meadow, grass at density 0.5, camera 4 m up. | Grass pass 1.0 ms |
| `meadow-0.5-25-m` | `meadow-0.5` | The same from 25 m, where the grass hands over to the ground's sheen. | None; for reference |
| `meadow-4-4-m` | `meadow-4` | The meadow with grass at density 4, camera 4 m up. | Grass pass 2.0 ms |
| `meadow-4-25-m` | `meadow-4` | The same from 25 m. | None; for reference |
| `sky-cumulus-from-below` | `sky` | Fair-weather cumulus from over the sea at (0, 40, -4000). | Whole frame 12 ms |
| `sky-low-sun` | `sky` | The same with the sun 12 degrees up. | Whole frame 12 ms |
| `sky-mackerel-sky` | `sky` | Altocumulus 0.7, looking up at 50 degrees. | Whole frame 12 ms |
| `sky-veiled-sun` | `sky` | Altostratus 0.7, looking at the sun 35 degrees up. | Whole frame 12 ms; the watery sun at most +0.05 ms |
| `sky-rain-deck` | `sky` | The rain deck from the sea. | Whole frame 14 ms; cloud bases at most +0.2 ms in the clouds pass |

The jungle close-up's budget holds at render scale 1. Three budgets are
differences, which one run cannot show on its own:

- **Cloud bases:** the clouds pass in `sky-rain-deck` with the bases on
    may cost at most 0.2 ms more than with them off.
- **Watery sun:** the veiled sun may cost at most 0.05 ms more than the
    version before it.
- **First frame:** `first frame ms` (logged by the visual-check page)
    may grow by at most 5 % from one release to the next.

Measure these by running the scene before and after the change, on the
same device.

## How the estimates so far were made

Before the measurements under [Results](#results), every GPU figure
on this page was an estimate; the estimates are kept for comparison. Pass times were measured under software WebGPU
(SwiftShader) in headless Chromium, where absolute times mean nothing
but the ratios between passes carry over roughly. They were then scaled
from one anchor: the terrain pass costs about 1.6 ms at 1920 x 1080 on a
mid-range GPU. That comes from an Apple M4 measuring 6.34 ms at 4K
(3840 x 2160), which has four times the pixels.

So a pass's estimate is its software time divided by the software
terrain time, times 1.6 ms. For example, a pass measured at 90 ms beside
a 60 ms terrain pass is estimated at 90 / 60 x 1.6 = 2.4 ms.

The ratios are only as good as software rendering's likeness to a GPU:
SwiftShader runs every shader on the CPU, so passes limited by texture
reads or by fill rate may compare differently on real hardware.

### Estimates

Measured with `scripts/visual-check/fixed-scene.mjs` at 960 x 600, each
run scaled by the terrain pass of its own first scene (the clear view
for the fixed scene, the close-up for the jungle). Whole frames sum
every pass once (the split tree passes are part of the trees pass).

| Scene | Budget | Estimate |
| --- | --- | --- |
| Fixed scene, clear (the default scene, grass on) | 12 ms | 3.90 ms |
| Fixed scene, rain | 14 ms | 5.33 ms |
| Fixed scene, pack ice | 12 ms | 1.75 ms |
| Jungle close-up, render scale 1 | 14 ms | 12.25 ms |
| Jungle clearing | none | 11.93 ms |
| Jungle hillside | none | 12.13 ms |
| Meadow, density 0.5, camera 4 m: grass pass | 1.0 ms | 0.77 ms |
| Meadow, density 4, camera 4 m: grass pass | 2.0 ms | 0.89 ms |
| Rain deck: cloud bases in the clouds pass | +0.2 ms | +0.10 ms |
| Veiled sun: sky and fog pass | +0.05 ms for the watery sun | 0.555 ms (+0.016) |

## CPU work, memory and size

Measured on a 4-core machine in headless Chromium.

**River build.** The browser's `"rivers"` phase, best of three:

| Map | Time |
| --- | --- |
| Default (12 m) | 141 ms |
| Rolling hills, 30 m | 500 ms |
| Continental, 30 m, open edges, automatic inflow | 602 ms |

Generating the default map takes about 1.8 s in all. Errors from a
terrain's GPU upload arrive as `"gpuError"` events once the GPU
answers, so a terrain call does not wait for the GPU to confirm the
upload. The river build's time is spread across hydrology (31 %),
conditioning (15 %), bank strips (9 %) and step-pools (7 %), with no
single hot spot.

**Memory.** Peak WebAssembly memory (`memory-check.mjs`), for the
largest terrain allowed, 2048 x 2048:

| Job | Peak | Time |
| --- | --- | --- |
| Generate at 12 m | 473 MiB | 20 s |
| Generate at 30 m, open edges | 930 MiB | 115 s |
| Load a float32 raw heightmap at 12 m (vegetation placed) | 1,130 MiB | 92 s |
| The same at 60 m (past the vegetation cap) | 1,465 MiB | 50 s |
| Load at 12 m, set a full-size painted water mask (a world rebuild), then export every map at full size | 1,364 MiB | 184 s |

A world rebuild (a painted map, or a water or biome option change)
frees everything it rebuilds before it starts: the rivers, drainage,
ground textures, surface and normals. The heights, options and painted
maps stay. Holding both worlds at once would have raised the last job's
peak to 1,908 MiB.

Terrains are at most 2048 samples a side. At 4096 x 4096 the same jobs
peaked at 1.6 to 3.7 GiB: a raw load at 30 m reached 3,469 MiB, and a
load, rebuild and full export 3,693 MiB, a few hundred MiB short of the
4 GiB WebAssembly can address. WebAssembly memory is never given back
while the engine lives, so that left too little room for heavier
options. An 8192 x 8192 raw load runs out of memory: natively it peaks
at 9.4 GiB, in surface classification (1.8 GiB), normals (768 MiB at a
time) and the river network (its ribbon alone 1.6 GiB). The limit is
set by WebAssembly's memory, not by a flaw in VistaWASM (see
[`security.md`](security.md#terrain-size)). Raw heightmaps are read
from the caller's buffer rather than copied into WASM memory whole, and
reeds are not placed past the vegetation cap.

**Per frame.** A native test renders 100 frames after warm-up and
counts no heap allocations; the `start()` loop builds no stats object
unless something listens for `"stats"`.

**Size.** The WASM is 532,241 bytes gzipped (433 KB with Brotli), and
the JavaScript glue 13,583 bytes; `scripts/check-size.mjs` holds every
release file to a budget.

## Results

Measurements from real devices replace the estimates. Record each
run as follows:

1. Run the page on the device with the defaults, then press "Download
    JSON".
2. Turn the JSON into rows with
    `node bench/gpu/markdown.mjs vistawasm-gpu-profile-<date>.json`.
3. Add the run below as a new subsection, headed with the device (for
    example "Apple M4, Safari 26"), above the software run.
4. Any scene over its budget shows **over** in the "Median within
    budget" column. List each one under "Budgets broken" below, with
    the scene, the pass and how far over it is, so it is fixed or its
    budget is revisited.

### Budgets broken

**Chrome and Edge, NVIDIA GeForce RTX 3060 Ti:** none. Every scene is
well within its budget; the heaviest budgeted scene, rain, takes 4.3 ms
of a 14 ms budget.

**Safari, Apple M4:** six scenes are over by their pass times:

| Scene | Budget | Median | Costliest passes (ms) | Submit to finish, median |
| --- | --- | --- | --- | --- |
| fixed-rain | frame 14 ms | 27.83 ms | water 11.68, skyAndFog 6.75, clouds 4.47 | 16 ms |
| fixed-rain-grass | frame 14 ms | 27.48 ms | water 11.62, skyAndFog 6.71, clouds 4.42 | 16 ms |
| jungle-closeUp | frame 14 ms | 20.84 ms | trees 12.21 (treeMeshes 7.28) | 13 ms |
| meadow-0.5-4-m | grass 1 ms | grass 4.09 ms | grass 4.09, skyAndFog 3.63, terrain 3.44 | 10 ms |
| meadow-4-4-m | grass 2 ms | grass 4.68 ms | grass 4.68, skyAndFog 4.42, terrain 2.81 | 10 ms |
| sky-rain-deck | frame 14 ms | 21.79 ms | skyAndFog 9.70, clouds 7.39 | 15 ms |

On this GPU the passes' times add up to more than the whole frame takes
from submission to finish (the wall clock, which also counts some CPU
time): rain sums to 27.8 ms of passes in a 16 ms frame. The passes
overlap, so their sum, and each pass's share, overstate the frame.
Confirm the costliest passes with Safari's own GPU profiler before
changing the budgets or the code. By the wall clock, every scene with a
frame budget is within it but rain (16 ms of 14) and the rain deck
(15 ms of 14).

**Firefox, NVIDIA GeForce RTX 3060 Ti:** ten scenes are over: the
fixed scene clear (36.34 ms), in rain (24.35 ms), clear with grass
(14.94 ms) and in rain with grass (24.19 ms); the jungle close-up
(24.96 ms); the meadow's grass at density 0.5 (3.30 ms of 1) and 4
(7.20 ms of 2); and the low sun (23.57 ms), the veiled sun (12.12 ms)
and the rain deck (33.75 ms). The terrain pass takes 11 to 31 ms where
Chrome and Edge take about 1 ms on the same GPU, and the identical clear
scenes, with and without pinned grass, differ by 21 ms. Firefox's
wall clock reads 100 ms in every scene, so it cannot confirm them.
Firefox's WebGPU runs VistaWASM far slower than Chrome and Edge on the
same hardware; the terrain pass is the place to start.

### NVIDIA GeForce RTX 3060 Ti, Chrome 154 (Windows)

2026-10-03; 1920 x 1080; 40 warm-up and 120 measured frames; timestamps
on; VistaWASM 2.0.0.

{rows['chrome']}

### NVIDIA GeForce RTX 3060 Ti, Edge 154 (Windows)

2026-10-03; 1920 x 1080; 40 warm-up and 120 measured frames; timestamps
on; VistaWASM 2.0.0.

{rows['edge']}

### Apple M4, Safari 27 (macOS)

2026-10-03; 1920 x 1080; 40 warm-up and 120 measured frames; timestamps
on; VistaWASM 2.0.0. The pass times overlap on this GPU (see above).

{rows['safari']}

### NVIDIA GeForce RTX 3060 Ti, Firefox 157 (Windows)

2026-10-03; 1920 x 1080; 40 warm-up and 120 measured frames; timestamps
on; VistaWASM 2.0.0. Firefox does not report the adapter.

{rows['firefox']}

### Software run (SwiftShader): not a budget check

This run only shows that every scene completes and how the passes
compare. It was rendered on the CPU by SwiftShader in headless Chromium,
on a 4-core machine, at a small canvas (480 x 270) and with few frames
(10 warm-up and 20 measured) so it finishes in reasonable time, with a
development build of 2.0.0. Its milliseconds are far above those of a GPU and
say nothing about the budgets, so no row is judged against one.

2026-09-30: google, swiftshader; Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) HeadlessChrome/141.0.7390.37 Safari/537.36; 480 x 270; 10 warm-up and 20 measured frames; timestamps on; VistaWASM 1.1.0 (the development build's version string at the time).

| Scene | Frame median (ms) | Frame p90 (ms) | Budget (ms) | Median within budget | Costliest passes, median (ms) |
| --- | --- | --- | --- | --- | --- |
| fixed-clear | 1714.97 | 2103.69 | frame 12 | not judged (software) | terrain 788.79, water 626.22, skyAndFog 144.96, shadows 57.27 |
| fixed-rain | 2067.32 | 2505.75 | frame 14 | not judged (software) | terrain 804.32, water 734.26, clouds 222.18, skyAndFog 150.83 |
| fixed-ice | 847.13 | 1001.03 | frame 12 | not judged (software) | water 472.35, terrain 240.14, skyAndFog 139.92, present 8.67 |
| fixed-clear-grass | 930.39 | 1076.63 | frame 12 | not judged (software) | terrain 413.77, water 352.65, skyAndFog 70.60, shadows 34.76 |
| fixed-rain-grass | 1812.92 | 2440.23 | frame 14 | not judged (software) | terrain 711.09, water 612.58, clouds 202.66, skyAndFog 157.85 |
| fixed-ice-grass | 857.32 | 926.84 | frame 12 | not judged (software) | water 443.70, terrain 250.05, skyAndFog 129.44, present 8.21 |
| jungle-closeUp | 7923.82 | 8725.92 | frame 14 | not judged (software) | trees 5433.57, treeMeshes 2974.70, treeImpostors 2315.93, shadows 718.07 |
| jungle-clearing | 7371.79 | 7835.99 |  |  | trees 4843.22, treeImpostors 3962.70, terrain 934.87, treeMeshes 909.44 |
| jungle-hillside | 4065.41 | 6728.42 |  |  | trees 2625.32, treeImpostors 2625.30, terrain 538.45, shadows 390.02 |
| meadow-0.5-4-m | 892.22 | 942.47 | grass 1 | not judged (software) | grass 319.22, terrain 289.92, water 211.35, skyAndFog 66.50 |
| meadow-0.5-25-m | 888.91 | 958.54 |  |  | terrain 350.68, grass 233.11, water 231.56, skyAndFog 66.66 |
| meadow-4-4-m | 845.84 | 889.17 | grass 2 | not judged (software) | terrain 288.26, grass 276.19, water 203.31, skyAndFog 66.63 |
| meadow-4-25-m | 780.10 | 820.17 |  |  | terrain 334.71, water 221.66, grass 153.18, skyAndFog 62.86 |
| sky-cumulus-from-below | 689.76 | 765.66 | frame 12 | not judged (software) | terrain 272.02, clouds 156.39, water 148.97, skyAndFog 70.79 |
| sky-low-sun | 702.67 | 739.04 | frame 12 | not judged (software) | terrain 272.68, clouds 152.19, water 151.37, skyAndFog 69.94 |
| sky-mackerel-sky | 527.67 | 569.23 | frame 12 | not judged (software) | terrain 171.43, clouds 161.45, water 74.99, skyAndFog 72.75 |
| sky-veiled-sun | 560.10 | 602.95 | frame 12 | not judged (software) | clouds 194.89, terrain 171.59, water 72.66, skyAndFog 71.87 |
| sky-rain-deck | 819.50 | 879.25 | frame 14 | not judged (software) | terrain 276.10, clouds 244.16, water 163.72, skyAndFog 96.69 |
