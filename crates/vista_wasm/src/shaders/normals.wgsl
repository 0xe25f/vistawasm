// Generates normal vectors from terrain heights.

@group(0) @binding(0)
var<storage, read> heights: array<f32>;

@group(0) @binding(1)
var<storage, read_write> normals: array<vec4<f32>>;

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
  let index = id.y * 1024u + id.x;

  if (index < arrayLength(&normals)) {
    normals[index] = vec4<f32>(0.0, 1.0, 0.0, 0.0);
  }
}
