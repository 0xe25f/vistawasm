# Plan 5: Vegetation density

## Goal

Tree and grass density sliders go high enough to make:

- real closed-canopy forests and jungles, roughly 400 to 1,000 trees per
    hectare near the camera where the land supports them;
- meadows fully covered in grass.

The scene holds 60 FPS: full density near the camera, stochastic
thinning further out that keeps the canopy looking solid, and a canopy
layer on distant hillsides. Under dense canopy, the ground becomes a
forest floor: leaf litter, ferns and undergrowth, in dappled, damp shade.

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

- Maximum means a real closed canopy. The sliders become 0 to 4, where 1
    is today's maximum. The default values stay the same, and so does the
    default look.
- Performance comes from three tools:
    - distance-based thinning with fewer, larger far trees;
    - a canopy layer on the terrain beyond a few kilometres;
    - an instance budget that the quality presets and dynamic resolution
        respect.
- The forest floor changes under dense canopy.

## Prerequisites

- **Plan 4** (tree placement) must be merged. Check for
    `mesh_surface_height` in `crates/vista_wasm/src/shaders/common.wgsl`,
    the `SpeciesNiche` table in `render/flora.rs`, and cull-pass
    grounding.
- Plan 4 depends on plans 1 to 3. If any is missing, carry it out first.

## Current state

- `render/flora.rs`, `build_tree_instances`, runs on the CPU. It uses at
    most one candidate per height-map sample, on a grid of at most
    `MAX_CANDIDATE_SAMPLES_PER_SIDE = 512` per side. At 12 m per sample
    that caps density near 70 trees per hectare, whatever the slider says.
    This is the hard ceiling to remove.
- `render/grass.rs` does the same with a 768-per-side limit.
- `FloraOptions`:
    - `density` from 0 to 1 (default 0.35);
    - `maxInstances` (default 500,000);
    - `meshDistanceMetres`.
- `GrassOptions`: `density` from 0 to 1 (default 0.5),
    `viewDistanceMetres`, `maxInstances` (200,000).
- `RenderQualityOptions.floraDensityScale` multiplies both.
- `shaders/tree_cull.wgsl`:
    - frustum-culls and picks full mesh or impostor by distance;
    - writes indirect draw counts;
    - with plan 4, also grounds each tree.
- Dynamic resolution lives in `crates/vista_wasm/src/pacing.rs`
    (`ResolutionController`).

## Design

### 1. One candidate lattice for the whole world

- Every potential tree is a point on a world-space lattice with a 3 m
    pitch (about 1,100 candidates per hectare).
    - The lattice is jittered per point by a hash of its integer
        coordinates, which gives three values: jitter x, jitter z, and
        rank r in [0, 1).
    - The hash is `pcg3d(ix, iz, seed)`, implemented identically in Rust
        and WGSL. A test checks bit-identical output on 10,000 points.
- A candidate is a tree when `r < p(x)`, where:
    - `p(x) = target_density(d) x suitability(x) / 1100`;
    - `d` is the slider value;
    - `suitability` comes from plan 4's rules (0 to 1).
- `target_density(d)`, in trees per hectare, is piecewise linear:

    | d | 0 | 0.35 | 1 | 2 | 3 | 4 |
    | --- | --- | --- | --- | --- | --- | --- |
    | Trees per hectare | 0 | 25 | 70 | 220 | 480 | 900 |

    The first three points match today's densities, so the default look
    is kept. Calibrate 0.35 and 1 against the old build's instance counts
    on the default map, within ±10 %.
- **Species and scale:** the species comes from plan 4's weighted choice
    at x, using a second hash. Clustering (plan 4's parents and
    seedlings) is kept by folding the grove noise and the parent boost
    into `p(x)`, so the lattice alone decides where trees are.

### 2. Suitability on the GPU

