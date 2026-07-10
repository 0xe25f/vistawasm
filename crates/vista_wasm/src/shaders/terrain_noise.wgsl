// Generates seeded fractal terrain heights in metres.

@group(0) @binding(0)
var<storage, read_write> heights: array<f32>;

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
  let index = id.y * 1024u + id.x;

  if (index < arrayLength(&heights)) {
    let x = f32(id.x) * 0.03125;
    let y = f32(id.y) * 0.03125;
    heights[index] = sin(x) * cos(y) * 120.0;
  }
}
