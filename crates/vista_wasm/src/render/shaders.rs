//! Minified WGSL shader sources (see `build.rs`).
//!
//! `COMMON` is embedded once and prepended to each render shader at
//! runtime with [`render_source`], rather than duplicated into every
//! shader string in the binary.

macro_rules! shader {
  ($file:literal) => {
    include_str!(concat!(env!("OUT_DIR"), "/", $file))
  };
}

/// Shared prelude for every render shader.
pub const COMMON: &str = shader!("common.wgsl");
/// Terrain.
pub const TERRAIN: &str = shader!("clipmap_render.wgsl");
/// Tree meshes, impostors, and impostor baking.
pub const TREES: &str = shader!("trees.wgsl");
/// Grass tufts.
pub const GRASS: &str = shader!("grass_instances.wgsl");
/// Sky, clouds, fog, weather, and tone mapping.
pub const ATMOSPHERE: &str = shader!("atmosphere.wgsl");
/// Ocean, rivers, and lakes.
pub const WATER: &str = shader!("water.wgsl");
/// Tree shadow map depth pass.
pub const SHADOW: &str = shader!("shadow.wgsl");
/// GPU tree culling (compute, standalone).
pub const TREE_CULL: &str = shader!("tree_cull.wgsl");
/// Terrain sun-shadow baking (compute, standalone).
pub const TERRAIN_SHADOW: &str = shader!("terrain_shadow.wgsl");
/// Procedural texture generation (compute, standalone).
pub const TEXTURE_GEN: &str = shader!("texture_gen.wgsl");
/// Mip generation (compute, standalone).
pub const MIPGEN: &str = shader!("mipgen.wgsl");
/// Hydraulic erosion (compute, standalone).
pub const HYDRAULIC_EROSION: &str = shader!("hydraulic_erosion.wgsl");
/// Thermal erosion (compute, standalone).
pub const THERMAL_EROSION: &str = shader!("thermal_erosion.wgsl");

/// Render shaders that are compiled with [`COMMON`] prepended.
pub const RENDER_SHADERS: [(&str, &str); 6] = [
  ("clipmap_render.wgsl", TERRAIN),
  ("trees.wgsl", TREES),
  ("grass_instances.wgsl", GRASS),
  ("atmosphere.wgsl", ATMOSPHERE),
  ("water.wgsl", WATER),
  ("shadow.wgsl", SHADOW),
];

/// Standalone compute shaders.
pub const COMPUTE_SHADERS: [(&str, &str); 6] = [
  ("tree_cull.wgsl", TREE_CULL),
  ("terrain_shadow.wgsl", TERRAIN_SHADOW),
  ("texture_gen.wgsl", TEXTURE_GEN),
  ("mipgen.wgsl", MIPGEN),
  ("hydraulic_erosion.wgsl", HYDRAULIC_EROSION),
  ("thermal_erosion.wgsl", THERMAL_EROSION),
];

/// A render shader's full source: the common prelude followed by `body`.
pub fn render_source(body: &str) -> String {
  let mut source = String::with_capacity(COMMON.len() + body.len() + 1);
  source.push_str(COMMON);
  source.push('\n');
  source.push_str(body);
  source
}

#[cfg(test)]
mod tests {
  use super::*;

  fn validate(name: &str, source: &str) {
    let module = naga::front::wgsl::parse_str(source)
      .unwrap_or_else(|error| panic!("{name} failed to parse: {}", error.emit_to_string(source)));
    naga::valid::Validator::new(
      naga::valid::ValidationFlags::all(),
      naga::valid::Capabilities::default(),
    )
    .validate(&module)
    .unwrap_or_else(|error| panic!("{name} failed validation: {error:?}"));
  }

  #[test]
  fn every_render_shader_validates_with_the_common_prelude() {
    for (name, body) in RENDER_SHADERS {
      validate(name, &render_source(body));
    }
  }

  #[test]
  fn every_compute_shader_validates() {
    for (name, source) in COMPUTE_SHADERS {
      validate(name, source);
    }
  }

  #[test]
  fn minification_removes_comments() {
    assert!(!COMMON.contains("//"));
    assert!(COMMON.len() < include_str!("../shaders/common.wgsl").len());
  }
}
