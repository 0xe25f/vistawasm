use vista_types::Vec3;

use crate::maths::normalise;
use crate::terrain::heightmap::HeightMap;

/// Generate terrain normals from height data.
pub fn generate_normals(map: &HeightMap) -> Vec<Vec3> {
  let width = map.metadata.width;
  let height = map.metadata.height;
  let mut normals = vec![[0.0, 1.0, 0.0]; map.heights.len()];
  let sample = map.metadata.metres_per_sample.max(0.001);

  for y in 0..height {
    for x in 0..width {
      let left = map.height_at(x.saturating_sub(1), y).unwrap_or(0.0);
      let right = map
        .height_at((x + 1).min(width.saturating_sub(1)), y)
        .unwrap_or(0.0);
      let down = map.height_at(x, y.saturating_sub(1)).unwrap_or(0.0);
      let up = map
        .height_at(x, (y + 1).min(height.saturating_sub(1)))
        .unwrap_or(0.0);
      let normal = normalise([left - right, sample * 2.0, down - up]);
      normals[(y * width + x) as usize] = normal;
    }
  }

  normals
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
