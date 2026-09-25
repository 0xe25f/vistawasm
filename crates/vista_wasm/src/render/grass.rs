use vista_types::GrassOptions;

use crate::maths::hash_noise;
use crate::maths::smoothstep;
use crate::render::flora::{
  unit_from_hash, FloraInstance, FloraVertex, GRASS_STYLE_REED, GRASS_STYLE_TUFT,
};
use crate::render::water::{WetBanks, WET_BANK_RANGE_METRES};
use crate::terrain::biomes::{
  SurfaceSample, MAT_DRY_GRASS, MAT_FOREST_FLOOR, MAT_LUSH_GRASS, MAT_TUNDRA,
};
use crate::terrain::heightmap::HeightMap;

/// Candidate grid resolution used when scattering grass. Grass reads much
/// smaller on screen than a tree, so it is scanned at a finer stride than
/// flora for the same terrain size (compare `MAX_CANDIDATE_SAMPLES_PER_SIDE`
/// in `render/flora.rs`).
const MAX_CANDIDATE_SAMPLES_PER_SIDE: u32 = 768;

/// Maximum slope ratio grass tolerates before a candidate is rejected.
/// Slightly more permissive than flora's, since ground cover survives
/// steeper ground than trees do.
const MAX_PLANTING_SLOPE: f32 = 0.85;

/// Tundra tufts grow at this fraction of the density of a meadow.
const TUNDRA_GRASS_DENSITY: f32 = 0.4;

/// Tundra tufts are this fraction of the height of meadow grass.
const TUNDRA_GRASS_HEIGHT: f32 = 0.4;

/// One quad's worth of vertices, reused for every one of a grass tuft's
/// three crossed blades.
const GRASS_QUAD: [FloraVertex; 6] = [
  FloraVertex {
    local_offset: [-0.5, 0.0],
    uv: [0.0, 0.0],
  },
  FloraVertex {
    local_offset: [0.5, 0.0],
    uv: [1.0, 0.0],
  },
  FloraVertex {
    local_offset: [-0.5, 1.0],
    uv: [0.0, 1.0],
  },
  FloraVertex {
    local_offset: [0.5, 0.0],
    uv: [1.0, 0.0],
  },
  FloraVertex {
    local_offset: [0.5, 1.0],
    uv: [1.0, 1.0],
  },
  FloraVertex {
    local_offset: [-0.5, 1.0],
    uv: [0.0, 1.0],
  },
];

/// The three static, 60-degree-apart crossed quads shared by every grass
/// tuft instance (18 vertices total, three copies of [`GRASS_QUAD`]).
///
/// Unlike flora billboards, grass tufts are never camera-facing: the
/// vertex shader orients each group of six vertices at a different fixed
/// world angle (plus a per-instance random offset), which gives real
/// parallax as the camera moves — important for foliage this close to the
/// lens, where a flat camera-facing cutout would be obvious.
pub const GRASS_BASE_TUFT: [FloraVertex; 18] = [
  GRASS_QUAD[0],
  GRASS_QUAD[1],
  GRASS_QUAD[2],
  GRASS_QUAD[3],
  GRASS_QUAD[4],
  GRASS_QUAD[5],
  GRASS_QUAD[0],
  GRASS_QUAD[1],
  GRASS_QUAD[2],
  GRASS_QUAD[3],
  GRASS_QUAD[4],
  GRASS_QUAD[5],
  GRASS_QUAD[0],
  GRASS_QUAD[1],
  GRASS_QUAD[2],
  GRASS_QUAD[3],
  GRASS_QUAD[4],
  GRASS_QUAD[5],
];

/// Clamp grass instances to a device or implementation limit.
pub fn clamp_grass_instances(options: &GrassOptions, device_limit: u32) -> u32 {
  if !options.enabled {
    return 0;
  }

  options.max_instances.min(device_limit)
}

