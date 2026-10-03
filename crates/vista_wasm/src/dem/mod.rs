//! DEM and raw heightmap ingestion.

pub mod geotiff;
pub mod metadata;
pub mod raw_heightmap;
pub mod stream;

pub use geotiff::{decode_geotiff, MAX_GEOTIFF_BYTES};
pub use raw_heightmap::{decode_raw_heightmap, RawBytes, MAX_RAW_HEIGHTMAP_BYTES};

use crate::config::{MAX_HEIGHT_METRES, MAX_TERRAIN_SIZE, MIN_TERRAIN_SIZE, TERRAIN_SIZE_REASON};

/// Check a decoded map's size before anything is allocated for it: from
/// [`MIN_TERRAIN_SIZE`] to [`MAX_TERRAIN_SIZE`] samples a side. Returns the
/// sample count.
pub(crate) fn sample_count(what: &str, width: u32, height: u32) -> Result<usize, String> {
  let sides = MIN_TERRAIN_SIZE..=MAX_TERRAIN_SIZE;

  if !sides.contains(&width) || !sides.contains(&height) {
    let reason = if width.max(height) > MAX_TERRAIN_SIZE {
      format!(" {TERRAIN_SIZE_REASON}")
    } else {
      String::new()
    };
    return Err(format!(
      "{what} width and height must be from {MIN_TERRAIN_SIZE} to {MAX_TERRAIN_SIZE} samples, but they are {width} and {height}.{reason}"
    ));
  }

  // At most 2048 x 2048, so this cannot overflow even a 32-bit usize.
  Ok(width as usize * height as usize)
}

/// Decoded heights and their no-data mask, filled one sample at a time.
pub(crate) struct Samples {
  pub heights: Vec<f32>,
  pub no_data: Vec<bool>,
  scale: f32,
  marker: Option<f32>,
  rejected: usize,
}

impl Samples {
  /// Room for `count` samples, each multiplied by `scale`, where values
  /// equal to `marker` are no data.
  pub fn new(count: usize, scale: f32, marker: Option<f32>) -> Self {
    Self {
      heights: Vec::with_capacity(count),
      no_data: Vec::with_capacity(count),
      scale,
      marker,
      rejected: 0,
    }
  }

  /// Add one raw value. Values that are not finite, or lie beyond
  /// [`MAX_HEIGHT_METRES`] once scaled, cannot be ground (they are usually
  /// an undeclared no-data marker), so they become no data too: left in,
  /// they would spread through every later stage of the terrain.
  pub fn push(&mut self, value: f32) {
    let marked = self.marker.is_some_and(|marker| {
      (value - marker).abs() <= f32::EPSILON || (marker.is_nan() && value.is_nan())
    });
    let metres = value * self.scale;
    let valid = !marked && metres.is_finite() && metres.abs() <= MAX_HEIGHT_METRES;

    self.rejected += usize::from(!marked && !valid);
    self.heights.push(if valid { metres } else { 0.0 });
    self.no_data.push(!valid);
  }

  /// A warning when values had to be treated as no data.
  pub fn warning(&self) -> Option<String> {
    (self.rejected > 0).then(|| {
      format!(
        "{} samples were not finite or lay beyond ±{MAX_HEIGHT_METRES} m once scaled, so they are treated as no data.",
        self.rejected
      )
    })
  }
}
