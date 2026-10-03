// Streamed grass: `generate_main` fills 16 m grass tiles near the camera,
// one invocation per lattice point (0.35 m apart), and `cull_main` picks
// the tufts to draw this frame. `ground.wgsl`, `lattice.wgsl` and
// `generate_common.wgsl` come first.
//
// Generation mirrors `tuft_at` in `render/grass.rs`: meadow grass by the
// ground's grass texture, denser and greener within 12 m of water and
// along the riparian field, thinned under canopy where ferns and
// undergrowth replace it, short tufts on tundra, nothing on glacier,
// nothing in any drawn channel and nothing under a boulder; a riparian
// band of tufts leaning over each channel's water and of tall herbs and
// ferns behind them; how readily
// grass grows is read between texels, turf thickens at the lip of rock
// outcrops, and all of it thins smoothly on steep ground. Tufts are
// grounded on the drawn mesh.
// Reeds are placed on the CPU and drawn from their own buffer.
//
// Culling keeps the tufts in the view, thins them with distance by rank
// and widens the ones it keeps so the cover stays the same. From the
// handover's start (`grass::handover_start`: 1.5 times the full-cover
// radius, at least 0.3 and at most 0.9 times the view distance) to the
// view distance, tufts hand over to the ground's own grass sheen: by
// rank, so the meadow thins out evenly, since their blades are finer
// than a pixel there and cost far more than they show. The sheen takes
// exactly the share they give up (`clipmap_render.wgsl`). It writes two indirect draws: tufts within 15 m as two
// crossed quads, at the front of `drawn`, and the rest as single cards
// after `NEAR_SLOTS`.

struct GrassParams {
  terrain: vec4<f32>,
  terrain2: vec4<f32>,
  mesh: vec4<f32>,
  // x: 1 / metres per texel, y: tuft probability on ideal meadow,
  // z: crown area per square metre per unit of cover share (0 without a
  // forest floor), w: meadow grass height.
  rules: vec4<f32>,
  // x: lattice seed, y: tile classes, z: pool instances, w: the
  // boulders' lattice seed.
  shape: vec4<u32>,
  // xyz: camera, w: full-density radius.
  camera: vec4<f32>,
  // x: view distance, y: 1 when there are boulders to leave room for,
  // z: 1 when a grass density mask is set, w: where tufts start to hand
  // over to the ground's grass sheen (`grass::handover_start`).
  view: vec4<f32>,
  planes: array<vec4<f32>, 6>,
  // Per class: first slot, first instance, capacity, slots.
  classes: array<vec4<u32>, 8>,
};

@group(0) @binding(0) var<uniform> params: GrassParams;
// Tufts as raw floats, 7 per tuft (`FloraInstance` in `render/flora.rs`).
@group(0) @binding(7) var<storage, read_write> tufts: array<f32>;
@group(0) @binding(8) var<storage, read_write> counts: array<atomic<u32>>;
// The grass texture (`grass::bake_grass`).
@group(0) @binding(9) var grass_texture: texture_2d<f32>;
@group(0) @binding(10) var<storage, read_write> drawn: array<f32>;
@group(0) @binding(11) var<storage, read_write> args: array<atomic<u32>>;
// The grass density mask (`GroundData::grass_mask`), capped to density 4;
// 1 x 1 and neutral without one. The terrain's grass sheen reads it too.
@group(0) @binding(12) var grass_mask: texture_2d<f32>;
@group(0) @binding(13) var mask_sampler: sampler;

// `render/lattice.rs`, `render/grass.rs` and `render/flora.rs`.
const GRASS_PITCH: f32 = 0.35;
const TRAITS_SALT: u32 = 0x2545f491u;
const GRASS_WATER_LINE: f32 = 0.5;
const NEAR_WATER_METRES: f32 = 12.0;
const FLOOR_DENSITY: f32 = 0.35;
const TUNDRA_GRASS_HEIGHT: f32 = 0.4;
const FERN: f32 = 2.0;
const UNDERGROWTH: f32 = 3.0;
const MAX_PLANTING_SLOPE: f32 = 50.0;
const TUFT_METRES: f32 = 0.6;
const HERB_METRES: f32 = 3.0;
const RIPARIAN_TUFTS: f32 = 0.5;
const RIPARIAN_HERBS: f32 = 0.12;
// `BiomeKind::AlpineTransition`: from it on, every biome lies above the
// trees.
const ALPINE_TRANSITION: u32 = 15u;
// `GRASS_NEAR_SLOTS` in `render/gpu.rs`, and the card distance in
// `grass_instances.wgsl`.
const NEAR_SLOTS: u32 = 8192u;
const CARD_METRES: f32 = 15.0;

