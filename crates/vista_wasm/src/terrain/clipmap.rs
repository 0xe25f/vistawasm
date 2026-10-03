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
  clipmap_levels(terrain_size, metres_per_sample, max_levels).collect()
}

/// The levels [`build_clipmap_levels`] lists, one at a time, for callers
/// that only total them each frame and so should not allocate.
pub fn clipmap_levels(
  terrain_size: u32,
  metres_per_sample: f32,
  max_levels: u32,
) -> impl Iterator<Item = ClipmapLevel> {
  (0..max_levels.clamp(1, 12))
    .map(|level_index| (level_index, 1_u32 << level_index))
    .take_while(move |&(_, sample_step)| sample_step < terrain_size)
    .map(move |(level_index, sample_step)| {
      let samples_per_side = (terrain_size / sample_step).max(2);
      let quads = samples_per_side.saturating_sub(1);

      ClipmapLevel {
        level_index,
        sample_step,
        world_size_metres: terrain_size as f32 * metres_per_sample * sample_step as f32,
        index_count: quads * quads * 6,
      }
    })
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