- Bake suitability at height-map resolution into a new rgba8 texture,
    the cover texture, at `group(1)` next to the surface texture:
    - r: `p` at d = 1;
    - g: the dominant species index / 255;
    - b: the second species index / 255;
    - a: the blend weight between the two, with bit 7 set for stunted
        (tree line and exposed ridges).
- The hard exclusions from plan 4 (water distance, sand, rock, glacier,
    heat and slope) are applied at bake time. Water distance and slope
    are also re-checked per candidate on the GPU, from the
    distance-to-water channel and the height texture, so exclusions stay
    sharp at sub-cell scale.

### 3. The near field on the GPU, the far set on the CPU

- **The far set is static** and built on the CPU as now. It holds the
    candidates with `r < p x k_far`, where `k_far` is the far keep
    fraction (0.12 by default). These are large, well-spaced trees that
    read from far away.
    - Build them from the lattice, not the old grid.
    - Walk only the lattice cells where the cover texture is above 0, so
        the build stays fast. It must be at most 120 ms at 512 x 512 in
        WASM, at d = 4.
- **The near field is streamed** by a new compute pass,
    `shaders/tree_generate.wgsl`.
    - It fills 64 m x 64 m tiles within `vegetationDetailMetres` of the
        camera (defaults from the preset in section 6), with candidates
        where `k_far x p <= r < p`: exactly the complement of the far
        set, so there are no duplicates.
    - Tiles live in a ring buffer of tile slots.
    - At most 8 new tiles are generated per frame, nearest first, so
        moving never stalls.
    - Each tile's instance count is written to its slot. The tree cull
        pass reads the far set and all live tiles, as one list or two
        dispatches.
- **Thinning with compensation:**
    - At distance D from the camera, keep a candidate when
        `r < p x keep(D)`, where
        `keep(D) = max(k_far, min(1, (R / D)^2))` and R is the
        near-field radius.
    - Scale kept trees by `sqrt(1 / keep(D))`, capped at 1.8. Their crown
        area then keeps the canopy cover constant.
    - Fade trees near the keep threshold over an r-band of 0.05, by
        shrinking to 0. This avoids popping.
    - Apply all of this in the cull pass, so it is continuous per frame.
- **Grass:** the same lattice scheme, with its own `grass_generate.wgsl`,
    replaces the CPU tuft build.
    - The pitch is 0.35 m, and the tiles are 16 m, within the grass view
        distance.
    - `grassDensity` maps 0 → 0, 0.5 → today's default count (calibrate),
        1 → today's maximum, and 4 → full cover (every lattice point, with
        tufts overlapping).
    - Thin with distance in the same way, widening tufts to compensate.

### 4. The canopy layer

- Beyond `canopyDistanceMetres` (from the preset, 2.5 km at balanced),
    individual trees give way to a canopy shell.
    - Draw the terrain mesh a second time, far bands only (sample step
        at least 4), with a new vertex entry point, `canopy_vertex_main`,
        in `clipmap_render.wgsl`.
    - It offsets each vertex upwards by canopy height:
        `mix(8, 28 m, species height factor) x cover`.
- Shading:
    - Canopy albedo comes from the species colours, broken up by a
        clump pattern: Worley noise at 6 to 12 m scale.
    - Self-shadowing uses the clump height, plus the terrain shadow map.
    - The fragment is discarded where cover is below 0.25, with a dither
        using `textureSampleLevel`-only sampling, so it stays
        uniform-safe.
    - Silhouettes on ridges become a leafy edge.
- **Crossfade:** from 0.8x to 1.0x `canopyDistanceMetres`, individual
    trees fade out (the cull pass drops them by hash) as the canopy fades
    in (hash-dithered discard). There is never a band with neither.
- The canopy casts into the far terrain's lighting (darker ground under
    canopy is hidden anyway), but it does not enter the tree shadow map.

### 5. The forest floor

- **Canopy shade** at a point is the cover texture value times the local
    near-tree density. It is baked into the cover texture's r channel
    relative to the maximum.
