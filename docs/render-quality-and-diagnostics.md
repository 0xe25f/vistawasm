# Render Quality and Diagnostics

This covers the three tools VistaWASM gives you for understanding and
tuning what it renders: `RenderQualityOptions`, `RenderStats`, and
`DebugView`.

## Find what is slow first

`RenderStats.gpuPassTimesMs` reports the GPU time of each pass: terrain, trees, grass, clouds, sky and fog, water,
shadows, tree culling, and upscaling with lens drops. The demo lists them, largest first,
in its stats panel. Look there before changing settings: the pass at the
top is the one worth making cheaper. It needs the browser's
`timestamp-query` feature (current Chrome and Edge have it); elsewhere it
is `null`. Readings arrive a few frames late and are only taken when the
previous one has arrived, so measuring never stalls rendering.

## Load time and pipeline warm-up

The first frame waits for the textures and pipelines it draws with, and
nothing else. Pipelines are created when the scene needs them, in the
order the frame draws: a map without rivers never compiles the river
pipeline, and one without grass never compiles grass. Procedural textures
are baked the same way: only the terrain materials the ground uses, the
bark and leaves of the species present, and the cloud volume when clouds
or volumetric mist need it. What the scene may need soon, such as rain
and its clouds when the weather can turn, is created after the first
frame is on screen, one pipeline per frame, so it never holds the first
frame back and is ready when it is needed.

To see the effect, time the first frame from `createVistaEngine()`:

```ts
const started = performance.now();
const engine = await createVistaEngine({ canvas });
await engine.generateFractal({
  seed: 1,
  size: 512,
  horizontalScaleMetres: 12,
  verticalScale: 1,
  noise: { kind: "ridged", octaves: 7, gain: 0.52, lacunarity: 2.05 }
});
engine.renderOnce();
await new Promise(requestAnimationFrame);
console.log(`first frame after ${Math.round(performance.now() - started)} ms`);
```

Under software WebGPU the default scene's first frame arrives in about
half the time it took before pipelines and textures were created on
demand. Turning a system on later (grass, clouds, screen reflections)
compiles its pipeline once, on the next frame.

## Frame rate and resolution

VistaWASM aims for a steady frame rate on any display, from a 1080p
laptop to a 4K monitor or a phone. Three settings do this:

| Setting | Default | What it does |
| --- | --- | --- |
| `maxFrameRate` | `60` | `start()` renders evenly spaced frames at this rate, so a 120 or 144 Hz display shows a steady 60 rather than a rate that swings with the scene. `0` renders on every animation frame. |
| `dynamicResolution` | `true` | Renders the scene below the canvas resolution when frames arrive late, and back up to `renderScale` when there is time to spare. |
| `renderScale` / `minRenderScale` | `1` / `0.5` | The highest and lowest fraction of the canvas resolution to render at. |

Most of the cost of clouds, sky, fog, and rain is per pixel, and a 4K
display at a device pixel ratio of 2 has four times the pixels of 1080p.
Rendering at 75 % of each side shades 56 % of the pixels. A final pass
upscales the image with contrast-adaptive sharpening, which keeps edges
crisp; at typical viewing distances 75 % is hard to tell from full
resolution. `RenderStats.renderScale` reports the scale in use.

```ts
engine.setRenderQuality({
  preset: "balanced",
  maxFrameRate: 60,
  dynamicResolution: true,
  minRenderScale: 0.5
});

// A fixed 75 %, for example on a phone:
engine.setRenderQuality({
  preset: "balanced",
  renderScale: 0.75,
  dynamicResolution: false
});
```

Dynamic resolution judges the real interval between rendered frames, so
it works in every browser, with or without GPU timing. It holds the cap,
or 60 when uncapped. It lowers the scale within half a second of frames
running late, raises it one step after two calm seconds, and avoids for
ten seconds a scale that has just dropped frames, so it settles instead
of swinging.

Overcast, rain, and storm skies also march cloud lighting with fewer
samples: under a thick grey layer the fine light detail that more samples
resolve cannot be seen.

## Distances and presets (`RenderQualityOptions`)

