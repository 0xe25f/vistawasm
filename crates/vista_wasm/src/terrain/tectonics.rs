//! Stage A of fractal generation: the tectonic base on a coarse grid.
//!
//! Continents come first: a warped low-frequency field whose threshold is
//! chosen by sorting, so the land fraction is exact. Mountain ranges are
//! an uplift field of warped ridged noise, confined by a low-frequency
//! mask to part of the land and faded in from the coast. The result is a
//! base elevation and an uplift rate for the stream-power stage.

use vista_types::LandformKind;

use crate::terrain::landforms::Landform;
use crate::terrain::noise::{fbm, noise_seed, ridged};

/// The coarse grid is at most this many samples per side.
pub const COARSE_MAX: u32 = 256;

/// The coarse grid is never finer than this many metres per sample. The
/// valley network is resolved at this scale; finer gullies come from the
/// full-resolution erosion stage. Finer stream-power grids break every
/// crest into single-cell summits.
pub const COARSE_MIN_SPACING: f32 = 40.0;

/// The tectonic base on the coarse grid.
pub struct Tectonics {
  /// Samples per side.
  pub size: u32,
  /// Distance between samples in metres.
  pub spacing: f32,
  /// Elevation relative to sea level, in metres.
  pub elevation: Vec<f64>,
  /// The lowland part of `elevation` (and the sea floor), without ranges.
  pub lowland: Vec<f64>,
  /// Uplift, 0 to 1, where 1 is the crest of a fully raised range.
  pub uplift: Vec<f32>,
  /// Whether each sample is land.
  pub land: Vec<bool>,
  /// The noise seed shared by later stages.
  pub seed: u64,
}

/// `1 - (1 - t)^2` on [0, 1]: rises with slope 2 from zero, so coasts
/// shelve instead of forming a flat band at sea level.
fn ease(t: f32) -> f32 {
  let t = t.clamp(0.0, 1.0);
  1.0 - (1.0 - t) * (1.0 - t)
}

