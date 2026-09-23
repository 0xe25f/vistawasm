/// Names of shader modules compiled by the renderer.
///
/// Render shaders are compiled with `common.wgsl` prepended; compute
/// shaders are standalone.
pub const SHADER_MODULES: &[&str] = &[
  "common.wgsl",
  "terrain_noise.wgsl",
  "hydraulic_erosion.wgsl",
  "thermal_erosion.wgsl",
  "normals.wgsl",
  "material_masks.wgsl",
  "texture_gen.wgsl",
  "mipgen.wgsl",
  "clipmap_render.wgsl",
  "trees.wgsl",
  "tree_cull.wgsl",
  "grass_instances.wgsl",
  "water.wgsl",
  "atmosphere.wgsl",
];