Three distances limit work far from the camera. See
[`docs/options-reference.md`](options-reference.md#renderqualityoptions)
for the exact fields.

| Setting | What it does | Cost it saves |
| --- | --- | --- |
| `renderDistanceMetres` | Terrain, trees, and water past it are not shaded; horizon-coloured fog thickens over the `renderFadeMetres` before it and is complete at it, so the edge is never seen. | Terrain and tree shading in the distance. |
| `detailDistanceMetres` | Past it, terrain takes one far-scale texture sample per material instead of up to eight. Textures there are so minified that the two look the same. | Terrain texture sampling. |
| `cloudDistanceMetres` | Clouds and rain curtains are raymarched only this far, thinning out over the `cloudFadeMetres` before it. | The longest cloud marches, near the horizon. |

`preset` fills in any distance you leave unset:

| `preset` | Render | Detail | Clouds |
| --- | --- | --- | --- |
| `"preview"` | 6 km | 400 m | 12 km |
| `"balanced"` (default) | unlimited | 2 km | 60 km |
| `"high"` | unlimited | 5 km | 90 km |
| `"offline"` | unlimited | unlimited | 90 km |

Measured in one scene (a 12 km island seen from 7.5 km away, software
GPU), GPU time per frame: `"offline"` 1330 ms, `"balanced"` 1164 ms (no
visible difference), `"preview"` 981 ms, and `"balanced"` with a 4 km
render distance 957 ms. Most of the saving was terrain shading. Real GPUs
are much faster, and the proportions differ, so check the profiler.

Both fades are yours to set, in metres: `renderFadeMetres` (default a
third of the render distance) and `cloudFadeMetres` (default 30 % of the
cloud distance). A long fade hides the edge more gently; `0` gives a hard
edge. Each is capped at its distance.

```ts
engine.setRenderQuality({
  preset: "balanced",
  renderDistanceMetres: 8000,
  renderFadeMetres: 3000, // fog from 5 km, complete at 8 km
  cloudDistanceMetres: 30000,
  cloudFadeMetres: 10000 // clouds thin out from 20 km
});
```

`preset` does not change `FloraOptions.treeQuality`, `GrassOptions`,
`CloudsOptions.style`, or `MistOptions.style`, and it does not limit
erosion (that is `ErosionOptions.quality`; see
[`docs/terrain-data.md`](terrain-data.md#erosion)).

`floraDensityScale` is a global multiplier applied to both
`FloraOptions.density` and `GrassOptions.density` (see
[`docs/vegetation.md`](vegetation.md#performance)).

`maxClipmapLevels` is only used by native/test builds without a GPU, to
compute a theoretical `RenderStats.terrainTriangles`/`clipmapLevels`
estimate; browser builds report the real uploaded mesh's stats regardless
of this value (see
[`docs/architecture.md`](architecture.md#terrain-rendering-and-level-of-detail)).

Other per-feature levers: `CloudsOptions.temporal` (reuse distant clouds
between frames), `CloudsOptions.raymarchSteps` (or `style: "painted"`),
`CloudsOptions.resolutionScale` (clouds render at half resolution by
default; `0.25` is cheaper still), `ShadowOptions.trees.resolution` and
`distanceMetres`, `FloraOptions.meshDistanceMetres`, and
`GrassOptions.viewDistanceMetres`. Storm weather (or
`CloudsOptions.towering`) makes the cloud layer up to 2.6 times as tall
and costs more than fair weather.

## Render statistics (`RenderStats`)

Returned by `engine.renderOnce()` and emitted on every rendered frame via
the `"stats"` event:

```ts
let lastFrameAt = performance.now();

engine.on("stats", (stats) => {
  const now = performance.now();
  const fps = Math.round(1000 / Math.max(1, now - lastFrameAt));
  lastFrameAt = now;
  updateHud({ fps, ...stats });
});
```

See [`docs/options-reference.md`](options-reference.md#renderstats-from-enginerenderonce-and-the-stats-event)
for the exact field list. `gpuFrameTimeMs` is the sum of
`gpuPassTimesMs`, when the browser supports timestamp queries.
`activeGpuMemoryBytes` is always `null` today.

`frameTimeMs` is the CPU time the JavaScript wrapper measures around the
call into the WASM module: the time to record and submit the frame. The
GPU runs that work afterwards, so a GPU-bound scene can drop frames while
`frameTimeMs` stays small. To measure the real frame rate, time the gap
between `"stats"` events with `performance.now()`, as below.

`weather` holds the dominant weather state while the weather system is
on, and `null` otherwise; the `"weatherChanged"` event fires when it
changes.

`terrainTriangles`/`clipmapLevels` reflect the real uploaded terrain mesh
on browser builds — a constant `512 × 512 × 2` triangle budget regardless
of terrain `size`, since the mesh always recentres on the camera at a fixed
sample count rather than growing with terrain size (see
[`docs/architecture.md`](architecture.md#terrain-rendering-and-level-of-detail)).
Native/test builds without a GPU report a theoretical estimate from
`terrain::clipmap::build_clipmap_levels` instead.

## Debug views (`DebugView`)

`engine.setDebugView(view)` accepts `"none" | "height" | "slope" |
"normals" | "lod" | "flow" | "materials" | "no-data" | "biomes"`. These
modes replace the terrain's textured shading with a flat-lit overlay:

| View | Shows |
| --- | --- |
| `"height"` | Height above sea level, green lowlands to pale peaks. |
| `"slope"` | Flat (green) to steep (red). |
| `"normals"` | World-space normals as colour. |
| `"materials"` | The dominant surface material (lush grass, dry grass, forest floor, sand, rock, snow, mud, volcanic, glacier ice, tundra, river gravel). |
| `"biomes"` | The biome map, one colour per biome (see [`docs/biomes.md`](biomes.md)). |

`"lod"`, `"flow"`, and `"no-data"` are accepted but currently render like
`"none"`. Trees, water, sky, and fog render normally in every mode.

## Building your own performance HUD

Dynamic resolution already holds the frame rate. To trade other detail
for speed as well, combine `RenderStats` with the levers above:

```ts
let lastFrameAt = performance.now();
let slowFrameStreak = 0;

engine.on("stats", () => {
  const now = performance.now();
  const frameIntervalMs = now - lastFrameAt;
  lastFrameAt = now;
  slowFrameStreak = frameIntervalMs > 20 ? slowFrameStreak + 1 : 0;

  if (slowFrameStreak > 100) {
    // About two seconds below 50 FPS.
    engine.setRenderQuality({ preset: "balanced", floraDensityScale: 0.5 });
    slowFrameStreak = 0;
  }
});
```

Tune the specific thresholds to your target frame budget and hardware —
these numbers are illustrative, not a recommendation.
