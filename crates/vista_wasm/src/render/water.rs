//! Water geometry: the open-ocean grid, rivers, and lakes.
//!
//! The ocean is a fixed, camera-following grid whose vertex spacing grows
//! exponentially with distance (like the terrain mesh), so Gerstner waves
//! can displace real geometry near the camera while one draw still reaches
//! the horizon. The vertex shader moves the grid with the camera, so it is
//! built once and never re-uploaded.
//!
//! Rivers and lakes come from the terrain's own drainage network: a
//! priority-flood fills depressions (which become lakes), flow is routed
//! downhill, and upstream catchment area is accumulated. Channels whose
//! catchment exceeds a threshold are traced into smoothed polylines, carved
//! into the heightmap, and turned into ribbons whose vertices carry the
//! local flow direction and speed for the animated current.

use vista_types::{RiverOptions, WaterOptions};

use crate::terrain::drainage;
use crate::terrain::heightmap::HeightMap;

/// CPU mirror of water uniforms used by shaders.
#[derive(Clone, Debug, PartialEq)]
pub struct WaterUniforms {
  /// Sea level in metres.
  pub sea_level_metres: f32,
  /// Wave scale.
  pub wave_scale: f32,
  /// Reflection strength.
  pub reflectivity: f32,
  /// Shoreline blend distance.
  pub shoreline_softness_metres: f32,
}

impl From<&WaterOptions> for WaterUniforms {
  fn from(options: &WaterOptions) -> Self {
    Self {
      sea_level_metres: options.sea_level_metres,
      wave_scale: options.wave_scale,
      reflectivity: options.reflectivity,
      shoreline_softness_metres: options.shoreline_softness_metres,
    }
  }
}

/// Ocean vertex kind (`params[0]`).
pub const WATER_KIND_OCEAN: f32 = 0.0;
/// River vertex kind (`params[0]`).
pub const WATER_KIND_RIVER: f32 = 1.0;
/// Lake vertex kind (`params[0]`).
pub const WATER_KIND_LAKE: f32 = 2.0;

/// One water vertex (32 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct WaterVertex {
  /// Ocean: local offset from the camera-snapped grid origin (y unused).
  /// Rivers and lakes: world position in terrain metres.
  pub position: [f32; 3],
  /// Surface current in metres per second (x, z).
  pub flow: [f32; 2],
  /// x: kind (`WATER_KIND_*`), y: across-channel coordinate (-1 to 1) for
  /// rivers, z: local vertex spacing in metres for the ocean grid.
  pub params: [f32; 3],
}

/// Build a flat quad covering the terrain footprint at the given sea level.
///
/// Winding is counter-clockwise when viewed from above, matching the
/// renderer's `front_face: Ccw` convention. Retained for simple hosts and
/// tests; the renderer itself draws [`build_ocean_grid`].
pub fn build_water_plane(
  half_width_metres: f32,
  half_height_metres: f32,
  sea_level_metres: f32,
) -> [WaterVertex; 6] {
  let y = sea_level_metres;
  let vertex = |x: f32, z: f32| WaterVertex {
    position: [x, y, z],
    flow: [0.0, 0.0],
    params: [WATER_KIND_OCEAN, 0.0, 0.0],
  };
  let a = vertex(-half_width_metres, -half_height_metres);
  let b = vertex(-half_width_metres, half_height_metres);
  let c = vertex(half_width_metres, -half_height_metres);
  let d = vertex(half_width_metres, half_height_metres);

  [a, b, c, b, d, c]
}

/// Grid steps per band before the ocean vertex spacing doubles.
const OCEAN_BAND_WIDTH: i32 = 16;

/// Finest ocean vertex spacing, in metres.
pub const OCEAN_BASE_SPACING_METRES: f32 = 1.5;

/// The ocean grid is recentred in steps of this many metres; must be a
/// multiple of every spacing used inside the displaced region.
pub const OCEAN_SNAP_METRES: f32 = 96.0;

