use vista_types::{BiomeKind, FloraOptions, FloraRule};

use crate::maths::hash_noise;
use crate::render::tree_models::TreeSpecies;
use crate::terrain::biomes::SurfaceSample;
use crate::terrain::heightmap::HeightMap;

/// Clamp flora instances to a device or implementation limit.
pub fn clamp_flora_instances(options: &FloraOptions, device_limit: u32) -> u32 {
  if !options.enabled {
    return 0;
  }

  options.max_instances.min(device_limit)
}

/// One GPU-ready grass tuft instance.
///
/// Layout must stay in sync with the per-instance vertex attributes declared
/// in `grass_instances.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FloraInstance {
  /// World position of the base of the plant, in terrain metres.
  pub position: [f32; 3],
  /// Tuft scale in metres.
  pub scale: f32,
  /// A deterministic 0 to 1 tint variation used to vary colour.
  pub tint: f32,
  /// Climate dryness from 0 (lush green) to 1 (straw).
  pub dryness: f32,
  /// [`GRASS_STYLE_TUFT`] or [`GRASS_STYLE_REED`].
  pub style: f32,
}

/// A grass tuft.
pub const GRASS_STYLE_TUFT: f32 = 0.0;
/// A clump of reeds, 1.4 to 2.2 m tall, beside still or slow water.
pub const GRASS_STYLE_REED: f32 = 1.0;

/// One base-geometry vertex shared by every grass tuft instance.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FloraVertex {
  /// Local offset in billboard space: x across the billboard, y from base
  /// (0) to top (1).
  pub local_offset: [f32; 2],
  /// Texture-free coordinate used by the fragment shader to carve out a
  /// trunk and canopy silhouette.
  pub uv: [f32; 2],
}

/// Candidate grid resolution used when scattering trees. Terrain larger
/// than this is scanned at a coarser stride, keeping placement fast
/// regardless of terrain size.
const MAX_CANDIDATE_SAMPLES_PER_SIDE: u32 = 512;

/// Maximum slope (1 - normal.y equivalent, as a height-difference ratio)
/// trees tolerate.
const MAX_PLANTING_SLOPE: f32 = 0.75;

/// One GPU-ready tree instance (32 bytes).
///
/// Layout must stay in sync with `TreeInstance` in `tree_cull.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TreeInstance {
  /// World position of the base of the trunk, in terrain metres.
  pub position: [f32; 3],
  /// Uniform scale applied to the species model.
  pub scale: f32,
  /// Rotation around the vertical axis in radians.
  pub rotation: f32,
  /// Colour variation from 0 to 1.
  pub tint: f32,
  /// [`TreeSpecies`] index.
  pub species: u32,
  /// Colour dryness from 0 (lush) to 1 (dry), from the local climate.
  pub dryness: f32,
}