- **Terrain:** where canopy shade is above 0.5, blend towards
    `MAT_FOREST_FLOOR` (leaf litter), plus mossier tones in wet climates,
    over rock, grass and dry grass.
- **Grass:** meadow grass density is multiplied by `1 - canopy shade`.
    Two new grass styles replace it under canopy:
    - **Ferns:** fronds as 5 to 7 curved cards, 0.4 to 0.9 m, in
        temperate and wet ground;
    - **Undergrowth:** low leafy clumps, 0.3 to 0.6 m, in any forest.
    - Both are generated procedurally at start-up into the flora texture
        array (two new layers, with shapes drawn by the texture
        generator), and drawn with the grass pipeline.
    - Add `GrassOptions.forestFloor?: boolean` (default true).
- **Dappled light:**
    - The tree shadow map (`shaders/shadow.wgsl`) already draws tree
        casters as alpha-tested impostors. Make sure its alpha test uses
        the leaf texture's holes, so light spots come through. Check the
        shadow map resolution defaults, and add no cost.
    - For trees beyond the shadow distance, darken the ground under
        canopy by canopy shade x 0.5, using the canopy layer's clump
        pattern as a coarse dapple.
- **Damp shade:** under canopy, terrain roughness is x 0.85 and
    saturation +5 %. Mist density near the ground gets +20 % locally
    when mist is on.

### 6. Budgets, presets and dynamic resolution

- Add these fields to `RenderQualityOptions`, with defaults from the
    preset:

    | Field | preview | balanced | high | offline |
    | --- | --- | --- | --- | --- |
    | `vegetationDetailMetres` (near radius) | 120 | 250 | 400 | 600 |
    | `canopyDistanceMetres` | 1500 | 2500 | 4000 | 8000 |
    | `maxTreeInstances` (drawn per frame) | 40,000 | 120,000 | 250,000 | 1,000,000 |
    | `maxGrassInstances` | 150,000 | 400,000 | 800,000 | 2,000,000 |

- **Budget enforcement:**
    - The CPU estimates each frame's drawn counts from the cover texture
        integral over live tiles and the far set in the frustum. This is
        cheap and needs no readback.
    - If over budget, shrink the near radius in steps of 10 % (never
        below 60 m for trees, and 25 m for grass).
    - Regrow by one step after 2 s with headroom.
- **Pressure from dynamic resolution:**
    - Extend `pacing.rs` `ResolutionController` with a `detail_pressure()`
        output in [0, 1]. It is 0 normally, and rises by 0.25 per window
        while the scale sits at its minimum and frames are still late. It
        falls by 0.25 per calm window.
    - The vegetation near radius is multiplied by
        `1 - 0.5 x detail_pressure`.
    - Add unit tests next to the existing pacing tests.
- `RenderStats.floraInstances` and `grassInstances` report the estimated
    drawn counts, so the demo shows real numbers.

## Public API

Additive, plus a wider validation range:

```ts
export interface FloraOptions {
  /**
   * Tree density, 0 to 4. 1 is the old maximum; 4 is a closed canopy
   * where the land supports it. Defaults to 0.35.
   */
  density: number;
}

export interface GrassOptions {
  /** Grass density, 0 to 4. 1 is the old maximum; 4 covers the ground. Defaults to 0.5. */
  density: number;
  /** Ferns and undergrowth replace grass under dense canopy. Defaults to true. */
  forestFloor?: boolean;
}

export interface RenderQualityOptions {
  // ...existing...
  /** Radius in metres of full-density vegetation around the camera. From the preset. */
  vegetationDetailMetres?: number;
  /** Distance where individual trees give way to a canopy layer. From the preset. */
  canopyDistanceMetres?: number;
  /** Most tree instances drawn per frame. From the preset. */
  maxTreeInstances?: number;
  /** Most grass tufts drawn per frame. From the preset. */
  maxGrassInstances?: number;
}
```

- `FloraOptions.maxInstances` and `GrassOptions.maxInstances` keep their
    meaning as the cap on generated instances. Raise their validation
    maximums to match the new budgets.