// `smoothstep` in `maths.rs`: clamped to 0 to 1.
fn smoothed(t: f32) -> f32 {
  let x = clamp(t, 0.0, 1.0);
  return x * x * (3.0 - 2.0 * x);
}

// The riparian band's grass at `xz`: x the chance of a tuft leaning over
// the water, y of a tall herb, fern or sedge, zw the way to the water:
// `grass::riparian_grass`.
fn riparian_grass(m: Mapping, xz: vec2<f32>, banks: vec4<u32>, snow: u32, mask: f32, shade: f32) -> vec4<f32> {
  if (banks.b == 0u || snow > 127u || water_distance(m, xz) > HERB_METRES + 0.75 / m.inverse) {
    return vec4<f32>(0.0);
  }

  let near = near_channel(xz, HERB_METRES);

  if (near.edge > HERB_METRES) {
    return vec4<f32>(0.0);
  }

  let band = bin_float(near.base + 9u) * mask;
  let tufts = select(0.0, band * RIPARIAN_TUFTS * (1.0 - smoothed((near.edge - 0.4) / 0.2)), near.edge >= 0.0);
  let herbs = band * RIPARIAN_HERBS * smoothed((near.edge - 0.3) / 0.2) * (1.0 - smoothed((near.edge - 2.5) / 0.5))
    * (1.0 + shade);
  let a = vec2<f32>(bin_float(near.base), bin_float(near.base + 1u));
  let d = vec2<f32>(bin_float(near.base + 2u), bin_float(near.base + 3u)) - a;
  let length_squared = d.x * d.x + d.y * d.y;
  var t = 0.0;

  if (length_squared > 0.0) {
    t = clamp(((xz.x - a.x) * d.x + (xz.y - a.y) * d.y) / length_squared, 0.0, 1.0);
  }

  let to = a + d * t - xz;
  return vec4<f32>(tufts, herbs, to / max(length(to), 1e-6));
}

// `grass::slope_fade`.
fn slope_fade(slope: f32) -> f32 {
  return 1.0 - smoothed((slope - 0.6 * MAX_PLANTING_SLOPE) / (0.4 * MAX_PLANTING_SLOPE));
}

// How readily grass grows, between texels: `GroundData::bilinear` of the
// grass texture's red channel.
fn grass_accept(m: Mapping, xz: vec2<f32>) -> f32 {
  let t = clamp(texel_position(m, xz), vec2<f32>(0.0), m.terrain2.xy - vec2<f32>(1.001));
  let base = floor(t);
  let f = t - base;
  let i = vec2<i32>(base);
  let a00 = textureLoad(grass_texture, i, 0).r;
  let a10 = textureLoad(grass_texture, i + vec2<i32>(1, 0), 0).r;
  let a01 = textureLoad(grass_texture, i + vec2<i32>(0, 1), 0).r;
  let a11 = textureLoad(grass_texture, i + vec2<i32>(1, 1), 0).r;
  return mix(mix(a00, a10, f.x), mix(a01, a11, f.x), f.y);
}

// The grass density mask's multiplier, 0 to 2: `GroundData::grass_multiplier`.
fn grass_mask_at(m: Mapping, xz: vec2<f32>) -> f32 {
  let uv = (texel_position(m, xz) + 0.5) / m.terrain2.xy;
  return density_multiplier(textureSampleLevel(grass_mask, mask_sampler, uv, 0.0).r);
}

// The rock's share of the ground between texels, from the grass
// texture's blue channel: `grass::rock_share` read like
// `GroundData::bilinear`.
fn grass_rock(m: Mapping, xz: vec2<f32>) -> f32 {
  let t = clamp(texel_position(m, xz), vec2<f32>(0.0), m.terrain2.xy - vec2<f32>(1.001));
  let base = floor(t);
  let f = t - base;
  let i = vec2<i32>(base);
  var r = vec4<f32>(
    textureLoad(grass_texture, i, 0).b,
    textureLoad(grass_texture, i + vec2<i32>(1, 0), 0).b,
    textureLoad(grass_texture, i + vec2<i32>(0, 1), 0).b,
    textureLoad(grass_texture, i + vec2<i32>(1, 1), 0).b
  );
  r = select(round(r * 255.0) / 127.0, vec4<f32>(0.0), r * 255.0 > vec4<f32>(127.5));
  return mix(mix(r.x, r.y, f.x), mix(r.z, r.w, f.x), f.y);
}