fn ocean_offset(grid_distance: i32, base_spacing: f32, half: i32, far_reach: f32) -> (f32, f32) {
  let sign = grid_distance.signum() as f32;
  let mut remaining = grid_distance.abs();

  if remaining >= half {
    // The outermost ring is pushed out to the horizon.
    return (sign * far_reach, far_reach);
  }

  let mut offset = 0.0;
  let mut step = base_spacing;

  while remaining > 0 {
    let take = remaining.min(OCEAN_BAND_WIDTH);
    offset += take as f32 * step;
    remaining -= take;

    if remaining > 0 {
      step *= 2.0;
    }
  }

  (sign * offset, step)
}

/// Build the camera-following ocean grid. `samples_per_side` must be odd.
///
/// Returns vertices in grid-local metres (the shader adds the snapped
/// camera position) and a triangle list.
pub fn build_ocean_grid(
  samples_per_side: u32,
  far_reach_metres: f32,
) -> (Vec<WaterVertex>, Vec<u32>) {
  let samples = samples_per_side.max(3) | 1;
  let half = (samples / 2) as i32;
  let mut vertices = Vec::with_capacity((samples * samples) as usize);

  for gz in 0..samples as i32 {
    let (z, spacing_z) = ocean_offset(gz - half, OCEAN_BASE_SPACING_METRES, half, far_reach_metres);

    for gx in 0..samples as i32 {
      let (x, spacing_x) =
        ocean_offset(gx - half, OCEAN_BASE_SPACING_METRES, half, far_reach_metres);

      vertices.push(WaterVertex {
        position: [x, 0.0, z],
        flow: [0.0, 0.0],
        params: [WATER_KIND_OCEAN, 0.0, spacing_x.max(spacing_z)],
      });
    }
  }

  let quads = samples - 1;
  let mut indices = Vec::with_capacity((quads * quads * 6) as usize);

  for gz in 0..quads {
    for gx in 0..quads {
      let a = gz * samples + gx;
      let b = a + 1;
      let c = a + samples;
      let d = c + 1;
      indices.extend_from_slice(&[a, c, b, b, c, d]);
    }
  }

  (vertices, indices)
}

/// Rivers and lakes derived from a heightmap.
#[derive(Clone, Debug, Default)]
pub struct RiverNetwork {
  /// River ribbon and lake vertices in world metres.
  pub vertices: Vec<WaterVertex>,
  /// Triangle list indices.
  pub indices: Vec<u32>,
  /// Full-resolution mask of samples under a river channel or lake.
  pub mask: Vec<bool>,
  /// Original heights of every carved sample, for restoring the terrain.
  pub carved: Vec<(usize, f32)>,
  /// Number of traced river polylines.
  pub river_count: u32,
}

/// Maximum number of traced river sources.
const MAX_RIVER_SOURCES: usize = 600;

/// Maximum flow grid samples per side.
const MAX_FLOW_GRID: u32 = 512;

struct FlowGrid {
  width: u32,
  height: u32,
  stride: u32,
  heights: Vec<f64>,
  filled: Vec<f64>,
  receiver: Vec<u32>,
  accumulation: Vec<f32>,
}

impl FlowGrid {
  fn new(map: &HeightMap) -> Self {
    let map_width = map.metadata.width;
    let map_height = map.metadata.height;
    let stride = (map_width.max(map_height).saturating_sub(1) / (MAX_FLOW_GRID - 1)).max(1);
    let width = (map_width - 1) / stride + 1;
    let height = (map_height - 1) / stride + 1;
    let sea = map.metadata.sea_level_metres as f64;
    let count = (width * height) as usize;
    let mut heights = Vec::with_capacity(count);

    for gy in 0..height {
      for gx in 0..width {
        let index = ((gy * stride) * map_width + gx * stride) as usize;
        heights.push(if map.no_data[index] {
          sea - 1.0
        } else {
          map.heights[index] as f64
        });
      }
    }

    let mut grid = Self {
      width,
      height,
      stride,
      filled: heights.clone(),
      heights,
      receiver: vec![u32::MAX; count],
      accumulation: vec![1.0; count],
    };
    grid.flood(sea);
    grid
  }

