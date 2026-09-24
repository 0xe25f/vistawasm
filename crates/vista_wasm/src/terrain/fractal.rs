use std::sync::Arc;

use vista_types::{FractalTerrainOptions, LandformKind, NoiseKind, TerrainEdges, TerrainMetadata};

use crate::errors::VistaResult;
use crate::maths::{hash_u64, lerp};
use crate::terrain::drainage::{
  accumulate, edge_or_sea_outlet, neighbours, priority_flood, stack_order, steepest_receivers,
};
use crate::terrain::erosion::apply_erosion;
use crate::terrain::heightmap::{update_stats, HeightMap, TerrainAux};
use crate::terrain::landforms::Landform;
use crate::terrain::noise::{noise_seed, simplex, simplex_d};
use crate::terrain::stream_power::{
  nth_value, plane_valley_floors, positive_p99, resample, stream_power,
  stream_power_coarse_to_fine, StreamPowerOptions, AREA_EXPONENT, CREST_PASSES,
  RANGE_AREA_EXPONENT, SETTLING_ITERATIONS,
};
use crate::terrain::tectonics::{tectonic_base, Tectonics, COAST_RIM, COAST_RIM_MIN};

const GENERATOR_VERSION: &str = "vistawasm-fractal-0.2.0";

/// Receives `(phase, progress)` as generation advances, with `progress`
/// from 0 to 1 within each phase.
pub type Progress<'a> = &'a mut dyn FnMut(&str, f32);

/// Stream-power iterations on the coarse grid.
const STREAM_POWER_ITERATIONS: u32 = 40;

/// Over-deepening of the largest glacial troughs, as a fraction of the
/// mountain relief.
const GLACIAL_DEEPENING: f64 = 0.18;

/// Degrees by which threshold hillslopes stand below the talus angle.
const HILLSLOPE_BELOW_TALUS: f32 = 6.0;

/// Stream-power iterations for the lowlands, which erode for a limited
/// time rather than to a steady state.
const LOWLAND_ITERATIONS: u32 = 10;

/// Lowland erosion rate per iteration for a cell draining only itself.
const LOWLAND_RATE: f64 = 0.01;

/// Lakes and closed basins smaller than this many samples are filled, so
/// that the finished map drains (see [`remove_small_pits`]).
pub const MIN_BASIN_SAMPLES: usize = 24;

/// Generate a deterministic heightmap on the CPU, including the CPU
/// reference erosion if requested.
///
/// Browser builds run erosion on the GPU instead (see
/// `render::erosion_compute`): they call [`generate_fractal_heightmap_base`],
/// erode, then [`finish_fractal_heightmap`]. This function is the
/// single-call path used by native builds, tests, and as a fallback.
pub fn generate_fractal_heightmap(options: &FractalTerrainOptions) -> VistaResult<HeightMap> {
  generate_fractal_heightmap_with_progress(options, &mut |_, _| {})
}

/// [`generate_fractal_heightmap`] with progress reporting.
pub fn generate_fractal_heightmap_with_progress(
  options: &FractalTerrainOptions,
  progress: Progress<'_>,
) -> VistaResult<HeightMap> {
  let mut map = generate_fractal_heightmap_base_with_progress(options, progress)?;

  if let Some(erosion) = &options.erosion {
    apply_erosion(&mut map, erosion, &fractal_landform(options), progress)?;
  }

  progress("finishing", 0.0);
  finish_fractal_heightmap(&mut map, options);
  Ok(map)
}

/// The landform preset fitted to the map described by `options`.
pub fn fractal_landform(options: &FractalTerrainOptions) -> Landform {
  let extent = (options.size.max(2) - 1) as f32 * options.horizontal_scale_metres;
  Landform::for_extent(options.landform, extent)
}

/// Generate stages A to C (tectonics, stream power, and detail) without
/// erosion, so a caller can erode separately (for example, on the GPU).
/// Call [`finish_fractal_heightmap`] afterwards.
pub fn generate_fractal_heightmap_base(options: &FractalTerrainOptions) -> VistaResult<HeightMap> {
  generate_fractal_heightmap_base_with_progress(options, &mut |_, _| {})
}

