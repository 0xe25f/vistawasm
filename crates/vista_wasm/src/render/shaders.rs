//! Minified WGSL shader sources (see `build.rs`).
//!
//! `COMMON` is embedded once and prepended to each render shader at
//! runtime with [`render_source`], rather than duplicated into every
//! shader string in the binary. `GROUND` goes in front of both it and the
//! pass that grounds trees and grass ([`grounding_source`]), so the ground
//! height rules exist once.

macro_rules! shader {
  ($file:literal) => {
    include_str!(concat!(env!("OUT_DIR"), "/", $file))
  };
}

/// Ground heights, shared by the render shaders and the tree cull pass.
pub const GROUND: &str = shader!("ground.wgsl");
/// Shared prelude for every render shader.
pub const COMMON: &str = shader!("common.wgsl");
/// Terrain.
pub const TERRAIN: &str = shader!("clipmap_render.wgsl");
/// Material sampling and bare rock shading, shared by the terrain and
/// the boulders.
pub const MATERIALS: &str = shader!("materials.wgsl");
/// Boulders and talus.
pub const BOULDERS: &str = shader!("boulders.wgsl");
/// Tree meshes, impostors, tree shadows, and impostor baking.
pub const TREES: &str = shader!("trees.wgsl");
/// Grass tufts.
pub const GRASS: &str = shader!("grass_instances.wgsl");
/// Sky, clouds, fog, weather, and tone mapping.
pub const ATMOSPHERE: &str = shader!("atmosphere.wgsl");
/// Ocean, rivers, and lakes.
pub const WATER: &str = shader!("water.wgsl");
/// GPU tree culling (compute, standalone).
pub const TREE_CULL: &str = shader!("tree_cull.wgsl");
/// Standing trees and grass on the drawn terrain (compute, with `GROUND`).
pub const GROUNDING: &str = shader!("grounding.wgsl");
/// The candidate lattice: hashing, clumps and thinning.
pub const LATTICE: &str = shader!("lattice.wgsl");
/// Bindings and helpers shared by the two generators.
pub const GENERATE_COMMON: &str = shader!("generate_common.wgsl");
/// Filling streamed tree tiles (compute, with `GROUND`, `LATTICE` and
/// `GENERATE_COMMON`).
pub const TREE_GENERATE: &str = shader!("tree_generate.wgsl");
/// Filling and culling streamed grass tiles (compute, likewise).
pub const GRASS_GENERATE: &str = shader!("grass_generate.wgsl");
/// Filling and culling streamed boulder tiles (compute, likewise).
pub const BOULDER_GENERATE: &str = shader!("boulder_generate.wgsl");
/// Terrain sun-shadow baking (compute, standalone).
pub const TERRAIN_SHADOW: &str = shader!("terrain_shadow.wgsl");
/// Wet ground, puddles and snow depth (compute, standalone).
pub const SURFACE_WEATHER: &str = shader!("surface_weather.wgsl");
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
  ("boulders.wgsl", BOULDERS),
  ("trees.wgsl", TREES),
  ("grass_instances.wgsl", GRASS),
  ("atmosphere.wgsl", ATMOSPHERE),
  ("water.wgsl", WATER),
];

/// Compute shaders: standalone, except those [`compute_source`]
/// completes.
pub const COMPUTE_SHADERS: [(&str, &str); 11] = [
  ("tree_cull.wgsl", TREE_CULL),
  ("tree_generate.wgsl", TREE_GENERATE),
  ("grass_generate.wgsl", GRASS_GENERATE),
  ("boulder_generate.wgsl", BOULDER_GENERATE),
  ("grounding.wgsl", GROUNDING),
  ("terrain_shadow.wgsl", TERRAIN_SHADOW),
  ("surface_weather.wgsl", SURFACE_WEATHER),
  ("texture_gen.wgsl", TEXTURE_GEN),
  ("mipgen.wgsl", MIPGEN),
  ("hydraulic_erosion.wgsl", HYDRAULIC_EROSION),
  ("thermal_erosion.wgsl", THERMAL_EROSION),
];

/// `GROUND` followed by `parts`, each on its own lines.
fn prepend_ground(parts: &[&str]) -> String {
  let mut source =
    String::with_capacity(GROUND.len() + parts.iter().map(|p| p.len() + 1).sum::<usize>());
  source.push_str(GROUND);

  for part in parts {
    source.push('\n');
    source.push_str(part);
  }

  source
}

/// A render shader's full source: the ground rules and the common
/// prelude, then the ground materials for the terrain and the boulders,
/// followed by `body`.
pub fn render_source(body: &str) -> String {
  if body == TERRAIN || body == BOULDERS {
    prepend_ground(&[COMMON, MATERIALS, body])
  } else if body == WATER {
    // The water breaks its foam on the stream stones' lattice.
    prepend_ground(&[COMMON, LATTICE, body])
  } else {
    prepend_ground(&[COMMON, body])
  }
}

/// The grounding pass's full source: the ground rules, then the pass.
pub fn grounding_source() -> String {
  prepend_ground(&[GROUNDING])
}

