use vista_types::TerrainMetadata;

use crate::errors::{VistaError, VistaResult};

/// CPU-side heightmap used for deterministic tests and DEM staging.
#[derive(Clone, Debug, PartialEq)]
pub struct HeightMap {
  /// Height samples in row-major order.
  pub heights: Vec<f32>,
  /// No-data mask in row-major order.
  pub no_data: Vec<bool>,
  /// Terrain metadata.
  pub metadata: TerrainMetadata,
}

impl HeightMap {
  /// Create a heightmap from raw values.
  pub fn from_values(
    width: u32,
    height: u32,
    heights: Vec<f32>,
    no_data: Vec<bool>,
    mut metadata: TerrainMetadata,
  ) -> VistaResult<Self> {
    let expected = width as usize * height as usize;

    if width == 0 || height == 0 {
      return Err(VistaError::options(
        "heightmap width and height must be at least 1.",
      ));
    }

    if heights.len() != expected {
      return Err(VistaError::options(
        "heightmap sample count does not match width and height.",
      ));
    }

    if no_data.len() != expected {
      return Err(VistaError::options(
        "heightmap no-data mask count does not match width and height.",
      ));
    }

    metadata.width = width;
    metadata.height = height;
    update_stats(&heights, &no_data, &mut metadata);

    Ok(Self {
      heights,
      no_data,
      metadata,
    })
  }

  /// Create a flat heightmap.
  pub fn flat(width: u32, height: u32, height_metres: f32, mut metadata: TerrainMetadata) -> Self {
    let len = width as usize * height as usize;
    let heights = vec![height_metres; len];
    let no_data = vec![false; len];
    metadata.width = width;
    metadata.height = height;
    update_stats(&heights, &no_data, &mut metadata);

    Self {
      heights,
      no_data,
      metadata,
    }
  }

  /// Return a row-major index if the coordinate is inside the heightmap.
  pub fn index(&self, x: u32, y: u32) -> Option<usize> {
    if x >= self.metadata.width || y >= self.metadata.height {
      return None;
    }

    Some((y * self.metadata.width + x) as usize)
  }

  /// Return a height sample if it exists.
  pub fn height_at(&self, x: u32, y: u32) -> Option<f32> {
    self.index(x, y).map(|index| self.heights[index])
  }

  /// Set a height sample.
  pub fn set_height(&mut self, x: u32, y: u32, height: f32) -> VistaResult<()> {
    let index = self
      .index(x, y)
      .ok_or_else(|| VistaError::options("height sample coordinate is outside the heightmap."))?;

    self.heights[index] = height;
    update_stats(&self.heights, &self.no_data, &mut self.metadata);
    Ok(())
  }

  /// Export heights as little-endian `f32` bytes.
  pub fn export_f32_le(&self) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(self.heights.len() * 4);

    for height in &self.heights {
      bytes.extend_from_slice(&height.to_le_bytes());
    }

    bytes
  }
}

/// Recalculate terrain statistics.
pub fn update_stats(heights: &[f32], no_data: &[bool], metadata: &mut TerrainMetadata) {
  let mut min = f32::INFINITY;
  let mut max = f32::NEG_INFINITY;
  let mut sum = 0.0_f64;
  let mut count = 0_u64;

  for (height, no_data) in heights.iter().zip(no_data.iter()) {
    if *no_data || !height.is_finite() {
      continue;
    }

    min = min.min(*height);
    max = max.max(*height);
    sum += *height as f64;
    count += 1;
  }

  if count == 0 {
    metadata.min_height_metres = 0.0;
    metadata.max_height_metres = 0.0;
    metadata.mean_height_metres = 0.0;
    return;
  }

  metadata.min_height_metres = min;
  metadata.max_height_metres = max;
  metadata.mean_height_metres = (sum / count as f64) as f32;
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn export_uses_little_endian_float_bytes() {
    let map = HeightMap::flat(1, 1, 2.5, TerrainMetadata::default());

    assert_eq!(map.export_f32_le(), 2.5_f32.to_le_bytes());
  }
}