/// [`generate_fractal_heightmap_base`] with progress reporting.
pub fn generate_fractal_heightmap_base_with_progress(
  options: &FractalTerrainOptions,
  progress: Progress<'_>,
) -> VistaResult<HeightMap> {
  crate::config::validate_fractal(options)?;

  let size = options.size;
  let spacing = options.horizontal_scale_metres;
  let extent = (size - 1) as f32 * spacing;
  let landform = fractal_landform(options);
  let sea = options.sea_level_metres.unwrap_or(0.0);

  // Stage A: continents and uplift on the coarse grid.
  progress("tectonics", 0.0);
  let mut base = tectonic_base(
    options.seed,
    size,
    extent,
    options.landform,
    &landform,
    options.edges == TerrainEdges::Coast,
  );
  progress("tectonics", 1.0);

  // Stage B: carve the valley network into the coarse grid.
  progress("drainage", 0.0);
  let spacing_coarse = base.spacing as f64;
  // Hillslopes stand a little below the angle of repose; the talus angle
  // itself is left for scree below cliffs in stage D.
  let threshold_slope = ((landform.talus_angle_degrees - HILLSLOPE_BELOW_TALUS) as f64)
    .to_radians()
    .tan();

  // Lowlands erode for a limited time, so big rivers open broad vales
  // while the plains between them keep their gentle relief.
  let mut coarse = std::mem::take(&mut base.lowland);
  erode_lowlands(
    base.size,
    spacing_coarse,
    &mut coarse,
    &StreamPowerOptions {
      iterations: LOWLAND_ITERATIONS,
      seed: options.seed,
      rate: Some(LOWLAND_RATE * landform.erodibility as f64),
      erodibility: 1.0,
      diffusion: landform.hillslope_diffusion as f64,
      glacial: 0.0,
      snowline: f64::INFINITY,
      threshold_slope,
      area_exponent: AREA_EXPONENT,
      target_relief: 0.0,
      settling_iterations: SETTLING_ITERATIONS,
      crest_passes: CREST_PASSES,
    },
  );
  progress("drainage", 0.3);

  // Ranges reach the steady state between uplift and erosion, standing on
  // the lowlands: anywhere without uplift is their base level.
  let mountain_relief = landform.mountain_relief as f64;
  // The belt's block is carved with the ranges: real ranges rise from
  // high ground, and their valleys are cut into it.
  let mut mountains: Vec<f64> = base
    .uplift
    .iter()
    .zip(&base.massif)
    .map(|(u, block)| {
      let u = *u + *block * landform.massif;

      if u > 0.02 {
        u as f64 * mountain_relief
      } else {
        0.0
      }
    })
    .collect();
  let uplift: Vec<f64> = mountains.iter().map(|m| m * 0.1).collect();
  let target_relief = positive_p99(&mountains);

  if target_relief > 0.0 {
    let snowline = landform.lowland_relief as f64 * 0.5 + 0.3 * mountain_relief;
    stream_power_coarse_to_fine(
      base.size,
      spacing_coarse,
      &mut mountains,
      &uplift,
      &StreamPowerOptions {
        iterations: STREAM_POWER_ITERATIONS,
        seed: options.seed ^ 0x6d6f_756e,
        rate: None,
        erodibility: landform.erodibility as f64,
        diffusion: landform.hillslope_diffusion as f64,
        glacial: landform.glacial as f64,
        snowline: snowline.max(1.0),
        threshold_slope,
        area_exponent: RANGE_AREA_EXPONENT,
        target_relief,
        settling_iterations: SETTLING_ITERATIONS,
        crest_passes: CREST_PASSES,
      },
    );
  }
  // Holding hillslopes at the threshold cuts the solved ranges well below
  // the relief the landform allows; raise them part of the way back. This
  // keeps every ridge and valley, steepening the hillslopes by at most a
  // quarter.
  if target_relief > 0.0 {
    let scale = (0.8 * target_relief / positive_p99(&mountains).max(1.0)).clamp(1.0, 1.25);

    for mountain in &mut mountains {
      *mountain *= scale;
    }
  }

  for (height, mountain) in coarse.iter_mut().zip(&mountains) {
    *height += *mountain;
  }

  let area = coarse_drainage_area(base.size, spacing_coarse, &coarse);
  plane_valley_floors(
    base.size,
    spacing_coarse,
    &mut coarse,
    &area,
    landform.glacial as f64,
    mountain_relief * GLACIAL_DEEPENING,
    threshold_slope,
  );
  smooth_land(base.size as usize, &mut coarse, COARSE_SMOOTHING);
  relevel_sea(&mut coarse, landform.land_fraction);
  breach_thin_land(base.size as usize, &mut coarse);
  // Kept for erosion and later stages. Floors, smoothing and breaching
  // barely move the drainage network, so it is not routed again.
  let drainage_area = area;

  if options.landform == LandformKind::VolcanicIsland {
    carve_caldera(&mut coarse, base.size, base.summit, &landform);
  }

  progress("drainage", 1.0);
  // Stage C: full-resolution detail.
  progress("detail", 0.0);
  let mut heights = add_detail(options, &landform, &base, &coarse);

  if options.edges == TerrainEdges::Coast {
    shelve_border(size as usize, spacing, &mut heights, &landform, 0.0);
  }

  progress("detail", 1.0);

  let relief = (landform.lowland_relief + landform.mountain_relief).max(100.0);
  let vertical_scale = options.vertical_scale;
  let base_height = options.base_height_metres.unwrap_or(0.0);

  for y in 0..size {
    for x in 0..size {
      let index = (y * size + x) as usize;
      let nx = x as f32 / (size - 1) as f32;
      let ny = y as f32 / (size - 1) as f32;
      // Shape masks work in units of the landform's relief, as the older
      // generator's did in units of its fixed 900 m range.
      let normalised = shape_noise(options, nx, ny, heights[index] / relief);
      heights[index] = sea + base_height + normalised * relief * vertical_scale;
    }
  }

  let metadata = TerrainMetadata {
    width: size,
    height: size,
    metres_per_sample: spacing,
    vertical_scale,
    sea_level_metres: sea,
    source: "fractal".to_string(),
    generator_version: GENERATOR_VERSION.to_string(),
    ..TerrainMetadata::default()
  };
  let mut map = HeightMap::from_values(
    size,
    size,
    heights,
    vec![false; (size * size) as usize],
    metadata,
  )?;
  map.aux = Some(Arc::new(TerrainAux {
    size: base.size,
    drainage_area,
  }));
  Ok(map)
}

