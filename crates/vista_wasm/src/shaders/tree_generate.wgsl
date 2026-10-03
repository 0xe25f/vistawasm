// Fills the streamed 64 m tree tiles near the camera, one invocation per
// lattice point, with the points whose rank relative to p lies from the
// far set's share up to the tile's keep: exactly what the static far
// set leaves out, then riparian scrub on a lattice of its own. `tree_at`
// and `scrub_at` in `render/vegetation.rs` decide each point the same
// way, and the tests hold the two together. Each tree is
// grounded on the terrain mesh being drawn as it is generated.
// `ground.wgsl`, `lattice.wgsl` and `generate_common.wgsl` come first.

struct TreeParams {
  terrain: vec4<f32>,
  terrain2: vec4<f32>,
  mesh: vec4<f32>,
  // x: 1 / metres per texel, y: p per cover red squared, z: species
  // variation, w: the far set's share of p.
  rules: vec4<f32>,
  // x: lattice seed, y: the understorey's first rank relative to p (f32
  // bits), z: the boulders' lattice seed, w: 1 when there are boulders to
  // leave room for.
  shape: vec4<u32>,
  // Steepest slope per species, and root radius per unit of scale, four
  // per vector.
  max_slopes: array<vec4<f32>, 2>,
  roots: array<vec4<f32>, 2>,
};

// `TreeInstance` in `render/flora.rs`.
struct TreeInstance {
  position: vec3<f32>,
  scale: f32,
  rotation: f32,
  tint: f32,
  species: u32,
  dryness: f32,
};

@group(0) @binding(0) var<uniform> params: TreeParams;
@group(0) @binding(7) var<storage, read_write> trees: array<TreeInstance>;
@group(0) @binding(8) var<storage, read_write> counts: array<atomic<u32>>;

// `render/lattice.rs` and `render/vegetation.rs`.
const TREE_PITCH: f32 = 3.0;
const TREE_TRAITS_SALT: u32 = 0x2545f491u;
const TREE_SHAPE_SALT: u32 = 0x5bd1e995u;
const LEAN_WATER_METRES: f32 = 8.0;
const OLD_SHARE: u32 = 70u;
const TREE_WATER_LINE: f32 = 0.6;
const TREE_WATER_CLEARANCE: f32 = 1.0;
const TRUNK_CLEARANCE: f32 = 0.5;
const YOUNG_SHARE: u32 = 90u;
const RANK_LEVELS: f32 = 2047.0;
// `render/flora.rs` and `render/tree_models.rs`.
const VARIANT_SHIFT: u32 = 10u;
const AGE_SHIFT: u32 = 12u;
const LEAN_SHIFT: u32 = 14u;
const PREVAILING_WIND: vec2<f32> = vec2<f32>(0.8, 0.6);
// `tree_growth::Age`.
const MATURE: u32 = 0u;
const YOUNG: u32 = 1u;
const OLD: u32 = 2u;
const KRUMMHOLZ: u32 = 3u;
const GROUNDED: u32 = 256u;
const STUNTED: u32 = 512u;
const LATTICE: u32 = 1048576u;
const RANK_SHIFT: u32 = 21u;
const COVER_STUNTED: u32 = 128u;
// `TreeSpecies` and `BiomeKind`.
const SHRUB: u32 = 7u;
const ICE_ARCTIC: u32 = 18u;
const ALPINE_TRANSITION: u32 = 15u;

