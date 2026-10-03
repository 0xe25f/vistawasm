// Bindings and helpers shared by the tree and grass generators
// (`tree_generate.wgsl`, `grass_generate.wgsl`), which each put
// `ground.wgsl` and `lattice.wgsl` first. The CPU mirrors are in
// `render/vegetation.rs` (`GroundData`, `in_channel`, `tile_first`).

// The per-terrain textures (see `GroundData`), read with `textureLoad`.
@group(0) @binding(1) var heights: texture_2d<f32>;
@group(0) @binding(2) var surface_texture: texture_2d<f32>;
@group(0) @binding(3) var banks_texture: texture_2d<f32>;
@group(0) @binding(4) var cover_texture: texture_2d<f32>;
// The drawn channels and plunge pools (`vegetation::channel_bins`), and
// the words each segment takes (`BIN_WORDS`).
@group(0) @binding(5) var<storage, read> bins: array<u32>;
const BIN_WORDS: u32 = 11u;

// A tile to fill: its coordinates, where its slot starts in the pool and
// how many it holds, the slot's index, and the rank (relative to p)
// candidates are stored below.
struct Job {
  tile: vec2<i32>,
  first: u32,
  capacity: u32,
  slot: u32,
  keep: f32,
  unused: vec2<u32>,
};

@group(0) @binding(6) var<storage, read> jobs: array<Job>;

// The terrain mapping: `terrain` and `terrain2` as in `ground.wgsl`,
// and `inverse` = 1 / metres per texel, as the CPU multiplies by it.
struct Mapping {
  terrain: vec4<f32>,
  terrain2: vec4<f32>,
  inverse: f32,
};

// The first lattice index of a tile along one axis: `tile_first`.
fn tile_first(tile: i32, size: i32, pitch: i32) -> i32 {
  let a = tile * size;
  return a / pitch + select(0, 1, a % pitch > 0);
}

fn on_map(m: Mapping, xz: vec2<f32>) -> bool {
  return all(abs(xz) <= m.terrain.xy);
}

// Texel coordinates of a world position.
fn texel_position(m: Mapping, xz: vec2<f32>) -> vec2<f32> {
  return (xz + m.terrain.xy) * m.inverse;
}

// The nearest texel, clamped: `GroundData::nearest`.
fn nearest(m: Mapping, xz: vec2<f32>) -> vec2<i32> {
  let t = floor(texel_position(m, xz) + 0.5);
  return clamp(vec2<i32>(t), vec2<i32>(0), vec2<i32>(m.terrain2.xy) - 1);
}

// A texel of an rgba8 texture as its bytes.
fn bytes_at(t: texture_2d<f32>, texel: vec2<i32>) -> vec4<u32> {
  return vec4<u32>(round(textureLoad(t, texel, 0) * 255.0));
}

// Bilinear ground height: `GroundData::height_at`.
fn height_at(m: Mapping, xz: vec2<f32>) -> f32 {
  return texel_height(heights, m.terrain2, texel_position(m, xz));
}

// Bilinear distance to water in metres: `GroundData::water_distance`.
fn water_distance(m: Mapping, xz: vec2<f32>) -> f32 {
  let t = clamp(texel_position(m, xz), vec2<f32>(0.0), m.terrain2.xy - vec2<f32>(1.001));
  let base = floor(t);
  let f = t - base;
  let i = vec2<i32>(base);
  let d00 = textureLoad(banks_texture, i, 0).r;
  let d10 = textureLoad(banks_texture, i + vec2<i32>(1, 0), 0).r;
  let d01 = textureLoad(banks_texture, i + vec2<i32>(0, 1), 0).r;
  let d11 = textureLoad(banks_texture, i + vec2<i32>(1, 1), 0).r;
  return mix(mix(d00, d10, f.x), mix(d01, d11, f.x), f.y) * 40.0;
}

// Whether `xz` is in a lake or river, or within `clearance` of it:
// `GroundData::in_water`.
fn in_water(m: Mapping, xz: vec2<f32>, clearance: f32) -> bool {
  return water_distance(m, xz) < 0.25 * m.terrain.z + clearance;
}

// `SLOPE_SPAN` in `render/vegetation.rs`.
const SLOPE_SPAN: f32 = 1.5;