fn smoothstep(edge0: f32, edge1: f32, value: f32) -> f32 {
  let t = ((value - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
  t * t * (3.0 - 2.0 * t)
}

/// The value below which `fraction` of `values` lie.
fn quantile(values: &[f32], fraction: f32) -> f32 {
  if values.is_empty() {
    return 0.0;
  }

  let mut sorted = values.to_vec();
  let index = ((sorted.len() as f32 * fraction) as usize).min(sorted.len() - 1);
  *sorted
    .select_nth_unstable_by(index, |a, b| a.total_cmp(b))
    .1
}

/// Distance in metres from each sample to the nearest sample of the other
/// class (land or sea), by a two-pass chamfer transform.
pub fn distance_to_coast(size: u32, land: &[bool], spacing: f32) -> Vec<f32> {
  let n = size as usize;
  let mut distance = vec![f32::INFINITY; n * n];
  let straight = spacing;
  let diagonal = spacing * std::f32::consts::SQRT_2;

  for y in 0..n {
    for x in 0..n {
      let i = y * n + x;
      let coast = [
        (x > 0, i.wrapping_sub(1)),
        (x + 1 < n, i + 1),
        (y > 0, i.wrapping_sub(n)),
        (y + 1 < n, i + n),
      ]
      .iter()
      .any(|(ok, j)| *ok && land[*j] != land[i]);

      if coast {
        distance[i] = spacing * 0.5;
      }
    }
  }

  let relax = |distance: &mut [f32], i: usize, j: usize, step: f32| {
    let candidate = distance[j] + step;

    if candidate < distance[i] {
      distance[i] = candidate;
    }
  };

  for y in 0..n {
    for x in 0..n {
      let i = y * n + x;

      if x > 0 {
        relax(&mut distance, i, i - 1, straight);
      }

      if y > 0 {
        relax(&mut distance, i, i - n, straight);

        if x > 0 {
          relax(&mut distance, i, i - n - 1, diagonal);
        }

        if x + 1 < n {
          relax(&mut distance, i, i - n + 1, diagonal);
        }
      }
    }
  }

  for y in (0..n).rev() {
    for x in (0..n).rev() {
      let i = y * n + x;

      if x + 1 < n {
        relax(&mut distance, i, i + 1, straight);
      }

      if y + 1 < n {
        relax(&mut distance, i, i + n, straight);

        if x + 1 < n {
          relax(&mut distance, i, i + n + 1, diagonal);
        }

        if x > 0 {
          relax(&mut distance, i, i + n - 1, diagonal);
        }
      }
    }
  }

  // A map that is all land or all sea has no coast.
  for value in &mut distance {
    if !value.is_finite() {
      *value = spacing * size as f32;
    }
  }

  distance
}

/// Build the tectonic base for a square map `extent` metres across.
pub fn tectonic_base(
  seed: u64,
  map_size: u32,
  extent: f32,
  kind: LandformKind,
  landform: &Landform,
) -> Tectonics {
  let by_spacing = (extent / COARSE_MIN_SPACING) as u32 + 1;
  let size = map_size.min(COARSE_MAX).min(by_spacing).max(16);
  let n = size as usize;
  let count = n * n;
  let spacing = extent / (size - 1) as f32;
  let half = extent * 0.5;
  let continent_wavelength = landform.continent_wavelength;
  let range_wavelength = landform.range_wavelength;
  let volcanic = kind == LandformKind::VolcanicIsland;
  // The radius of a round island with the requested land fraction.
  let island_radius = (landform.land_fraction * extent * extent / std::f32::consts::PI).sqrt();

  let continent_seed = noise_seed(seed, 1);
  let continent_warp_x = noise_seed(seed, 2);
  let continent_warp_y = noise_seed(seed, 3);
  let range_seed = noise_seed(seed, 4);
  let range_warp_x = noise_seed(seed, 5);
  let range_warp_y = noise_seed(seed, 6);
  let mask_seed = noise_seed(seed, 7);
  let hill_seed = noise_seed(seed, 8);

  let mut raw = Vec::with_capacity(count);
  let mut ranges = Vec::with_capacity(count);
  let mut mask_noise = Vec::with_capacity(count);
  let mut hills = Vec::with_capacity(count);
  let mut cone = vec![0.0f32; count];

  for gy in 0..n {
    for gx in 0..n {
      let x = gx as f32 * spacing - half;
      let y = gy as f32 * spacing - half;

      let cx = x / continent_wavelength;
      let cy = y / continent_wavelength;
      let warp = 0.35;
      let wx = cx + fbm(continent_warp_x, cx, cy, 2, 0.5, 2.0) * warp;
      let wy = cy + fbm(continent_warp_y, cx, cy, 2, 0.5, 2.0) * warp;
      let mut continent = fbm(continent_seed, wx, wy, 3, 0.5, 2.0);

      if volcanic {
        let r = (x * x + y * y).sqrt();
        continent = continent * 0.12 + 1.0 - r / half;
        let profile = (1.0 - r / (island_radius * 1.05)).clamp(0.0, 1.0);
        let lumps = 1.0
          + 0.1
            * fbm(
              range_seed,
              x / range_wavelength,
              y / range_wavelength,
              3,
              0.5,
              2.0,
            );
        cone[gy * n + gx] = profile.powf(1.3) * lumps;
      }

      raw.push(continent);

      // Ranges bend and branch: the ridged field is warped by half its
      // own wavelength.
      let rx = x / range_wavelength;
      let ry = y / range_wavelength;
      let range_warp = 0.5;
      let qx = rx + fbm(range_warp_x, rx * 0.5, ry * 0.5, 2, 0.5, 2.0) * range_warp * 2.0;
      let qy = ry + fbm(range_warp_y, rx * 0.5, ry * 0.5, 2, 0.5, 2.0) * range_warp * 2.0;
      ranges.push(smoothstep(
        0.1,
        0.95,
        ridged(range_seed, qx, qy, 2, 0.5, 2.0),
      ));
      mask_noise.push(fbm(mask_seed, rx / 2.2, ry / 2.2, 2, 0.5, 2.0));
      hills.push(fbm(hill_seed, rx * 2.0, ry * 2.0, 3, 0.5, 2.0));
    }
  }

  let threshold = quantile(&raw, 1.0 - landform.land_fraction);
  let land: Vec<bool> = if landform.land_fraction >= 1.0 {
    vec![true; count]
  } else {
    raw.iter().map(|value| *value > threshold).collect()
  };
  let coast_distance = distance_to_coast(size, &land, spacing);
  let land_raw: Vec<f32> = raw
    .iter()
    .zip(&land)
    .filter_map(|(value, is_land)| is_land.then_some(*value))
    .collect();
  let high = quantile(&land_raw, 0.95).max(threshold + 1e-3);
  let max_inland = coast_distance
    .iter()
    .zip(&land)
    .filter_map(|(d, is_land)| is_land.then_some(*d))
    .fold(spacing, f32::max);

  let land_mask: Vec<f32> = mask_noise
    .iter()
    .zip(&land)
    .filter_map(|(value, is_land)| is_land.then_some(*value))
    .collect();
  let mask_threshold = quantile(&land_mask, 1.0 - landform.range_coverage);
  let inland_scale = continent_wavelength * 0.15;
  let sea_scale = continent_wavelength * 0.08;
  let shelf_width = extent * 0.05;

  let mut elevation = vec![0.0f64; count];
  let mut lowlands = vec![0.0f64; count];
  let mut uplift = vec![0.0f32; count];

  for i in 0..count {
    let d = coast_distance[i];

    if !land[i] {
      let depth = if volcanic {
        // A shallow reef shelf with a raised rim, then the drop-off.
        if d < shelf_width {
          let t = d / shelf_width;
          -3.0 - 6.0 * t + 4.0 * smoothstep(0.75, 0.9, t) * (1.0 - smoothstep(0.9, 1.0, t))
        } else {
          -9.0 + (landform.sea_floor + 9.0) * ease((d - shelf_width) / sea_scale)
        }
      } else {
        landform.sea_floor * ease(d / sea_scale)
      };
      elevation[i] = depth.min(-0.5) as f64;
      lowlands[i] = elevation[i];
      continue;
    }

    let inland = ease(d / inland_scale);
    let height = ((raw[i] - threshold) / (high - threshold)).clamp(0.0, 1.0);
    let hill = (hills[i] * 0.5 + 0.5) * inland;
    let lowland = landform.lowland_relief * (0.3 * inland + 0.5 * height + 0.2 * hill);

    let fade = if landform.coastal_cliffs {
      smoothstep(0.0, spacing * 2.0, d)
    } else {
      smoothstep(0.0, max_inland * 0.2, d)
    };
    let mask = if landform.range_coverage <= 0.0 {
      0.0
    } else if landform.range_coverage >= 1.0 {
      1.0
    } else {
      smoothstep(mask_threshold - 0.08, mask_threshold + 0.08, mask_noise[i])
    };
    let mut raised = ranges[i] * mask * fade;

    if volcanic {
      raised = (raised * 0.3).max(cone[i]);
    }

    uplift[i] = raised;
    lowlands[i] = (lowland + 0.5) as f64;
    elevation[i] = lowlands[i] + (raised * landform.mountain_relief) as f64;
  }

  Tectonics {
    size,
    spacing,
    elevation,
    lowland: lowlands,
    uplift,
    land,
    seed,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn land_fraction_is_exact_on_the_coarse_grid() {
    for kind in [
      LandformKind::Continental,
      LandformKind::Archipelago,
      LandformKind::VolcanicIsland,
    ] {
      let landform = Landform::for_extent(kind, 6000.0);
      let base = tectonic_base(3, 256, 6000.0, kind, &landform);
      let fraction = base.land.iter().filter(|l| **l).count() as f32 / base.land.len() as f32;
      assert!(
        (fraction - landform.land_fraction).abs() < 0.01,
        "{kind:?} {fraction}"
      );

      for (elevation, land) in base.elevation.iter().zip(&base.land) {
        assert_eq!(*elevation > 0.0, *land);
      }
    }
  }

  #[test]
  fn ranges_cover_only_part_of_the_land_and_none_of_the_sea() {
    let kind = LandformKind::Continental;
    let landform = Landform::for_extent(kind, 20_000.0);
    let base = tectonic_base(5, 128, 20_000.0, kind, &landform);
    let land = base.land.iter().filter(|l| **l).count() as f32;
    let raised = base.uplift.iter().filter(|u| **u > 0.05).count() as f32;

    assert!(raised > 0.0);
    assert!(raised / land < 0.6);

    for (uplift, land) in base.uplift.iter().zip(&base.land) {
      if !land {
        assert_eq!(*uplift, 0.0);
      }
    }

    let hills = Landform::for_extent(LandformKind::RollingHills, 20_000.0);
    let base = tectonic_base(5, 128, 20_000.0, LandformKind::RollingHills, &hills);
    assert!(base.uplift.iter().all(|u| *u == 0.0));
  }

  #[test]
  fn coast_distance_grows_inland() {
    let mut land = vec![false; 16 * 16];

    for y in 0..16 {
      for x in 8..16 {
        land[y * 16 + x] = true;
      }
    }

    let distance = distance_to_coast(16, &land, 10.0);
    assert_eq!(distance[5 * 16 + 8], 5.0);
    assert_eq!(distance[5 * 16 + 12], 45.0);
    assert_eq!(distance[5 * 16 + 3], 45.0);
  }
}