// The unit direction to the nearest point on a channel's centreline
// within LEAN_WATER_METRES, or zero: `vegetation::channel_lean`.
fn channel_lean(xz: vec2<f32>) -> vec2<f32> {
  let origin = vec2<f32>(bin_float(0u), bin_float(1u));
  let cell = bin_float(2u);
  let columns = i32(bins[3]);
  let rows = i32(bins[4]);
  let first_item = 5u + u32(columns * rows) + 1u;
  let c = vec2<i32>(floor((xz - origin) / cell));
  let x0 = max(c.x - 1, 0);
  let x1 = min(c.x + 1, columns - 1);
  var best = LEAN_WATER_METRES * LEAN_WATER_METRES;
  var towards = vec2<f32>(0.0);

  if (x0 > x1) {
    return towards;
  }

  for (var row = max(c.y - 1, 0); row <= min(c.y + 1, rows - 1); row = row + 1) {
    let first = bins[5 + row * columns + x0];
    let last = bins[5 + row * columns + x1 + 1];

    for (var item = first; item < last; item = item + 1u) {
      let base = first_item + item * BIN_WORDS;
      let a = vec2<f32>(bin_float(base), bin_float(base + 1u));
      let d = vec2<f32>(bin_float(base + 2u), bin_float(base + 3u)) - a;
      let length_squared = d.x * d.x + d.y * d.y;
      var t = 0.0;

      if (length_squared > 0.0) {
        t = clamp(((xz.x - a.x) * d.x + (xz.y - a.y) * d.y) / length_squared, 0.0, 1.0);
      }

      let p = a + d * t - xz;
      let distance = p.x * p.x + p.y * p.y;

      if (distance < best && distance > 1e-6) {
        best = distance;
        towards = p / sqrt(distance);
      }
    }
  }

  return towards;
}

// Variant, age class and lean: `vegetation::shape_bits`.
fn shape_bits(m: Mapping, hash: u32, xz: vec2<f32>, young: bool, stunted: bool) -> u32 {
  let water = channel_lean(xz);
  let by_water = any(water != vec2<f32>(0.0));
  var age = MATURE;

  if (stunted) {
    age = KRUMMHOLZ;
  } else if (young) {
    age = YOUNG;
  } else if (((hash >> 2u) & 255u) < OLD_SHARE && !by_water) {
    age = OLD;
  }

  let dx = height_at(m, xz + vec2<f32>(SLOPE_SPAN, 0.0)) - height_at(m, xz - vec2<f32>(SLOPE_SPAN, 0.0));
  let dz = height_at(m, xz + vec2<f32>(0.0, SLOPE_SPAN)) - height_at(m, xz - vec2<f32>(0.0, SLOPE_SPAN));
  let run = 2.0 * SLOPE_SPAN;
  let rise = length(vec2<f32>(dx, dz));
  let slope = degrees(atan(rise / run));
  let fall = min(rise / run * 10.0, 2.0);
  var lean = -vec2<f32>(dx, dz) / max(rise, 1e-6) * fall + PREVAILING_WIND * 0.5;

  if (by_water) {
    lean = water;
  }

  let sector = u32(i32(round(atan2(lean.x, lean.y) / (6.2831855 / 16.0))) & 15);
  let roll = (hash >> 10u) & 255u;
  var strength = select(0u, 1u, slope >= 12.0) + select(0u, 1u, slope >= 25.0) + select(0u, 1u, slope >= 35.0);
  strength = max(strength, select(0u, 1u, roll >= 128u) + select(0u, 1u, roll >= 218u));

  if (by_water) {
    strength = max(strength, 2u);
  }

  return ((hash & 3u) << VARIANT_SHIFT) | (age << AGE_SHIFT) | ((sector | (min(strength, 3u) << 4u)) << LEAN_SHIFT);
}

// Riparian scrub (`vegetation::scrub_at`): its own lattice, its mix by
// biome (`SCRUB_SPECIES`), from 1 m to 4 m plus half the width outside a
// channel's edge.
const RIPARIAN_SALT: u32 = 0x68e31da5u;
const SCRUB_CHANCE: f32 = 0.45;
const SCRUB_FROM: f32 = 1.0;
const SCRUB_DESERT: f32 = 2.5;
const SAVANNAH: u32 = 8u;
const SCRUB_SPECIES = array<u32, 19>(2567u, 2567u, 2311u, 2055u, 3959u, 3959u, 3959u, 0u, 3959u, 2359u, 3959u, 2359u, 2103u, 2391u, 0u, 0u, 0u, 0u, 0u);