/// Scatter deterministic grass tuft instances across the terrain.
///
/// Placement reuses the terrain's already-baked surface samples when
/// available (computed once per terrain by `bake_terrain_shading`) rather
/// than recomputing slope/height thresholds independently — grass naturally
/// avoids rock/snow/sand/mud/underwater terrain because that is exactly
/// what the grass material weights already encode, and it takes its
/// colour (lush green to savannah straw) from the local climate. When cached materials are not
/// available (native builds/tests), a simpler height/slope heuristic is
/// used instead so the function still produces a sensible, deterministic
/// result. `density_scale` applies the active render quality preset's
/// flora density multiplier, mirroring `build_tree_instances`.
pub fn build_grass_instances(
  map: &HeightMap,
  materials: Option<&[SurfaceSample]>,
  options: &GrassOptions,
  density_scale: f32,
) -> Vec<FloraInstance> {
  build_grass_instances_by_water(map, materials, None, options, density_scale)
}

/// Reeds grow within this distance of still or slow water, in metres, or
/// on the first samples from the shore where samples are further apart,
/// as long as the wet-bank field still measures that far.
const REED_METRES: f32 = 3.0;

/// Reeds need a mean temperature above this, in °C.
const REED_CELSIUS: f32 = 4.0;

/// [`build_grass_instances`] beside water: within 12 m of it grass grows
/// denser and greener, and reeds grow by still or slow water in
/// temperate and warm climates.
pub fn build_grass_instances_by_water(
  map: &HeightMap,
  materials: Option<&[SurfaceSample]>,
  wet: Option<&WetBanks>,
  options: &GrassOptions,
  density_scale: f32,
) -> Vec<FloraInstance> {
  let density = (options.density * density_scale).clamp(0.0, 1.0);

  if !options.enabled || density <= 0.0 || options.max_instances == 0 {
    return Vec::new();
  }

  let width = map.metadata.width;
  let height = map.metadata.height;

  if width < 2 || height < 2 {
    return Vec::new();
  }

  let stride = (width.max(height).saturating_sub(1)
    / (MAX_CANDIDATE_SAMPLES_PER_SIDE.saturating_sub(1)).max(1))
  .max(1);
  let metres_per_sample = map.metadata.metres_per_sample.max(0.001);
  let half_width = (width as f32 - 1.0) * metres_per_sample * 0.5;
  let half_height = (height as f32 - 1.0) * metres_per_sample * 0.5;
  let seed = options.seed_offset;
  let water_line = map.metadata.sea_level_metres + 0.5;

  let mut candidates = Vec::new();
  let mut y = 0;

  while y < height {
    let mut x = 0;

    while x < width {
      if let Some(instance) = candidate_at(
        map,
        materials,
        wet,
        x,
        y,
        width,
        height,
        metres_per_sample,
        water_line,
        density,
        seed,
      ) {
        let jitter_x = unit_from_hash(hash_noise(seed ^ 0x2545_f491, x as i32, y as i32)) - 0.5;
        let jitter_z = unit_from_hash(hash_noise(seed ^ 0x38b3_4ae5, x as i32, y as i32)) - 0.5;
        let cell_span = stride as f32 * metres_per_sample;

        candidates.push(FloraInstance {
          position: [
            instance.position[0] - half_width + jitter_x * cell_span,
            instance.position[1],
            instance.position[2] - half_height + jitter_z * cell_span,
          ],
          scale: instance.scale,
          tint: instance.tint,
          dryness: instance.dryness,
          style: instance.style,
        });
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
  materials: Option<&[SurfaceSample]>,
  wet: Option<&WetBanks>,
  x: u32,
  y: u32,
  width: u32,
  height: u32,
  metres_per_sample: f32,
  water_line: f32,
  density: f32,
  seed: u64,
) -> Option<FloraInstance> {
  let index = (y * width + x) as usize;

  if map.no_data[index] {
    return None;
  }

  let elevation = map.heights[index];

  if elevation <= water_line {
    return None;
  }

  let sample = materials.and_then(|weights| weights.get(index));
  let acceptance_weight = match sample {
    // Nothing grows on glacier ice; tundra carries sparse sedge tufts.
    Some(sample) if sample.is_glacier() => 0.0,
    Some(sample) => {
      sample.weight(MAT_LUSH_GRASS)
        + sample.weight(MAT_DRY_GRASS) * 0.9
        + sample.weight(MAT_FOREST_FLOOR) * 0.25
        + sample.weight(MAT_TUNDRA) * TUNDRA_GRASS_DENSITY
    }
    None => {
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
        0.0
      } else {
        1.0
      }
    }
  };

  let (water_metres, still_metres) = wet.map_or((f32::MAX, f32::MAX), |wet| wet.at(x, y));
  let shore = wet.map_or(0.0, |wet| wet.stride as f32 * metres_per_sample * 0.5 + 0.5);
  // The field saturates at its range, which reads as "far", not "near".
  let reeds = still_metres > 0.0
    && still_metres < WET_BANK_RANGE_METRES
    && still_metres <= REED_METRES.max(shore)
    && sample.is_some_and(|sample| !sample.is_glacier() && sample.celsius() > REED_CELSIUS);
  // Within 12 m of water grass grows denser and greener.
  let near_water = 1.0 - smoothstep(water_metres / 12.0);
  let acceptance_weight = if reeds {
    acceptance_weight.max(0.9)
  } else {
    acceptance_weight * (1.0 + 0.6 * near_water)
  };

  if acceptance_weight <= 0.0 {
    return None;
  }

  let placement_roll = unit_from_hash(hash_noise(seed, x as i32, y as i32));

  if placement_roll > density * acceptance_weight {
    return None;
  }

  let scale_roll = unit_from_hash(hash_noise(seed ^ 0x0a2b_c3d4, x as i32, y as i32));
  let tint_roll = unit_from_hash(hash_noise(seed ^ 0x5f2e_1d0c, x as i32, y as i32));

  if reeds {
    return Some(FloraInstance {
      position: [
        x as f32 * metres_per_sample,
        elevation,
        y as f32 * metres_per_sample,
      ],
      scale: 1.4 + scale_roll * 0.8,
      tint: tint_roll,
      dryness: 0.0,
      style: GRASS_STYLE_REED,
    });
  }

  if let Some(sample) = sample.filter(|sample| sample.is_tundra()) {
    let tundra = sample.weight(MAT_TUNDRA);
    let grass = sample.weight(MAT_LUSH_GRASS) + sample.weight(MAT_DRY_GRASS);

    // Where tundra is the ground, tufts are short and ochre-green.
    if tundra >= grass {
      return Some(FloraInstance {
        position: [
          x as f32 * metres_per_sample,
          elevation,
          y as f32 * metres_per_sample,
        ],
        scale: (0.5 + scale_roll * 0.6) * TUNDRA_GRASS_HEIGHT,
        tint: tint_roll,
        dryness: 0.5,
        style: GRASS_STYLE_TUFT,
      });
    }
  }

  Some(FloraInstance {
    position: [
      x as f32 * metres_per_sample,
      elevation,
      y as f32 * metres_per_sample,
    ],
    scale: 0.5 + scale_roll * 0.6,
    tint: tint_roll,
    dryness: sample.map_or(0.0, |sample| {
      let total = sample.weight(MAT_LUSH_GRASS) + sample.weight(MAT_DRY_GRASS);
      if total > 0.0 {
        sample.weight(MAT_DRY_GRASS) / total
      } else {
        0.0
      }
    }) * (1.0 - 0.7 * near_water),
    style: GRASS_STYLE_TUFT,
  })
}

#[cfg(test)]
mod tests {
  use super::*;
  use vista_types::TerrainMetadata;

  fn flat_map(size: u32, elevation: f32) -> HeightMap {
    let metadata = TerrainMetadata {
      width: size,
      height: size,
      metres_per_sample: 4.0,
      sea_level_metres: 0.0,
      ..TerrainMetadata::default()
    };

    HeightMap::flat(size, size, elevation, metadata)
  }

  fn grass_options() -> GrassOptions {
    GrassOptions {
      enabled: true,
      density: 1.0,
      seed_offset: 99,
      max_instances: 50_000,
      ..GrassOptions::default()
    }
  }

  fn cold_surface(map: &HeightMap, celsius: f32) -> Vec<SurfaceSample> {
    let options = vista_types::BiomeOptions {
      mean_temperature_celsius: Some(celsius),
      volcanism: 0.0,
      ..vista_types::BiomeOptions::default()
    };
    let normals = crate::terrain::normals::generate_normals(map);
    crate::terrain::biomes::classify_surface(map, &normals, None, &options)
  }

  #[test]
  fn glaciers_grow_no_grass_and_tundra_grows_short_sparse_tufts() {
    let map = flat_map(64, 20.0);
    let glacier = cold_surface(&map, -20.0);
    assert!(glacier.iter().all(|sample| sample.is_glacier()));
    assert!(build_grass_instances(&map, Some(&glacier), &grass_options(), 1.0).is_empty());

    let tundra = cold_surface(&map, 1.0);
    assert!(tundra.iter().all(|sample| sample.is_tundra()));
    let tufts = build_grass_instances(&map, Some(&tundra), &grass_options(), 1.0);
    let meadow = build_grass_instances(&map, None, &grass_options(), 1.0);

    assert!(!tufts.is_empty());
    assert!(tufts.len() * 2 < meadow.len());
    assert!(tufts
      .iter()
      .all(|tuft| tuft.scale <= 1.1 * TUNDRA_GRASS_HEIGHT && tuft.dryness == 0.5));
  }

  /// A lake down the middle column band of a flat map, and the wet-bank
  /// field for it.
  fn lakeside(size: u32) -> (HeightMap, WetBanks) {
    let map = flat_map(size, 20.0);
    let water: Vec<bool> = (0..size * size).map(|i| (i % size) < 8).collect();
    let wet = WetBanks::build(&map, &water, &water);
    (map, wet)
  }

  #[test]
  fn the_wet_bank_field_measures_from_the_water_edge() {
    let (_, wet) = lakeside(64);
    let metres = 4.0;

    assert_eq!(wet.at(3, 10), (0.0, 0.0));
    // Three samples from the last water sample: 2.5 samples from the edge.
    let (water, still) = wet.at(10, 10);
    assert!((water - 2.5 * metres).abs() < 0.2, "{water}");
    assert_eq!(water, still);
    assert_eq!(wet.at(60, 10).0, 40.0);
  }

  #[test]
  fn grass_is_denser_near_water_and_reeds_grow_by_warm_still_water() {
    let (map, wet) = lakeside(96);
    let surface = |celsius: f32| {
      vec![
        SurfaceSample {
          materials: [120, 0, 0, 0, 0, 0, 0, 0, 0, 0],
          celsius_hundredths: (celsius * 100.0) as i16,
          ..SurfaceSample::default()
        };
        (96 * 96) as usize
      ]
    };
    let options = GrassOptions {
      density: 0.6,
      ..grass_options()
    };
    let warm = surface(12.0);
    let with_water = build_grass_instances_by_water(&map, Some(&warm), Some(&wet), &options, 1.0);
    let without = build_grass_instances(&map, Some(&warm), &options, 1.0);
    // Instances are centred on the map; back to sample columns.
    let column = |i: &FloraInstance| (i.position[0] + 95.0 * 2.0) / 4.0;
    let near = |instances: &[FloraInstance]| {
      instances
        .iter()
        .filter(|i| i.style == GRASS_STYLE_TUFT && column(i) > 8.5 && column(i) < 13.0)
        .count()
    };
    let reeds: Vec<&FloraInstance> = with_water
      .iter()
      .filter(|i| i.style == GRASS_STYLE_REED)
      .collect();

    assert!(near(&with_water) > near(&without));
    assert!(!reeds.is_empty());
    assert!(reeds
      .iter()
      .all(|reed| reed.scale >= 1.4 && reed.scale <= 2.2 && column(reed) <= 9.0));

    let cold = surface(1.0);
    let cold_grass = build_grass_instances_by_water(&map, Some(&cold), Some(&wet), &options, 1.0);
    assert!(cold_grass.iter().all(|i| i.style == GRASS_STYLE_TUFT));
  }

  #[test]
  fn reeds_do_not_spread_over_coarse_maps() {
    // Samples 120 m apart: even the first shore sample lies beyond the
    // wet-bank field's range, where it reads as far from water.
    let size = 64;
    let metadata = TerrainMetadata {
      width: size,
      height: size,
      metres_per_sample: 120.0,
      sea_level_metres: 0.0,
      ..TerrainMetadata::default()
    };
    let map = HeightMap::flat(size, size, 20.0, metadata);
    let water: Vec<bool> = (0..size * size).map(|i| (i % size) < 8).collect();
    let wet = WetBanks::build(&map, &water, &water);
    let surface = vec![
      SurfaceSample {
        materials: [120, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        celsius_hundredths: 1200,
        ..SurfaceSample::default()
      };
      (size * size) as usize
    ];
    let options = GrassOptions {
      density: 1.0,
      ..grass_options()
    };
    let grass = build_grass_instances_by_water(&map, Some(&surface), Some(&wet), &options, 1.0);

    assert!(!grass.is_empty());
    assert!(grass.iter().all(|i| i.style == GRASS_STYLE_TUFT));
  }

  #[test]
  fn disabled_grass_produces_no_instances() {
    let map = flat_map(64, 20.0);
    let mut options = grass_options();
    options.enabled = false;

    assert!(build_grass_instances(&map, None, &options, 1.0).is_empty());
  }

  #[test]
  fn zero_density_produces_no_instances() {
    let map = flat_map(64, 20.0);
    let mut options = grass_options();
    options.density = 0.0;

    assert!(build_grass_instances(&map, None, &options, 1.0).is_empty());
  }

  #[test]
  fn underwater_terrain_produces_no_instances() {
    let map = flat_map(64, -5.0);
    let options = grass_options();

    assert!(build_grass_instances(&map, None, &options, 1.0).is_empty());
  }

  #[test]
  fn placement_is_deterministic_for_the_same_seed() {
    let map = flat_map(64, 20.0);
    let options = grass_options();

    let left = build_grass_instances(&map, None, &options, 1.0);
    let right = build_grass_instances(&map, None, &options, 1.0);

    assert_eq!(left.len(), right.len());
    for (a, b) in left.iter().zip(right.iter()) {
      assert_eq!(a.position, b.position);
      assert_eq!(a.scale, b.scale);
      assert_eq!(a.tint, b.tint);
    }
  }

  #[test]
  fn respects_max_instances() {
    let map = flat_map(128, 20.0);
    let mut options = grass_options();
    options.max_instances = 10;

    let instances = build_grass_instances(&map, None, &options, 1.0);
    assert!(instances.len() <= 10);
  }

  #[test]
  fn zero_material_grass_weight_blocks_placement() {
    let map = flat_map(64, 20.0);
    let options = grass_options();
    let bare_materials = vec![
      SurfaceSample {
        materials: [0, 0, 0, 0, 255, 0, 0, 0, 0, 0],
        ..SurfaceSample::default()
      };
      (map.metadata.width * map.metadata.height) as usize
    ];

    assert!(build_grass_instances(&map, Some(&bare_materials), &options, 1.0).is_empty());
  }
}
