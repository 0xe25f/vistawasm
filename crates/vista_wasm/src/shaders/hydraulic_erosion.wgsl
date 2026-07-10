// Applies one budgeted hydraulic erosion step to terrain heights.
//
// Each invocation reads its own height and its four direct neighbours from
// `heights_in` and writes the eroded result for its own cell to
// `heights_out`. This "gather" formulation (every cell only ever writes its
// own output cell) is deliberately race-free on the GPU: no cell ever
// writes to a neighbour's cell, so many invocations can run in parallel
// with no synchronisation between them.
//
// Material flows towards lower neighbours (erosion) and arrives from higher
// neighbours (deposition) every step. Deposition happens at a reduced rate
// so that some sediment is treated as carried away by runoff, matching the
// CPU reference erosion in `terrain/erosion.rs`.

struct HydraulicParams {
  width: u32,
  height: u32,
  transfer_rate: f32,
  _padding: u32,
};

@group(0) @binding(0)
var<uniform> params: HydraulicParams;

@group(0) @binding(1)
var<storage, read> heights_in: array<f32>;

@group(0) @binding(2)
var<storage, read_write> heights_out: array<f32>;

// Deposited material is scaled down relative to eroded material, matching
// the CPU reference's 0.55 deposit fraction (the remainder represents
// sediment carried away by runoff).
const DEPOSIT_FRACTION: f32 = 0.55;

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
  let x = id.x;
  let y = id.y;

  if (x >= params.width || y >= params.height) {
    return;
  }

  let index = y * params.width + x;
  let current = heights_in[index];
  var delta: f32 = 0.0;

  if (x > 0u) {
    delta = delta + edge_delta(current, heights_in[index - 1u], params.transfer_rate);
  }

  if (x + 1u < params.width) {
    delta = delta + edge_delta(current, heights_in[index + 1u], params.transfer_rate);
  }

  if (y > 0u) {
    delta = delta + edge_delta(current, heights_in[index - params.width], params.transfer_rate);
  }

  if (y + 1u < params.height) {
    delta = delta + edge_delta(current, heights_in[index + params.width], params.transfer_rate);
  }

  heights_out[index] = current + delta;
}

// Return this cell's height change from a single neighbour: full-rate
// erosion downhill, reduced-rate deposition uphill.
fn edge_delta(current: f32, neighbour: f32, transfer_rate: f32) -> f32 {
  let difference = current - neighbour;
  let eroded = max(difference, 0.0) * transfer_rate;
  let deposited = max(-difference, 0.0) * transfer_rate * DEPOSIT_FRACTION;
  return deposited - eroded;
}