// Slope in degrees: `GroundData::slope_degrees`.
fn slope_degrees(m: Mapping, xz: vec2<f32>) -> f32 {
  let dx = height_at(m, xz + vec2<f32>(SLOPE_SPAN, 0.0)) - height_at(m, xz - vec2<f32>(SLOPE_SPAN, 0.0));
  let dz = height_at(m, xz + vec2<f32>(0.0, SLOPE_SPAN)) - height_at(m, xz - vec2<f32>(0.0, SLOPE_SPAN));
  return degrees(atan(length(vec2<f32>(dx, dz)) / (2.0 * SLOPE_SPAN)));
}

fn bin_float(index: u32) -> f32 {
  return bitcast<f32>(bins[index]);
}

// How far `xz` is inside one binned water's clearance, with `clearance`
// scaling the clearance (1 for trees, 0 for grass): `Water::intrusion`.
fn intrusion(base: u32, xz: vec2<f32>, clearance: f32) -> f32 {
  let a = vec2<f32>(bin_float(base), bin_float(base + 1u));
  let b = vec2<f32>(bin_float(base + 2u), bin_float(base + 3u));
  let d = b - a;
  let length_squared = d.x * d.x + d.y * d.y;
  var t = 0.0;

  if (length_squared > 0.0) {
    t = clamp(((xz.x - a.x) * d.x + (xz.y - a.y) * d.y) / length_squared, 0.0, 1.0);
  }

  let p = a + d * t - xz;
  let half = bin_float(base + 4u) + (bin_float(base + 5u) - bin_float(base + 4u)) * t;
  return half + bin_float(base + 6u) * clearance - sqrt(p.x * p.x + p.y * p.y);
}

// Whether `xz` is in a channel's water, or its clearance too:
// `vegetation::in_channel`.
fn in_channel(xz: vec2<f32>, clearance: f32) -> bool {
  let origin = vec2<f32>(bin_float(0u), bin_float(1u));
  let cell = bin_float(2u);
  let columns = i32(bins[3]);
  let rows = i32(bins[4]);
  let first_item = 5u + u32(columns * rows) + 1u;
  let c = vec2<i32>(floor((xz - origin) / cell));
  let x0 = max(c.x - 1, 0);
  let x1 = min(c.x + 1, columns - 1);

  if (x0 > x1) {
    return false;
  }

  for (var row = max(c.y - 1, 0); row <= min(c.y + 1, rows - 1); row = row + 1) {
    let first = bins[5 + row * columns + x0];
    let last = bins[5 + row * columns + x1 + 1];

    for (var item = first; item < last; item = item + 1u) {
      if (intrusion(first_item + item * BIN_WORDS, xz, clearance) > 0.0) {
        return true;
      }
    }
  }

  return false;
}

// The channel whose water's edge is nearest `xz`, within `reach` metres
// outside it: how far outside its edge (negative in the water), above
// `reach` where none is, and where its words start in `bins`:
// `vegetation::near_channel`.
struct NearChannel {
  edge: f32,
  base: u32,
};

fn near_channel(xz: vec2<f32>, reach: f32) -> NearChannel {
  let origin = vec2<f32>(bin_float(0u), bin_float(1u));
  let cell = bin_float(2u);
  let columns = i32(bins[3]);
  let rows = i32(bins[4]);
  let first_item = 5u + u32(columns * rows) + 1u;
  let c = vec2<i32>(floor((xz - origin) / cell));
  let x0 = max(c.x - 1, 0);
  let x1 = min(c.x + 1, columns - 1);
  var best = NearChannel(reach + 1.0, 0u);

  if (x0 > x1) {
    return best;
  }

  for (var row = max(c.y - 1, 0); row <= min(c.y + 1, rows - 1); row = row + 1) {
    let first = bins[5 + row * columns + x0];
    let last = bins[5 + row * columns + x1 + 1];

    for (var item = first; item < last; item = item + 1u) {
      let base = first_item + item * BIN_WORDS;
      let edge = -intrusion(base, xz, 0.0);

      if (edge <= reach && edge < best.edge) {
        best = NearChannel(edge, base);
      }
    }
  }

  return best;
}

// Boulders (`render/boulders.rs`): their lattice, and the room trees and
// grass leave for them.
const BOULDER_PITCH: f32 = 2.0;
const BOULDER_SALT: u32 = 0x7f4a7c15u;
const BOULDER_WATER_LINE: f32 = 0.3;
const BOULDER_WATER_CLEARANCE: f32 = 1.0;
const SIZE_EXPONENT: f32 = 1.2;
const FOOTPRINT: f32 = 0.55;
// `soil::MAX_TALUS_PROBABILITY`.
const TALUS_PROBABILITY: f32 = 0.6;