/// A compute shader's full source: the cull pass after the lattice, a
/// generator after the ground rules, the lattice and the generators'
/// common part, and the grounding pass after the ground rules.
pub fn compute_source(body: &'static str) -> std::borrow::Cow<'static, str> {
  if body == TREE_CULL {
    format!("{LATTICE}\n{TREE_CULL}").into()
  } else if body == TREE_GENERATE || body == GRASS_GENERATE || body == BOULDER_GENERATE {
    prepend_ground(&[LATTICE, GENERATE_COMMON, body]).into()
  } else if body == GROUNDING {
    grounding_source().into()
  } else {
    body.into()
  }
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
      validate(name, &compute_source(source));
    }
  }

  /// The name the build gave a shader's `name` (see `build.rs`).
  fn minified_name(name: &str) -> &str {
    const NAMES: &[(&str, &str)] = include!(concat!(env!("OUT_DIR"), "/shader_names.rs"));
    NAMES
      .iter()
      .find(|(from, _)| *from == name)
      .map_or(name, |(_, to)| to)
  }

  #[test]
  fn the_ground_layers_are_bound_where_every_render_shader_expects_them() {
    let ground_layers = minified_name("ground_layers");

    for (name, body) in RENDER_SHADERS {
      let source = render_source(body);
      let module = naga::front::wgsl::parse_str(&source).unwrap();
      let binding = module
        .global_variables
        .iter()
        .find(|(_, variable)| variable.name.as_deref() == Some(ground_layers))
        .and_then(|(_, variable)| variable.binding)
        .unwrap_or_else(|| panic!("{name} has no ground_layers"));

      assert_eq!((binding.group, binding.binding), (1, 12), "{name}");
    }
  }

  /// The entry points `raw` declares: the function after each `@vertex`,
  /// `@fragment` or `@compute` attribute.
  fn raw_entry_points(raw: &str) -> Vec<String> {
    raw
      .split("fn ")
      .zip(raw.split("fn ").skip(1))
      .filter(|(before, _)| {
        let attributes = before.rsplit(['}', ';']).next().unwrap_or("");
        ["@vertex", "@fragment", "@compute"]
          .iter()
          .any(|stage| attributes.contains(stage))
      })
      .map(|(_, after)| after.split('(').next().unwrap_or("").trim().to_string())
      .collect()
  }

  #[test]
  fn renaming_keeps_every_entry_point_name() {
    let raw = [
      (
        "clipmap_render.wgsl",
        include_str!("../shaders/clipmap_render.wgsl"),
      ),
      ("boulders.wgsl", include_str!("../shaders/boulders.wgsl")),
      ("trees.wgsl", include_str!("../shaders/trees.wgsl")),
      (
        "grass_instances.wgsl",
        include_str!("../shaders/grass_instances.wgsl"),
      ),
      (
        "atmosphere.wgsl",
        include_str!("../shaders/atmosphere.wgsl"),
      ),
      ("water.wgsl", include_str!("../shaders/water.wgsl")),
      ("tree_cull.wgsl", include_str!("../shaders/tree_cull.wgsl")),
      (
        "tree_generate.wgsl",
        include_str!("../shaders/tree_generate.wgsl"),
      ),
      (
        "grass_generate.wgsl",
        include_str!("../shaders/grass_generate.wgsl"),
      ),
      (
        "boulder_generate.wgsl",
        include_str!("../shaders/boulder_generate.wgsl"),
      ),
      ("grounding.wgsl", include_str!("../shaders/grounding.wgsl")),
      (
        "terrain_shadow.wgsl",
        include_str!("../shaders/terrain_shadow.wgsl"),
      ),
      (
        "surface_weather.wgsl",
        include_str!("../shaders/surface_weather.wgsl"),
      ),
      (
        "texture_gen.wgsl",
        include_str!("../shaders/texture_gen.wgsl"),
      ),
      ("mipgen.wgsl", include_str!("../shaders/mipgen.wgsl")),
      (
        "hydraulic_erosion.wgsl",
        include_str!("../shaders/hydraulic_erosion.wgsl"),
      ),
      (
        "thermal_erosion.wgsl",
        include_str!("../shaders/thermal_erosion.wgsl"),
      ),
    ];
    let minified = RENDER_SHADERS
      .iter()
      .map(|(name, body)| (*name, render_source(body)))
      .chain(
        COMPUTE_SHADERS
          .iter()
          .map(|(name, body)| (*name, compute_source(body).into_owned())),
      );

    for ((name, source), (raw_name, raw)) in minified.zip(raw) {
      assert_eq!(name, raw_name);
      let module = naga::front::wgsl::parse_str(&source).unwrap();
      let names: Vec<&str> = module
        .entry_points
        .iter()
        .map(|e| e.name.as_str())
        .collect();
      let expected = raw_entry_points(raw);
      assert!(!expected.is_empty(), "{name}");

      for entry in expected {
        assert!(
          names.contains(&entry.as_str()),
          "{name}: {entry} is not in {names:?}"
        );
      }
    }
  }

  #[test]
  fn renaming_shortens_declared_names() {
    // A member of the water's vertex output, which the engine never asks
    // for by name.
    assert!(include_str!("../shaders/water.wgsl").contains("world_position"));
    assert!(!WATER.contains("world_position"));
    assert_ne!(minified_name("world_position"), "world_position");
  }

  #[test]
  fn minification_removes_comments() {
    assert!(!COMMON.contains("//"));
    assert!(COMMON.len() < include_str!("../shaders/common.wgsl").len());
  }
}