fn riparian_scrub(m: Mapping, cell: vec2<i32>, job: Job) {
  let seed = params.shape.x ^ RIPARIAN_SALT;
  let hash = pcg3d(vec3<u32>(bitcast<vec2<u32>>(cell), seed));
  let xz = jittered(cell, hash, TREE_PITCH);

  if (!on_map(m, xz)) {
    return;
  }

  let near = near_channel(xz, 1e9);
  let surface = bytes_at(surface_texture, nearest(m, xz));
  var table = SCRUB_SPECIES;
  let mix = table[min(surface.a, 18u)];
  var outer = 4.0 + 0.5 * (bin_float(near.base + 4u) + bin_float(near.base + 5u));

  if (surface.a == SAVANNAH) {
    outer = SCRUB_DESERT;
  }

  if (mix == 0u || near.edge < SCRUB_FROM || near.edge > outer || surface.b > 127u) {
    return;
  }

  let gaps = saturate((clump(cell, seed) - 0.15) / 0.5);
  let steep = saturate((45.0 - slope_degrees(m, xz)) / 10.0);
  let chance = SCRUB_CHANCE * bin_float(near.base + 10u) * gaps * steep;
  let rank = unit(hash.z);

  if (chance <= 0.0 || rank < chance * params.rules.w || rank >= chance * job.keep
    || height_at(m, xz) <= params.terrain2.w + TREE_WATER_LINE
    || water_distance(m, xz) <= 0.0
    || in_channel(xz, 1.0)
    || (params.shape.w == 1u && under_boulder(m, xz, params.shape.z, TRUNK_CLEARANCE))) {
    return;
  }

  let traits = pcg3d(vec3<u32>(bitcast<vec2<u32>>(cell), seed ^ TREE_TRAITS_SALT));
  // Cold streams are lined with willow alone.
  let first = surface.r < 97u || unit(traits.x) * 15.0 < f32((mix >> 8u) & 15u);
  let species = select((mix >> 4u) & 15u, mix & 15u, first);
  let shrub = species == SHRUB;
  let size_roll = unit(traits.y);
  let scale = select(0.15 + 0.15 * size_roll, 1.0 + 1.3 * size_roll, shrub);
  let shape = pcg3d(vec3<u32>(bitcast<vec2<u32>>(cell), seed ^ TREE_SHAPE_SALT)).x;
  let rank_bits = min(u32(rank / chance * RANK_LEVELS), 2046u);
  var tree: TreeInstance;
  tree.position = vec3<f32>(xz.x, grounded_base(heights, params.terrain, params.terrain2, params.mesh, xz, params.roots[species / 4u][species % 4u] * scale), xz.y);
  tree.scale = scale;
  tree.rotation = f32(traits.z & 255u) * (6.2831855 / 256.0);
  tree.tint = 0.35 + 0.3 * unit(traits.z);
  tree.species = species | GROUNDED | LATTICE | shape_bits(m, shape, xz, !shrub, false) | (rank_bits << RANK_SHIFT);
  tree.dryness = 0.0;
  let slot = atomicAdd(&counts[job.slot], 1u);

  if (slot < job.capacity) {
    trees[job.first + slot] = tree;
  }
}

// `vegetation::krummholz_rotation`.
fn krummholz_rotation(hash: u32) -> f32 {
  return atan2(PREVAILING_WIND.y, -PREVAILING_WIND.x) + (f32((hash >> 18u) & 255u) / 255.0 - 0.5) * 0.7;
}

