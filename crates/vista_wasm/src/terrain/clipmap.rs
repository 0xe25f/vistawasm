/// Description of a terrain clipmap level.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipmapLevel {
  /// Level index where 0 is nearest to the camera.
  pub level_index: u32,
  /// Heightmap sample step used by this level.
  pub sample_step: u32,
  /// World size covered by this level in metres.
  pub world_size_metres: f32,
  /// Number of indices submitted by this level.
  pub index_count: u32,
}

/// Build CPU-side clipmap level metadata.
pub fn build_clipmap_levels(
  terrain_size: u32,
  metres_per_sample: f32,
  max_levels: u32,
) -> Vec<ClipmapLevel> {
  let mut levels = Vec::new();
  let max_levels = max_levels.clamp(1, 12);

  for level_index in 0..max_levels {
    let sample_step = 1_u32 << level_index;

    if sample_step >= terrain_size {
      break;
    }

    let samples_per_side = (terrain_size / sample_step).max(2);
    let quads = samples_per_side.saturating_sub(1);

    levels.push(ClipmapLevel {
      level_index,
      sample_step,
      world_size_metres: terrain_size as f32 * metres_per_sample * sample_step as f32,
      index_count: quads * quads * 6,
    });
  }

  levels
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn clipmaps_stop_before_oversized_step() {
    let levels = build_clipmap_levels(16, 10.0, 8);

    assert_eq!(levels.last().unwrap().sample_step, 8);
  }
}
