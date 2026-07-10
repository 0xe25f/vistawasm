use vista_types::GrassOptions;

use crate::maths::hash_noise;
use crate::render::flora::{unit_from_hash, FloraInstance, FloraVertex};
use crate::terrain::heightmap::HeightMap;
use crate::terrain::materials::MaterialWeights;

/// Candidate grid resolution used when scattering grass. Grass reads much
/// smaller on screen than a tree, so it is scanned at a finer stride than
/// flora for the same terrain size (compare `MAX_CANDIDATE_SAMPLES_PER_SIDE`
/// in `render/flora.rs`).
const MAX_CANDIDATE_SAMPLES_PER_SIDE: u32 = 768;

/// Maximum slope ratio grass tolerates before a candidate is rejected.
/// Slightly more permissive than flora's, since ground cover survives
/// steeper ground than trees do.
const MAX_PLANTING_SLOPE: f32 = 0.85;

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
/// Placement reuses the terrain's already-baked `MaterialWeights.grass`
/// weight when available (browser builds, via `EngineCore::terrain_materials`,
/// computed once per terrain by `bake_terrain_shading`) rather than
/// recomputing slope/height thresholds independently — grass naturally
/// avoids rock/snow/mud/underwater terrain because that is exactly what the
/// `grass` material weight already encodes. When cached materials are not
/// available (native builds/tests), a simpler height/slope heuristic is
/// used instead so the function still produces a sensible, deterministic
/// result. `density_scale` applies the active render quality preset's
/// flora density multiplier, mirroring `build_flora_instances`.
pub fn build_grass_instances(
  map: &HeightMap,
  materials: Option<&[MaterialWeights]>,
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
  materials: Option<&[MaterialWeights]>,
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

  let acceptance_weight = match materials.and_then(|weights| weights.get(index)) {
    Some(weights) => weights.grass,
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

  if acceptance_weight <= 0.0 {
    return None;
  }

  let placement_roll = unit_from_hash(hash_noise(seed, x as i32, y as i32));

  if placement_roll > density * acceptance_weight {
    return None;
  }

  let scale_roll = unit_from_hash(hash_noise(seed ^ 0x0a2b_c3d4, x as i32, y as i32));
  let tint_roll = unit_from_hash(hash_noise(seed ^ 0x5f2e_1d0c, x as i32, y as i32));

  Some(FloraInstance {
    position: [
      x as f32 * metres_per_sample,
      elevation,
      y as f32 * metres_per_sample,
    ],
    scale: 0.5 + scale_roll * 0.6,
    tint: tint_roll,
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
      MaterialWeights {
        grass: 0.0,
        rock: 1.0,
        snow: 0.0,
        wet_mud: 0.0,
      };
      (map.metadata.width * map.metadata.height) as usize
    ];

    assert!(build_grass_instances(&map, Some(&bare_materials), &options, 1.0).is_empty());
  }
}