- Validation: `density` in [0, 4]; the distances at least 50 m;
    instance budgets from 1,000 to 4,000,000.

## Steps

1. Capture the reference shots, plus:
    - the forest close-up at `density` 1;
    - an inner-jungle close-up;
    - a hillside forest at 1.5 km.
    Record per-pass GPU times from `gpuPassTimesMs` for each.
2. Add the lattice hash (Rust and WGSL) with the parity test, and the
    `target_density` table with calibration tests.
3. Add the cover-texture bake.
4. Move the far set onto the lattice.
5. Add `tree_generate.wgsl`, the tile ring and cull-pass integration,
    with thinning and compensation.
6. Add `grass_generate.wgsl`, and retire the CPU grass build. Keep
    `build_grass_instances` only if tests need it as a CPU reference,
    renamed accordingly. It must not be dead code.
7. Add the canopy layer and the crossfade.
8. Add the forest floor (materials, ferns and undergrowth textures and
    styles, dapple, damp shade).
9. Add the budgets, the presets, `detail_pressure` and stats.
10. Demo:
    - tree and grass density sliders from 0 to 4, step 0.05, with
        readouts ("1.00 (old maximum)");
    - a "Forest floor" checkbox;
    - render-section selects for vegetation detail and canopy distance;
    - the stats panel shows drawn trees and grass.
11. Docs:
    - `docs/vegetation.md`: density scale, lattice, thinning, canopy,
        forest floor, budgets, and a jungle example;
    - `docs/render-quality-and-diagnostics.md` (budgets and pressure);
    - `docs/options-reference.md`;
    - `docs/architecture.md` (generation passes, draw order, cover
        texture);
    - `CHANGELOG.md`.
12. Verify, then commit and push.

## Tests

- **Lattice hash:** Rust and WGSL agree bit-for-bit, checked with a
    compute-shader test vector run in the browser harness through a
    `window.engine` debug hook compiled only in debug builds. Or it is
    proved by a Rust port of the exact WGSL arithmetic; say which in the
    report.
- **No duplicates:** CPU emulation of far set plus near tiles over a
    512 m square gives each lattice point at most once.
- **Canopy cover:** over annuli at 100, 300, 800 and 1600 m, the sum of
    crown areas stays within 7 % of the full-density value.
- **Calibration:** densities 0.35 and 1 give instance counts within 10 %
    of the old build's, on the default map.
- **Budget:** a synthetic over-budget estimate shrinks the radius in
    10 % steps down to the floor, and regrows after 2 s.
- **Pressure:** `detail_pressure` rises only when the scale is at its
    minimum and frames are late.
- **Validation:** density 4.01 and -0.01 are rejected.

## Verification

- The forest close-up at density 4: a closed canopy with a forest floor
    of ferns and litter.
- The inner jungle at density 4.
- The hillside at 1.5 km, and a far valley at 5 km: a solid canopy, a
    smooth transition, and no band with neither trees nor canopy.
- A meadow with grass at 4: full cover.
- The default density shots match the before images: the same look, the
    same counts within 10 %.
- GPU times: in the default scene, total GPU time is at most +0.3 ms
    (scaled from the software ratio to terrain). At density 4 in the
    jungle, estimate against the budget in the report: trees at most
    5 ms, grass at most 2.5 ms, canopy at most 0.8 ms, and generation at
    most 0.4 ms per frame on a mid-range GPU.

## Budgets

- WASM growth: at most +15 KB gzipped.
- GPU: as above. Presets hold 60 FPS at 1080p on a mid-range GPU at
    density 4.

## Out of scope

Tree mesh detail and wind (plan 6), and painted density maps (plans 10
and 11, which will feed the cover-texture bake through a density mask
this plan's bake must accept as an optional multiplier input). Expose
that input internally as `Option<&[u8]>` on the bake function now,
tested with a synthetic mask. It must not be a stub.
