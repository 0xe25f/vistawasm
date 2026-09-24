// Virtual-pipe hydraulic erosion (Mei, Decaudin and Hu, 2007), one entry
// point per pass. `terrain/erosion.rs` is the CPU reference and runs the
// same passes with the same constants, which arrive in `params` from Rust.
//
// Every pass is a "gather": an invocation writes only its own cell, and
// reads neighbours only from buffers the pass does not write. WebGPU
// orders successive dispatches, so each pass sees the previous one's
// results. All heights are in units of the cell size.

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

@group(0) @binding(2)
var<storage, read_write> water: array<f32>;

@group(0) @binding(3)
var<storage, read_write> sediment: array<f32>;

// Advected sediment, and thermal exchange in the thermal passes.
@group(0) @binding(4)
var<storage, read_write> scratch: array<f32>;

// Outflow towards -x, +x, -y, +y.
@group(0) @binding(5)
var<storage, read_write> flux: array<vec4<f32>>;

// Velocity x, velocity y, sine of the slope, rain weight (-1 is sea).
@group(0) @binding(6)
var<storage, read_write> velocity: array<vec4<f32>>;

fn is_edge(x: u32, y: u32) -> bool {
  let last = params.size - 1u;
  return x == 0u || y == 0u || x == last || y == last;
}

@compute @workgroup_size(8, 8, 1)
fn rain(@builtin(global_invocation_id) id: vec3<u32>) {
  if (id.x >= params.size || id.y >= params.size) {
    return;
  }

  let i = id.y * params.size + id.x;
  let weight = velocity[i].w;

  if (weight >= 0.0) {
    water[i] = water[i] + params.rain * weight;
  }
}

fn pipe(surface: f32, current: f32, j: u32) -> f32 {
  return max(current + params.pipe_gain * (surface - terrain[j] - water[j]), 0.0);
}

@compute @workgroup_size(8, 8, 1)
fn outflow(@builtin(global_invocation_id) id: vec3<u32>) {
  let n = params.size;

  if (id.x >= n || id.y >= n) {
    return;
  }

  let i = id.y * n + id.x;
  let surface = terrain[i] + water[i];
  let current = flux[i];
  var out = vec4<f32>(0.0);

  if (id.x > 0u) {
    out.x = pipe(surface, current.x, i - 1u);
  }

  if (id.x + 1u < n) {
    out.y = pipe(surface, current.y, i + 1u);
  }

  if (id.y > 0u) {
    out.z = pipe(surface, current.z, i - n);
  }

  if (id.y + 1u < n) {
    out.w = pipe(surface, current.w, i + n);
  }

  // Never let more water leave than the cell holds.
  let total = out.x + out.y + out.z + out.w;

  if (total > water[i] && total > 0.0) {
    out = out * (water[i] / total);
  }

  flux[i] = out;
}

@compute @workgroup_size(8, 8, 1)
fn update_water(@builtin(global_invocation_id) id: vec3<u32>) {
  let n = params.size;

  if (id.x >= n || id.y >= n) {
    return;
  }

  let x = id.x;
  let y = id.y;
  let i = y * n + x;
  let out = flux[i];
  // Water arriving from each side is that neighbour's opposite outflow.
  var inflow = vec4<f32>(0.0);

  if (x > 0u) {
    inflow.x = flux[i - 1u].y;
  }

  if (x + 1u < n) {
    inflow.y = flux[i + 1u].x;
  }

  if (y > 0u) {
    inflow.z = flux[i - n].w;
  }

  if (y + 1u < n) {
    inflow.w = flux[i + n].z;
  }

  let weight = velocity[i].w;
  let before = water[i];
  var after = max(before + inflow.x + inflow.y + inflow.z + inflow.w - out.x - out.y - out.z - out.w, 0.0);

  if (weight < 0.0 || is_edge(x, y)) {
    after = 0.0;
  }

  let depth = max((before + after) * 0.5, 1e-4);
  let flow_x = (inflow.x - out.x + out.y - inflow.y) * 0.5;
  let flow_y = (inflow.z - out.z + out.w - inflow.w) * 0.5;
  let left = terrain[y * n + select(x - 1u, x, x == 0u)];
  let right = terrain[y * n + min(x + 1u, n - 1u)];
  let up = terrain[select(y - 1u, y, y == 0u) * n + x];
  let down = terrain[min(y + 1u, n - 1u) * n + x];
  let dx = (right - left) * 0.5;
  let dy = (down - up) * 0.5;
  let tangent_squared = dx * dx + dy * dy;

  water[i] = after;
  velocity[i] = vec4<f32>(
    clamp(flow_x / depth, -1.0, 1.0),
    clamp(flow_y / depth, -1.0, 1.0),
    sqrt(tangent_squared / (1.0 + tangent_squared)),
    weight
  );
}