/// Pick a species for a tree growing in `biome`, or `None` when this
/// biome should stay treeless at this roll.
pub fn choose_species(biome: BiomeKind, temperature: f32, roll: f32) -> Option<TreeSpecies> {
  use TreeSpecies::*;

  let cold = temperature < 0.38;
  let warm = temperature > 0.55;
  let table: &[(TreeSpecies, f32)] = match biome {
    BiomeKind::GrassyMeadows => &[(Oak, 0.65), (Shrub, 0.35)],
    BiomeKind::OuterThicket if cold => &[(Shrub, 0.5), (Spruce, 0.25), (Pine, 0.25)],
    BiomeKind::OuterThicket => &[(Shrub, 0.55), (Oak, 0.35), (Pine, 0.1)],
    BiomeKind::OuterForest if cold => &[(Pine, 0.5), (Spruce, 0.4), (Shrub, 0.1)],
    BiomeKind::OuterForest => &[(Oak, 0.55), (Pine, 0.3), (Shrub, 0.15)],
    BiomeKind::InnerForest if cold => &[(Spruce, 0.65), (Pine, 0.35)],
    BiomeKind::InnerForest => &[(Oak, 0.6), (Pine, 0.25), (Spruce, 0.15)],
    BiomeKind::MountainFoothills => &[(Pine, 0.5), (Spruce, 0.4), (Shrub, 0.1)],
    BiomeKind::MountainProper => &[(Spruce, 0.8), (Pine, 0.2)],
    BiomeKind::OuterVolcanic => &[(Pine, 0.7), (Shrub, 0.3)],
    BiomeKind::SavannahExpanse => &[(Acacia, 0.8), (Shrub, 0.2)],
    BiomeKind::CoastalBeach if warm => &[(Palm, 1.0)],
    BiomeKind::CoastalBeach => &[(Pine, 0.6), (Shrub, 0.4)],
    BiomeKind::CoastalRocky if warm => &[(Palm, 0.4), (Shrub, 0.6)],
    BiomeKind::CoastalRocky => &[(Pine, 0.6), (Shrub, 0.4)],
    BiomeKind::OuterJungle => &[(Jungle, 0.5), (Palm, 0.3), (Shrub, 0.2)],
    BiomeKind::InnerJungle => &[(Jungle, 0.85), (Palm, 0.15)],
    BiomeKind::SwampWetlands => &[(Cypress, 0.8), (Shrub, 0.2)],
    BiomeKind::AlpineTransition => &[(Shrub, 0.75), (Spruce, 0.25)],
    // Only dwarf shrubs survive on the tundra; nothing grows on the ice.
    BiomeKind::IceArctic => &[(Shrub, 1.0)],
    BiomeKind::CalderaVolcanic
    | BiomeKind::Ocean
    | BiomeKind::LowerSnowyPeaks
    | BiomeKind::UpperSnowyPeaks => &[],
  };

  let mut remaining = roll.clamp(0.0, 0.9999);

  for (species, weight) in table {
    if remaining < *weight {
      return Some(*species);
    }

    remaining -= weight;
  }

  table.last().map(|(species, _)| *species)
}

/// Pick a species using a host-supplied rule instead of the built-in mix.
pub fn choose_species_by_rule(rule: &FloraRule, roll: f32) -> Option<TreeSpecies> {
  let total: f32 = rule
    .species
    .iter()
    .map(|choice| choice.weight.max(0.0))
    .sum();

  if total <= 0.0 {
    return None;
  }

  let mut remaining = roll.clamp(0.0, 0.9999) * total;

  for choice in &rule.species {
    let weight = choice.weight.max(0.0);

    if remaining < weight {
      return Some(TreeSpecies::ALL[choice.species.index()]);
    }

    remaining -= weight;
  }

  None
}

