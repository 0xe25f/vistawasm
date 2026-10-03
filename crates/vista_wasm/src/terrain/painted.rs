//! Painted inputs: a biome map that overrides classification, and tree
//! and grass density masks that scale the vegetation.
//!
//! A painted biome is absolute. Its borders are domain-warped so they
//! interlock naturally instead of following the painted pixels, and the
//! ground's materials blend across them (see `biomes::classify_into`).

use vista_types::BiomeKind;

use crate::errors::{VistaError, VistaResult};
use crate::maths::value_noise;

/// The biome map value for a sample the engine classifies itself.
pub const NOT_PAINTED: u8 = 255;

/// The widest dithered border, in samples.
pub const MAX_BORDER_SAMPLES: u32 = 8;

/// A painted biome map, resampled to the terrain's size.
#[derive(Clone, Debug, PartialEq)]
pub struct PaintedBiomes {
  /// One [`BiomeKind`] index a sample, row-major, or [`NOT_PAINTED`].
  pub data: Vec<u8>,
  /// Samples per row.
  pub width: u32,
  /// Rows.
  pub height: u32,
  /// How far, in samples, the borders are warped and blended.
  pub border: u32,
  /// The warp's noise seed, from the terrain, so a map painted for the
  /// same heights always lands the same way.
  pub seed: u64,
}

impl PaintedBiomes {
  /// Check a biome map from the host and resample it (nearest) to
  /// `width x height`. Returns a warning when it had to be resampled.
  pub fn new(
    map: (u32, u32, &[u8]),
    border: u32,
    (width, height): (u32, u32),
    seed: u64,
  ) -> VistaResult<(Self, Option<String>)> {
    let (map_width, map_height, data) = map;
    check_size("setBiomeMap", map_width, map_height, data.len())?;

    if border > MAX_BORDER_SAMPLES {
      return Err(VistaError::options(format!(
        "setBiomeMap borderSamples must be from 0 to {MAX_BORDER_SAMPLES}, but it is {border}."
      )));
    }

    if let Some(bad) = data
      .iter()
      .find(|value| **value != NOT_PAINTED && usize::from(**value) >= BiomeKind::ALL.len())
    {
      return Err(VistaError::options(format!(
        "setBiomeMap data holds {bad}, but biome indices run from 0 to {}, or 255 for not painted.",
        BiomeKind::ALL.len() - 1
      )));
    }

    let warning = resample_warning("biome map", (map_width, map_height), (width, height));
    let data = resample(map, (width, height), nearest);
    Ok((
      Self {
        data,
        width,
        height,
        border,
        seed,
      },
      warning,
    ))
  }

  /// The painted biome at sample `(x, y)`, looked up through the border
  /// warp: a two-octave noise (7 and 23 samples across) moves the lookup
  /// by up to `border` samples on each axis, so interiors keep their
  /// biome and only samples within `border` of a painted edge can change.
  pub fn at(&self, x: u32, y: u32) -> Option<BiomeKind> {
    let (fx, fy) = (x as f32, y as f32);
    let border = self.border as f32;
    let warp = |salt: u64| {
      0.6 * value_noise(self.seed ^ salt, fx / 7.0, fy / 7.0)
        + 0.4 * value_noise(self.seed ^ salt ^ 0x5bd1_e995, fx / 23.0, fy / 23.0)
    };
    let lx = (fx + warp(0x9e37) * border)
      .round()
      .clamp(0.0, (self.width - 1) as f32) as u32;
    let ly = (fy + warp(0x7f4a) * border)
      .round()
      .clamp(0.0, (self.height - 1) as f32) as u32;
    let value = self.data[(ly * self.width + lx) as usize];
    (value != NOT_PAINTED).then(|| BiomeKind::from_index(value))
  }
}

/// A density mask value as a multiplier on the vegetation: 0 none, 128
/// unchanged, 255 twice as dense, linear between.
pub fn density_multiplier(value: u8) -> f32 {
  if value <= 128 {
    f32::from(value) / 128.0
  } else {
    1.0 + f32::from(value - 128) / 127.0
  }
}

