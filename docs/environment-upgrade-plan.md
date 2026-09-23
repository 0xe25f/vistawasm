# Environment Upgrade Plan: Trees, Grass, Clouds, and Mist

This is an implementation plan for four environmental rendering gaps
identified in the current renderer:

1. Tree quality (the current billboards look flat and repetitive).
2. Grass vegetation (no ground-cover layer exists at all).
3. Clouds (the sky dome has no cloud layer).
4. Mist/ground fog (only a single distance-haze blend exists, baked into
    the terrain shader only).

Every option introduced here follows the same rule: **it can be used,
tweaked, or disabled independently of the others**, and the existing,
cheaper look stays the default. Each feature also gets an opt-in
higher-fidelity tier so the same engine can render either a light,
"simplified" scene or a "hyper-realistic" one, chosen per feature rather
than as one all-or-nothing global switch.

This document assumes familiarity with [`docs/architecture.md`](architecture.md)
and [`docs/world-design-guide.md`](world-design-guide.md). It does not repeat
background covered there.

## Implementation status

> **Superseded in part.** The realism work described in
> [`docs/architecture.md`](architecture.md), [`docs/biomes.md`](biomes.md),
> [`docs/vegetation.md`](vegetation.md), [`docs/water.md`](water.md), and
> [`docs/sky-atmosphere-and-weather.md`](sky-atmosphere-and-weather.md)
> replaced several details below: the tree `mesh` tier now draws real
> procedurally modelled species with impostors; clouds use baked 3D noise
> and cast shadows; haze and mist are applied in a depth-aware composite
> pass; and `FrameUniforms` is declared once in `shaders/common.wgsl`, which
> is prepended to every render shader, so the append-only prefix rule in
> §1.2 no longer applies. The notes below are kept as a historical record.

Phases 1–5 of §14 (mist `flat`/`volumetric`, tree `cross-quad`, grass
`billboard-blades`/`dense-blades`, clouds `painted`/`volumetric`) are
implemented. A few details landed slightly differently from the sketch
below, recorded here rather than silently left inconsistent:

- **Tree `mesh` tier (phase 6) is deferred, as planned.** `TreeQuality::Mesh`
  exists in the public API and currently renders identically to
  `CrossQuad` (see the doc comment on the type). No LOD-fade acceptance
  criterion applies yet since no separate mesh geometry exists.
- **`RenderQualityOptions.preset`-driven defaults (§1.3) were not
  implemented.** Every new option struct defaults to its cheapest/`off`
  state regardless of the active quality preset; a host must opt in to
  `treeQuality`, `GrassOptions.enabled`, `CloudsOptions.style`, and
  `MistOptions.style` explicitly. This was cut to keep the first pass
  additive and simple; it remains a reasonable follow-up.
- **`wind_params` became `vegetation_params`** and gained a fourth,
  grass-specific field (`grassViewDistanceMetres`) instead of the
  originally-sketched `gustScale`/`directionRadians` — the simpler sway
  model implemented did not need either of those.
- **Volumetric clouds live entirely inside `atmosphere.wgsl`**, not a
  separate `clouds_pipeline`/`clouds.wgsl` as §4.2 sketched. Both the
  `painted` and `volumetric` styles are one function
  (`cloud_density`) selected by whether `cloud_params.w` (the raymarch
  step count) is zero, which turned out to be simpler and lower-risk than
  a second pipeline/pass while still meeting the acceptance criteria in
  §14.
- Cloud/mist density accumulation must use `1 - transmittance` (a
  bounded opacity composite), not a raw sum of per-step densities — an
  early implementation summed density directly, which saturated to a
  solid, texture-less overcast at moderate `coverage` values well before
  reaching `1.0`. Caught via in-browser verification, not by any
  automated test (WGSL logic like this has no unit-test coverage), which
  is exactly why §11's "browser verification" step is not optional for
  shader changes.

## 0. Current state (baseline, so regressions are easy to spot)