// `grass::edge_turf`.
fn edge_turf(rock: f32) -> f32 {
  return 1.2 * smoothed((rock - 0.2) / 0.2) * (1.0 - smoothed((rock - 0.45) / 0.1));
}

@compute @workgroup_size(8, 8)
fn generate_main(@builtin(global_invocation_id) id: vec3<u32>) {
  let job = jobs[id.z];
  let first = vec2<i32>(tile_first(job.tile.x, 1600, 35), tile_first(job.tile.y, 1600, 35));
  let next = vec2<i32>(tile_first(job.tile.x + 1, 1600, 35), tile_first(job.tile.y + 1, 1600, 35));
  let cell = first + vec2<i32>(id.xy);

  if (any(cell >= next)) {
    return;
  }

  let m = Mapping(params.terrain, params.terrain2, params.rules.x);
  let seed = params.shape.x;
  let hash = pcg3d(vec3<u32>(bitcast<vec2<u32>>(cell), seed));
  let xz = jittered(cell, hash, GRASS_PITCH);

  if (!on_map(m, xz)) {
    return;
  }

  let texel = nearest(m, xz);
  let grass = bytes_at(grass_texture, texel);
  let rank = unit(hash.z);
  let water = water_distance(m, xz);
  let banks = bytes_at(banks_texture, texel);
  let near = max(1.0 - smoothed(water / NEAR_WATER_METRES), f32(banks.b) / 255.0);
  let red = f32(bytes_at(cover_texture, texel).r);
  let shade = 1.0 - exp(-params.rules.z * red * red * (4.0 / 65025.0));
  // Without a mask the sample is skipped: a uniform branch.
  var mask = 1.0;

  if (params.view.z > 0.5) {
    mask = grass_mask_at(m, xz);
  }

  var slope = 0.0;

  if (rank < params.rules.y * mask * job.keep) {
    slope = slope_fade(slope_degrees(m, xz));
  }

  let meadow = params.rules.y * min(grass_accept(m, xz) * (1.0 + 0.6 * near), 1.0) * (1.0 - shade) * slope
    * (1.0 + edge_turf(grass_rock(m, xz))) * mask;
  let floor_plants = params.rules.y * FLOOR_DENSITY * shade * f32(grass.a) / 255.0 * slope * mask;
  let surface = bytes_at(surface_texture, texel);
  let riparian = riparian_grass(m, xz, banks, surface.b, mask, shade);
  let p = meadow + floor_plants + riparian.x + riparian.y;

  if (p <= 0.0 || rank >= p * job.keep) {
    return;
  }

  // The band keeps out of water only where the coarse wet-bank field is
  // water all round: `grass::tuft_at`.
  let wet = select(in_water(m, xz, 0.0), water <= 0.0, rank < riparian.x + riparian.y);

  if (height_at(m, xz) <= params.terrain2.w + GRASS_WATER_LINE || wet || in_channel(xz, 0.0)
    || (params.view.y > 0.5 && under_boulder(m, xz, params.shape.w, 0.0))) {
    return;
  }

  let traits = pcg3d(vec3<u32>(bitcast<vec2<u32>>(cell), seed ^ TRAITS_SALT));
  let size_roll = unit(traits.x);
  let relative = rank / p;
  var style = 0.0;
  var scale = 0.5 + size_roll * 0.6;
  var dryness = f32(grass.g) / 255.0 * (1.0 - 0.7 * near);
  let tundra = grass.b > 127u;
  let band = riparian.x + riparian.y;

  // The band takes the first ranks: by the water it replaces the meadow.
  if (rank < riparian.x) {
    // Riparian tufts lean over the water: the eighth of a turn towards
    // it rides on the dryness (`grass_instances.wgsl`).
    let sector = i32(round(atan2(riparian.w, riparian.z) / (6.2831853 / 8.0))) & 7;
    scale = scale * 1.3 * select(params.rules.w, TUNDRA_GRASS_HEIGHT, tundra);
    dryness = dryness + 2.0 * f32(sector + 1);
  } else if (rank < band && (tundra || surface.a >= ALPINE_TRANSITION)) {
    // Above the trees the herbs are sedges.
    scale = scale * 1.2;
    dryness = 0.3;
  } else if (rank < band) {
    // Tall herbs and ferns, 0.5 to 1.5 m.
    let fern = surface.g > 140u && surface.r >= 100u && surface.r < 200u;
    style = select(UNDERGROWTH, FERN, fern);
    scale = 0.7 + 1.3 * size_roll * select(0.6, 1.0, fern);
    dryness = 0.0;
  } else if (rank >= band + meadow) {
    // Ferns in temperate and wet ground, undergrowth in any forest.
    let fern = surface.g > 140u && surface.r >= 100u && surface.r < 200u;
    style = select(UNDERGROWTH, FERN, fern);
    scale = select(0.3 + size_roll * 0.3, 0.4 + size_roll * 0.5, fern);
    dryness = 0.0;
  } else if (tundra) {
    // Where tundra is the ground, tufts are short and ochre-green.
    scale = scale * TUNDRA_GRASS_HEIGHT;
    dryness = 0.5;
  } else {
    scale = scale * params.rules.w;
  }

  let slot = atomicAdd(&counts[job.slot], 1u);

  if (slot < job.capacity) {
    let base = (job.first + slot) * 7u;
    tufts[base] = xz.x;
    tufts[base + 1u] = grounded_base(heights, params.terrain, params.terrain2, params.mesh, xz, 0.0);
    tufts[base + 2u] = xz.y;
    tufts[base + 3u] = scale;
    tufts[base + 4u] = unit(traits.y);
    tufts[base + 5u] = dryness;
    tufts[base + 6u] = style + min(relative, 0.999);
  }
}