/// The mask value for a multiplier: the inverse of [`density_multiplier`].
pub fn density_byte(multiplier: f32) -> u8 {
  let multiplier = multiplier.clamp(0.0, 2.0);

  if multiplier <= 1.0 {
    (multiplier * 128.0).round() as u8
  } else {
    (128.0 + (multiplier - 1.0) * 127.0).round() as u8
  }
}

/// Check a tree or grass density mask from the host and resample it
/// (bilinear) to `width x height`. Returns a warning when it had to be
/// resampled.
pub fn density_mask(
  name: &str,
  map: (u32, u32, &[u8]),
  (width, height): (u32, u32),
) -> VistaResult<(Vec<u8>, Option<String>)> {
  check_size("setVegetationMasks", map.0, map.1, map.2.len())?;
  let warning = resample_warning(&format!("{name} mask"), (map.0, map.1), (width, height));
  let data = resample(map, (width, height), |values, [tx, ty]| {
    let top = f32::from(values[0]) + (f32::from(values[1]) - f32::from(values[0])) * tx;
    let bottom = f32::from(values[2]) + (f32::from(values[3]) - f32::from(values[2])) * tx;
    (top + (bottom - top) * ty).round() as u8
  });
  Ok((data, warning))
}

/// Check a painted map's size (2 to 2048 a side, the largest terrain it
/// is resampled to: detail beyond it would be thrown away) and data
/// length.
pub fn check_size(call: &str, width: u32, height: u32, length: usize) -> VistaResult<()> {
  use crate::config::{MAX_TERRAIN_SIZE, MIN_TERRAIN_SIZE, TERRAIN_SIZE_REASON};
  let sides = MIN_TERRAIN_SIZE..=MAX_TERRAIN_SIZE;

  if !sides.contains(&width) || !sides.contains(&height) {
    let reason = if width.max(height) > MAX_TERRAIN_SIZE {
      format!(" {TERRAIN_SIZE_REASON}")
    } else {
      String::new()
    };
    return Err(VistaError::options(format!(
      "{call} width and height must be from {MIN_TERRAIN_SIZE} to {MAX_TERRAIN_SIZE}, but they are {width} and {height}.{reason}"
    )));
  }

  let expected = width as usize * height as usize;

  if length != expected {
    return Err(VistaError::options(format!(
      "{call} data must hold width x height = {expected} bytes, but it holds {length}."
    )));
  }

  Ok(())
}

/// The warning for a painted map resampled to the terrain's size.
pub fn resample_warning(what: &str, from: (u32, u32), to: (u32, u32)) -> Option<String> {
  (from != to).then(|| {
    format!(
      "The {what} is {} x {} but the terrain is {} x {}, so it was resampled to fit.",
      from.0, from.1, to.0, to.1
    )
  })
}

/// Resample `map` to `width x height`, edge to edge: `pick` gets the four
/// values around each position (row by row) and where it lies between
/// them. Maps already the right size are copied as they are.
fn resample(
  (map_width, map_height, data): (u32, u32, &[u8]),
  (width, height): (u32, u32),
  pick: impl Fn([u8; 4], [f32; 2]) -> u8,
) -> Vec<u8> {
  if (map_width, map_height) == (width, height) {
    return data.to_vec();
  }

  let at = |x: u32, y: u32| data[(y * map_width + x) as usize];
  let mut out = Vec::with_capacity(width as usize * height as usize);

  for y in 0..height {
    for x in 0..width {
      let fx = x as f32 * (map_width - 1) as f32 / (width - 1) as f32;
      let fy = y as f32 * (map_height - 1) as f32 / (height - 1) as f32;
      let (x0, y0) = (fx as u32, fy as u32);
      let (x1, y1) = ((x0 + 1).min(map_width - 1), (y0 + 1).min(map_height - 1));
      out.push(pick(
        [at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1)],
        [fx - x0 as f32, fy - y0 as f32],
      ));
    }
  }

  out
}

/// The nearest of four values from [`resample`], for maps of categories.
fn nearest(values: [u8; 4], [tx, ty]: [f32; 2]) -> u8 {
  values[usize::from(tx >= 0.5) + 2 * usize::from(ty >= 0.5)]
}