| Area | File(s) | Today |
| --- | --- | --- |
| Trees | [`render/flora.rs`](../crates/vista_wasm/src/render/flora.rs), [`shaders/flora_instances.wgsl`](../crates/vista_wasm/src/shaders/flora_instances.wgsl) | One camera-facing billboard quad per tree, trunk/canopy carved with `discard`, one ellipse silhouette, one tint scalar. No mesh, no LOD, no wind. |
| Grass | [`terrain/materials.rs`](../crates/vista_wasm/src/terrain/materials.rs) | `grass` is a terrain *material weight* used only to tint the ground colour. There is no grass geometry. |
| Clouds | [`shaders/atmosphere.wgsl`](../crates/vista_wasm/src/shaders/atmosphere.wgsl) | Analytic Rayleigh/Mie sky gradient, sun disc, horizon haze. No cloud layer of any kind. |
| Mist/fog | [`shaders/clipmap_render.wgsl`](../crates/vista_wasm/src/shaders/clipmap_render.wgsl#L70-L75), `AtmosphereOptions.hazeDistanceMetres` | One `distance / hazeDistance` lerp to sky colour, applied only in the terrain shader. Flora and water do not receive it. |

Everything below is additive to this baseline.

## 1. Cross-cutting rules (apply to all four features)

These rules exist specifically to satisfy "no regressions, no breaking
changes, no race conditions, no memory leaks, no security risks."

### 1.1 Additive-only public API

- Every new field on a public `vista_types` struct is `Option<T>` (or has a
  `#[serde(default)]` attribute with a concrete default), so JSON from
  existing callers that omits the field keeps working unchanged.
- No existing field is renamed, retyped, or removed.
- New TypeScript fields in [`js/src/types.ts`](../js/src/types.ts) are
  optional (`field?: T`) for the same reason, and existing fields are left
  untouched.
- New engine methods (`setGrass`, `setClouds`, `setMist`, and the
  `treeQuality` addition to `setFlora`) are new, additive entries on
  `VistaEngine`/`VistaWasmRawEngine` — no existing method signature changes.
- Any new `DebugView` variants are appended after `NoData` (Rust enums
  require exhaustive `match`, so the compiler forces every render-side
  switch to be updated deliberately; this is a compile-time safety net,
  not a runtime break, and the TS side already models `DebugView` as a
  plain string union, so it is additive there too).

### 1.2 `FrameUniforms` growth stays append-only

[`render/gpu.rs`](../crates/vista_wasm/src/render/gpu.rs) documents this
rule already: "New fields always appended at the END so existing shader
struct prefixes stay compatible." Two currently-unused components are
worth knowing about before adding new fields:

- `fog.z` and `fog.w` are declared but never read by any shader today.
- `atmosphere_params.z`/`.w` and `water_params.w` are also unused.

Reusing these is *possible* without changing the struct's size, but this
plan does **not** rely on quietly repurposing unlabelled float slots —
that is exactly the kind of hidden coupling `AGENTS.md` warns against
("avoid hidden global state", "prefer clear code over clever code"). New
named `vec4<f32>` fields are appended at the end instead, each with a
one-line comment stating what each component means. The only reuse this
plan does take advantage of is the already-shared **frame clock**:
`water_params.z` (`frame_counter / 60.0`) is already computed once per
frame and available to every shader through the one shared bind group, so
wind animation for grass/flora reads it directly instead of adding a
duplicate time field.

New fields proposed by this plan, all appended after `atmosphere_params`:

```rust
struct FrameUniforms {
  view_proj: [f32; 16],
  camera_position: [f32; 4],
  sun_direction: [f32; 4],
  sun_colour_intensity: [f32; 4],
  fog: [f32; 4],
  water_params: [f32; 4],
  camera_forward: [f32; 4],
  camera_right: [f32; 4],
  camera_up: [f32; 4],
  camera_params: [f32; 4],
  sky_tint: [f32; 4],
  atmosphere_params: [f32; 4],
  // --- new fields below, appended in this order ---
  mist_params: [f32; 4],   // density, height_falloff_metres, base_height_metres, animation_phase
  mist_colour: [f32; 4],   // r, g, b, reserved
  cloud_params: [f32; 4],  // coverage, speed, height_metres, seed_phase
  cloud_colour: [f32; 4],  // r, g, b, reserved
  wind_params: [f32; 4],   // strength, gust_scale, direction_radians, reserved
}
```

Each new field is a full `vec4<f32>` (16 bytes) to preserve the struct's
existing 16-byte-aligned layout — no packing tricks that could silently
misalign a field on some backends. The struct grows from 240 bytes to 320
bytes; this is a pure size increase, not a reordering, so every existing
shader's `struct FrameUniforms { ... }` prefix continues to parse and bind
identically. Every shader that binds this struct
(`clipmap_render.wgsl`, `flora_instances.wgsl`, `water.wgsl`,
`atmosphere.wgsl`, and the new `grass_instances.wgsl`/`clouds.wgsl`) must
declare the *same* full struct (WGSL has no partial-struct binding), even
if a given shader only reads a subset of the fields — this already true
today (e.g. `flora_instances.wgsl` declares `fog` but never reads it) and
is the established pattern to keep one bind group shared by every
pipeline.

### 1.3 Per-feature quality tiers, not one global switch

Each new option struct gets its own two-tier (or three-tier, for trees)
"style"/"quality" enum:

- `FloraOptions.treeQuality: TreeQuality` — `billboard` (default,
  today's look) | `crossQuad` | `mesh` (hyper-realistic).
- `GrassOptions.style: GrassStyle` — `off` | `billboardBlades` (default
  once enabled) | `denseBlades` (hyper-realistic).
- `CloudsOptions.style: CloudStyle` — `off` (default) | `painted` (2D
  noise layer in the sky dome) | `volumetric` (hyper-realistic raymarch).
- `MistOptions.style: MistStyle` — `off` (default) | `flat` (height-based
  ground fog, cheap) | `volumetric` (hyper-realistic, animated density
  noise).

To avoid forcing every host application to hand-tune four separate enums
just to get "the nice version", `RenderQualityOptions.preset`
(`Preview`/`Balanced`/`High`/`Offline`, already public) additionally
supplies **default values** for any of the four new option structs the
host does not explicitly set: `Preview`/`Balanced` default to the cheap
tier for all four, `High`/`Offline` default to the hyper-realistic tier.
An explicit value on `FloraOptions`/`GrassOptions`/`CloudsOptions`/
`MistOptions` always wins over the preset-derived default — presets only
fill gaps, they never override an explicit choice. This gives one dial
for "just make it look good" while every knob stays individually
reachable, matching both parts of the request.

### 1.4 GPU resource lifecycle (no leaks)

- New GPU buffers/textures (grass instance buffer, cloud noise texture,
  any new pipeline objects) are stored as fields on `GpuContext`, exactly
  like `flora: Option<FloraGpu>` / `water: Option<WaterGpu>` today.
  Reassigning these fields drops the previous `wgpu::Buffer`/`Texture`
  automatically (Rust's ordinary move semantics release the old GPU
  resource before the new one is stored) — no manual `destroy()` calls
  are needed, but this plan explicitly calls it out per new field so
  review can check it, since a forgotten `Option::take()`/overwrite is
  the classic way to leak a GPU buffer.
- Regeneration only happens on option/terrain change (mirroring
  `refresh_flora`/`refresh_water`), never per rendered frame. A new
  `refresh_grass()` follows the exact same call sites as
  `refresh_flora()` (`install_terrain`, `set_grass`, `set_render_quality`).
- Any procedurally-baked texture (cloud noise, see §4) is generated once
  at `EngineCore` construction and re-baked only when its seed/parameters
  change — never per frame — and is stored as a single owned
  `wgpu::Texture` field, dropped the same way as other GPU fields.
- `EngineCore::dispose()` does not need new code for these fields:
  dropping `EngineCore` (via `VistaEngine::dispose()`'s
  `self.inner.take()`) already drops every field on it, including `gpu:
  GpuContext`, which in turn drops every `wgpu::Buffer`/`Texture`/
  `Pipeline` field on it. This plan preserves that existing invariant
  rather than adding a parallel manual-cleanup path that could drift out
  of sync with it.

### 1.5 No new race conditions

- All four new setters (`setFlora` with `treeQuality`, `setGrass`,
  `setClouds`, `setMist`) are **synchronous**, exactly like today's
  `setFlora`/`setWater`. They must be added to the same early-return guard
  already used in [`js/src/index.ts`](../js/src/index.ts)
  (`if (this.pendingCall) { return; }`) so they no-op instead of
  reentering the WASM object while an async call
  (`generateFractal`/`loadDemFromArrayBuffer`/`loadRawHeightmap`) is in
  flight. This is the exact reentrancy hazard already fixed once for
  `setFlora`/`setWater`/`setRenderQuality` — see the "CRITICAL BUG"
  history in this repo's own notes; the same audit applies to every new
  synchronous method touching `this.raw`.
- None of the four features need to become `async fn` on the Rust side
  (grass/cloud generation is cheap CPU/GPU work comparable to today's
  flora scatter, not a multi-frame GPU readback like erosion), so this
  plan deliberately avoids introducing any new multi-frame await — the
  single highest-risk source of reentrancy bugs in this codebase. If a
  future revision of the volumetric cloud tier ever needs an async GPU
  bake, it must go through `callAsync()` (chained on `pendingCall`) rather
  than a bare `async` method call, exactly like `generateFractal` does.

### 1.6 Validation and security (untrusted JS boundary)

Every new numeric option is validated in
[`config.rs`](../crates/vista_wasm/src/config.rs) the same way existing
ones are (`validate_finite`, `validate_positive`, `validate_non_negative`),
because JS input is untrusted per this repository's own `AGENTS.md`
security rules. Concretely:

- `GrassOptions.density`/`maxInstances` — same non-negative + device-limit
  clamp `flora.maxInstances` already gets in
  [`render/flora.rs`](../crates/vista_wasm/src/render/flora.rs)
  (`clamp_flora_instances`), reused verbatim for grass so a caller cannot
  request an unbounded instance count that exhausts GPU memory.
- `CloudsOptions` volumetric raymarch step count — **hard-capped in Rust**
  regardless of what JS requests (proposed range `8..=64` steps). This is
  the one new option in this plan that feeds a shader loop bound; an
  unclamped value is a real DoS vector (a caller — malicious or just
  careless — could request a step count that hangs the GPU driver or the
  tab). `validate_range("clouds.raymarchSteps", steps, 8, 64)` is added to
  `config.rs` for this reason.
- `MistOptions.densityMetres`/`heightFalloffMetres` — `validate_non_negative`,
  matching `water.waveScale`/`flora.density` today.
- No new option accepts a URL, file path, or arbitrary string that is
  interpreted as code, a shader, or a file location — every new control is
  a bounded numeric or enum value, consistent with "avoid APIs that
  require users to understand WASM internals" and this project's existing
  no-eval, no-arbitrary-fetch stance.

### 1.7 Determinism and testing

- Grass and tree-variation placement must be deterministic for a given
  seed, mirroring `build_flora_instances`'s existing use of
  `hash_noise`/`options.seed_offset` — required both for the "no
  regressions" bar (existing deterministic-terrain tests must keep
  passing unchanged) and so hosts can reproduce a scene from a saved seed.
- Cloud noise (painted or volumetric) is seeded from a new
  `CloudsOptions.seedOffset: u64` field, following the same pattern as
  `FloraOptions.seedOffset`, so the same seed always produces the same
  cloud pattern (only its *animated drift* varies with time, via the
  shared frame clock).
- New Rust tests live in `crates/vista_wasm/tests/`, following the shape
  of the existing suite (`camera_projection.rs`, `dem_decode.rs`,
  `deterministic_terrain.rs`): a new `vegetation_placement.rs` covering
  grass scatter counts/repeatability and tree-variation seeding, and
  option-validation tests added to wherever `config.rs`'s existing
  validation is exercised today.
- New JS/TS tests live in `js/tests/`, following `api.test.ts`'s shape —
  primarily option normalisation/shape tests, not WebGPU rendering (which
  vitest cannot execute; WebGPU behaviour stays verified in-browser, per
  §10 below).

### 1.8 Documentation stays in sync

Each feature section below ends with a short list of the docs it touches.
[`README.md`](../README.md)'s feature bullet list, [`docs/architecture.md`](architecture.md),
and [`docs/world-design-guide.md`](world-design-guide.md) all currently
describe the exact baseline in §0, so all three need updates once any
tier of any feature ships — not just the "hyper-realistic" work.

## 2. Feature: Higher-quality trees

### 2.1 Public API

New field on the existing `FloraOptions` (additive, defaults preserve
today's exact look):

```rust
/// Tree rendering fidelity.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TreeQuality {
  /// Today's single camera-facing billboard (default, cheapest).
  Billboard,
  /// Two billboards crossed at 90 degrees for parallax and volume.
  CrossQuad,
  /// A real low-poly instanced mesh, LOD-faded to CrossQuad at distance.
  Mesh,
}

pub struct FloraOptions {
  pub enabled: bool,
  pub density: f32,
  pub tree_line_metres: f32,
  pub seed_offset: u64,
  pub max_instances: u32,
  // New, all defaulted so existing callers are unaffected:
  #[serde(default)]
  pub tree_quality: TreeQuality,       // defaults to Billboard
  #[serde(default = "default_species_variation")]
  pub species_variation: f32,          // 0..1 silhouette/colour variety
  #[serde(default)]
  pub wind_strength: f32,              // 0..1 canopy sway amount
}
```

`TreeQuality::default()` returns `Billboard`, so any existing serialised
`FloraOptions` JSON without these keys deserialises to today's exact
rendering. `species_variation`/`wind_strength` default to values that
reproduce today's uniform, static look (`0.0`) unless the plan's own
demo/example defaults opt them in.

### 2.2 Rendering approach per tier

- **Billboard (unchanged baseline)** — kept byte-for-byte as the default
  path; this is the regression guard for this feature. No existing user
  should see any visual change unless they set `treeQuality`.
- **CrossQuad** — the existing `FLORA_BASE_QUAD` becomes two quads at 90°
  to each other sharing one instance (a second draw per instance or a
  6-vertex extension per instance, decided during implementation based on
  measured instancing cost). This alone fixes most of the "goofy" look:
  from any camera angle there is now a visible second silhouette plane
  instead of a single flat cutout. The fragment shader also stops using
  one fixed ellipse: `species_variation` perturbs the canopy mask with a
  small per-instance noise offset (still `discard`-based, still no image
  textures, consistent with this project's no-texture vegetation
  approach) so neighbouring trees do not look identical.
- **Mesh** — a small shared procedural mesh (tapered cylinder trunk + 2–3
  overlapping low-poly canopy lobes, generated once at module init like
  `FLORA_BASE_QUAD` is today, not per tree) is instanced the same way
  billboards are today. This tier gives real depth and correct
  self-occlusion from any angle, at a real triangle-count cost, so it
  LOD-fades to `CrossQuad` beyond a configurable distance
  (`RenderQualityOptions`-driven, similar in spirit to
  `maxClipmapLevels`) to bound worst-case triangle count on large,
  densely-forested terrain.
- **Wind** — all tiers read the shared `wind_params`/frame-clock fields
  from `FrameUniforms` (§1.2) in the vertex shader to bend the canopy top
  proportional to `wind_strength`; trunks do not move. This is purely
  additive to the vertex shader and costs one `sin()` per vertex.

### 2.3 Engine wiring

- `render/flora.rs` gains a `tree_quality`/`species_variation`/
  `wind_strength` parameter path through `build_flora_instances` (already
  the single call site rebuilding instances on terrain/option change —
  no new call sites needed).
- `render/gpu.rs` gains one additional pipeline
  (`flora_mesh_pipeline`) used only when any active instance requests
  `Mesh` quality; `CrossQuad` reuses the existing `flora_pipeline` with an
  extended vertex layout. `Billboard` is untouched.
- `EngineCore::refresh_flora()` is the only call site that needs updating
  — it already reruns on terrain change, `setFlora`, and
  `setRenderQuality`.

### 2.4 Tests and docs

- Rust: extend the existing flora placement tests with a
  `tree_quality`/`species_variation` seeding-repeatability case.
- Docs: [`docs/world-design-guide.md`](world-design-guide.md) §6
  ("Vegetation") gains a subsection on `treeQuality`; demo/examples gain a
  tree-quality selector next to the existing flora density slider.

## 3. Feature: Grass vegetation

### 3.1 Public API

A new, independent options struct (not a field bolted onto
`FloraOptions`, since grass and trees are placed, styled, and toggled
separately in the request):

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GrassStyle {
  /// Crossed billboard blades in clumps (default once enabled, cheap).
  BillboardBlades,
  /// Denser per-blade instancing with wider view-distance (hyper-realistic).
  DenseBlades,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrassOptions {
  pub enabled: bool,               // default false — fully opt-in
  pub style: GrassStyle,           // default BillboardBlades
  pub density: f32,                // 0..1, like flora.density
  pub view_distance_metres: f32,   // grass fade-out radius from camera
  pub seed_offset: u64,
  pub max_instances: u32,
}
```

`enabled: false` by default keeps every existing scene byte-for-byte
unchanged until a host opts in — this is a brand-new visual element, so
unlike the other three features it defaults fully off rather than merely
defaulting to the cheap tier.

### 3.2 Placement: reusing existing baked data

Grass placement reuses the terrain's already-baked
`MaterialWeights.grass` weight (`terrain/materials.rs`) — computed once
per terrain in `bake_terrain_shading` and already cached on `EngineCore`
as `terrain_materials` — instead of recomputing slope/height thresholds
independently the way `render/flora.rs` does today for trees. This has
two benefits: no new per-terrain CPU cost (the data already exists), and
grass naturally avoids rock/snow/mud/underwater terrain because that is
exactly what the `grass` material weight already encodes. `render/grass.rs`
takes `&HeightMap` plus `&[MaterialWeights]` and scatters candidates the
same deterministic-grid way `build_flora_instances` does, weighting
acceptance probability by the cached `grass` value at each candidate
instead of by a hand-rolled slope check.

### 3.3 Rendering

- **BillboardBlades** — 2–3 crossed thin quads per clump (same crossed-quad
  technique as tree `CrossQuad`, sharing the vertex-layout approach so the
  two features do not duplicate a second, subtly-different billboard
  implementation), coloured from the existing grass/rock/wet-mud material
  blend rather than a flat green, with wind sway driven by the same
  `wind_params` field trees use (blades sway more than canopies — a
  per-shader multiplier on the shared field, not a second wind system).
- **DenseBlades** — the same geometry at a higher instance count and a
  shorter default `viewDistanceMetres` cutoff (grass is small on screen at
  range, so hyper-realistic density is spent where it is visible, not
  wasted past where it would alias to noise).
- Both tiers **fade out with distance** (alpha fade over the last ~15% of
  `viewDistanceMetres`, computed in the vertex/fragment shader from
  `camera_position`, already available in `FrameUniforms`) rather than a
  hard pop — grass is not expected to render to the terrain's horizon the
  way trees currently do, both for realism and for the instance-count
  budget.
- A new `grass_instances.wgsl` shader and `grass_pipeline` in
  `render/gpu.rs` mirror `flora_instances.wgsl`/`flora_pipeline` exactly
  in structure (own instance buffer, same shared `FrameUniforms` bind
  group), which keeps the implementation reviewable as "the same pattern
  as flora, applied to a second instance stream" rather than a new
  rendering architecture.

### 3.4 Engine wiring, tests, docs

- `EngineCore` gains a `grass: GrassOptions` field and `refresh_grass()`,
  called from the same three sites as `refresh_flora()`
  (`install_terrain`, a new `set_grass`, `set_render_quality`).
- `api.rs` gains `setGrass`, following `setFlora`'s exact shape.
- Rust: grass placement determinism/count tests alongside the flora ones.
- Docs: new "Ground cover" subsection in
  [`docs/world-design-guide.md`](world-design-guide.md) §6; demo/examples
  gain a grass enable checkbox + density + style controls, mirroring the
  existing flora controls in `demo/src/main.ts`.

## 4. Feature: Clouds

### 4.1 Public API

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CloudStyle {
  /// No cloud layer (default).
  Off,
  /// A 2D noise layer blended into the existing sky dome shader.
  Painted,
  /// A raymarched volumetric layer with self-shading (hyper-realistic).
  Volumetric,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudsOptions {
  pub style: CloudStyle,             // default Off
  pub coverage: f32,                 // 0..1
  pub speed: f32,                    // drift speed, metres/second-equivalent
  pub height_metres: f32,            // cloud layer altitude
  pub colour: Rgb,                   // base tint, blended with sun/sky colour
  pub seed_offset: u64,
  pub raymarch_steps: Option<u32>,   // Volumetric only; validated to 8..=64
}
```

`style: Off` by default, matching the "no clouds today" baseline exactly.

### 4.2 Rendering approach per tier — designed to avoid a new pipeline where possible

- **Painted (simplified, recommended default once enabled)** — implemented
  as an *extension of the existing `atmosphere.wgsl` fullscreen pass*, not
  a new pipeline. That pass already reconstructs a view ray per pixel from
  the camera basis and already runs once, first, before terrain (depth
  write off, `Less` compare) — exactly the right place to blend in a 2D
  value-noise cloud layer sampled along the same ray at `cloud_params.z`
  (height), using the same noise primitives already used by
  `terrain_noise.wgsl`'s value-noise approach rather than introducing a
  second noise implementation. This keeps the "simplified" cloud tier at
  effectively zero new GPU pipeline/state — the single highest-leverage,
  lowest-risk option in this whole plan.
- **Volumetric (hyper-realistic)** — a small number of raymarch steps
  (validated to the `8..=64` range from §1.6) through a layered 3D noise
  density field between two altitude planes, with a cheap single-scatter
  approximation (sun-facing brightening, similar in spirit to the existing
  Mie phase term) for real depth and light shafts. This *is* a new
  pipeline/pass (`clouds.wgsl` + `clouds_pipeline`), inserted between the
  atmosphere pass and the terrain pass so mountains still correctly
  occlude distant clouds at grazing angles, but sky-only pixels show the
  volumetric layer. Gated by the quality-preset default from §1.3
  (`High`/`Offline` only, by default) so a `Preview`/`Balanced` host does
  not silently inherit a heavy pass just because clouds were turned on.
- Both tiers read `sun_direction`/`sun_colour_intensity` (already in
  `FrameUniforms`) so clouds brighten toward the sun and darken away from
  it, and both are driven by the same seeded, deterministic noise plus the
  shared frame clock for drift (§1.7) — a cloud pattern is reproducible
  for a given seed even though it animates.
- **Cloud shadows on terrain (optional, either tier)** — a single extra
  `cloud_shadow_strength` sample (the same 2D noise function evaluated at
  the terrain sample's XZ position projected onto the cloud layer) applied
  as a multiplier on `sun_colour_intensity` in `clipmap_render.wgsl`. This
  is intentionally scoped as an independent, separately-toggleable
  sub-option (`CloudsOptions` could grow a `castShadows: bool` later) so
  it can ship after the base cloud layer without blocking on it, and so
  terrain rendering is not forced to change unless a host actually enables
  clouds.

### 4.3 Engine wiring, tests, docs

- `EngineCore` gains a `clouds: CloudsOptions` field; unlike flora/water,
  clouds do not need per-terrain regeneration (no placement on the
  heightmap), so no `refresh_clouds()` call site is needed from
  `install_terrain` — only from `set_clouds`/`set_render_quality`, and
  only to re-bake the (small, procedural) noise texture if the seed
  changed, not on every option tweak.
- `api.rs` gains `setClouds`, following `setSun`'s shape (a plain
  synchronous replace, no terrain dependency).
- Rust: a noise-seed repeatability test (same seed/time produces the same
  sampled density at a fixed point) and the `raymarchSteps` validation
  range test from §1.6.
- Docs: new "Sky and clouds" subsection in
  [`docs/world-design-guide.md`](world-design-guide.md) (currently §5
  covers "Sea level, atmosphere, and mood" — clouds fit naturally
  alongside it); demo/examples gain a clouds enable + style + coverage +
  speed control group.

## 5. Feature: Mist and ground fog

### 5.1 Naming, kept distinct from the existing haze

`AtmosphereOptions.hazeDistanceMetres` already exists and is documented
(world-design-guide.md §5) as "how far can you see clearly" — a
uniform, distance-only blend to sky colour. This plan does not repurpose
or rename it (that would be a breaking change to a documented control).
Instead it adds a new, separate `MistOptions` for **height-based ground
fog** — a physically distinct effect (mist pools in valleys and near
water; haze is purely a function of view distance) that today does not
exist in any form.

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MistStyle {
  /// No ground mist (default).
  Off,
  /// A static height-falloff blend (cheap).
  Flat,
  /// Animated density noise drifting across the terrain (hyper-realistic).
  Volumetric,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MistOptions {
  pub style: MistStyle,               // default Off
  pub density: f32,                   // 0..1
  pub base_height_metres: f32,        // altitude mist is thickest at
  pub height_falloff_metres: f32,     // how quickly it thins with altitude
  pub colour: Rgb,
  pub rise_above_water: bool,         // extra mist near WaterOptions.seaLevelMetres
  pub seed_offset: u64,
}
```

`style: Off` by default preserves today's baseline exactly.

### 5.2 Applying it consistently across pipelines (fixing an existing gap)

Today's haze blend is only ever computed in `clipmap_render.wgsl` — flora
and water do not receive it, which is already a minor inconsistency
(§0). This plan fixes that as part of adding mist, rather than adding a
second inconsistency on top of the first: the new `mist_params`/
`mist_colour` fields (§1.2) are read by `clipmap_render.wgsl`,
`flora_instances.wgsl`, `water.wgsl`, and `grass_instances.wgsl`
identically, using each pixel's/vertex's world-space height and distance
from camera. A tree or a patch of grass standing in a misty valley is
therefore tinted the same as the ground beneath it, instead of popping
out unaffected the way it would if mist were terrain-only.

- **Flat** — `clamp((mist.baseHeight + falloff - world_y) / falloff, 0, 1)
  * density`, then a distance fade so mist does not extend to the far
  clip plane at full strength. One `clamp`/`mix`, comparable cost to
  today's haze term.
- **Volumetric** — the flat formula modulated by the same seeded 2D noise
  approach used for painted clouds (deliberately reusing that noise
  utility rather than writing a third implementation), animated by the
  shared frame clock so mist drifts rather than sitting static.
- **`riseAboveWater`** — when true and `WaterOptions.enabled`, an extra
  mist contribution is added near `seaLevelMetres` regardless of
  `baseHeightMetres`, giving the "mist rising off the lake" look
  world-design-guide.md's recipes already gesture at conceptually via
  `hazeDistanceMetres` but cannot actually produce today (haze has no
  height component at all).

### 5.3 Engine wiring, tests, docs

- `EngineCore` gains a `mist: MistOptions` field, `api.rs` gains
  `setMist` following `setAtmosphere`'s shape (no terrain dependency, no
  regeneration needed — purely a per-frame uniform update, cheapest of
  the four features to wire up).
- Rust: option-validation tests (`density`/`heightFalloffMetres`
  non-negative, mirroring `atmosphere.hazeDistanceMetres`'s existing
  `validate_positive` check).
- Docs: [`docs/world-design-guide.md`](world-design-guide.md) §5 gains an
  explicit "haze vs. mist" clarifying paragraph so world-builders do not
  conflate the two controls; demo/examples gain a mist enable + style +
  density + rise-above-water control group next to the existing
  atmosphere sliders.

## 6. Public API summary

| Struct | New/changed | Default preserves baseline? |
| --- | --- | --- |
| `FloraOptions` | `+treeQuality`, `+speciesVariation`, `+windStrength` | Yes — `Billboard`, `0.0`, `0.0` |
| `GrassOptions` | New struct | Yes — `enabled: false` |
| `CloudsOptions` | New struct | Yes — `style: Off` |
| `MistOptions` | New struct | Yes — `style: Off` |
| `VistaEngineOptions` | `+grass`, `+clouds`, `+mist` (all `Option<T>`) | Yes |
| `VistaEngine`/`VistaWasmRawEngine` | `+setGrass`, `+setClouds`, `+setMist` | Yes — additive methods only |
| `RenderStats` | `+grassInstances: u32` (mirrors `floraInstances`) | Yes — new field only |

## 7. Engine plumbing — file-by-file task list

- `crates/vista_types/src/lib.rs` — add `TreeQuality`, `GrassStyle`,
  `GrassOptions`, `CloudStyle`, `CloudsOptions`, `MistStyle`,
  `MistOptions`; extend `FloraOptions`, `VistaEngineOptions`,
  `RenderStats`.
- `crates/vista_wasm/src/config.rs` — validate every new numeric field;
  add the `raymarchSteps` range clamp described in §1.6.
- `crates/vista_wasm/src/engine.rs` — add `grass`/`clouds`/`mist` fields
  to `EngineCore`; add `set_grass`/`set_clouds`/`set_mist`; add
  `refresh_grass()` alongside `refresh_flora()`/`refresh_water()`; extend
  `render_once()`'s per-frame uniform update to also push mist/cloud/wind
  parameters (same call site that already pushes atmosphere params).
- `crates/vista_wasm/src/api.rs` — add `setGrass`/`setClouds`/`setMist`
  wasm-bindgen methods, following the existing `setFlora`/`setWater`
  pattern exactly (options decoded via `from_js`, delegated to
  `core_mut()`).
- `crates/vista_wasm/src/render/flora.rs` — extend
  `build_flora_instances` for tree quality/variation/wind parameters.
- `crates/vista_wasm/src/render/grass.rs` — new module, mirrors
  `flora.rs`'s structure, takes cached `MaterialWeights` for placement.
- `crates/vista_wasm/src/render/clouds.rs` — new module: noise-texture
  baking (once, on seed change) for both cloud tiers.
- `crates/vista_wasm/src/render/gpu.rs` — extend `FrameUniforms` (§1.2);
  add `grass_pipeline`, `flora_mesh_pipeline`, `clouds_pipeline` (the
  volumetric tier only); add `GrassGpu`/mesh-tree GPU structs mirroring
  `FloraGpu`; ensure new pipelines are created lazily (only when a
  feature's active tier needs them) to avoid paying pipeline-creation
  cost for hosts that never enable these features.

## 8. Shader changes — file-by-file

- `shaders/atmosphere.wgsl` — add painted-cloud blending using the
  existing view-ray reconstruction; read `cloud_params`/`cloud_colour`.
- `shaders/clipmap_render.wgsl` — add the mist height-falloff term
  alongside the existing haze term; both contribute to the same final
  `mix`, mist applied after haze so mist can only add ground-level fog,
  never remove the existing distance haze.
- `shaders/flora_instances.wgsl` — add mist tinting, wind sway, tree
  quality (CrossQuad/Mesh) vertex handling, species-variation canopy mask
  perturbation.
- `shaders/water.wgsl` — add mist tinting (water surfaces sit at/near
  `baseHeightMetres`, so `riseAboveWater` mist should visibly touch it).
- `shaders/grass_instances.wgsl` — new, mirrors `flora_instances.wgsl`'s
  structure (crossed quads, mist tinting, wind, distance fade).
- `shaders/clouds.wgsl` — new, volumetric tier only; raymarch loop bound
  by the validated, capped step count from §1.6.

## 9. JavaScript/TypeScript changes

- `js/src/types.ts` — add `TreeQuality`, `GrassStyle`, `GrassOptions`,
  `CloudStyle`, `CloudsOptions`, `MistStyle`, `MistOptions` types; extend
  `FloraOptions`, `VistaEngineOptions`, `RenderStats`,
  `VistaWasmRawEngine` (new methods).
- `js/src/index.ts` — add `setGrass`/`setClouds`/`setMist` on
  `VistaEngineWrapper`, each guarded by the existing
  `if (this.pendingCall) { return; }` check (§1.5); extend
  `normaliseFloraOptions`-style helpers if any client-side normalisation
  is needed for the new enums (matching the existing pattern for flora).
- No changes needed to `js/src/camera-controls.ts` or
  `js/src/terrain-export.ts` — these features do not affect camera
  control or export formats.

## 10. Demo and example UI changes

`demo/` and all four `examples/*` currently share one feature set via
near-identical `main.ts`/framework-equivalent files (per this repo's own
notes on why they are kept as near-duplicates). Each gets, per feature:

- Trees: a `treeQuality` select next to the existing flora density
  slider, plus `speciesVariation`/`windStrength` sliders.
- Grass: an enable checkbox, style select, density slider, and
  view-distance slider, styled consistently with the existing flora
  control group.
- Clouds: an enable/style control, coverage/speed/height sliders.
- Mist: an enable/style control, density/height/rise-above-water controls.

All new inputs follow the existing convention (`input()` lookup by id in
vanilla/demo, uncontrolled-ref-based field reads in React/Vue/Svelte) —
no new UI architecture, just more fields in the same control panel.

## 11. Testing plan

- **Rust unit/integration** (`cargo test --workspace`, native, already the
  verified-working command in this repo): placement determinism for
  grass, tree-variation seeding, option validation ranges (including the
  new `raymarchSteps` clamp), `FrameUniforms` size/alignment assertion
  (a `const _: () = assert!(...)` style check is cheap insurance against
  an accidental misalignment when the struct grows).
- **JS/TS** (`npm test`, vitest): option shape/normalisation tests
  following `api.test.ts`; no WebGPU execution in vitest (unchanged from
  today).
- **Browser verification** (manual, via this repo's established
  Playwright-driven workflow): each feature toggled on/off and through
  both quality tiers, camera flown through misty valleys and under cloud
  cover, confirming no reentrancy panics when toggling controls while
  `generateFractal()`/DEM loads are in flight (the exact scenario the
  existing reentrancy fix was verified against) and no visible seams
  between LOD-faded tree tiers.
- **Type/build checks**: `npm run build:ts`, `wasm-pack build ... --target
  web`, `npm run lint:indent` — all three are already-verified-working
  commands per this repo's build notes and must stay green.
- **Size check**: compare `dist/pkg/*.wasm` size before/after, since a
  raymarched volumetric cloud shader and additional mesh geometry are the
  parts of this plan most likely to move the needle — flagged here so a
  regression in bundle size is caught deliberately rather than noticed
  later.

## 12. Performance budget

- Painted clouds and flat mist: effectively free (extend an existing
  fullscreen pass / an existing per-vertex term).
- CrossQuad trees and billboard grass: roughly double the per-instance
  vertex cost of today's flora billboards; bounded by the same
  `maxInstances`/device-limit clamp already in place.
- Volumetric clouds: the single largest new cost in this plan, which is
  exactly why it defaults off, is capped to 8–64 raymarch steps, and is
  only defaulted on for the `High`/`Offline` quality presets (§1.3) —
  never for `Preview`/`Balanced`, which is what the demo's control panel
  uses while scrubbing sliders.
- Mesh-tier trees: bounded by LOD-fade distance so worst case is capped
  by "trees near the camera only", not "every tree on the terrain".

## 13. Risk register

| Risk | Mitigation |
| --- | --- |
| Regression in existing scenes | Every new option defaults to today's exact behaviour (§0, §1.1); billboard/off/flat tiers are byte-for-byte the current code paths, not reimplementations of them. |
| Breaking change to public API | All additions are new optional fields/structs/methods; nothing existing is renamed, retyped, removed, or made required (§1.1). |
| Race condition / reentrancy panic | New setters are synchronous and follow the existing `pendingCall` guard; no new multi-frame async method is introduced (§1.5). |
| GPU memory leak | New GPU resources are ordinary struct fields following the existing `Option<FloraGpu>`/`Option<WaterGpu>` drop-on-reassign pattern; no parallel manual-cleanup path (§1.4). |
| Security (untrusted JS input) | Every new numeric option is validated in `config.rs`; the one option that bounds a shader loop (`raymarchSteps`) is hard-clamped server-side regardless of client input (§1.6). |
| `FrameUniforms` layout corruption | Fields are appended only, in full `vec4` increments, never reordered or packed into unused slots of existing fields (§1.2). |
| Bundle size / performance creep | Volumetric tiers default off and are quality-preset-gated; size is checked explicitly in CI/testing (§11, §12). |

## 14. Phased rollout

Each phase is independently shippable and independently testable against
the acceptance criteria below — later phases do not block on earlier ones
except where noted.

1. **Mist/fog (flat tier)** — lowest risk, smallest surface area, and it
    also closes the existing terrain-only haze inconsistency (§5.2) as a
    side effect. *Acceptance*: `MistOptions` round-trips through
    `setMist`; flat mist visibly pools in valleys in a browser check;
    existing haze-only scenes (`mist.style: "off"`) are pixel-identical to
    before.
2. **Tree quality (CrossQuad tier)** — directly answers the "goofy trees"
    complaint with the best risk/reward ratio (extends the existing flora
    pipeline, no new pipeline object). *Acceptance*: `treeQuality:
    "billboard"` scenes are pixel-identical to before; `"crossQuad"` shows
    a visible second silhouette plane from a rotated camera.
3. **Grass (BillboardBlades tier)** — reuses cached material data and the
    crossed-quad technique validated in phase 2. *Acceptance*: grass is
    fully absent when `enabled: false` (default); enabling it places
    blades only where the cached `grass` material weight is high, fading
    out at `viewDistanceMetres`.
4. **Clouds (Painted tier)** — extends the existing atmosphere pass only.
    *Acceptance*: `style: "off"` sky is pixel-identical to before;
    `"painted"` shows a drifting, seed-reproducible cloud layer.
5. **Mist (Volumetric) and Clouds (Volumetric)** — the two hyper-realistic
    tiers that need real perf validation, shipped together since both
    reuse the same seeded-noise-plus-frame-clock approach. *Acceptance*:
    both are off by default at `Preview`/`Balanced`, on by default at
    `High`/`Offline`; raymarch step count is verified clamped even when a
    caller requests an out-of-range value; frame time measured before/after
    on the existing demo terrain to confirm the cost matches §12's budget.
6. **Trees (Mesh tier)** — the most expensive and highest-implementation-
    risk item, deliberately last. *Acceptance*: LOD-fades to `CrossQuad`
    with no visible pop at the fade boundary; triangle count stays bounded
    on a densely-forested large terrain per §12.

## 15. Documentation update checklist

- [`README.md`](../README.md) — update the feature bullet list (§0's
  table) once each phase ships.
- [`docs/architecture.md`](architecture.md) — add short subsections
  describing the new pipelines/modules (grass, clouds) alongside the
  existing "flora billboards" and "water plane" descriptions.
- [`docs/world-design-guide.md`](world-design-guide.md) — extend §5
  ("Sea level, atmosphere, and mood") with clouds and the haze-vs-mist
  distinction; extend §6 ("Vegetation") with tree quality and grass.
- [`docs/game-development.md`](game-development.md) — note if any new
  option affects gameplay-relevant queries (none currently planned to,
  since none of these features change terrain height/collision data).
- [`docs/engine-integration.md`](engine-integration.md) — note if hosts
  rendering their own scene around VistaWASM's output (Pattern B) need to
  know about the new `RenderStats.grassInstances` field for their own
  budget accounting.
