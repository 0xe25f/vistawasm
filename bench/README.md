# Benchmarks

Size and speed comparisons between VistaWASM and two other open-source
terrain libraries for the web:

- [THREE.Terrain](https://github.com/IceCreamYou/THREE.Terrain)
  (`three.terrain.js`), a procedural terrain generator for three.js.
- [three-terrain](https://github.com/danielesteban/terrain)
  (`three-terrain`), a fast heightmap voxeliser for three.js.

Both are good at what they set out to do, and both run on WebGL, so they
reach more browsers than VistaWASM, which needs WebGPU. THREE.Terrain is
actively maintained; three-terrain was last published in 2022 and depends
on three.js 0.133. These numbers compare like with like where the
libraries overlap, and say plainly where they do not.

This folder is not part of the published package. It has its own
`package.json`, so the library gains no dependencies.

## Running them

Build VistaWASM first (from the repository root), then install and run the
benchmarks:

```bash
npm ci && npm run build
cd bench
npm install
npm run size    # production bundle sizes, printed as a table
npm run speed   # opens the speed page in your browser
```

The speed page needs a browser with WebGPU for the VistaWASM rows; the
other rows run anywhere. Versions are pinned in `package.json`.

## Download size

`npm run size` builds the smallest useful app for each library (generate
a terrain, light it, and draw it; see `size/`) with Vite in production
mode, minified and tree-shaken, and totals the gzipped JavaScript and
WebAssembly a browser downloads (1 KB = 1,000 bytes).

| App | Raw | Gzipped |
| --- | --- | --- |
| VistaWASM 1.1.0 | 755 KB | 262 KB |
| three.js 0.186 + THREE.Terrain 3.1.1 | 579 KB | 148 KB |
| three.js 0.133 + three-terrain 0.0.10 | 1,077 KB | 268 KB |

VistaWASM's download is bigger than three.js with THREE.Terrain. It is
also the whole engine: the same 262 KB draws the sky and clouds, water,
biomes and trees, weather, and shadows, while the other two apps draw a
plainly lit terrain and would grow with each of those features. If you
only need a terrain mesh inside an existing three.js scene, THREE.Terrain
is the smaller choice.

## Terrain generation speed

`npm run speed` times how long each library takes to turn a 512 × 512
terrain request into a mesh ready to draw. Each case runs once to warm up,
then nine times.

| Library | What is measured | Median | Fastest |
| --- | --- | --- | --- |
| VistaWASM 1.1.0 | 5-octave simplex fractal, normals, surface materials, and GPU mesh upload | 161 ms | 152 ms |
| VistaWASM 1.1.0 | The same, plus biomes, rivers and lakes, and tree placement | 267 ms | 257 ms |
| THREE.Terrain 3.1.1 | 5-octave simplex (`SimplexLayers`) and mesh with normals | 256 ms | 150 ms |
| three-terrain 0.0.10 | Voxel mesh of a 512 × 512 heightmap supplied by the caller | 67 ms | 57 ms |

Measured on 23 September 2026 in headless Chromium 141 on a 4-core Intel
Xeon at 2.1 GHz, with WebGPU provided by SwiftShader (software). Your
numbers will differ; run the page on your own hardware.

What this shows:

- For a comparable fractal terrain, VistaWASM and THREE.Terrain have about
  the same best case (152 ms and 150 ms). VistaWASM was steadier from run
  to run, so its median was lower (161 ms against 256 ms).
- VistaWASM builds a complete world (biomes, rivers, lakes, and tree
  placement) in about 270 ms, work the other libraries leave to you.
- three-terrain is fastest at its own job, which is different: it
  voxelises a heightmap you give it and does no noise generation, so its
  time is not directly comparable.

## What is not measured

Frame rate. Rendering speed depends on the GPU, and the three libraries draw
very different scenes (VistaWASM draws water, vegetation, volumetric clouds,
and shadows; the others draw a lit terrain), so a frame-rate race would not
be a fair comparison. The machine used here also has no hardware GPU. Use
`RenderStats.frameTimeMs` in your own scene instead; see
[`docs/render-quality-and-diagnostics.md`](../docs/render-quality-and-diagnostics.md).

## Features at a glance

From each project's published README and source, as of the versions above.

| | VistaWASM | THREE.Terrain | three-terrain |
| --- | --- | --- | --- |
| Graphics API | WebGPU | WebGL (three.js) | WebGL (three.js) |
| Browser reach | WebGPU browsers only | WebGL 2 browsers (three.js 0.186) | WebGL browsers (three.js 0.133) |
| Works inside an existing three.js scene | No, it owns its canvas | Yes | Yes |
| Procedural generation | Fractal noise with shaping and GPU erosion | Many generators (Diamond-Square, Perlin, Simplex, Worley, and more) and filters | No; supply a heightmap image or function |
| Real-world elevation data | GeoTIFF DEMs and raw heightmaps | Heightmap images | Heightmap images |
| Texturing | Procedural, biome-driven materials | Height- and slope-blended textures you supply | Colour map image or height colour bands |
| Vegetation | Eight modelled tree species and grass | Scattered meshes and instanced grass | Not included |
| Water | Waves, currents, rivers, and lakes | Not included | Not included |
| Sky, clouds, and weather | Atmosphere, volumetric clouds, weather system | Not included (use three.js) | Not included (use three.js) |
| Shadows | Terrain, tree, and cloud | three.js shadow maps | None (unlit, with baked ambient occlusion) |
| Licence | AGPL-3.0 | MIT | MIT |
