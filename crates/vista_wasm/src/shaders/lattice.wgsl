// The candidate lattice every tree and grass tuft stands on, shared by
// `tree_generate.wgsl`, `grass_generate.wgsl` and the tree cull pass. The
// CPU mirror, and the tests that hold the two together, are in
// `render/lattice.rs`.

// The 3D PCG hash of Jarzynski and Olano (2020).
fn pcg3d(input: vec3<u32>) -> vec3<u32> {
  var v = input;
  v = v * 1664525u + 1013904223u;
  v.x += v.y * v.z;
  v.y += v.z * v.x;
  v.z += v.x * v.y;
  v ^= v >> vec3<u32>(16u);
  v.x += v.y * v.z;
  v.y += v.z * v.x;
  v.z += v.x * v.y;
  return v;
}

// A hash word as a number in [0, 1), from its top 24 bits: exact in f32.
fn unit(word: u32) -> f32 {
  return f32(word >> 8u) * (1.0 / 16777216.0);
}

// `CLUMP_CELLS` and `CLUMP_SALT` in `render/lattice.rs`.
const CLUMP_CELLS: i32 = 4;
const CLUMP_SALT: u32 = 0x3c6ef372u;

// Floor division and its remainder, for negative lattice indices too.
fn floor_div(a: i32, n: i32) -> i32 {
  return a / n - select(0, 1, a % n < 0);
}

fn fourth_power(value: i32) -> i32 {
  return value * value * value * value;
}

// `clump` in `render/lattice.rs`: 0 to 4.6, the same bits as the CPU.
fn clump(cell: vec2<i32>, seed: u32) -> f32 {
  let n = CLUMP_CELLS;
  let c = vec2<i32>(floor_div(cell.x, n), floor_div(cell.y, n));
  let f = cell - c * n;
  let s = seed ^ CLUMP_SALT;
  let c00 = fourth_power(i32(pcg3d(vec3<u32>(bitcast<vec2<u32>>(c), s)).x >> 28u));
  let c10 = fourth_power(i32(pcg3d(vec3<u32>(bitcast<vec2<u32>>(c + vec2<i32>(1, 0)), s)).x >> 28u));
  let c01 = fourth_power(i32(pcg3d(vec3<u32>(bitcast<vec2<u32>>(c + vec2<i32>(0, 1)), s)).x >> 28u));
  let c11 = fourth_power(i32(pcg3d(vec3<u32>(bitcast<vec2<u32>>(c + vec2<i32>(1, 1)), s)).x >> 28u));
  let sum = c00 * (n - f.x) * (n - f.y) + c10 * f.x * (n - f.y) + c01 * (n - f.x) * f.y + c11 * f.x * f.y;
  return f32(sum) * (47.0 / 8388608.0);
}

// `JITTER` in `render/lattice.rs`.
const JITTER: f32 = 0.45;

// The jittered position of lattice point `cell` with pitch `pitch`.
fn jittered(cell: vec2<i32>, hash: vec3<u32>, pitch: f32) -> vec2<f32> {
  let jitter = (vec2<f32>(unit(hash.x), unit(hash.y)) - 0.5) * (2.0 * JITTER);
  return (vec2<f32>(cell) + 0.5 + jitter) * pitch;
}

// `FADE_BAND` and `MAX_GROWTH` in `render/lattice.rs`.
const FADE_BAND: f32 = 0.05;
const MAX_GROWTH: f32 = 1.8;

// `thinning` in `render/lattice.rs`: x keep, y band, z height scale,
// w extra crown width.
fn thinning(distance: f32, radius: f32, floor: f32) -> vec4<f32> {
  let ratio = radius / max(distance, 0.001);
  let keep = clamp(ratio * ratio, floor, 1.0);
  let band = min(FADE_BAND, keep - floor);
  let end = min(keep + band, 1.0);
  var share = keep;

  if (band > 0.0) {
    let cut = (keep + band - end) / band;
    share = keep + band / 3.0 * (1.0 - cut * cut * cut);
  }

  let growth = sqrt(1.0 / max(share, 0.001));
  let height = min(growth, MAX_GROWTH);
  return vec4<f32>(keep, band, height, growth / height);
}

// `Thinning::fade`: the size of a candidate of rank `u` (relative to p).
fn thinning_fade(t: vec4<f32>, u: f32) -> f32 {
  if (u < t.x) {
    return 1.0;
  }

  if (t.y > 0.0) {
    return clamp((t.x + t.y - u) / t.y, 0.0, 1.0);
  }

  return 0.0;
}