  fn neighbours(&self, index: u32) -> impl Iterator<Item = u32> {
    drainage::neighbours(self.width, self.height, index)
  }

  /// Fill depressions, route each cell to its steepest downhill
  /// neighbour, and accumulate upstream catchment.
  fn flood(&mut self, sea: f64) {
    let flood = drainage::priority_flood(
      self.width,
      self.height,
      &self.heights,
      // A tiny gradient across filled flats so water always has somewhere
      // to go; small enough to be invisible on lake surfaces.
      1e-4,
      drainage::edge_or_sea_outlet(self.width, self.height, &self.heights, sea),
    );
    self.filled = flood.filled;
    self.receiver = flood.receiver;
    drainage::steepest_receivers(self.width, self.height, &self.filled, &mut self.receiver);
    self.accumulation = drainage::accumulate(
      &flood.order,
      &self.receiver,
      std::mem::take(&mut self.accumulation),
    );
  }
}

/// Extract rivers and lakes from `map`, carving river channels into it.
///
/// The original height of every carved sample is recorded in
/// [`RiverNetwork::carved`] so the caller can restore the terrain later.
pub fn build_river_network(map: &mut HeightMap, options: &RiverOptions) -> RiverNetwork {
  let map_width = map.metadata.width;
  let map_height = map.metadata.height;
  let mut network = RiverNetwork {
    mask: vec![false; map.heights.len()],
    ..RiverNetwork::default()
  };

  if !options.enabled || map_width < 8 || map_height < 8 {
    return network;
  }

  let grid = FlowGrid::new(map);
  let metres_per_sample = map.metadata.metres_per_sample.max(0.001);
  let cell_metres = grid.stride as f32 * metres_per_sample;
  let cell_area_km2 = cell_metres * cell_metres / 1_000_000.0;
  let min_cells = (options.min_catchment_km2.max(0.01) / cell_area_km2).max(24.0);
  let sea = map.metadata.sea_level_metres;
  let half_width = (map_width as f32 - 1.0) * metres_per_sample * 0.5;
  let half_height = (map_height as f32 - 1.0) * metres_per_sample * 0.5;
  let count = grid.filled.len();
  let is_river = |i: usize| grid.accumulation[i] >= min_cells && grid.heights[i] > sea as f64;

  // A cell is a source when no river cell drains into it.
  let mut has_upstream = vec![false; count];

  for i in 0..count {
    if is_river(i) && grid.receiver[i] != u32::MAX {
      has_upstream[grid.receiver[i] as usize] = true;
    }
  }

  let mut sources: Vec<usize> = (0..count)
    .filter(|i| is_river(*i) && !has_upstream[*i])
    .collect();
  // Trace the largest catchments first so main stems are continuous and
  // tributaries end where they meet them.
  sources.sort_by(|a, b| {
    grid.accumulation[*b]
      .total_cmp(&grid.accumulation[*a])
      .then_with(|| a.cmp(b))
  });

  // Keep the network readable (and the vertex count bounded) on very large
  // terrain: only the largest catchments get their own river.
  sources.truncate(MAX_RIVER_SOURCES);

  let mut visited = vec![false; count];
  let mut original: Vec<Option<f32>> = vec![None; map.heights.len()];
  let width_scale = options.width_scale.clamp(0.1, 10.0);
  let current_scale = options.current_speed.max(0.0);

  for source in sources {
    // Walk downstream collecting (sample x, sample y, level, catchment).
    let mut path: Vec<([f32; 2], f32, f32)> = Vec::new();
    let mut cell = source;

    loop {
      let gx = (cell as u32 % grid.width) as f32 * grid.stride as f32;
      let gy = (cell as u32 / grid.width) as f32 * grid.stride as f32;
      let level = if grid.heights[cell] <= sea as f64 {
        sea
      } else {
        grid.filled[cell] as f32
      };
      path.push(([gx, gy], level, grid.accumulation[cell] * cell_area_km2));

      if visited[cell] || grid.heights[cell] <= sea as f64 {
        break;
      }

      visited[cell] = true;
      let next = grid.receiver[cell];

      if next == u32::MAX {
        break;
      }

      cell = next as usize;
    }

    if path.len() < 4 {
      continue;
    }

    let path = smooth_path(&path);
    network.river_count += 1;
    add_river_ribbon(
      &mut network,
      &path,
      metres_per_sample,
      half_width,
      half_height,
      width_scale,
      current_scale,
    );
    carve_river(
      map,
      &mut original,
      &mut network.mask,
      &path,
      metres_per_sample,
      width_scale,
    );
  }

  add_lakes(
    &mut network,
    &grid,
    map,
    metres_per_sample,
    half_width,
    half_height,
  );

  network.carved = original
    .iter()
    .enumerate()
    .filter_map(|(index, height)| height.map(|h| (index, h)))
    .collect();
  crate::terrain::heightmap::update_stats(&map.heights, &map.no_data, &mut map.metadata);
  network
}

