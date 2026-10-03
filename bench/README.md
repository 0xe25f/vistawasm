# Benchmarks

Size and speed comparisons between VistaWASM and three other open-source
terrain libraries for the web:

- [THREE.Terrain](https://github.com/IceCreamYou/THREE.Terrain)
  (`three.terrain.js`), a procedural terrain generator for three.js.
- [three-terrain](https://github.com/danielesteban/terrain)
  (`three-terrain`), a fast heightmap voxeliser for three.js.
- [CesiumJS](https://github.com/CesiumGS/cesium) (`cesium`), a 3D globe
  that streams real-world terrain, imagery and 3D Tiles.

Each is good at what it sets out to do, and all three run on WebGL, so
they reach more browsers than VistaWASM, which needs WebGPU.
THREE.Terrain and CesiumJS are actively maintained; three-terrain was
last published in 2022 and depends on three.js 0.133. These numbers
compare like with like where the libraries overlap, and say plainly where
they do not.

This folder is not part of the published package. It has its own
`package.json`, so the library gains no dependencies.

## Running them

Build VistaWASM first (from the repository root), then install and run the
benchmarks:

```bash
npm ci && npm run build
cd bench
npm install
npm run size    # production bundle sizes, printed as a table (needs Playwright)
npm run speed   # opens the speed page in your browser
npm run gpu     # opens the GPU profile page (VistaWASM only)
```

The speed page needs a browser with WebGPU for the VistaWASM rows; the
other rows run anywhere. Versions are pinned in `package.json`. The size
script opens the CesiumJS app in Chromium through Playwright, which is
not a dependency: set `PLAYWRIGHT_MODULE` to its path if it is not
resolvable, and `CHROMIUM_PATH` to a Chromium build if needed.

## Download size

`npm run size` builds the smallest useful app for each library (generate
a terrain, light it, and draw it; see `size/`) with Vite in production
mode, minified and tree-shaken, and totals the gzipped JavaScript and
WebAssembly a browser downloads (1 KB = 1,000 bytes; gzip level 9 from
Node's zlib, which compresses a little differently from the `gzip` tool
`scripts/check-size.mjs` uses). CesiumJS fetches its web workers at run
time, so its app is opened in Chromium and the workers it fetches are
counted too. Its sky textures, which it also fetches, are not.

| App | Raw | Gzipped |
| --- | --- | --- |
| VistaWASM 2.0.0 | 1,383 KB | 566 KB |
| three.js 0.186 + THREE.Terrain 3.1.1 | 577 KB | 147 KB |
| three.js 0.133 + three-terrain 0.0.10 | 1,075 KB | 268 KB |
| CesiumJS 1.146 | 4,446 KB | 1,178 KB |

Measured on 2 October 2026. The figure given before for VistaWASM,
530 KB, left out part of the WebAssembly; the WebAssembly itself has
since become 8 KB smaller. Served with Brotli, VistaWASM's app is about
460 KB (see [deployment](../docs/getting-started.md#7-deploy)).

VistaWASM's download is bigger than the two three.js apps. It is also the
whole engine: the same 566 KB draws the sky and clouds, water, biomes and
trees, weather, and shadows, while those apps draw a plainly lit terrain
and would grow with each of those features. If you only need a terrain
mesh inside an existing three.js scene, THREE.Terrain is the smaller
choice. CesiumJS is twice VistaWASM's size: it is a whole globe, with
streaming, imagery, 3D Tiles, entities and a camera for the planet.

## Terrain generation speed

`npm run speed` times how long each library takes to turn a 512 × 512
terrain request into a mesh ready to draw. Each case runs once to warm up,
then nine times. CesiumJS streams terrain as tiles meshed on web workers,
so its time runs from handing it a new terrain provider to every tile in
view being loaded.

| Library | What is measured | Median | Fastest |
| --- | --- | --- | --- |
| VistaWASM 2.0.0 | Continental terrain from tectonic uplift and drainage with 5-octave simplex detail, normals, surface materials, and GPU mesh upload | 623 ms | 604 ms |
| VistaWASM 2.0.0 | The same, plus biomes, rivers and lakes, rock, and tree placement | 1,981 ms | 1,884 ms |
| THREE.Terrain 3.1.1 | 5-octave simplex (`SimplexLayers`) and mesh with normals | 240 ms | 184 ms |
| three-terrain 0.0.10 | Voxel mesh of a 512 × 512 heightmap supplied by the caller | 101 ms | 70 ms |
| CesiumJS 1.146 | Terrain tiles in view of a 512 × 512 height field supplied by the caller, meshed on workers | 3,830 ms | 3,631 ms |

Measured on 2 October 2026 in headless Chromium 141 on a 4-core Intel
Xeon at 2.8 GHz, with WebGPU and WebGL provided by SwiftShader
(software). Your numbers will differ; run the page on your own hardware.
This machine is slower than the one used on 30 September: a build from
before the input checks and error recovery were added took 634 ms and
1,967 ms on it, so they cost no measurable speed.

What this shows:

- VistaWASM is the slowest to a first terrain. Since 2.0 even its
    plainest terrain is geology: tectonic uplift and a drainage network
    shape the land before the noise adds detail, where THREE.Terrain sums
    noise alone. On 1.1.0's noise-only generator, the same case took
    161 ms.
- A complete world (biomes, routed rivers and lakes, rock where the soil
    is thin, and tree placement) takes about 1.8 s here, work the other
    libraries leave to you. With erosion, which runs on the GPU, it takes
    longer again.
- three-terrain is fastest at its own job, which is different: it
    voxelises a heightmap you give it and does no noise generation, so its
    time is not directly comparable.
- CesiumJS takes longest, though it generates nothing: its time is the
    tile pipeline (requests, a worker per tile, level of detail for the
    view), which is built to stream a planet rather than to make one
    map quickly. Its time also depends on the view, since more tiles are
    in view near the horizon.

## What is not measured

Frame rate. Rendering speed depends on the GPU, and the four libraries draw
very different scenes (VistaWASM draws water, vegetation, volumetric clouds,
and shadows; the others draw a lit terrain, CesiumJS with its sky), so a frame-rate race would not
be a fair comparison. The machine used here also has no hardware GPU.
Measure the time between frames in your own scene instead; see
[`docs/render-quality-and-diagnostics.md`](../docs/render-quality-and-diagnostics.md).

## GPU profile

`npm run gpu` opens `gpu/`, which times VistaWASM's own budgeted scenes
pass by pass on your GPU and gives the results as a table and as JSON.
It compares VistaWASM with its budgets, not with other libraries; see
[`docs/performance.md`](../docs/performance.md).

## Features at a glance

From each project's published README and source, as of the versions above.

| | VistaWASM | THREE.Terrain | three-terrain | CesiumJS |
| --- | --- | --- | --- | --- |
| Graphics API | WebGPU | WebGL (three.js) | WebGL (three.js) | WebGL |
| Browser reach | WebGPU browsers only | WebGL 2 browsers (three.js 0.186) | WebGL browsers (three.js 0.133) | WebGL browsers |
| Works inside an existing three.js scene | As an overlay on its own canvas, or as exported terrain ([guide](../docs/threejs.md)) | Yes | Yes | No; its own renderer and canvas |
| Procedural generation | Geology-led: tectonic uplift, drainage, fractal detail, GPU erosion | Many generators (Diamond-Square, Perlin, Simplex, Worley, and more) and filters | No; supply a heightmap image or function | No; supply terrain tiles or a height callback |
| Real-world elevation data | GeoTIFF DEMs and raw heightmaps, up to 2048 × 2048 | Heightmap images | Heightmap images | The whole planet, streamed as tiles (Cesium World Terrain needs a Cesium ion account), or your own tile server |
| Texturing | Procedural, biome-driven materials | Height- and slope-blended textures you supply | Colour map image or height colour bands | Imagery layers (satellite and map tiles) and elevation or slope colour ramps |
| Vegetation | Eight modelled tree species and grass | Scattered meshes and instanced grass | Not included | Not included; place your own glTF models or 3D Tiles |
| Water | Waves, currents, rivers, and lakes | Not included | Not included | An animated ocean surface where terrain tiles carry a water mask |
| Sky, clouds, and weather | Atmosphere, volumetric clouds, weather system | Not included (use three.js) | Not included (use three.js) | Atmosphere, sun, moon, stars and fog; billboard clouds; no weather |
| Shadows | Terrain, tree, and cloud | three.js shadow maps | None (unlit, with baked ambient occlusion) | Shadow maps for terrain and models |
| Licence | AGPL-3.0 | MIT | MIT | Apache-2.0 |