/// Islands smaller than this many samples are drowned by
/// [`remove_islets`].
pub const MIN_ISLET_SAMPLES: usize = 6;

/// Summits less than this fraction of the sample spacing above their col
/// are levelled by [`remove_minor_summits`].
pub const MINOR_SUMMIT_FRACTION: f32 = 0.3;

/// Final conditioning after erosion: shelve the border again for coast
/// edges (erosion slumps coastal land into a thin strip of sea), drown
/// specks of land, level minor summits, and fill closed depressions
/// smaller than [`MIN_BASIN_SAMPLES`], then refresh statistics.
pub fn finish_fractal_heightmap(map: &mut HeightMap, options: &FractalTerrainOptions) {
  if options.edges == TerrainEdges::Coast {
    shelve_border(
      map.metadata.width as usize,
      map.metadata.metres_per_sample,
      &mut map.heights,
      &fractal_landform(options),
      map.metadata.sea_level_metres,
    );
  }

  let prominence = map.metadata.metres_per_sample * MINOR_SUMMIT_FRACTION;
  remove_islets(map, MIN_ISLET_SAMPLES);
  remove_minor_summits(map, prominence);
  remove_small_pits(map, MIN_BASIN_SAMPLES);
  update_stats(&map.heights, &map.no_data, &mut map.metadata);
}

/// Drown islands of fewer than `min_samples` samples. Detail and
/// upsampling leave single samples poking through the sea surface off
/// low coasts, which read as specks rather than islands.
pub fn remove_islets(map: &mut HeightMap, min_samples: usize) {
  let width = map.metadata.width;
  let height = map.metadata.height;
  let sea = map.metadata.sea_level_metres;
  let mut seen = vec![false; map.heights.len()];
  let mut members = Vec::new();

  for start in 0..map.heights.len() {
    if seen[start] || map.heights[start] <= sea {
      continue;
    }

    members.clear();
    let mut stack = vec![start];
    seen[start] = true;

    while let Some(cell) = stack.pop() {
      members.push(cell);

      for neighbour in neighbours(width, height, cell as u32) {
        let n = neighbour as usize;

        if !seen[n] && map.heights[n] > sea {
          seen[n] = true;
          stack.push(n);
        }
      }
    }

    if members.len() < min_samples {
      for cell in &members {
        map.heights[*cell] = sea - 0.5;
      }
    }
  }
}

/// Level every summit less than `max_prominence` metres above its col
/// (the highest pass leading to higher ground). Noise, bicubic upsampling
/// and deposition leave crests and plains dotted with bumps a fraction of
/// a metre high; each would read as a separate peak. The summit is cut
/// down level with its col. Real summits are far more prominent and are
/// untouched.
pub fn remove_minor_summits(map: &mut HeightMap, max_prominence: f32) {
  use std::collections::BinaryHeap;

  // Summits not resolved within this many samples are major peaks.
  const SEARCH_LIMIT: usize = 4096;
  let width = map.metadata.width;
  let height = map.metadata.height;
  let heights = &mut map.heights;
  // The search that last visited each sample, so no per-search clearing.
  let mut visited = vec![u32::MAX; heights.len()];
  let mut region: Vec<u32> = Vec::new();

  for start in 0..heights.len() as u32 {
    let peak = heights[start as usize];

    if neighbours(width, height, start).any(|n| heights[n as usize] >= peak)
      || neighbours(width, height, start).count() < 8
    {
      continue;
    }

    // Best-first search that always steps to the highest unvisited cell,
    // so it descends no further than it must to reach higher ground.
    let mut frontier = BinaryHeap::new();
    region.clear();
    visited[start as usize] = start;
    frontier.push((OrderedHeight(peak), start));
    let mut col = peak;
    let mut escape: Option<u32> = None;

    while let Some((OrderedHeight(level), cell)) = frontier.pop() {
      if level > peak {
        escape = Some(cell);
        break;
      }

      if region.len() >= SEARCH_LIMIT || peak - level.min(col) >= max_prominence {
        break;
      }

      col = col.min(level);
      region.push(cell);

      for neighbour in neighbours(width, height, cell) {
        if visited[neighbour as usize] != start {
          visited[neighbour as usize] = start;
          frontier.push((OrderedHeight(heights[neighbour as usize]), neighbour));
        }
      }
    }

    if escape.is_none() {
      continue;
    }

    for cell in &region {
      let slot = &mut heights[*cell as usize];
      *slot = slot.min(col);
    }
  }
}