/// Scatter deterministic tree instances according to the biome map.
///
/// Placement is driven entirely by the terrain heightmap, the baked
/// `surface` samples, and `options`, so the same seed and options always
/// scatter the same trees. `density_scale` applies an additional
/// multiplier from the active render quality preset.
pub fn build_tree_instances(
  map: &HeightMap,
  surface: &[SurfaceSample],
  options: &FloraOptions,
  density_scale: f32,
) -> Vec<TreeInstance> {
  let density = (options.density * density_scale).clamp(0.0, 1.0);

  if !options.enabled || density <= 0.0 || options.max_instances == 0 {
    return Vec::new();
  }

  let width = map.metadata.width;
  let height = map.metadata.height;

  if width < 2 || height < 2 || surface.len() != map.heights.len() {
    return Vec::new();
  }

  let stride = (width.max(height).saturating_sub(1)
    / (MAX_CANDIDATE_SAMPLES_PER_SIDE.saturating_sub(1)).max(1))
  .max(1);
  let metres_per_sample = map.metadata.metres_per_sample.max(0.001);
  let half_width = (width as f32 - 1.0) * metres_per_sample * 0.5;
  let half_height = (height as f32 - 1.0) * metres_per_sample * 0.5;
  let seed = options.seed_offset;
  let water_line = map.metadata.sea_level_metres + 0.6;
  let variation = options.species_variation.clamp(0.0, 1.0);
  let cell_span = stride as f32 * metres_per_sample;
  // Dense forests want more than one tree per 12 m cell; allow up to two
  // candidates per cell on fine grids.
  let per_cell = if cell_span > 9.0 { 2 } else { 1 };

  let mut candidates = Vec::new();
  let mut y = 0;

  while y < height {
    let mut x = 0;

    while x < width {
      for slot in 0..per_cell {
        let slot_seed = seed ^ (slot as u64).wrapping_mul(0x632b_e59b_d9b4_e019);
        let jitter_x =
          unit_from_hash(hash_noise(slot_seed ^ 0x9e37_79b9, x as i32, y as i32)) - 0.5;
        let jitter_z =
          unit_from_hash(hash_noise(slot_seed ^ 0x85eb_ca6b, x as i32, y as i32)) - 0.5;
        let sample_x = (x as f32 + jitter_x * stride as f32)
          .clamp(0.0, (width - 1) as f32)
          .round() as u32;
        let sample_y = (y as f32 + jitter_z * stride as f32)
          .clamp(0.0, (height - 1) as f32)
          .round() as u32;

        // Beside water a tree stands at its sample's centre, at least
        // half a sample clear of the channel mask (over 1 m on grids of
        // 2 m and more).
        let river = |dx: i32, dy: i32| {
          let nx = (sample_x as i32 + dx).clamp(0, width as i32 - 1) as u32;
          let ny = (sample_y as i32 + dy).clamp(0, height as i32 - 1) as u32;
          surface[(ny * width + nx) as usize].river > 0
        };
        let beside = river(-1, 0) || river(1, 0) || river(0, -1) || river(0, 1);

        if let Some(instance) = candidate_at(
          map,
          surface,
          sample_x,
          sample_y,
          metres_per_sample,
          water_line,
          options,
          density,
          variation,
          slot_seed,
        ) {
          let position = if beside {
            [
              sample_x as f32 * metres_per_sample - half_width,
              sample_y as f32 * metres_per_sample - half_height,
            ]
          } else {
            [
              x as f32 * metres_per_sample + jitter_x * cell_span - half_width,
              y as f32 * metres_per_sample + jitter_z * cell_span - half_height,
            ]
          };
          candidates.push(TreeInstance {
            position: [position[0], instance.position[1], position[1]],
            ..instance
          });
        }
      }

      x += stride;
    }

    y += stride;
  }

  let max_instances = options.max_instances as usize;

  if candidates.len() <= max_instances {
    return candidates;
  }

  let keep_every = (candidates.len() as f32 / max_instances as f32).ceil() as usize;

  candidates
    .into_iter()
    .step_by(keep_every.max(1))
    .take(max_instances)
    .collect()
}

