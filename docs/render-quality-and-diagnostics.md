# Render Quality and Diagnostics

This covers the three tools VistaWASM gives you for understanding and
tuning what it renders: `RenderQualityOptions`, `RenderStats`, and
`DebugView`.

## Render quality presets (`RenderQualityOptions`)

See [`docs/options-reference.md`](options-reference.md#renderqualityoptions)
for the exact field list. The one field with real, direct effect today is
`preset`, which caps the total erosion iteration budget requested by
`ErosionOptions` regardless of what a caller asks for — `"preview"` keeps a
UI responsive while a user drags sliders, `"balanced"` is a reasonable
default for normal play, `"high"`/`"offline"` allow the full requested
iteration count for a final export or screenshot. See
[`docs/terrain-data.md`](terrain-data.md#erosion) for the exact caps.

`floraDensityScale` is a global multiplier applied to both
`FloraOptions.density` and `GrassOptions.density` — the cheapest lever if
vegetation fill-rate is your bottleneck (see
[`docs/vegetation.md`](vegetation.md#performance)).

`maxClipmapLevels` is only used by native/test builds without a GPU, to
compute a theoretical `RenderStats.terrainTriangles`/`clipmapLevels`
estimate; browser builds report the real uploaded mesh's stats regardless
of this value (the terrain mesh's actual vertex budget is fixed — see
[`docs/architecture.md`](architecture.md#terrain-rendering-and-level-of-detail)).

`preset` does **not** automatically change `FloraOptions.treeQuality`,
`GrassOptions`, `CloudsOptions.style`, or `MistOptions.style` — those
default independently and a host sets each explicitly. The most effective
per-feature levers are `CloudsOptions.raymarchSteps` (or `style:
"painted"`), `CloudsOptions.resolutionScale` (clouds render at half
resolution by default; `0.25` is cheaper still),
`ShadowOptions.trees.resolution` and `distanceMetres`,
`FloraOptions.meshDistanceMetres`, and `GrassOptions.viewDistanceMetres`.
Storm weather (or `CloudsOptions.towering`) makes the cloud layer up to
2.6 times as tall and costs more than fair weather; `rainShafts` adds a
short march in the cloud pass. If you want
"one dial" behaviour (cheap tiers at `"preview"`/`"balanced"`, expensive
tiers at `"high"`/`"offline"`), implement that mapping yourself in your own
UI/settings code.

## Render statistics (`RenderStats`)

Returned by `engine.renderOnce()` and emitted on every rendered frame via
the `"stats"` event:

```ts
engine.on("stats", (stats) => {
  const fps = stats.frameTimeMs > 0 ? Math.round(1000 / stats.frameTimeMs) : 0;
  updateHud({ fps, ...stats });
});
```

See [`docs/options-reference.md`](options-reference.md#renderstats-from-enginerenderonce-and-the-stats-event)
for the exact field list. Two fields are always `null` today regardless of
platform: `gpuFrameTimeMs` and `activeGpuMemoryBytes` — the type reserves
space for them, but no current backend populates either. Use
`frameTimeMs` (measured by the JavaScript wrapper around the call into the
WASM module) for real performance measurement instead.

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
| `"materials"` | The dominant surface material (lush grass, dry grass, forest floor, sand, rock, snow, mud, volcanic). |
| `"biomes"` | The biome map, one colour per biome (see [`docs/biomes.md`](biomes.md)). |

`"lod"`, `"flow"`, and `"no-data"` are accepted but currently render like
`"none"`. Trees, water, sky, and fog render normally in every mode.

## Building your own performance HUD

Combine `RenderStats` with the levers above for an adaptive-quality loop:

```ts
let lowFrameTimeStreak = 0;

engine.on("stats", (stats) => {
  if (stats.frameTimeMs > 20) {
    lowFrameTimeStreak += 1;
  } else {
    lowFrameTimeStreak = 0;
  }

  if (lowFrameTimeStreak > 120) {
    // Roughly two seconds of a real terrain frame budget breach at ~60 FPS.
    engine.setRenderQuality({ preset: "balanced", floraDensityScale: 0.5 });
    lowFrameTimeStreak = 0;
  }
});
```

Tune the specific thresholds to your target frame budget and hardware —
these numbers are illustrative, not a recommendation.
