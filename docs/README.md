# VistaWASM Documentation

Start with the path that matches what you want to do.

## I want to use VistaWASM in my app

Read these in order:

1. [Getting started](getting-started.md): install the package, create an
    engine, generate terrain, render, resize, and deploy.
2. [Options reference](options-reference.md): every option, its default,
    and its valid range.
3. [Events, errors, and lifecycle](events-errors-and-lifecycle.md): what
    the engine tells you, and how to handle failures.

Then pick the guides for the features you use.

### Building the world

| Guide | What it covers |
| --- | --- |
| [World design guide](world-design-guide.md) | What each terrain, erosion, and sky control does, with recipes |
| [Terrain data](terrain-data.md) | Fractal generation, GeoTIFF elevation files, and raw heightmaps |
| [Biomes](biomes.md) | The nineteen biomes, ice and tundra, and how to shape the climate |
| [Vegetation](vegetation.md) | Tree species, grass, quality tiers, and performance |
| [Water](water.md) | Ocean waves, currents, rivers, and lakes |

### Sky, weather, and light

| Guide | What it covers |
| --- | --- |
| [Sky, atmosphere, and weather](sky-atmosphere-and-weather.md) | Sun, sky, clouds, cloud types, cirrus, and mist |
| [Weather](weather.md) | Weather states, transitions, cycling, and what the weather drives |
| [Shadows](shadows.md) | Terrain, tree, and cloud shadows, and their costs |

### Your own assets and your own app

| Guide | What it covers |
| --- | --- |
| [Replacing trees, textures, and placement](hooks.md) | Custom tree models, species rules, hand-placed trees, and textures |
| [React, Vue, and Svelte](frameworks.md) | Complete components for each framework |
| [three.js](threejs.md) | three.js objects over a VistaWASM world, or VistaWASM terrain in a three.js scene |
| [Camera and controls](camera-and-controls.md) | The camera model and the ready-made fly camera |
| [Export and snapshots](export-and-snapshots.md) | Heightmap, PNG, OBJ, and screenshot export |
| [Render quality and diagnostics](render-quality-and-diagnostics.md) | Quality settings, render statistics, and debug views |
| [Building games](game-development.md) | Game loops, height queries, collision, and performance |
| [Game engine integration](engine-integration.md) | Using VistaWASM alongside Babylon.js, PlayCanvas, and other engines |

## I want to change VistaWASM itself

1. [Contributing](../CONTRIBUTING.md): set up, build, test, and open a pull
    request.
2. [Testing and contributing reference](testing-and-contributing.md):
    tool versions, offline builds, and the details behind each check.
3. [Architecture](architecture.md): how the engine works, frame by frame.
4. [`AGENTS.md`](../AGENTS.md): the project's style and process rules.

## Project history and comparisons

- [Changelog](../CHANGELOG.md): what changed in each release.
- [Benchmarks](../bench/README.md): download size and generation speed
  compared with THREE.Terrain and three-terrain, and how to reproduce them.
- [Environment upgrade plan](environment-upgrade-plan.md): the original
  plan for trees, grass, clouds, and mist, kept as a historical record.