// `boulder_size`: a power law from 0.3 to 3 m across, larger down the
// talus cone.
fn boulder_size(u: f32, position: f32) -> f32 {
  let tail = pow(0.1, SIZE_EXPONENT);
  let size = 0.3 * pow(1.0 - u * (1.0 - tail), -1.0 / SIZE_EXPONENT);
  return clamp(size * (0.75 + 0.6 * position), 0.3, 3.0);
}

// `boulder_candidate`: x, z, size and rank relative to p of the boulder
// at lattice point `cell`, or a negative size where none lies.
fn boulder_candidate(m: Mapping, cell: vec2<i32>, seed: u32) -> vec4<f32> {
  let hash = pcg3d(vec3<u32>(bitcast<vec2<u32>>(cell), seed));
  let xz = jittered(cell, hash, BOULDER_PITCH);

  if (!on_map(m, xz)) {
    return vec4<f32>(0.0, 0.0, -1.0, 0.0);
  }

  let talus = bytes_at(banks_texture, nearest(m, xz)).a;
  let p = f32(talus >> 4u) / 15.0 * TALUS_PROBABILITY;
  let rank = unit(hash.z);

  if (rank >= p || height_at(m, xz) <= m.terrain2.w + BOULDER_WATER_LINE
    || in_water(m, xz, BOULDER_WATER_CLEARANCE) || in_channel(xz, 1.0)) {
    return vec4<f32>(0.0, 0.0, -1.0, 0.0);
  }

  let traits = pcg3d(vec3<u32>(bitcast<vec2<u32>>(cell), seed ^ 0x2545f491u));
  return vec4<f32>(xz, boulder_size(unit(traits.x), f32(talus & 15u) / 15.0), rank / p);
}

// Stream stones (`render/boulders.rs`): their lattice and sizes.
const STONE_PITCH: f32 = 1.0;
const STONE_REACH: f32 = 1.5;
const COBBLE_METRES: f32 = 0.5;

// `stone_size`: a power law from 0.71 to 3 times the stream's median.
fn stone_size(u: f32, median: f32) -> f32 {
  return clamp(0.71 * median / sqrt(1.0 - u * (1.0 - 0.056)), 0.05, 3.0);
}

// `stone_candidate`: x, z, size and rank relative to its chance of the
// stream stone at stone lattice point `cell`, or a negative size where
// none lies.
fn stone_candidate(cell: vec2<i32>, seed: u32) -> vec4<f32> {
  let hash = pcg3d(vec3<u32>(bitcast<vec2<u32>>(cell), seed));
  let xz = jittered(cell, hash, STONE_PITCH);
  let near = near_channel(xz, STONE_REACH);

  if (near.edge > STONE_REACH) {
    return vec4<f32>(0.0, 0.0, -1.0, 0.0);
  }

  var chance = bin_float(near.base + 8u);
  var scale = 1.0;

  if (near.edge > COBBLE_METRES) {
    chance = chance / 3.0;
  } else if (near.edge > 0.0) {
    chance = min(1.5 * chance, 0.5);
    scale = 0.33;
  }

  let rank = unit(hash.z);

  if (rank >= chance) {
    return vec4<f32>(0.0, 0.0, -1.0, 0.0);
  }

  let traits = pcg3d(vec3<u32>(bitcast<vec2<u32>>(cell), seed ^ 0x2545f491u));
  return vec4<f32>(xz, max(stone_size(unit(traits.x), bin_float(near.base + 7u)) * scale, 0.05), rank / chance);
}

// `under_boulder`: whether `xz` lies within `margin` metres of a
// boulder's footprint.
fn under_boulder(m: Mapping, xz: vec2<f32>, seed: u32, margin: f32) -> bool {
  let c = vec2<i32>(floor(xz / BOULDER_PITCH));

  for (var dz = -1; dz <= 1; dz = dz + 1) {
    for (var dx = -1; dx <= 1; dx = dx + 1) {
      let b = boulder_candidate(m, c + vec2<i32>(dx, dz), seed);

      if (b.z > 0.0 && length(b.xy - xz) < b.z * FOOTPRINT + margin) {
        return true;
      }
    }
  }

  return false;
}
