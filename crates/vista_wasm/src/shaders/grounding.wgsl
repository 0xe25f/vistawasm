// Stands trees, grass and boulders on the ground as the terrain mesh draws it
// (`ground.wgsl`, which is prepended), one invocation per tree or tuft,
// writing each one's height into its instance. It runs when the drawn
// mesh recentres or the heights change, not every frame, so the cull
// pass and the grass pass pay nothing for it.

struct GroundParams {
  // The terrain mapping and the mesh being drawn (see `ground.wgsl`).
  terrain: vec4<f32>,
  terrain2: vec4<f32>,
  mesh: vec4<f32>,
  // x: instance count, y: floats per instance, z: 1 for trees (which
  // carry a species and flags), 2 for boulders (sunk by their fifth
  // float), 0 for grass.
  shape: vec4<u32>,
  // Root radius per tree species, four per vector.
  roots: array<vec4<f32>, 2>,
};

@group(0) @binding(0) var<uniform> params: GroundParams;
// Instances as raw floats, starting x, y, z. Trees are `TreeInstance`
// and grass `FloraInstance` (`render/flora.rs`), boulders `Boulder`
// (`render/boulders.rs`).
@group(0) @binding(1) var<storage, read_write> items: array<f32>;
@group(0) @binding(2) var heights: texture_2d<f32>;

// `TREE_GROUNDED` in `render/flora.rs`.
const GROUNDED: u32 = 256u;

@compute @workgroup_size(64)
fn ground_main(@builtin(global_invocation_id) id: vec3<u32>) {
  if (id.x >= params.shape.x) {
    return;
  }

  let base = id.x * params.shape.y;
  var root = 0.0;

  // Trees keep their own height unless they ask to be grounded; they
  // stand on the lowest ground under their roots.
  if (params.shape.z == 1u) {
    let word = bitcast<u32>(items[base + 6u]);

    if ((word & GROUNDED) == 0u) {
      return;
    }

    let species = min(word & 255u, 7u);
    root = params.roots[species / 4u][species % 4u] * items[base + 3u];
  }

  let xz = vec2<f32>(items[base], items[base + 2u]);
  let sink = select(0.0, items[base + 4u], params.shape.z == 2u);
  items[base + 1u] = grounded_base(heights, params.terrain, params.terrain2, params.mesh, xz, root) - sink;
}