/// `f32` ordered for the search heap.
#[derive(Clone, Copy, PartialEq)]
struct OrderedHeight(f32);

impl Eq for OrderedHeight {}

impl PartialOrd for OrderedHeight {
  fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
    Some(self.cmp(other))
  }
}

impl Ord for OrderedHeight {
  fn cmp(&self, other: &Self) -> std::cmp::Ordering {
    self.0.total_cmp(&other.0)
  }
}

/// Fill every closed depression with fewer than `min_samples` samples to
/// its spill level. Noise and deposition leave pits a few samples across
/// that real terrain does not have; larger basins are kept, and become
/// lakes. Filled pits are left exactly level rather than tilted, since a
/// tilted fill would itself leave a millimetre-high summit at its far
/// end; drainage routing crosses level ground by priority-flood order.
pub fn remove_small_pits(map: &mut HeightMap, min_samples: usize) {
  let width = map.metadata.width;
  let height = map.metadata.height;
  let sea = map.metadata.sea_level_metres as f64;
  let heights: Vec<f64> = map.heights.iter().map(|h| *h as f64).collect();
  let flood = priority_flood(
    width,
    height,
    &heights,
    0.0,
    edge_or_sea_outlet(width, height, &heights, sea),
  );
  let count = heights.len();
  let in_pit = |i: usize| flood.filled[i] > heights[i];
  let mut label = vec![u32::MAX; count];
  let mut stack = Vec::new();
  let mut members = Vec::new();

  for start in 0..count {
    if label[start] != u32::MAX || !in_pit(start) {
      continue;
    }

    members.clear();
    stack.push(start);
    label[start] = start as u32;

    while let Some(cell) = stack.pop() {
      members.push(cell);

      for neighbour in neighbours(width, height, cell as u32) {
        let n = neighbour as usize;

        if label[n] == u32::MAX && in_pit(n) {
          label[n] = start as u32;
          stack.push(n);
        }
      }
    }

    if members.len() < min_samples {
      for cell in &members {
        map.heights[*cell] = flood.filled[*cell] as f32;
      }
    }
  }
}

/// Erode the lowlands. They are gentle, and later get floors, smoothing
/// and detail, so grids of 128 samples or more erode at half size, a
/// quarter of the work, and are upsampled. The coast and sea floor stay
/// exactly as stage A left them.
fn erode_lowlands(size: u32, spacing: f64, heights: &mut [f64], options: &StreamPowerOptions) {
  if size < 128 {
    stream_power(size, spacing, heights, &vec![0.0; heights.len()], options);
    return;
  }

  let half = size.div_ceil(2);
  let scale = (size - 1) as f64 / (half - 1) as f64;
  let mut coarse = resample(heights, size, half);
  let rate = options.rate.map(|rate| {
    // Cells cover `scale^2` times the area, so the same drainage area is
    // fewer cells; scale the rate to erode just as fast.
    rate * (scale * scale).powf(AREA_EXPONENT)
  });
  let no_uplift = vec![0.0; coarse.len()];
  stream_power(
    half,
    spacing * scale,
    &mut coarse,
    &no_uplift,
    &StreamPowerOptions { rate, ..*options },
  );

  for (height, eroded) in heights.iter_mut().zip(resample(&coarse, half, size)) {
    if *height > 0.0 {
      *height = eroded.max(0.5);
    }
  }
}