/// Restore every sample carved by [`build_river_network`].
pub fn restore_carving(map: &mut HeightMap, carved: &[(usize, f32)]) {
  for (index, height) in carved {
    if let Some(slot) = map.heights.get_mut(*index) {
      *slot = *height;
    }
  }

  crate::terrain::heightmap::update_stats(&map.heights, &map.no_data, &mut map.metadata);
}

/// One Chaikin corner-cutting pass, which turns the 45-degree zig-zags of
/// eight-way flow routing into flowing curves. Levels are kept
/// non-increasing downstream so the water surface never runs uphill.
fn smooth_path(path: &[([f32; 2], f32, f32)]) -> Vec<([f32; 2], f32, f32)> {
  let mut smoothed = Vec::with_capacity(path.len() * 2);
  smoothed.push(path[0]);

  for pair in path.windows(2) {
    let (a, b) = (pair[0], pair[1]);
    let lerp = |t: f32| {
      (
        [
          a.0[0] + (b.0[0] - a.0[0]) * t,
          a.0[1] + (b.0[1] - a.0[1]) * t,
        ],
        a.1 + (b.1 - a.1) * t,
        a.2 + (b.2 - a.2) * t,
      )
    };
    smoothed.push(lerp(0.25));
    smoothed.push(lerp(0.75));
  }

  smoothed.push(path[path.len() - 1]);

  for i in 1..smoothed.len() {
    smoothed[i].1 = smoothed[i].1.min(smoothed[i - 1].1);
  }

  smoothed
}

fn river_width_metres(catchment_km2: f32, width_scale: f32) -> f32 {
  ((3.0 + catchment_km2.sqrt() * 2.6) * width_scale).clamp(2.0, 90.0)
}

fn add_river_ribbon(
  network: &mut RiverNetwork,
  path: &[([f32; 2], f32, f32)],
  metres_per_sample: f32,
  half_width: f32,
  half_height: f32,
  width_scale: f32,
  current_scale: f32,
) {
  let first = network.vertices.len() as u32;
  let world = |p: [f32; 2]| {
    [
      p[0] * metres_per_sample - half_width,
      p[1] * metres_per_sample - half_height,
    ]
  };

  for (i, (point, level, catchment)) in path.iter().enumerate() {
    let previous = world(path[i.saturating_sub(1)].0);
    let next = world(path[(i + 1).min(path.len() - 1)].0);
    let mut tangent = [next[0] - previous[0], next[1] - previous[1]];
    let tangent_length = (tangent[0] * tangent[0] + tangent[1] * tangent[1])
      .sqrt()
      .max(0.0001);
    tangent = [tangent[0] / tangent_length, tangent[1] / tangent_length];
    let side = [-tangent[1], tangent[0]];
    let centre = world(*point);
    let width = river_width_metres(*catchment, width_scale);
    // Speed follows the local surface gradient: steep reaches race, flat
    // lowland reaches meander slowly.
    let previous_level = path[i.saturating_sub(1)].1;
    let next_level = path[(i + 1).min(path.len() - 1)].1;
    let gradient = ((previous_level - next_level) / tangent_length).max(0.0);
    let speed = (0.35 + gradient.sqrt() * 9.0).clamp(0.35, 4.0) * current_scale;
    let flow = [tangent[0] * speed, tangent[1] * speed];
    let y = level + 0.05;

    for across in [-1.0f32, 1.0] {
      network.vertices.push(WaterVertex {
        position: [
          centre[0] + side[0] * width * 0.5 * across,
          y,
          centre[1] + side[1] * width * 0.5 * across,
        ],
        flow,
        params: [WATER_KIND_RIVER, across, 0.0],
      });
    }
  }

  for i in 0..(path.len() as u32 - 1) {
    let a = first + i * 2;
    let b = a + 1;
    let c = a + 2;
    let d = a + 3;
    network.indices.extend_from_slice(&[a, b, c, c, b, d]);
  }
}

