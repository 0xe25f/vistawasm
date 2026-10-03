use vista_types::Vec3;

use crate::maths::normalise;
use crate::terrain::heightmap::HeightMap;

/// Generate terrain normals from height data.
pub fn generate_normals(map: &HeightMap) -> Vec<Vec3> {
  let width = map.metadata.width;
  let mut normals = vec![[0.0, 1.0, 0.0]; map.heights.len()];

  for y in 0..map.metadata.height {
    for x in 0..width {
      normals[(y * width + x) as usize] = normal_at(map, x, y);
    }
  }

  normals
}

/// The normal at sample `(x, y)`, from central differences clamped at the
/// edges: the one [`generate_normals`] gives there, for readers that need
/// a few samples without the whole array.
pub fn normal_at(map: &HeightMap, x: u32, y: u32) -> Vec3 {
  let width = map.metadata.width;
  let height = map.metadata.height;
  let sample = map.metadata.metres_per_sample.max(0.001);
  let left = map.height_at(x.saturating_sub(1), y).unwrap_or(0.0);
  let right = map
    .height_at((x + 1).min(width.saturating_sub(1)), y)
    .unwrap_or(0.0);
  let down = map.height_at(x, y.saturating_sub(1)).unwrap_or(0.0);
  let up = map
    .height_at(x, (y + 1).min(height.saturating_sub(1)))
    .unwrap_or(0.0);
  normalise([left - right, sample * 2.0, down - up])
}

#[cfg(test)]
mod tests {
  use super::*;
  use vista_types::TerrainMetadata;

  #[test]
  fn flat_map_normals_point_up() {
    let map = HeightMap::flat(4, 4, 2.0, TerrainMetadata::default());
    let normals = generate_normals(&map);

    assert_eq!(normals[0], [0.0, 1.0, 0.0]);
  }
}