/// Blend the ground down to a third of the landform's sea floor at the
/// border, over a band a tenth of the coast rim wide. A map whose land
/// nearly fills it has only a thin strip of sea at the edge; the shelf
/// keeps erosion from filling it back above water, and meets the
/// renderer's skirt beyond the map with open sea. It is a shelf rather
/// than a trench, since erosion slumps coastal land into a trench.
fn shelve_border(size: usize, spacing: f32, heights: &mut [f32], landform: &Landform, sea: f32) {
  let extent = (size - 1) as f32 * spacing;
  let band = (extent * COAST_RIM).max(COAST_RIM_MIN) * 0.1;
  let samples = (band / spacing).ceil() as usize + 1;

  for y in 0..size {
    for x in 0..size {
      let edge = x.min(y).min(size - 1 - x).min(size - 1 - y);

      if edge >= samples {
        continue;
      }

      let t = smoothstep_between(0.0, band, edge as f32 * spacing);
      let height = &mut heights[y * size + x];
      *height = lerp(sea + landform.sea_floor * 0.35, *height, t);
    }
  }
}

/// Diffusion passes over the land at the end of stage B.
const COARSE_SMOOTHING: u32 = 4;

/// Light diffusion over land cells. Threshold hillslopes routed along
/// eight directions leave grid-aligned facets, and floodplain floors keep
/// the octagonal outline of their distance search; a few passes blend
/// both away while moving heights by far less than the relief. The sea
/// floor stays fixed.
fn smooth_land(n: usize, heights: &mut [f64], passes: u32) {
  let mut previous = heights.to_vec();

  for _ in 0..passes {
    previous.copy_from_slice(heights);

    for y in 1..n - 1 {
      for x in 1..n - 1 {
        let i = y * n + x;
        if previous[i] <= 0.0 {
          continue;
        }

        // The sea counts at sea level, so shores ease down into it rather
        // than standing as a rim.
        let around = [
          previous[i - 1],
          previous[i + 1],
          previous[i - n],
          previous[i + n],
        ]
        .map(|h| h.max(0.0));

        let laplacian = around.iter().sum::<f64>() - 4.0 * previous[i];
        heights[i] = previous[i] + 0.2 * laplacian;
      }
    }
  }
}

/// Drown strips of land with sea within two cells on opposite sides.
/// Flooded troughs running beside the coast leave knife-thin barriers
/// that waves would breach, and whose crests read as rows of summits.
fn breach_thin_land(n: usize, heights: &mut [f64]) {
  let mut sea: Vec<bool> = heights.iter().map(|h| *h <= 0.0).collect();

  for _ in 0..2 {
    let mut drowned = Vec::new();

    for y in 0..n as isize {
      for x in 0..n as isize {
        if sea[y as usize * n + x as usize] {
          continue;
        }

        // Sea (not the map edge) within two cells along a line, on both
        // sides.
        let open = |dx: isize, dy: isize| {
          (1..=2).any(|k| {
            let (sx, sy) = (x + dx * k, y + dy * k);
            sx >= 0
              && sy >= 0
              && sx < n as isize
              && sy < n as isize
              && sea[sy as usize * n + sx as usize]
          })
        };
        let thin = [(1, 0), (0, 1), (1, 1), (1, -1)]
          .iter()
          .any(|(dx, dy)| open(*dx, *dy) && open(-dx, -dy));

        if thin {
          drowned.push(y as usize * n + x as usize);
        }
      }
    }

    for i in drowned {
      heights[i] = -1.0;
      sea[i] = true;
    }
  }
}

/// Shift heights so exactly `land_fraction` of the grid is above sea
/// level again. Flooded glacial troughs and coastal smoothing move the
/// coast; the shift is a few metres and keeps the landform's balance of
/// land and sea.
fn relevel_sea(heights: &mut [f64], land_fraction: f32) {
  if land_fraction >= 1.0 || heights.is_empty() {
    return;
  }

  let mut sorted = heights.to_vec();
  let index = ((sorted.len() as f32 * (1.0 - land_fraction)) as usize).min(sorted.len() - 1);
  let shift = nth_value(&mut sorted, index);

  for height in heights.iter_mut() {
    *height -= shift;
  }
}

/// Upstream drainage area in square metres on a coarse grid, after
/// filling depressions.
fn coarse_drainage_area(size: u32, spacing: f64, heights: &[f64]) -> Vec<f32> {
  let flood = priority_flood(
    size,
    size,
    heights,
    1e-4,
    edge_or_sea_outlet(size, size, heights, 0.0),
  );
  let mut receiver = flood.receiver;
  steepest_receivers(size, size, &flood.filled, &mut receiver);
  let order = stack_order(&receiver);
  accumulate(
    &order,
    &receiver,
    vec![(spacing * spacing) as f32; heights.len()],
  )
}

