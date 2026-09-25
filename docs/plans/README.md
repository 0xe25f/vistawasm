# Realism plans

These plans cover the next round of realism work. Each plan is complete:
carrying it out delivers the whole feature, including tests, docs, demo
controls and verification, with no stubs and no regressions. Each plan
repeats the shared ground rules so it can be handed over on its own.

Carry them out in this order. Later plans build on what earlier plans add,
and each plan's **Prerequisites** section says what it needs and how to
check it is there.

| Order | Plan | Summary |
| --- | --- | --- |
| 1 | [Realistic terrain generation](01-realistic-terrain-generation.md) | Geology-led landforms, multi-scale GPU erosion, landform presets. |
| 2 | [Ice and arctic biome](02-ice-arctic-biome.md) | `iceArctic`: glaciers, tundra fringe, sea ice and permanent snow, driven by climate temperature. |
| 2b | [Course correction](02b-course-correction.md) | Sea-ringed map edges and a skirt beyond them, real alpine relief, realistic pack ice, stretched-snow fix, configurable lens drops that are never cut off, and a like-for-like performance gate. |
| 3 | [Rivers, lakes and waterfalls](03-rivers-lakes-and-waterfalls.md) | Erosion-coupled drainage from rain, snowmelt, springs and overflowing lakes; flowing water, real waterfalls, wet banks and audio queries. |
| 3b | [Rivers at landscape scale](03b-rivers-at-landscape-scale.md) | Big rivers fed from beyond the map, screen-space reflections, gravel bars and stones, rock-walled rapids, green banks, small streams at their true size, falls and pools at their size, and a faster first load. |
| 4 | [Tree placement](04-tree-placement.md) | Trees on the rendered ground at every distance, and ecological placement rules. |
| 5 | [Vegetation density](05-vegetation-density.md) | Closed-canopy forests and full grass cover at 60 FPS: streamed placement, a far canopy layer and a forest floor. |
| 5b | [Rock outcrops and scree](05b-rock-outcrops-and-scree.md) | Rock where soil is thin, in banded crags and ledges; jointed, lichened, streaked stone; scree fans and grounded boulders below outcrops. |
| 6 | [Tree realism](06-tree-realism.md) | Grown branching, real leaf clumps, per-tree variation and layered wind. |
| 7 | [Sky and weather presets](07-sky-and-weather-presets.md) | One blended, editable preset table; regional weather cells; humidity haze; wet surfaces that dry; wind on trees and sea; overcast light; time of day with a golden-hour clearing bias. |
| 8 | [Cloud bases and mid-level clouds](08-cloud-bases-and-mid-level-clouds.md) | Lumpy, shaded low-cloud bases and an optional altocumulus and altostratus layer. |
| 9 | [Map export](09-map-export.md) | Biome, water, flow, material, slope, normal, occlusion and vegetation maps, as PNG, raw data and a zip bundle. |
| 10 | [Map import](10-map-import.md) | Import heightmaps with painted biome, water and vegetation maps, and round-trip export bundles. |
| 11 | [Demo overlays and paint tab](11-demo-overlays-and-paint-tab.md) | Toggle the overlays, and paint height, biomes, water and vegetation in 2D, then render in 3D. |

Every plan carries the same ground rules, including the commit identity
and the rule that nothing is signed as AI-written: no `Co-Authored-By:
Claude` lines, no "Generated with Claude Code" footers and no session
links, in commits, pull requests, code or docs.

Budgets across all the plans, measured at 1920 x 1080 on a mid-range
GPU:

- Default scene: at most 12 ms of GPU time per frame. Rain or storm: at
    most 14 ms. That leaves headroom under the 16.7 ms of a 60 FPS frame.
- Gzipped WASM: at most +176 KB over 237,820 bytes in total. Each plan
    states its own share. Plans 1 and 2 were allowed up to 32 KB and
    12 KB, and plans 1, 2 and 2b together up to 44 KB. They added 39,131
    bytes, reaching 276,951.
- Every byte must buy real value that can't be done smaller, without lag,
    frame drops, regressions or any loss of realism.
- From plan 3 onwards, each plan's size figure is a soft target, and
    double it is the hard limit. Plan 2b added 8,560 bytes (within its
    8,900), reaching 285,511. Plan 3 reached 323,501. Plan 3b added
    25,553 bytes (a 14 KB soft target and a 28 KB hard limit), reaching
    349,054.