@compute @workgroup_size(8, 8, 1)
fn erode(@builtin(global_invocation_id) id: vec3<u32>) {
  if (id.x >= params.size || id.y >= params.size) {
    return;
  }

  let i = id.y * params.size + id.x;
  let v = velocity[i];

  // Load reaching the sea or the map edge leaves the model.
  if (v.w < 0.0 || is_edge(id.x, id.y)) {
    sediment[i] = 0.0;
    return;
  }

  let speed = length(v.xy);
  let depth = clamp(water[i] / params.full_depth, 0.0, 1.0);
  let capacity = params.capacity * max(v.z, params.min_tilt) * speed * depth;
  let load = sediment[i];

  if (capacity > load) {
    // Water on level ground drops its load but barely cuts.
    let cutting = clamp((v.z - params.level_sine) / params.level_sine, 0.0, 1.0);
    let amount = params.dissolve_rate * (capacity - load) * cutting;
    terrain[i] = terrain[i] - amount;
    sediment[i] = load + amount;
  } else {
    let amount = params.deposit_rate * (load - capacity);
    terrain[i] = terrain[i] + amount;
    sediment[i] = load - amount;
  }
}

// Forward semi-Lagrangian advection, gathered: each neighbour's load lands
// at its position plus its velocity (at most one cell away) and is split
// bilinearly between the four cells around that point.
@compute @workgroup_size(8, 8, 1)
fn advect(@builtin(global_invocation_id) id: vec3<u32>) {
  let n = i32(params.size);
  let x = i32(id.x);
  let y = i32(id.y);

  if (x >= n || y >= n) {
    return;
  }

  let limit = f32(n - 1);
  var gathered = 0.0;

  for (var oy = -1; oy <= 1; oy = oy + 1) {
    for (var ox = -1; ox <= 1; ox = ox + 1) {
      let sx = x + ox;
      let sy = y + oy;

      if (sx < 0 || sy < 0 || sx >= n || sy >= n) {
        continue;
      }

      let j = u32(sy * n + sx);
      let v = velocity[j];
      let tx = clamp(f32(sx) + v.x, 0.0, limit);
      let ty = clamp(f32(sy) + v.y, 0.0, limit);
      let wx = max(1.0 - abs(tx - f32(x)), 0.0);
      let wy = max(1.0 - abs(ty - f32(y)), 0.0);
      gathered = gathered + sediment[j] * wx * wy;
    }
  }

  scratch[u32(y * n + x)] = gathered;
}

@compute @workgroup_size(8, 8, 1)
fn evaporate(@builtin(global_invocation_id) id: vec3<u32>) {
  if (id.x >= params.size || id.y >= params.size) {
    return;
  }

  let i = id.y * params.size + id.x;
  water[i] = water[i] * (1.0 - params.evaporation);
  sediment[i] = scratch[i];
}

// Remove the water and settle the remaining load where it is.
@compute @workgroup_size(8, 8, 1)
fn settle(@builtin(global_invocation_id) id: vec3<u32>) {
  if (id.x >= params.size || id.y >= params.size) {
    return;
  }

  let i = id.y * params.size + id.x;
  terrain[i] = terrain[i] + sediment[i];
  sediment[i] = 0.0;
  water[i] = 0.0;
  flux[i] = vec4<f32>(0.0);
}
