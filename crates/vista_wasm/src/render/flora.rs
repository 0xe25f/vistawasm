use vista_types::FloraOptions;

use crate::maths::hash_noise;
use crate::terrain::heightmap::HeightMap;

/// Clamp flora instances to a device or implementation limit.
pub fn clamp_flora_instances(options: &FloraOptions, device_limit: u32) -> u32 {
  if !options.enabled {
    return 0;
  }

  options.max_instances.min(device_limit)
}

/// One GPU-ready flora instance transform.
///
/// Layout must stay in sync with the per-instance vertex attributes declared
/// in `flora_instances.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FloraInstance {
  /// World position of the base of the plant, in terrain metres.
  pub position: [f32; 3],
  /// Billboard scale in metres.
  pub scale: f32,
  /// A deterministic 0 to 1 tint variation used to vary foliage colour.
  pub tint: f32,
}

/// One base-geometry vertex shared by every flora billboard instance.
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

/// The static two-triangle quad shared by every flora billboard instance.
pub const FLORA_BASE_QUAD: [FloraVertex; 6] = [
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

/// The `FLORA_BASE_QUAD` shape repeated twice (12 vertices).
///
/// This is the vertex buffer actually uploaded to the GPU for flora
/// (`GpuContext::new`); the `Billboard` tree style only ever draws the
/// first 6 vertices (byte-identical to `FLORA_BASE_QUAD`, so that style's
/// rendering is unchanged), while `CrossQuad`/`Mesh` draw all 12. The
/// second 6 vertices are the *same* local shape — `flora_instances.wgsl`
/// decides each quad's world orientation from `@builtin(vertex_index)`
/// rather than from any extra vertex data, so no new vertex attributes are
/// needed to support the second quad.
pub const FLORA_BASE_QUAD_CROSS: [FloraVertex; 12] = [
  FLORA_BASE_QUAD[0],
  FLORA_BASE_QUAD[1],
  FLORA_BASE_QUAD[2],
  FLORA_BASE_QUAD[3],
  FLORA_BASE_QUAD[4],
  FLORA_BASE_QUAD[5],
  FLORA_BASE_QUAD[0],
  FLORA_BASE_QUAD[1],
  FLORA_BASE_QUAD[2],
  FLORA_BASE_QUAD[3],
  FLORA_BASE_QUAD[4],
  FLORA_BASE_QUAD[5],
];

/// Candidate grid resolution used when scattering flora. Terrain larger than
/// this is scanned at a coarser stride, keeping placement fast regardless of
/// terrain size.
const MAX_CANDIDATE_SAMPLES_PER_SIDE: u32 = 512;

/// Maximum fraction of local height difference treated as "flat enough" for
/// planting, expressed as a slope ratio.
const MAX_PLANTING_SLOPE: f32 = 0.6;

/// Scatter deterministic flora instances across grass below the tree line.
///
/// Placement is driven entirely by the terrain heightmap and `options`, so
/// the same seed and options always scatter the same plants. `density_scale`
/// applies an additional multiplier from the active render quality preset.
pub fn build_flora_instances(
  map: &HeightMap,
  options: &FloraOptions,
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
  let water_line = map.metadata.sea_level_metres + 1.0;

  let mut candidates = Vec::new();
  let mut y = 0;

  while y < height {
    let mut x = 0;

    while x < width {
      if let Some(instance) = candidate_at(
        map,
        x,
        y,
        width,
        height,
        metres_per_sample,
        water_line,
        options,
        seed,
      ) {
        let placement_roll = unit_from_hash(hash_noise(seed, x as i32, y as i32));

        if placement_roll <= density {
          let jitter_x = unit_from_hash(hash_noise(seed ^ 0x9e37_79b9, x as i32, y as i32)) - 0.5;
          let jitter_z = unit_from_hash(hash_noise(seed ^ 0x85eb_ca6b, x as i32, y as i32)) - 0.5;
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
  x: u32,
  y: u32,
  width: u32,
  height: u32,
  metres_per_sample: f32,
  water_line: f32,
  options: &FloraOptions,
  seed: u64,
) -> Option<FloraInstance> {
  let index = (y * width + x) as usize;

  if map.no_data[index] {
    return None;
  }

  let elevation = map.heights[index];

  if elevation <= water_line || elevation > options.tree_line_metres {
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

  let scale_roll = unit_from_hash(hash_noise(seed ^ 0x1234_5678, x as i32, y as i32));
  let tint_roll = unit_from_hash(hash_noise(seed ^ 0x4321_dcba, x as i32, y as i32));

  Some(FloraInstance {
    position: [
      x as f32 * metres_per_sample,
      elevation,
      y as f32 * metres_per_sample,
    ],
    scale: 6.0 + scale_roll * 6.0,
    tint: tint_roll,
  })
}

pub(crate) fn unit_from_hash(value: f32) -> f32 {
  (value + 1.0) * 0.5
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
  fn disabled_flora_produces_no_instances() {
    let map = flat_map(32, 50.0);
    let mut options = flora_options();
    options.enabled = false;

    assert!(build_flora_instances(&map, &options, 1.0).is_empty());
  }

  #[test]
  fn underwater_terrain_produces_no_instances() {
    let map = flat_map(32, -10.0);
    let options = flora_options();

    assert!(build_flora_instances(&map, &options, 1.0).is_empty());
  }

  #[test]
  fn flat_grass_above_sea_level_produces_instances() {
    let map = flat_map(32, 50.0);
    let options = flora_options();

    assert!(!build_flora_instances(&map, &options, 1.0).is_empty());
  }

  #[test]
  fn max_instances_is_respected() {
    let map = flat_map(64, 50.0);
    let mut options = flora_options();
    options.max_instances = 5;

    assert!(build_flora_instances(&map, &options, 1.0).len() <= 5);
  }

  #[test]
  fn same_seed_is_deterministic() {
    let map = flat_map(32, 50.0);
    let options = flora_options();

    let first = build_flora_instances(&map, &options, 1.0);
    let second = build_flora_instances(&map, &options, 1.0);

    assert_eq!(first.len(), second.len());

    for (a, b) in first.iter().zip(second.iter()) {
      assert_eq!(a.position, b.position);
    }
  }
}
