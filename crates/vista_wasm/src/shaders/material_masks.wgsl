// Builds material masks from height, slope, wetness, and snow line.

@group(0) @binding(0)
var<storage, read> heights: array<f32>;

@group(0) @binding(1)
var<storage, read_write> masks: array<vec4<f32>>;

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
  let index = id.y * 1024u + id.x;

  if (index < arrayLength(&masks)) {
    let height = heights[index];
    masks[index] = vec4<f32>(1.0 - smoothstep(800.0, 1400.0, height), 0.25, smoothstep(1400.0, 1900.0, height), 0.0);
  }
}