fn carve_river(
  map: &mut HeightMap,
  original: &mut [Option<f32>],
  mask: &mut [bool],
  path: &[([f32; 2], f32, f32)],
  metres_per_sample: f32,
  width_scale: f32,
) {
  let width = map.metadata.width as i32;
  let height = map.metadata.height as i32;

  for pair in path.windows(2) {
    let (a, b) = (pair[0], pair[1]);
    let river_width = river_width_metres(a.2.max(b.2), width_scale);
    let radius = river_width * 0.5 / metres_per_sample;
    let bank = 1.5_f32.max(radius * 0.5);
    let depth = (0.7 + river_width * 0.04).min(4.0);
    let reach = (radius + bank).ceil() as i32 + 1;
    let min_x = (a.0[0].min(b.0[0]).floor() as i32 - reach).max(0);
    let max_x = (a.0[0].max(b.0[0]).ceil() as i32 + reach).min(width - 1);
    let min_y = (a.0[1].min(b.0[1]).floor() as i32 - reach).max(0);
    let max_y = (a.0[1].max(b.0[1]).ceil() as i32 + reach).min(height - 1);
    let segment = [b.0[0] - a.0[0], b.0[1] - a.0[1]];
    let segment_length_sq = (segment[0] * segment[0] + segment[1] * segment[1]).max(1e-6);

    for y in min_y..=max_y {
      for x in min_x..=max_x {
        let px = x as f32 - a.0[0];
        let py = y as f32 - a.0[1];
        let t = ((px * segment[0] + py * segment[1]) / segment_length_sq).clamp(0.0, 1.0);
        let dx = px - segment[0] * t;
        let dy = py - segment[1] * t;
        let distance = (dx * dx + dy * dy).sqrt();

        if distance > radius + bank {
          continue;
        }

        let level = a.1 + (b.1 - a.1) * t;
        let target = if distance <= radius {
          let u = distance / radius.max(0.001);
          level - depth * (1.0 - u * u) - 0.15
        } else {
          level - 0.15 + (distance - radius) / bank * (depth * 0.6 + 0.8)
        };
        let index = (y * width + x) as usize;

        if map.no_data[index] {
          continue;
        }

        if distance <= radius {
          mask[index] = true;
        }

        if target < map.heights[index] {
          if original[index].is_none() {
            original[index] = Some(map.heights[index]);
          }

          map.heights[index] = target;
        }
      }
    }
  }
}

/// Minimum lake size, in flow-grid cells, before a filled depression is
/// drawn as a lake. Smaller pits stay dry so rugged terrain is not dotted
/// with puddles.
const MIN_LAKE_CELLS: usize = 24;

/// Minimum depth of a lake's deepest point, in metres.
const MIN_LAKE_DEPTH_METRES: f64 = 1.5;

