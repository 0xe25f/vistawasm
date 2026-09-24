//! Glacier surface shaping.
//!
//! Ice fills valleys: a glacier's surface is smooth and gently curved where
//! the rock beneath it is rough. Inside the glacier mask, heights are raised
//! towards a blurred copy of the terrain, by at most [`MAX_RAISE_METRES`].
//! Every changed sample is recorded with its original height, so turning
//! the climate warm again, or switching biomes off, restores the terrain
//! exactly.

use vista_types::BiomeOptions;

use crate::terrain::biomes::glacier_mask;
use crate::terrain::heightmap::{update_stats, HeightMap};
use crate::terrain::normals::generate_normals;

/// The furthest a glacier surface is raised above the rock, in metres.
pub const MAX_RAISE_METRES: f32 = 40.0;

/// Radius of the blur that gives the ice its smooth surface, in metres.
const BLUR_RADIUS_METRES: f32 = 60.0;

/// Raise glacier samples towards a smooth ice surface. Returns the original
/// height of every changed sample, for [`restore_glaciers`].
pub fn shape_glaciers(map: &mut HeightMap, options: &BiomeOptions) -> Vec<(usize, f32)> {
  let width = map.metadata.width as usize;
  let height = map.metadata.height as usize;

  if width < 3 || height < 3 || !options.enabled {
    return Vec::new();
  }

  let normals = generate_normals(map);
  let mask = glacier_mask(map, &normals, options);

  if !mask.iter().any(|glacier| *glacier) {
    return Vec::new();
  }

  let metres = map.metadata.metres_per_sample.max(0.001);
  // Three box blurs approximate a Gaussian; each box's half-width is a
  // third of the radius.
  let box_radius = ((BLUR_RADIUS_METRES / metres) / 3.0).round().max(1.0) as usize;
  let surface = blur(&map.heights, width, height, box_radius);
  // A feathered mask keeps the edge of the ice from forming a step.
  let weights: Vec<f32> = mask
    .iter()
    .map(|glacier| if *glacier { 1.0 } else { 0.0 })
    .collect();
  let feather = blur(&weights, width, height, box_radius.div_ceil(2));
  let mut changed = Vec::new();

  for index in 0..map.heights.len() {
    let weight = if mask[index] {
      feather[index].clamp(0.0, 1.0)
    } else {
      0.0
    };

    if weight <= 0.0 || map.no_data[index] {
      continue;
    }

    let original = map.heights[index];
    let raise = (surface[index] - original).clamp(0.0, MAX_RAISE_METRES) * weight;

    if raise > 0.0 {
      changed.push((index, original));
      map.heights[index] = original + raise;
    }
  }

  if !changed.is_empty() {
    update_stats(&map.heights, &map.no_data, &mut map.metadata);
  }

  changed
}

/// Put back every sample changed by [`shape_glaciers`].
pub fn restore_glaciers(map: &mut HeightMap, changed: &[(usize, f32)]) {
  if changed.is_empty() {
    return;
  }

  for (index, height) in changed {
    if let Some(slot) = map.heights.get_mut(*index) {
      *slot = *height;
    }
  }

  update_stats(&map.heights, &map.no_data, &mut map.metadata);
}

/// Three passes of a separable box blur with running sums, so the cost does
/// not grow with the radius.
fn blur(values: &[f32], width: usize, height: usize, radius: usize) -> Vec<f32> {
  let mut current = values.to_vec();
  let mut scratch = vec![0.0; values.len()];

  for _ in 0..3 {
    box_pass(&current, &mut scratch, width, height, radius, true);
    box_pass(&scratch, &mut current, width, height, radius, false);
  }

  current
}

fn box_pass(
  input: &[f32],
  output: &mut [f32],
  width: usize,
  height: usize,
  radius: usize,
  horizontal: bool,
) {
  let (lines, length) = if horizontal {
    (height, width)
  } else {
    (width, height)
  };
  let at = |line: usize, position: usize| {
    if horizontal {
      line * width + position
    } else {
      position * width + line
    }
  };
  let window = (radius * 2 + 1) as f64;

  for line in 0..lines {
    // Edges repeat the outermost sample.
    let sample = |position: isize| {
      let clamped = position.clamp(0, length as isize - 1) as usize;
      input[at(line, clamped)] as f64
    };
    let mut sum = 0.0;

    for offset in -(radius as isize)..=(radius as isize) {
      sum += sample(offset);
    }

    for position in 0..length {
      output[at(line, position)] = (sum / window) as f32;
      let entering = position as isize + radius as isize + 1;
      let leaving = position as isize - radius as isize;
      sum += sample(entering) - sample(leaving);
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use vista_types::TerrainMetadata;

  /// Rough, high ground cut by a deep, narrow valley.
  fn valley_map() -> HeightMap {
    let size = 96;
    let metadata = TerrainMetadata {
      width: size,
      height: size,
      metres_per_sample: 20.0,
      sea_level_metres: 0.0,
      ..TerrainMetadata::default()
    };
    let mut map = HeightMap::flat(size, size, 0.0, metadata);

    for y in 0..size {
      for x in 0..size {
        let across = (x as f32 - 48.0).abs();
        let valley = (across / 6.0).min(1.0) * 90.0;
        let rough = ((x * 7 + y * 13) % 5) as f32 * 3.0;
        let _ = map.set_height(x, y, 400.0 + valley + rough);
      }
    }

    update_stats(&map.heights, &map.no_data, &mut map.metadata);
    map
  }

  fn climate(celsius: Option<f32>) -> BiomeOptions {
    BiomeOptions {
      mean_temperature_celsius: celsius,
      volcanism: 0.0,
      ..BiomeOptions::default()
    }
  }

  #[test]
  fn glaciers_fill_valleys_by_at_most_the_limit() {
    let mut map = valley_map();
    let before = map.heights.clone();
    let changed = shape_glaciers(&mut map, &climate(Some(-15.0)));

    assert!(!changed.is_empty());

    let mut largest: f32 = 0.0;

    for (after, before) in map.heights.iter().zip(&before) {
      assert!(after >= before);
      largest = largest.max(after - before);
    }

    assert!(largest > 1.0);
    assert!(largest <= MAX_RAISE_METRES + 1e-3, "raised {largest} m");
  }

  #[test]
  fn restoring_gives_bit_identical_heights() {
    let mut map = valley_map();
    let original = map.clone();
    let changed = shape_glaciers(&mut map, &climate(Some(-15.0)));

    assert_ne!(map.heights, original.heights);
    restore_glaciers(&mut map, &changed);
    assert_eq!(map, original);
  }

  #[test]
  fn warm_or_disabled_climates_leave_the_terrain_alone() {
    let mut map = valley_map();
    let original = map.clone();

    assert!(shape_glaciers(&mut map, &climate(Some(20.0))).is_empty());
    assert!(shape_glaciers(
      &mut map,
      &BiomeOptions {
        enabled: false,
        ..climate(Some(-15.0))
      }
    )
    .is_empty());
    assert_eq!(map, original);
  }

  #[test]
  fn blur_keeps_a_constant_field() {
    let values = vec![3.5; 40 * 30];
    let blurred = blur(&values, 40, 30, 4);

    assert!(blurred.iter().all(|value| (value - 3.5).abs() < 1e-5));
  }
}