#[allow(clippy::too_many_arguments)]
fn candidate_at(
  map: &HeightMap,
  surface: &[SurfaceSample],
  x: u32,
  y: u32,
  metres_per_sample: f32,
  water_line: f32,
  options: &FloraOptions,
  density: f32,
  variation: f32,
  seed: u64,
) -> Option<TreeInstance> {
  let width = map.metadata.width;
  let height = map.metadata.height;
  let index = (y * width + x) as usize;

  if map.no_data[index] {
    return None;
  }

  let elevation = map.heights[index];
  let sample = surface[index];

  if elevation <= water_line || elevation > options.tree_line_metres || sample.river > 0 {
    return None;
  }

  let left = map.height_at(x.saturating_sub(1), y).unwrap_or(elevation);
  let right = map
    .height_at((x + 1).min(width - 1), y)
    .unwrap_or(elevation);
  let down = map.height_at(x, y.saturating_sub(1)).unwrap_or(elevation);
  let up = map
    .height_at(x, (y + 1).min(height - 1))
    .unwrap_or(elevation);
  let slope = ((left - right).abs() + (down - up).abs()) / (4.0 * metres_per_sample);

  if slope > MAX_PLANTING_SLOPE {
    return None;
  }

  // Thin trees out towards the tree line so forests fade rather than stop.
  let tree_line_fade = ((options.tree_line_metres - elevation) / 150.0).clamp(0.0, 1.0);
  let cover = sample.forest as f32 / 255.0 * tree_line_fade;
  let placement_roll = unit_from_hash(hash_noise(seed, x as i32, y as i32));

  let biome = sample.biome_kind();
  let rule = options
    .species_rules
    .iter()
    .find(|rule| rule.biome == biome);
  let rule_density = rule.map_or(1.0, |rule| rule.density.clamp(0.0, 4.0));

  if placement_roll > density * cover * 1.6 * rule_density {
    return None;
  }

  let species_roll = unit_from_hash(hash_noise(seed ^ 0x7f4a_7c15, x as i32, y as i32));
  let species = match rule {
    Some(rule) => choose_species_by_rule(rule, species_roll)?,
    None => choose_species(biome, sample.temperature_unit(), species_roll)?,
  };
  let scale_roll = unit_from_hash(hash_noise(seed ^ 0x1234_5678, x as i32, y as i32));
  let tint_roll = unit_from_hash(hash_noise(seed ^ 0x4321_dcba, x as i32, y as i32));
  let rotation_roll = unit_from_hash(hash_noise(seed ^ 0x2468_ace0, x as i32, y as i32));
  // Trees at the forest edge and on poor ground grow smaller.
  let vigour = 0.8 + cover.min(1.0) * 0.2;
  let mut tree = TreeInstance {
    position: [
      x as f32 * metres_per_sample,
      elevation,
      y as f32 * metres_per_sample,
    ],
    scale: (1.0 + (scale_roll - 0.5) * 0.55 * variation.max(0.15)) * vigour,
    rotation: rotation_roll * std::f32::consts::TAU,
    tint: 0.5 + (tint_roll - 0.5) * variation.max(0.1),
    species: species as u32,
    dryness: ((sample.temperature_unit() - 0.45) * 1.5 + (0.5 - sample.moisture_unit()) * 1.5)
      .clamp(0.0, 1.0),
  };

  // Tundra shrubs are dwarf willow and birch: knee to waist high, and
  // brown rather than green for most of the year.
  if sample.is_tundra() && rule.is_none() {
    tree.scale = 0.35 + scale_roll * 0.25;
    tree.tint = 0.15 + tint_roll * 0.2;
    tree.dryness = 0.75;
  }

  Some(tree)
}