@compute @workgroup_size(8, 8)
fn generate_main(@builtin(global_invocation_id) id: vec3<u32>) {
  let job = jobs[id.z];
  let first = vec2<i32>(tile_first(job.tile.x, 64, 3), tile_first(job.tile.y, 64, 3));
  let next = vec2<i32>(tile_first(job.tile.x + 1, 64, 3), tile_first(job.tile.y + 1, 64, 3));
  let cell = first + vec2<i32>(id.xy);

  if (any(cell >= next)) {
    return;
  }

  let m = Mapping(params.terrain, params.terrain2, params.rules.x);
  riparian_scrub(m, cell, job);
  let seed = params.shape.x;
  let hash = pcg3d(vec3<u32>(bitcast<vec2<u32>>(cell), seed));
  let xz = jittered(cell, hash, TREE_PITCH);

  if (!on_map(m, xz)) {
    return;
  }

  let texel = nearest(m, xz);
  let cover = bytes_at(cover_texture, texel);
  let red = f32(cover.r);
  let p = red * red * params.rules.y * clump(cell, seed);
  let rank = unit(hash.z);

  if (cover.r == 0u || rank < p * params.rules.w || rank >= p * job.keep) {
    return;
  }

  let traits = pcg3d(vec3<u32>(bitcast<vec2<u32>>(cell), seed ^ TREE_TRAITS_SALT));
  let second = unit(traits.x) < f32(cover.a & 127u) / 127.0;
  let species = min(select(cover.g, cover.b, second), 7u);
  let elevation = height_at(m, xz);

  if (elevation <= params.terrain2.w + TREE_WATER_LINE
    || in_water(m, xz, TREE_WATER_CLEARANCE)
    || slope_degrees(m, xz) >= params.max_slopes[species / 4u][species % 4u]
    || in_channel(xz, 1.0)
    || (params.shape.w == 1u && under_boulder(m, xz, params.shape.z, TRUNK_CLEARANCE))) {
    return;
  }

  let surface = bytes_at(surface_texture, texel);
  let temperature = f32(surface.r) / 255.0;
  let moisture = f32(surface.g) / 255.0;
  let biome = surface.a;
  let size_roll = unit(traits.y);
  let tint_roll = unit(traits.z);
  let variation = params.rules.z;
  // Trees on poor ground and at forest edges grow smaller.
  let share = red * red * (4.0 / 65025.0);
  let vigour = 0.75 + 0.3 * min(share * 3.0, 1.0);
  var scale = (1.0 + (size_roll - 0.5) * 0.55 * max(variation, 0.15)) * vigour;
  var tint = 0.5 + (tint_roll - 0.5) * max(variation, 0.1);
  var dryness = clamp((temperature - 0.45) * 1.5 + (0.5 - moisture) * 1.5, 0.0, 1.0);

  var young = (traits.y & 255u) < YOUNG_SHARE;

  if (young) {
    scale = 0.45 + 0.35 * size_roll;
  }

  if (rank / p >= bitcast<f32>(params.shape.y)) {
    scale = 0.25 + 0.25 * size_roll;
    young = true;
  }

  let shrub = species == SHRUB;
  let tundra = biome == ICE_ARCTIC;
  let stunted = !shrub && (cover.a & COVER_STUNTED) != 0u;

  if (shrub && (tundra || biome == ALPINE_TRANSITION)) {
    scale = 0.6 + 0.6 * size_roll;
    young = true;

    if (tundra) {
      tint = 0.15 + 0.2 * tint_roll;
      dryness = 0.75;
    }
  } else if (stunted) {
    scale = scale * 0.7;
  }

  let rank_bits = min(u32(rank / p * RANK_LEVELS), 2046u);
  let root = params.roots[species / 4u][species % 4u] * scale;
  var tree: TreeInstance;
  tree.position = vec3<f32>(xz.x, grounded_base(heights, params.terrain, params.terrain2, params.mesh, xz, root), xz.y);
  tree.scale = scale;
  let shape = pcg3d(vec3<u32>(bitcast<vec2<u32>>(cell), seed ^ TREE_SHAPE_SALT)).x;
  tree.rotation = select(f32(traits.z & 255u) * (6.2831855 / 256.0), krummholz_rotation(shape), stunted);
  tree.tint = tint;
  tree.species = species | GROUNDED | LATTICE | select(0u, STUNTED, stunted) | shape_bits(m, shape, xz, young, stunted)
    | (rank_bits << RANK_SHIFT);
  tree.dryness = dryness;
  let slot = atomicAdd(&counts[job.slot], 1u);

  if (slot < job.capacity) {
    trees[job.first + slot] = tree;
  }
}
