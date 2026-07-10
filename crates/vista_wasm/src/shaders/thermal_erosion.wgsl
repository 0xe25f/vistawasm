// Softens slopes that exceed a talus angle, moving material from a cell
// to any direct neighbour that is more than `talus_threshold` metres lower.
//
// Like `hydraulic_erosion.wgsl`, this is a "gather" formulation: a cell's
// thermal exchange with a neighbour depends only on their two heights, so
// every invocation can compute its own output cell independently and in
// parallel with no cross-cell writes.

struct ThermalParams {
  width: u32,
  height: u32,
  talus_threshold: f32,
  transfer_rate: f32,
};

@group(0) @binding(0)
var<uniform> params: ThermalParams;

@group(0) @binding(1)
var<storage, read> heights_in: array<f32>;

@group(0) @binding(2)
var<storage, read_write> heights_out: array<f32>;

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
    delta = delta + edge_delta(current, heights_in[index - 1u], params.talus_threshold, params.transfer_rate);
  }

  if (x + 1u < params.width) {
    delta = delta + edge_delta(current, heights_in[index + 1u], params.talus_threshold, params.transfer_rate);
  }

  if (y > 0u) {
    delta = delta
      + edge_delta(current, heights_in[index - params.width], params.talus_threshold, params.transfer_rate);
  }

  if (y + 1u < params.height) {
    delta = delta
      + edge_delta(current, heights_in[index + params.width], params.talus_threshold, params.transfer_rate);
  }

  heights_out[index] = current + delta;
}

// Return this cell's height change from a single neighbour: material moves
// downhill once the slope to that neighbour exceeds the talus threshold.
fn edge_delta(current: f32, neighbour: f32, talus_threshold: f32, transfer_rate: f32) -> f32 {
  let difference = current - neighbour;
  var delta: f32 = 0.0;

  if (difference > talus_threshold) {
    delta = delta - (difference - talus_threshold) * transfer_rate;
  }

  if (-difference > talus_threshold) {
    delta = delta + (-difference - talus_threshold) * transfer_rate;
  }

  return delta;
}
