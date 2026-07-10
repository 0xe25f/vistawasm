use vista_types::{ErosionOptions, ErosionQuality};

use crate::errors::VistaResult;
use crate::terrain::heightmap::{update_stats, HeightMap};

/// Apply budgeted CPU reference erosion to a heightmap.
pub fn apply_erosion(map: &mut HeightMap, options: &ErosionOptions) -> VistaResult<()> {
  let quality = options.quality.unwrap_or(ErosionQuality::Preview);
  let budget = match quality {
    ErosionQuality::Preview => 16,
    ErosionQuality::Balanced => 64,
    ErosionQuality::High => 160,
    ErosionQuality::Offline => 320,
  };
  let hydraulic = options.hydraulic_iterations.unwrap_or(0).min(budget);
  let thermal = options.thermal_iterations.unwrap_or(0).min(budget);

  for _ in 0..hydraulic {
    hydraulic_step(map, options);
  }

  for _ in 0..thermal {
    thermal_step(map, options);
  }

  update_stats(&map.heights, &map.no_data, &mut map.metadata);
  Ok(())
}

fn hydraulic_step(map: &mut HeightMap, options: &ErosionOptions) {
  let width = map.metadata.width;
  let height = map.metadata.height;
  let len = map.heights.len();
  let rain = options.rain_amount.unwrap_or(0.02).clamp(0.0, 1.0);
  let capacity = options.sediment_capacity.unwrap_or(0.04).clamp(0.0, 1.0);
  let mut delta = vec![0.0; len];

  for y in 1..height.saturating_sub(1) {
    for x in 1..width.saturating_sub(1) {
      let index = (y * width + x) as usize;

      if map.no_data[index] {
        continue;
      }

      let current = map.heights[index];
      let neighbours = [
        (
          (y * width + (x - 1)) as usize,
          map.heights[(y * width + (x - 1)) as usize],
        ),
        (
          (y * width + (x + 1)) as usize,
          map.heights[(y * width + (x + 1)) as usize],
        ),
        (
          ((y - 1) * width + x) as usize,
          map.heights[((y - 1) * width + x) as usize],
        ),
        (
          ((y + 1) * width + x) as usize,
          map.heights[((y + 1) * width + x) as usize],
        ),
      ];

      let Some((target_index, target_height)) = neighbours
        .iter()
        .copied()
        .filter(|(neighbour, _)| !map.no_data[*neighbour])
        .min_by(|a, b| a.1.total_cmp(&b.1))
      else {
        continue;
      };

      let slope = (current - target_height).max(0.0);
      let moved = slope * rain * capacity;
      delta[index] -= moved;
      delta[target_index] += moved * 0.55;
    }
  }

  for (height, change) in map.heights.iter_mut().zip(delta.iter()) {
    *height += *change;
  }
}

fn thermal_step(map: &mut HeightMap, options: &ErosionOptions) {
  let width = map.metadata.width;
  let height = map.metadata.height;
  let len = map.heights.len();
  let talus = options
    .talus_angle_degrees
    .unwrap_or(35.0)
    .to_radians()
    .tan()
    * map.metadata.metres_per_sample
    * 0.15;
  let mut delta = vec![0.0; len];

  for y in 1..height.saturating_sub(1) {
    for x in 1..width.saturating_sub(1) {
      let index = (y * width + x) as usize;

      if map.no_data[index] {
        continue;
      }

      for neighbour in [
        (y * width + (x - 1)) as usize,
        (y * width + (x + 1)) as usize,
        ((y - 1) * width + x) as usize,
        ((y + 1) * width + x) as usize,
      ] {
        if map.no_data[neighbour] {
          continue;
        }

        let difference = map.heights[index] - map.heights[neighbour];

        if difference > talus {
          let move_amount = (difference - talus) * 0.08;
          delta[index] -= move_amount;
          delta[neighbour] += move_amount;
        }
      }
    }
  }

  for (height, change) in map.heights.iter_mut().zip(delta.iter()) {
    *height += *change;
  }
}