pub(crate) fn unit_from_hash(value: f32) -> f32 {
  (value + 1.0) * 0.5
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::terrain::biomes::classify_surface;
  use crate::terrain::normals::generate_normals;
  use vista_types::{BiomeOptions, TerrainMetadata};

  fn flat_map(size: u32, elevation: f32) -> HeightMap {
    let metadata = TerrainMetadata {
      width: size,
      height: size,
      metres_per_sample: 4.0,
      sea_level_metres: 0.0,
      max_height_metres: 600.0,
      ..TerrainMetadata::default()
    };

    HeightMap::flat(size, size, elevation, metadata)
  }

  fn forest_surface(map: &HeightMap) -> Vec<SurfaceSample> {
    let options = BiomeOptions {
      moisture_bias: 1.0,
      temperature_bias: -0.3,
      volcanism: 0.0,
      ..BiomeOptions::default()
    };
    classify_surface(map, &generate_normals(map), None, &[], &options)
  }

  fn flora_options() -> FloraOptions {
    FloraOptions {
      enabled: true,
      density: 1.0,
      tree_line_metres: 1_800.0,
      seed_offset: 42,
      max_instances: 10_000,
      ..FloraOptions::default()
    }
  }

  #[test]
  fn species_rules_replace_the_built_in_mix() {
    let map = flat_map(48, 50.0);
    let surface = forest_surface(&map);
    let biome = surface[48 * 24 + 24].biome_kind();
    let mut options = flora_options();
    options.species_rules = vec![FloraRule {
      biome,
      species: vec![vista_types::SpeciesWeight {
        species: vista_types::TreeSpeciesKind::Palm,
        weight: 2.0,
      }],
      density: 1.0,
    }];
    let trees = build_tree_instances(&map, &surface, &options, 1.0);

    assert!(!trees.is_empty());
    assert!(trees
      .iter()
      .all(|tree| tree.species == TreeSpecies::Palm as u32));

    options.species_rules[0].species.clear();
    assert!(build_tree_instances(&map, &surface, &options, 1.0).is_empty());
  }

  #[test]
  fn disabled_flora_produces_no_instances() {
    let map = flat_map(32, 50.0);
    let mut options = flora_options();
    options.enabled = false;

    assert!(build_tree_instances(&map, &forest_surface(&map), &options, 1.0).is_empty());
  }

  #[test]
  fn underwater_terrain_produces_no_instances() {
    let map = flat_map(32, -10.0);
    let options = flora_options();

    assert!(build_tree_instances(&map, &forest_surface(&map), &options, 1.0).is_empty());
  }

  #[test]
  fn forest_above_sea_level_produces_instances() {
    let map = flat_map(32, 50.0);
    let options = flora_options();

    assert!(!build_tree_instances(&map, &forest_surface(&map), &options, 1.0).is_empty());
  }

  #[test]
  fn max_instances_is_respected() {
    let map = flat_map(64, 50.0);
    let mut options = flora_options();
    options.max_instances = 5;

    assert!(build_tree_instances(&map, &forest_surface(&map), &options, 1.0).len() <= 5);
  }

  #[test]
  fn same_seed_is_deterministic() {
    let map = flat_map(32, 50.0);
    let options = flora_options();
    let surface = forest_surface(&map);

    let first = build_tree_instances(&map, &surface, &options, 1.0);
    let second = build_tree_instances(&map, &surface, &options, 1.0);

    assert_eq!(first, second);
  }

  fn cold_surface(map: &HeightMap, celsius: f32) -> Vec<SurfaceSample> {
    let options = BiomeOptions {
      mean_temperature_celsius: Some(celsius),
      volcanism: 0.0,
      ..BiomeOptions::default()
    };
    classify_surface(map, &generate_normals(map), None, &[], &options)
  }

  #[test]
  fn glaciers_are_treeless_and_tundra_grows_only_dwarf_shrubs() {
    let map = flat_map(96, 50.0);
    let options = flora_options();
    let glacier = cold_surface(&map, -20.0);

    assert!(glacier.iter().all(|sample| sample.is_glacier()));
    assert!(build_tree_instances(&map, &glacier, &options, 1.0).is_empty());

    let tundra = cold_surface(&map, 1.0);
    assert!(tundra.iter().all(|sample| sample.is_tundra()));
    let shrubs = build_tree_instances(&map, &tundra, &options, 1.0);

    // A tenth of full forest cover: one candidate per sample on this grid,
    // so well under a fifth of them become shrubs.
    assert!(!shrubs.is_empty());
    assert!(shrubs.len() * 5 < 96 * 96, "{} shrubs", shrubs.len());
    assert!(shrubs.iter().all(|tree| {
      tree.species == TreeSpecies::Shrub as u32 && (0.35..=0.6).contains(&tree.scale)
    }));
  }

  #[test]
  fn species_follow_the_biome() {
    assert_eq!(
      choose_species(BiomeKind::SwampWetlands, 0.6, 0.1),
      Some(TreeSpecies::Cypress)
    );
    assert_eq!(
      choose_species(BiomeKind::InnerJungle, 0.8, 0.1),
      Some(TreeSpecies::Jungle)
    );
    assert_eq!(
      choose_species(BiomeKind::CoastalBeach, 0.8, 0.5),
      Some(TreeSpecies::Palm)
    );
    assert_eq!(
      choose_species(BiomeKind::InnerForest, 0.2, 0.1),
      Some(TreeSpecies::Spruce)
    );
    assert_eq!(
      choose_species(BiomeKind::SavannahExpanse, 0.8, 0.1),
      Some(TreeSpecies::Acacia)
    );
    assert_eq!(choose_species(BiomeKind::Ocean, 0.5, 0.1), None);
  }
}