@compute @workgroup_size(64)
fn cull_main(@builtin(global_invocation_id) id: vec3<u32>) {
  let index = id.x;

  if (index >= params.shape.z) {
    return;
  }

  // The distance class, slot and place in the slot of this pool entry.
  var group = 0u;

  for (var c = 1u; c < params.shape.y; c = c + 1u) {
    if (index >= params.classes[c].y) {
      group = c;
    }
  }

  let spec = params.classes[group];
  let offset = index - spec.y;
  let slot = spec.x + offset / spec.z;

  if (offset % spec.z >= min(atomicLoad(&counts[slot]), spec.z)) {
    return;
  }

  let base = index * 7u;
  let position = vec3<f32>(tufts[base], tufts[base + 1u], tufts[base + 2u]);
  let distance = length(position - params.camera.xyz);

  if (distance > params.view.x) {
    return;
  }

  let style = tufts[base + 6u];
  let kind = floor(style);
  let t = thinning(distance, params.camera.w, 0.0);
  let fade = thinning_fade(t, fract(style));
  let start = params.view.w;
  let handover = 1.0 - smoothed((distance - start) / max(params.view.x - start, 1.0));

  if (fade <= 0.0 || (handover < 1.0 && fract(style) >= handover * (t.x + t.y))) {
    return;
  }

  let scale = tufts[base + 3u] * fade * t.z;
  let centre = position + vec3<f32>(0.0, scale * 0.5, 0.0);
  let radius = scale * 1.5 * t.w + 0.5;

  for (var p = 0; p < 6; p = p + 1) {
    let plane = params.planes[p];

    if (dot(plane.xyz, centre) + plane.w < -radius) {
      return;
    }
  }

  var place = NEAR_SLOTS;

  if (distance < CARD_METRES) {
    place = atomicAdd(&args[1], 1u);

    if (place >= NEAR_SLOTS) {
      atomicSub(&args[1], 1u);
    }
  }

  if (place >= NEAR_SLOTS) {
    place = NEAR_SLOTS + atomicAdd(&args[5], 1u);
  }

  let out = place * 7u;
  drawn[out] = position.x;
  drawn[out + 1u] = position.y;
  drawn[out + 2u] = position.z;
  drawn[out + 3u] = scale;
  drawn[out + 4u] = tufts[base + 4u];
  drawn[out + 5u] = tufts[base + 5u];
  // The drawn tuft keeps its style and carries its extra width.
  drawn[out + 6u] = kind + min((t.w - 1.0) * 0.25, 0.999);
}