/// Sink a flat-floored caldera into the summit of a volcanic cone. The
/// floor lies below the lowest point of the rim, so the caldera holds a
/// crater lake instead of draining through a gully that notches the rim.
fn carve_caldera(heights: &mut [f64], size: u32, summit: (f32, f32), landform: &Landform) {
  let n = size as usize;
  let radius =
    ((landform.land_fraction / std::f32::consts::PI).sqrt() * (n - 1) as f32 * 0.22).max(2.0);
  let depth = (landform.mountain_relief * 0.1) as f64;
  let distance = |i: usize| {
    let dx = (i % n) as f32 - summit.0;
    let dy = (i / n) as f32 - summit.1;
    (dx * dx + dy * dy).sqrt() / radius
  };
  let rim = (0..n * n)
    .filter(|i| (0.95..1.05).contains(&distance(*i)))
    .map(|i| heights[i])
    .fold(f64::INFINITY, f64::min);

  if !rim.is_finite() {
    return;
  }

  let floor = rim - depth;

  for (i, height) in heights.iter_mut().enumerate() {
    let inside = 1.0 - smoothstep_between(0.45, 0.95, distance(i)) as f64;
    *height += (floor.min(*height) - *height) * inside;
  }
}

fn smoothstep_between(edge0: f32, edge1: f32, value: f32) -> f32 {
  let t = ((value - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
  t * t * (3.0 - 2.0 * t)
}

/// Catmull-Rom sample of a square `n` x `n` grid at fractional `(u, v)`
/// grid coordinates.
fn bicubic(grid: &[f64], n: usize, u: f32, v: f32) -> f64 {
  let x = u.floor() as isize;
  let y = v.floor() as isize;
  let tx = (u - x as f32) as f64;
  let ty = (v - y as f32) as f64;
  let at = |xx: isize, yy: isize| {
    let cx = xx.clamp(0, n as isize - 1) as usize;
    let cy = yy.clamp(0, n as isize - 1) as usize;
    grid[cy * n + cx]
  };
  let cubic = |p0: f64, p1: f64, p2: f64, p3: f64, t: f64| {
    p1 + 0.5
      * t
      * (p2 - p0 + t * (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3 + t * (3.0 * (p1 - p2) + p3 - p0)))
  };
  let mut rows = [0.0; 4];

  for (k, row) in rows.iter_mut().enumerate() {
    let yy = y + k as isize - 1;
    *row = cubic(at(x - 1, yy), at(x, yy), at(x + 1, yy), at(x + 2, yy), tx);
  }

  cubic(rows[0], rows[1], rows[2], rows[3], ty)
}

fn bilinear(grid: &[f32], n: usize, u: f32, v: f32) -> f32 {
  let x0 = (u.floor() as usize).min(n - 1);
  let y0 = (v.floor() as usize).min(n - 1);
  let x1 = (x0 + 1).min(n - 1);
  let y1 = (y0 + 1).min(n - 1);
  let tx = u - x0 as f32;
  let ty = v - y0 as f32;
  let top = lerp(grid[y0 * n + x0], grid[y0 * n + x1], tx);
  let bottom = lerp(grid[y1 * n + x0], grid[y1 * n + x1], tx);
  lerp(top, bottom, ty)
}

/// Stage C: upsample the eroded coarse grid and add derivative-damped
/// detail, returning heights relative to sea level at full resolution.
fn add_detail(
  options: &FractalTerrainOptions,
  landform: &Landform,
  base: &Tectonics,
  coarse: &[f64],
) -> Vec<f32> {
  let size = options.size as usize;
  let spacing = options.horizontal_scale_metres;
  let n = base.size as usize;
  let scale = (n - 1) as f32 / (size - 1) as f32;
  let mut upsampled = vec![0.0f32; size * size];

  for y in 0..size {
    for x in 0..size {
      upsampled[y * size + x] = if n == size {
        coarse[y * size + x] as f32
      } else {
        bicubic(coarse, n, x as f32 * scale, y as f32 * scale) as f32
      };
    }
  }

  if landform.terrace > 0.0 {
    let step = (landform.mountain_relief + landform.lowland_relief) / 9.0;

    for value in upsampled.iter_mut() {
      if *value > 0.0 {
        *value = lerp(*value, terrace_plateau(*value, step), landform.terrace);
      }
    }
  }

  // Detail starts at eight coarse cells (what the coarse grid cannot
  // hold) and stops above two and a half samples, below which octaves
  // alias into single-sample spikes.
  let start_wavelength = (base.spacing * 8.0).max(spacing * 8.0);
  let min_wavelength = spacing * 2.5;
  let lacunarity = options.noise.lacunarity;
  let mut octaves = 0;
  let mut wavelength = start_wavelength;

  while octaves < options.noise.octaves && wavelength >= min_wavelength {
    octaves += 1;
    wavelength /= lacunarity;
  }

  let kind = options.noise.kind;
  let warp = options.noise.warp.unwrap_or(0.0).clamp(0.0, 4.0);
  let seed = options.seed;
  let warp_x = noise_seed(seed, 20);
  let warp_y = noise_seed(seed, 21);
  let half = (size - 1) as f32 * spacing * 0.5;
  let mut heights = vec![0.0f32; size * size];

  for y in 0..size {
    for x in 0..size {
      let index = y * size + x;
      let base_height = upsampled[index];
      let left = upsampled[y * size + x.saturating_sub(1)];
      let right = upsampled[y * size + (x + 1).min(size - 1)];
      let up = upsampled[y.saturating_sub(1) * size + x];
      let down = upsampled[(y + 1).min(size - 1) * size + x];
      let slope = ((right - left).powi(2) + (down - up).powi(2)).sqrt() / (2.0 * spacing);
      let uplift = bilinear(&base.uplift, n, x as f32 * scale, y as f32 * scale);
      // Steep ground in the ranges is rough; valley floors inside the
      // ranges are as smooth as the plains.
      let ruggedness =
        smoothstep_between(0.08, 0.4, slope) * (0.4 + 0.6 * smoothstep_between(0.0, 0.3, uplift));
      let roughness = lerp(
        landform.plains_roughness,
        landform.mountain_roughness,
        ruggedness,
      );

      let mut px = x as f32 * spacing - half;
      let mut py = y as f32 * spacing - half;

      if warp > 0.0 {
        let wx = px / start_wavelength;
        let wy = py / start_wavelength;
        px += simplex(warp_x, wx, wy) * warp * start_wavelength * 0.5;
        py += simplex(warp_y, wx, wy) * warp * start_wavelength * 0.5;
      }

      let range_mask = smoothstep_between(0.05, 0.4, uplift);
      let (detail, gradient) = damped_fbm(
        seed,
        kind,
        px,
        py,
        start_wavelength,
        octaves,
        options,
        range_mask,
      );
      // Detail may texture a slope but never be steeper than a quarter of it,
      // so it cannot raise new summits or pits on plains and crests.
      let detail_slope = gradient * roughness;
      let limit = if detail_slope > slope * 0.25 {
        slope * 0.25 / detail_slope
      } else {
        1.0
      };
      // Short wavelengths bend the surface sharply even when their slope
      // is small, so detail fades out entirely on near-level ground.
      let level_fade = smoothstep_between(0.05, 0.14, slope);
      heights[index] = base_height + detail * roughness * limit * level_fade;
    }
  }

  heights
}

/// Derivative-damped fBm in metres: each octave's amplitude is divided by
/// `1 + k |gradient|^2` of the octaves before it, so detail fades on the
/// flanks of earlier octaves and gathers on crests and in hollows. Each
/// octave's amplitude is a fixed fraction of its wavelength, so every
/// octave adds the same slope. Returns the detail and the magnitude of
/// its gradient in metres per metre.
#[allow(clippy::too_many_arguments)]
fn damped_fbm(
  seed: u64,
  kind: NoiseKind,
  x: f32,
  y: f32,
  start_wavelength: f32,
  octaves: u32,
  options: &FractalTerrainOptions,
  range_mask: f32,
) -> (f32, f32) {
  const SLOPE_PER_OCTAVE: f32 = 0.012;
  const DAMPING: f32 = 30.0;
  let mut wavelength = start_wavelength;
  let mut amplitude = start_wavelength * SLOPE_PER_OCTAVE;
  let mut gradient = (0.0f32, 0.0f32);
  let mut sum = 0.0;

  for octave in 0..octaves {
    let octave_seed = noise_seed(hash_u64(seed), 40 + octave as u64);
    let (value, dx, dy) = simplex_d(octave_seed, x / wavelength, y / wavelength);
    let ridge = 1.0 - 2.0 * value.abs();
    let (shaped, sx, sy) = match kind {
      NoiseKind::Ridged => mix_ridge(value, dx, dy, ridge, range_mask),
      NoiseKind::Hybrid => mix_ridge(value, dx, dy, ridge, range_mask * 0.5),
      NoiseKind::Classic => {
        let sign = value.signum();
        (
          classic_sample(2.0 * value.abs() - 1.0),
          2.0 * sign * dx,
          2.0 * sign * dy,
        )
      }
      _ => (value, dx, dy),
    };
    let damping = 1.0 / (1.0 + DAMPING * (gradient.0 * gradient.0 + gradient.1 * gradient.1));
    let weight = amplitude * damping;
    sum += shaped * weight;
    // Gradient in metres per metre.
    gradient.0 += sx * weight / wavelength;
    gradient.1 += sy * weight / wavelength;
    amplitude *= options.noise.gain * options.noise.lacunarity;
    wavelength /= options.noise.lacunarity;
  }

  (
    sum,
    (gradient.0 * gradient.0 + gradient.1 * gradient.1).sqrt(),
  )
}

/// Blend smooth noise towards ridged noise by `mask`, with the gradient.
fn mix_ridge(value: f32, dx: f32, dy: f32, ridge: f32, mask: f32) -> (f32, f32, f32) {
  let sign = -2.0 * value.signum();
  (
    lerp(value, ridge, mask),
    lerp(dx, sign * dx, mask),
    lerp(dy, sign * dy, mask),
  )
}

/// Flat treads with steep but not vertical risers between them.
fn terrace_plateau(value: f32, step: f32) -> f32 {
  let level = value / step;
  let floor = level.floor();
  let t = level - floor;
  (floor + smoothstep_between(0.25, 0.75, t)) * step
}

fn shape_noise(options: &FractalTerrainOptions, x: f32, y: f32, mut value: f32) -> f32 {
  let kind = options.noise.kind;

  if matches!(kind, NoiseKind::Island) {
    value = apply_island(x, y, value, 0.85);
  }

  if matches!(kind, NoiseKind::Canyon) {
    value -= canyon_mask(x, y) * 0.6;
  }

  if matches!(kind, NoiseKind::Cratered) {
    value -= crater_mask(options.seed, x, y) * 0.5;
  }

  if let Some(shape) = &options.shape {
    if let Some(amount) = shape.island {
      value = apply_island(x, y, value, amount);
    }

    if let Some(amount) = shape.basin {
      let centre = distance_from_centre(x, y);
      value -= (1.0 - centre).max(0.0) * amount.clamp(0.0, 1.0);
    }

    if let Some(amount) = shape.canyon {
      value -= canyon_mask(x, y) * amount.clamp(0.0, 1.0);
    }

    if let Some(amount) = shape.crater {
      value -= crater_mask(options.seed, x, y) * amount.clamp(0.0, 1.0);
    }

    if let Some(terrace) = shape.terrace {
      value = terrace_value(value, terrace);
    }
  }

  value
}

fn apply_island(x: f32, y: f32, value: f32, amount: f32) -> f32 {
  let falloff = distance_from_centre(x, y).powf(1.5).clamp(0.0, 1.0);
  value - falloff * amount.clamp(0.0, 1.2)
}

fn distance_from_centre(x: f32, y: f32) -> f32 {
  let dx = x * 2.0 - 1.0;
  let dy = y * 2.0 - 1.0;
  (dx * dx + dy * dy).sqrt().clamp(0.0, 1.0)
}

fn canyon_mask(x: f32, y: f32) -> f32 {
  let channel = (x * 1.8 + (y * 8.0).sin() * 0.08 - 0.9).abs();
  (1.0 - channel * 5.0).clamp(0.0, 1.0)
}

fn crater_mask(seed: u64, x: f32, y: f32) -> f32 {
  let mut mask: f32 = 0.0;

  for index in 0..5 {
    let hx = (hash_u64(seed ^ (index * 19) as u64) % 10_000) as f32 / 10_000.0;
    let hy = (hash_u64(seed ^ (index * 31) as u64) % 10_000) as f32 / 10_000.0;
    let radius = 0.04 + (hash_u64(seed ^ (index * 43) as u64) % 800) as f32 / 10_000.0;
    let dx = x - hx;
    let dy = y - hy;
    let distance = (dx * dx + dy * dy).sqrt();
    let bowl = (1.0 - distance / radius).clamp(0.0, 1.0);
    let rim = (1.0 - ((distance - radius) / (radius * 0.35)).abs()).clamp(0.0, 1.0);
    mask = mask.max(bowl * 0.8 - rim * 0.25);
  }

  mask
}

fn classic_sample(sample: f32) -> f32 {
  let stepped = (sample * 9.0).round() / 9.0;
  lerp(sample, stepped, 0.25)
}

fn terrace_value(value: f32, terrace: f32) -> f32 {
  let amount = terrace.clamp(0.0, 1.0);

  if amount <= 0.0 {
    return value;
  }

  let steps = 12.0;
  let stepped = ((value + 1.0) * 0.5 * steps).round() / steps * 2.0 - 1.0;
  lerp(value, stepped, amount)
}

#[cfg(test)]
mod tests {
  use super::*;
  use vista_types::{NoiseKind, NoiseOptions};

  #[test]
  fn same_seed_produces_same_heights() {
    let options = FractalTerrainOptions {
      size: 32,
      noise: NoiseOptions {
        kind: NoiseKind::Ridged,
        octaves: 4,
        gain: 0.5,
        lacunarity: 2.0,
        warp: Some(0.2),
      },
      ..FractalTerrainOptions::default()
    };
    let left = generate_fractal_heightmap(&options).unwrap();
    let right = generate_fractal_heightmap(&options).unwrap();

    assert_eq!(left.heights, right.heights);
  }
}
