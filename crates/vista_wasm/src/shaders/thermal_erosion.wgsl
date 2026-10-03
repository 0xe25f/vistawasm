// Talus-angle thermal erosion in two passes. `exchange` sums, for every
// cell, the material it gains from or loses to each of its eight
// neighbours in proportion to how far the slope between them exceeds the
// talus angle, plus a slow linear soil creep; `apply` adds it. The
// exchange between two cells is computed identically from either side,
// so material is conserved, and each pass writes only its own cell.
// `terrain/erosion.rs` is the CPU reference.
//
// Bindings match `hydraulic_erosion.wgsl`, so both use one layout.

struct ErosionParams {
  size: u32,
  pipe_gain: f32,
  min_tilt: f32,
  level_sine: f32,
  dissolve_rate: f32,
  deposit_rate: f32,
  thermal_rate: f32,
  rain: f32,
  evaporation: f32,
  capacity: f32,
  full_depth: f32,
  talus: f32,
  creep_rate: f32,
  _pad0: f32,
  _pad1: f32,
  _pad2: f32,
};

@group(0) @binding(0)
var<uniform> params: ErosionParams;

@group(0) @binding(1)
var<storage, read_write> terrain: array<f32>;

@group(0) @binding(4)
var<storage, read_write> scratch: array<f32>;

@compute @workgroup_size(8, 8, 1)
fn exchange(@builtin(global_invocation_id) id: vec3<u32>) {
  let n = i32(params.size);
  let x = i32(id.x);
  let y = i32(id.y);

  if (x >= n || y >= n) {
    return;
  }

  let here = terrain[u32(y * n + x)];
  var change = 0.0;

  for (var oy = -1; oy <= 1; oy = oy + 1) {
    for (var ox = -1; ox <= 1; ox = ox + 1) {
      let nx = x + ox;
      let ny = y + oy;

      if ((ox == 0 && oy == 0) || nx < 0 || ny < 0 || nx >= n || ny >= n) {
        continue;
      }

      var critical = params.talus;

      if (ox != 0 && oy != 0) {
        critical = critical * 1.4142135;
      }

      let difference = terrain[u32(ny * n + nx)] - here;
      change = change + params.thermal_rate * (max(difference - critical, 0.0) - max(-difference - critical, 0.0));

      // Soil creep between edge neighbours.
      if (ox == 0 || oy == 0) {
        change = change + params.creep_rate * difference;
      }
    }
  }

  scratch[u32(y * n + x)] = change;
}

@compute @workgroup_size(8, 8, 1)
fn apply(@builtin(global_invocation_id) id: vec3<u32>) {
  if (id.x >= params.size || id.y >= params.size) {
    return;
  }

  let i = id.y * params.size + id.x;
  terrain[i] = terrain[i] + scratch[i];
}