fn add_lakes(
  network: &mut RiverNetwork,
  grid: &FlowGrid,
  map: &HeightMap,
  metres_per_sample: f32,
  half_width: f32,
  half_height: f32,
) {
  let sea = map.metadata.sea_level_metres as f64;
  let half_cell = grid.stride as f32 * metres_per_sample * 0.5;
  let map_width = map.metadata.width;
  let mask_radius = (grid.stride / 2) as i32;
  let count = grid.filled.len();
  let is_lake = |i: usize| grid.filled[i] - grid.heights[i] >= 0.35 && grid.heights[i] > sea;
  let mut component = vec![u32::MAX; count];
  let mut keep = Vec::new();

  // Label connected lake cells so tiny pits can be discarded.
  for start in 0..count {
    if component[start] != u32::MAX || !is_lake(start) {
      continue;
    }

    let label = keep.len() as u32;
    let mut stack = vec![start];
    let mut cells = 0usize;
    let mut deepest = 0.0f64;
    component[start] = label;

    while let Some(cell) = stack.pop() {
      cells += 1;
      deepest = deepest.max(grid.filled[cell] - grid.heights[cell]);

      for neighbour in grid.neighbours(cell as u32) {
        let n = neighbour as usize;

        if component[n] == u32::MAX && is_lake(n) {
          component[n] = label;
          stack.push(n);
        }
      }
    }

    keep.push(cells >= MIN_LAKE_CELLS && deepest >= MIN_LAKE_DEPTH_METRES);
  }

  for index in 0..count {
    if component[index] == u32::MAX || !keep[component[index] as usize] {
      continue;
    }

    let gx = index as u32 % grid.width;
    let gy = index as u32 / grid.width;
    let centre_x = (gx * grid.stride) as f32 * metres_per_sample - half_width;
    let centre_z = (gy * grid.stride) as f32 * metres_per_sample - half_height;
    let y = grid.filled[index] as f32;
    let first = network.vertices.len() as u32;

    for (dx, dz) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
      network.vertices.push(WaterVertex {
        position: [centre_x + dx * half_cell, y, centre_z + dz * half_cell],
        flow: [0.0, 0.0],
        params: [WATER_KIND_LAKE, 0.0, 0.0],
      });
    }

    network.indices.extend_from_slice(&[
      first,
      first + 2,
      first + 1,
      first + 1,
      first + 2,
      first + 3,
    ]);

    for oy in -mask_radius..=mask_radius {
      for ox in -mask_radius..=mask_radius {
        let sx = (gx * grid.stride) as i32 + ox;
        let sy = (gy * grid.stride) as i32 + oy;

        if sx >= 0 && sy >= 0 && (sx as u32) < map_width && (sy as u32) < map.metadata.height {
          let sample = (sy as u32 * map_width + sx as u32) as usize;

          if (map.heights[sample] as f64) < grid.filled[index] {
            network.mask[sample] = true;
          }
        }
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::terrain::heightmap::update_stats;
  use vista_types::TerrainMetadata;

  #[test]
  fn plane_sits_at_the_requested_sea_level() {
    let plane = build_water_plane(100.0, 50.0, 12.5);

    assert!(plane.iter().all(|vertex| vertex.position[1] == 12.5));
  }

  #[test]
  fn plane_spans_the_requested_footprint() {
    let plane = build_water_plane(100.0, 50.0, 0.0);
    let xs: Vec<f32> = plane.iter().map(|vertex| vertex.position[0]).collect();
    let zs: Vec<f32> = plane.iter().map(|vertex| vertex.position[2]).collect();

    assert!(xs.contains(&-100.0) && xs.contains(&100.0));
    assert!(zs.contains(&-50.0) && zs.contains(&50.0));
  }

  #[test]
  fn ocean_grid_is_fine_near_the_centre_and_reaches_the_horizon() {
    let (vertices, indices) = build_ocean_grid(129, 50_000.0);

    assert_eq!(vertices.len(), 129 * 129);
    assert_eq!(indices.len(), 128 * 128 * 6);
    let centre = &vertices[64 * 129 + 64];
    assert_eq!(centre.position[0], 0.0);
    assert!(centre.params[2] <= OCEAN_BASE_SPACING_METRES * 1.01);
    assert!(vertices.iter().any(|vertex| vertex.position[0] >= 50_000.0));
  }

  #[test]
  fn ocean_band_spacing_divides_the_snap_distance() {
    // Grid vertices inside the displaced region must stay put when the
    // grid origin jumps by `OCEAN_SNAP_METRES`.
    let mut step = OCEAN_BASE_SPACING_METRES;

    for _ in 0..6 {
      assert!((OCEAN_SNAP_METRES / step).fract().abs() < 1e-4);
      step *= 2.0;
    }
  }

  fn valley_map() -> HeightMap {
    let size = 96;
    let metadata = TerrainMetadata {
      width: size,
      height: size,
      metres_per_sample: 40.0,
      sea_level_metres: 0.0,
      ..TerrainMetadata::default()
    };
    let mut map = HeightMap::flat(size, size, 0.0, metadata);

    // A V-shaped valley draining north to the sea.
    for y in 0..size {
      for x in 0..size {
        let across = (x as f32 - 48.0).abs() * 6.0;
        let along = y as f32 * 4.0 - 20.0;
        let _ = map.set_height(x, y, along + across);
      }
    }

    update_stats(&map.heights, &map.no_data, &mut map.metadata);
    map
  }

  #[test]
  fn rivers_follow_the_valley_and_are_carved() {
    let mut map = valley_map();
    let before = map.heights.clone();
    let options = RiverOptions {
      min_catchment_km2: 0.5,
      ..RiverOptions::default()
    };
    let network = build_river_network(&mut map, &options);

    assert!(network.river_count > 0);
    assert!(!network.indices.is_empty());
    assert!(!network.carved.is_empty());
    assert!(network.mask.iter().any(|value| *value));

    // River vertices hug the valley floor.
    let river_x: Vec<f32> = network
      .vertices
      .iter()
      .filter(|vertex| vertex.params[0] == WATER_KIND_RIVER)
      .map(|vertex| vertex.position[0])
      .collect();
    let mean_x = river_x.iter().sum::<f32>() / river_x.len() as f32;
    assert!(mean_x.abs() < 400.0, "mean river x {mean_x}");

    // Every river vertex flows north (towards lower y / negative z).
    assert!(network
      .vertices
      .iter()
      .filter(|vertex| vertex.params[0] == WATER_KIND_RIVER)
      .all(|vertex| vertex.flow[1] <= 0.01));

    restore_carving(&mut map, &network.carved);
    assert_eq!(map.heights, before);
  }

  #[test]
  fn river_extraction_is_deterministic() {
    let options = RiverOptions {
      min_catchment_km2: 0.5,
      ..RiverOptions::default()
    };
    let mut first_map = valley_map();
    let mut second_map = valley_map();
    let first = build_river_network(&mut first_map, &options);
    let second = build_river_network(&mut second_map, &options);

    assert_eq!(first.vertices, second.vertices);
    assert_eq!(first_map.heights, second_map.heights);
  }

  #[test]
  fn disabled_rivers_leave_the_terrain_untouched() {
    let mut map = valley_map();
    let before = map.heights.clone();
    let options = RiverOptions {
      enabled: false,
      ..RiverOptions::default()
    };
    let network = build_river_network(&mut map, &options);

    assert!(network.vertices.is_empty());
    assert_eq!(map.heights, before);
  }

  #[test]
  fn closed_basins_become_lakes() {
    let size = 64;
    let metadata = TerrainMetadata {
      width: size,
      height: size,
      metres_per_sample: 20.0,
      sea_level_metres: 0.0,
      ..TerrainMetadata::default()
    };
    let mut map = HeightMap::flat(size, size, 100.0, metadata);

    for y in 20..44 {
      for x in 20..44 {
        let _ = map.set_height(x, y, 80.0);
      }
    }

    update_stats(&map.heights, &map.no_data, &mut map.metadata);
    let network = build_river_network(&mut map, &RiverOptions::default());

    assert!(network.vertices.iter().any(
      |vertex| vertex.params[0] == WATER_KIND_LAKE && (vertex.position[1] - 100.0).abs() < 0.5
    ));
  }
}
