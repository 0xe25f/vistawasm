//! Stage B of fractal generation: stream-power erosion on the coarse grid.
//!
//! The implicit solver of Braun and Willett (2013). Each iteration routes
//! water down the steepest descent (D8), orders cells from the outlets
//! upwards, accumulates drainage area, and lowers every cell towards its
//! receiver at a rate that grows with drainage area. Because the solve is
//! implicit, large time steps stay stable, so a few dozen iterations carve
//! a dendritic valley network with ridge spurs between the valleys.
//! Uplift keeps the ranges standing while they are carved.
//!
//! The time step is long enough that the landscape reaches the steady
//! state where erosion balances uplift within the 40 iterations, so the
//! uplift field decides where the land is high and the drainage network
//! decides its shape. Hillslopes are capped at a threshold slope, as real
//! slopes fail beyond it. Because steady relief is linear in uplift, the
//! result is then rescaled so the highest ground matches the target
//! relief.
//!
//! Glacial carving uses the nonlinear `n = 2` law against the mean height
//! around the receiver, which flattens valley floors and steepens their
//! walls into a U shape. [`plane_valley_floors`] then gives valleys flat
//! floors and over-deepens the largest glacial troughs.

use crate::terrain::drainage::{
  accumulate, accumulate_into, edge_or_sea_outlet, neighbours, priority_flood, stack_order,
  total_order_bits, StackOrder, NO_RECEIVER,
};

/// Drainage-area exponent `m` of the stream-power law.
pub const AREA_EXPONENT: f64 = 0.45;

/// Drainage-area exponent for mountain ranges. Rivers in young, rising
/// ranges keep steep profiles far downstream (low concavity), which holds
/// their valleys high between the ridges; with the lowlands' exponent, a
/// range small enough to fit a map erodes to a few hundred metres.
pub const RANGE_AREA_EXPONENT: f64 = 0.1;

/// A cell draining only itself, under the strongest uplift, settles this
/// fraction of the threshold slope above its receiver at erodibility 1.
/// This sets the erosion rate from the uplift, so relief does not depend
/// on the units of uplift and hillslopes on plains stay gentle.
const HILLTOP_SLOPE_FRACTION: f64 = 0.8;

/// The slope at which the glacial `n = 2` law erodes as fast as `n = 1`.
const GLACIAL_REFERENCE_SLOPE: f64 = 0.35;

/// Iterations that reroute every time, while the network forms.
pub const SETTLING_ITERATIONS: u32 = 5;
/// After settling, routing is refreshed every this many iterations.
const REROUTE_INTERVAL: u32 = 5;

/// Diffusion passes over the crests after the last iteration, for a
/// solve from scratch.
pub const CREST_PASSES: u32 = 24;
/// Crest passes for the full-size refinement, whose crests were already
/// rounded at half size.
const FINE_CREST_PASSES: u32 = 8;

/// Cells draining at most this many cells count as crests.
const CREST_CELLS: f32 = 8.0;

/// Smallest drop kept between a cell and its receiver, so the receiver
/// graph stays acyclic and every cell keeps draining.
const EPSILON: f64 = 1e-3;

/// Stream-power settings.
#[derive(Clone, Copy, Debug)]
pub struct StreamPowerOptions {
  /// Number of iterations.
  pub iterations: u32,
  /// Seed for choosing between downhill neighbours.
  pub seed: u64,
  /// Erosion rate per iteration of a cell draining only itself. `None`
  /// derives it from the strongest uplift, for the steady state.
  pub rate: Option<f64>,
  /// Erodibility multiplier (the landform's `erodibility`).
  pub erodibility: f64,
  /// Explicit hillslope diffusion per iteration, 0 to 0.25.
  pub diffusion: f64,
  /// Glacial strength, 0 to 1.
  pub glacial: f64,
  /// Heights above this (relative to sea level) are glaciated.
  pub snowline: f64,
  /// Steepest hillslope, as a gradient (rise over run).
  pub threshold_slope: f64,
  /// Drainage-area exponent `m` ([`AREA_EXPONENT`] or
  /// [`RANGE_AREA_EXPONENT`]).
  pub area_exponent: f64,
  /// The 99th percentile of land height to rescale to, in metres, or 0
  /// to keep the raw steady-state heights.
  pub target_relief: f64,
  /// Diffusion passes over the crests after the last iteration.
  pub crest_passes: u32,
  /// Iterations that reroute every time before rerouting slows to every
  /// [`REROUTE_INTERVAL`]. A solve starting from a settled network needs
  /// fewer.
  pub settling_iterations: u32,
}

/// Erode `heights` (metres relative to sea level, row-major, `size` per
/// side, `spacing` metres apart) in place, raising each cell by `uplift`
/// metres per iteration. Cells at or below 0 and cells on the map edge
/// are fixed outlets. Returns the drainage area of the last iteration in
/// square metres.
pub fn stream_power(
  size: u32,
  spacing: f64,
  heights: &mut [f64],
  uplift: &[f64],
  options: &StreamPowerOptions,
) -> Vec<f32> {
  let count = heights.len();
  // Every edge cell is an outlet; routing relies on it.
  let outlet: Vec<bool> = {
    let is_outlet = edge_or_sea_outlet(size, size, heights, 0.0);
    (0..count as u32).map(is_outlet).collect()
  };
  let cell_area = (spacing * spacing) as f32;

  // Fill depressions once. Every later step keeps each cell above its
  // receiver, so no new depressions can form.
  let flood = priority_flood(size, size, heights, EPSILON, |i| outlet[i as usize]);
  heights.copy_from_slice(&flood.filled);
  let mut receiver = flood.receiver;
  let mut area = vec![cell_area; count];
  let mut order: Vec<u32> = Vec::with_capacity(count);
  let mut stacker = StackOrder::default();
  let mut scratch = vec![0.0f64; count];
  let strongest = uplift.iter().cloned().fold(1e-6, f64::max);
  let base_rate = options
    .rate
    .unwrap_or(strongest / (HILLTOP_SLOPE_FRACTION * options.threshold_slope * spacing));

  // `(area / cell area)^m` for every possible cell count, since areas are
  // whole numbers of cells and `powf` dominates the solve otherwise.
  let area_power: Vec<f64> = (0..=count)
    .map(|cells| (cells as f64).powf(options.area_exponent))
    .collect();

  for iteration in 0..options.iterations.max(1) {
    // Routing settles within the first iterations. After that it is
    // refreshed every few iterations: in between, every step keeps each
    // cell above its receiver, so the old routing stays valid and acyclic.
    if iteration < options.settling_iterations || iteration % REROUTE_INTERVAL == 0 {
      let routing_seed = crate::maths::hash_u64(options.seed ^ ((iteration as u64) << 32));
      stochastic_descent(size, heights, &outlet, &mut receiver, routing_seed);
      stacker.order(&receiver, &mut order);
      area.fill(cell_area);
      accumulate_into(&order, &receiver, &mut area);
    }

    for index in &order {
      let i = *index as usize;
      let r = receiver[i];

      if r == NO_RECEIVER {
        continue;
      }

      let r = r as usize;
      let distance = cell_distance(size, i, r) * spacing;
      let rate = options.erodibility
        * base_rate
        * area_power[((area[i] / cell_area).round() as usize).min(count)]
        * spacing
        / distance;
      let raised = heights[i] + uplift[i];
      let fluvial = (raised + rate * heights[r]) / (1.0 + rate);
      let ice = glacial_weight(heights[i], options);

      heights[i] = if ice > 0.0 {
        let trough = mean_around(size, heights, r).min(heights[r]);
        let glacial = glacial_solve(raised, trough, rate / GLACIAL_REFERENCE_SLOPE / distance);
        fluvial + (glacial - fluvial) * ice
      } else {
        fluvial
      };
      heights[i] = heights[i]
        .max(heights[r] + EPSILON)
        .min(heights[r] + options.threshold_slope * distance);
    }

    diffuse(size, heights, &outlet, options.diffusion, &mut scratch);
    // Diffusion can lift a valley floor above the next cell downstream;
    // re-impose the drop from the outlets upwards so drainage survives.
    enforce_drops(heights, &receiver, &order, size, spacing, f64::INFINITY);
  }

  if options.target_relief > 0.0 {
    let mut land: Vec<f64> = heights
      .iter()
      .zip(&outlet)
      .filter_map(|(h, o)| (!o).then_some(*h))
      .collect();

    if !land.is_empty() {
      let index = (land.len() * 99 / 100).min(land.len() - 1);
      let high = nth_value(&mut land, index);

      if high > 1.0 {
        let scale = options.target_relief / high;

        for (h, o) in heights.iter_mut().zip(&outlet) {
          if !o {
            *h *= scale;
          }
        }
      }
    }
  }

  let order = stack_order(&receiver);
  enforce_drops(
    heights,
    &receiver,
    &order,
    size,
    spacing,
    options.threshold_slope,
  );

  // Round off the crests the threshold slopes meet at, which would
  // otherwise stand as knife-edges and single-cell summits.
  let area = accumulate(&order, &receiver, vec![cell_area; count]);
  let n = size as usize;
  let crest: Vec<usize> = (0..count)
    .filter(|i| !outlet[*i] && area[*i] <= cell_area * CREST_CELLS)
    .collect();
  let mut update = vec![0.0f64; crest.len()];

  for _ in 0..options.crest_passes {
    for (slot, i) in update.iter_mut().zip(&crest) {
      let i = *i;
      let laplacian =
        heights[i - 1] + heights[i + 1] + heights[i - n] + heights[i + n] - 4.0 * heights[i];
      *slot = heights[i] + 0.25 * laplacian;
    }

    for (value, i) in update.iter().zip(&crest) {
      heights[*i] = *value;
    }
  }

  enforce_drops(
    heights,
    &receiver,
    &order,
    size,
    spacing,
    options.threshold_slope,
  );

  area
}

/// Grids at least this many samples across solve most iterations at half
/// size first (see [`stream_power_coarse_to_fine`]).
const COARSE_TO_FINE_MIN_SIZE: u32 = 128;
/// Iterations kept for the full-size grid in coarse-to-fine solves.
const FINE_ITERATIONS: u32 = 4;

/// [`stream_power`] solved coarse to fine: all but [`FINE_ITERATIONS`] of
/// the iterations run on a half-size grid, where each costs a quarter as
/// much and the steady state settles just the same, and the rest refine
/// the upsampled result at full size. Small grids are solved directly.
pub fn stream_power_coarse_to_fine(
  size: u32,
  spacing: f64,
  heights: &mut [f64],
  uplift: &[f64],
  options: &StreamPowerOptions,
) -> Vec<f32> {
  if size < COARSE_TO_FINE_MIN_SIZE || options.iterations <= FINE_ITERATIONS {
    return stream_power(size, spacing, heights, uplift, options);
  }

  let half = size.div_ceil(2);
  let half_spacing = spacing * (size - 1) as f64 / (half - 1) as f64;
  let mut coarse = resample(heights, size, half);
  // Uplift is a rate per iteration, so it scales with the cell size to
  // keep hillslope drops (and so slopes) the same.
  let coarse_uplift: Vec<f64> = resample(uplift, size, half)
    .iter()
    .map(|u| u * half_spacing / spacing)
    .collect();
  stream_power(
    half,
    half_spacing,
    &mut coarse,
    &coarse_uplift,
    &StreamPowerOptions {
      iterations: options.iterations - FINE_ITERATIONS,
      ..*options
    },
  );

  // Keep the sea and the base level exactly where they were.
  for (height, refined) in heights.iter_mut().zip(resample(&coarse, half, size)) {
    if *height > 0.0 {
      *height = refined.max(1e-3);
    }
  }

  stream_power(
    size,
    spacing,
    heights,
    uplift,
    &StreamPowerOptions {
      iterations: FINE_ITERATIONS,
      // The upsampled network is already settled.
      settling_iterations: 1,
      crest_passes: FINE_CREST_PASSES,
      ..*options
    },
  )
}

/// The value that would sit at `index` if `values` were sorted. One shared
/// copy, since each call site would otherwise build its own selection.
#[inline(never)]
pub fn nth_value(values: &mut [f64], index: usize) -> f64 {
  *values
    .select_nth_unstable_by(index, |a, b| a.total_cmp(b))
    .1
}

/// The 99th percentile of the positive `values`, or 0 when there are none.
pub fn positive_p99(values: &[f64]) -> f64 {
  let mut raised: Vec<f64> = values.iter().copied().filter(|v| *v > 0.0).collect();
  let index = raised.len() * 99 / 100;

  if index < raised.len() {
    nth_value(&mut raised, index)
  } else {
    0.0
  }
}

/// Bilinearly resample a square grid of `from` samples per side onto one
/// of `to` samples per side spanning the same extent.
pub fn resample(values: &[f64], from: u32, to: u32) -> Vec<f64> {
  let (from, to) = (from as usize, to as usize);
  let scale = (from - 1) as f64 / (to - 1) as f64;
  let mut out = Vec::with_capacity(to * to);

  for y in 0..to {
    let fy = y as f64 * scale;
    let y0 = (fy as usize).min(from - 2);
    let ty = fy - y0 as f64;

    for x in 0..to {
      let fx = x as f64 * scale;
      let x0 = (fx as usize).min(from - 2);
      let tx = fx - x0 as f64;
      let at = |xx: usize, yy: usize| values[yy * from + xx];
      let top = at(x0, y0) + (at(x0 + 1, y0) - at(x0, y0)) * tx;
      let bottom = at(x0, y0 + 1) + (at(x0 + 1, y0 + 1) - at(x0, y0 + 1)) * tx;
      out.push(top + (bottom - top) * ty);
    }
  }

  out
}

/// Keep every cell between `EPSILON` and `max_slope` above its receiver,
/// walking from the outlets upwards.
fn enforce_drops(
  heights: &mut [f64],
  receiver: &[u32],
  order: &[u32],
  size: u32,
  spacing: f64,
  max_slope: f64,
) {
  for index in order {
    let i = *index as usize;
    let r = receiver[i];

    if r != NO_RECEIVER {
      let r = r as usize;
      let limit = max_slope * cell_distance(size, i, r) * spacing;
      heights[i] = heights[i].max(heights[r] + EPSILON).min(heights[r] + limit);
    }
  }
}

/// Distance in cells between two eight-way neighbours: 1 along an axis,
/// the square root of 2 along a diagonal.
fn cell_distance(size: u32, a: usize, b: usize) -> f64 {
  let step = a.abs_diff(b);

  if step == 1 || step == size as usize {
    1.0
  } else {
    std::f64::consts::SQRT_2
  }
}

/// D8 steepest descent, used by the tests to measure channel lengths.
/// Every non-outlet cell keeps a strictly lower neighbour (the invariant
/// the solver maintains), so each finds one.
#[cfg(test)]
fn steepest_descent(size: u32, heights: &[f64], outlet: &[bool], receiver: &mut [u32]) {
  for index in 0..heights.len() as u32 {
    let i = index as usize;

    if outlet[i] {
      receiver[i] = NO_RECEIVER;
      continue;
    }

    let mut best = receiver[i];
    let mut best_drop = 0.0;

    for neighbour in neighbours(size, size, index) {
      let n = neighbour as usize;
      let drop = (heights[i] - heights[n]) / cell_distance(size, i, n);

      if drop > best_drop {
        best_drop = drop;
        best = neighbour;
      }
    }

    receiver[i] = best;
  }
}

/// Route each cell to a downhill neighbour chosen at random with odds in
/// proportion to the squared slope. Always taking the steepest neighbour
/// (D8) lays parallel channels along the eight grid directions; spreading
/// the choice lets channels wander and merge as real ones do. Every
/// non-outlet cell has a lower neighbour, so each finds one.
fn stochastic_descent(
  size: u32,
  heights: &[f64],
  outlet: &[bool],
  receiver: &mut [u32],
  seed: u64,
) {
  let n = size as isize;
  let diagonal = std::f64::consts::FRAC_1_SQRT_2;
  // Index steps with the reciprocal of their length. Every edge cell is
  // an outlet, so interior cells never step off the grid.
  let offsets: [(isize, f64); 8] = [
    (-1, 1.0),
    (1, 1.0),
    (-n, 1.0),
    (n, 1.0),
    (-n - 1, diagonal),
    (-n + 1, diagonal),
    (n - 1, diagonal),
    (n + 1, diagonal),
  ];

  for i in 0..heights.len() {
    if outlet[i] {
      receiver[i] = NO_RECEIVER;
      continue;
    }

    // Uphill neighbours get zero weight; computing all eight without
    // branching is faster than skipping them.
    let here = heights[i];
    let mut weights = [0.0f64; 8];
    let mut total = 0.0;

    for (slot, (step, inverse)) in offsets.iter().enumerate() {
      let j = (i as isize + step) as usize;
      let drop = ((here - heights[j]) * inverse).max(0.0);
      weights[slot] = drop * drop;
      total += weights[slot];
    }

    if total <= 0.0 {
      continue;
    }

    // A cheap 32-bit mix is plenty to pick between eight neighbours.
    let mut hash = (seed as u32) ^ (i as u32).wrapping_mul(0x9e37_79b9);
    hash ^= hash >> 16;
    hash = hash.wrapping_mul(0x85eb_ca6b);
    hash ^= hash >> 13;
    let mut pick = hash as f64 * (total / u32::MAX as f64);
    let mut chosen = 7;

    for (slot, weight) in weights.iter().enumerate() {
      if *weight > 0.0 {
        chosen = slot;

        if pick < *weight {
          break;
        }

        pick -= weight;
      }
    }

    receiver[i] = (i as isize + offsets[chosen].0) as u32;
  }
}

/// How glaciated a cell at `height` is, 0 to 1.
fn glacial_weight(height: f64, options: &StreamPowerOptions) -> f64 {
  if options.glacial <= 0.0 {
    return 0.0;
  }

  let band = (options.snowline * 0.15).max(1.0);
  let t = ((height - options.snowline + band) / (2.0 * band)).clamp(0.0, 1.0);
  options.glacial * t * t * (3.0 - 2.0 * t)
}

/// The mean height of the 3 x 3 block around `index`: glaciers erode
/// against a wider base than a single receiver, which widens valleys.
fn mean_around(size: u32, heights: &[f64], index: usize) -> f64 {
  let mut sum = heights[index];
  let mut count = 1.0;

  for neighbour in neighbours(size, size, index as u32) {
    sum += heights[neighbour as usize];
    count += 1.0;
  }

  sum / count
}

/// Implicit `n = 2` step: solve `x + c x^2 = raised - base` for the drop
/// `x = h - base`, which is the positive root of the quadratic.
fn glacial_solve(raised: f64, base: f64, c: f64) -> f64 {
  let excess = raised - base;

  if excess <= 0.0 || c <= 0.0 {
    return raised;
  }

  let drop = (-1.0 + (1.0 + 4.0 * c * excess).sqrt()) / (2.0 * c);
  base + drop
}

/// One explicit hillslope diffusion step on non-outlet cells.
fn diffuse(size: u32, heights: &mut [f64], outlet: &[bool], amount: f64, scratch: &mut [f64]) {
  if amount <= 0.0 {
    return;
  }

  let n = size as usize;
  scratch.copy_from_slice(heights);

  for y in 1..n - 1 {
    for x in 1..n - 1 {
      let i = y * n + x;

      if outlet[i] {
        continue;
      }

      let laplacian =
        scratch[i - 1] + scratch[i + 1] + scratch[i - n] + scratch[i + n] - 4.0 * scratch[i];
      heights[i] = scratch[i] + amount.min(0.25) * laplacian;
    }
  }
}

/// Channels draining fewer cells than this get no floodplain.
const FLOODPLAIN_MIN_CELLS: f32 = 12.0;
/// Floodplain half-width per square root of drainage area (metres per
/// metre).
const FLOODPLAIN_WIDTH: f64 = 0.15;
/// Blur passes over the floodplain cut.
const FLOODPLAIN_BLUR: u32 = 3;
/// Cross-valley gradient of a floodplain towards its channel.
const FLOODPLAIN_GRADIENT: f64 = 0.015;

/// Plane flat floors into valleys, as rivers do by swinging from side to
/// side and as glaciers do by scouring. Each channel cell lowers the
/// ground within a half-width that grows with the square root of its
/// drainage area (wider under ice) to a floor that rises gently away from
/// the channel. Stream power alone leaves every valley a V with no floor.
///
/// Beyond the floor the valley wall is cut back at `wall_slope`, so a
/// wider floor moves the wall rather than steepening it.
///
/// Under ice (`glacial > 0`), trunk valleys are also over-deepened by up
/// to `deepening` metres, growing with ice flux (drainage area on a log
/// scale). Near the coast this cuts troughs below sea level, which the
/// sea floods as fjords; inland it leaves basins that hold lakes.
#[allow(clippy::too_many_arguments)]
pub fn plane_valley_floors(
  size: u32,
  spacing: f64,
  heights: &mut [f64],
  area: &[f32],
  glacial: f64,
  deepening: f64,
  wall_slope: f64,
) {
  use std::collections::BinaryHeap;

  let count = heights.len();
  let cell_area = (spacing * spacing) as f32;
  let widest = spacing * (4.0 + 2.0 * glacial);
  let mut floor = vec![f64::INFINITY; count];
  // How far each floor is over-deepened below the level its walls rise
  // from.
  let mut trough = vec![0.0f64; count];
  let mut heap = BinaryHeap::new();
  let largest = area.iter().cloned().fold(cell_area, f32::max);
  let log_largest = ((largest / cell_area) as f64).ln().max(1.0);

  for i in 0..count {
    if heights[i] > 0.0 && area[i] >= cell_area * FLOODPLAIN_MIN_CELLS {
      let flux = ((area[i] / cell_area) as f64).ln() / log_largest;
      let ice = glacial * smooth(0.3, 0.8, flux);
      let width = (FLOODPLAIN_WIDTH * (area[i] as f64).sqrt() * (1.0 + ice)).min(widest);
      let level = heights[i];
      floor[i] = level;
      trough[i] = deepening * ice * smooth(0.45, 0.9, flux);
      heap.push(FloorCell {
        floor: -level,
        index: i as u32,
        travelled: 0.0,
        width,
        deepen: trough[i],
      });
    }
  }

  while let Some(cell) = heap.pop() {
    let i = cell.index as usize;

    if -cell.floor > floor[i] {
      continue;
    }

    for neighbour in neighbours(size, size, cell.index) {
      let n = neighbour as usize;
      let step = cell_distance(size, i, n) * spacing;
      let travelled = cell.travelled + step;

      if heights[n] <= 0.0 {
        continue;
      }

      // The wall steepens from the floor gradient over a couple of cells
      // beyond the floor edge, rather than breaking from it.
      let ramp = spacing * 3.0;
      let beyond = (travelled - cell.width).max(0.0);
      let t = (beyond / ramp).clamp(0.0, 1.0);
      let gradient = FLOODPLAIN_GRADIENT + (wall_slope - FLOODPLAIN_GRADIENT) * t * t;
      let level = -cell.floor + gradient * step;

      if level >= heights[n] {
        continue;
      }

      if level < floor[n] {
        floor[n] = level;
        // Ice deepens the trough's flat floor, fading out at its walls;
        // the walls rise from the undeepened level, so the ridges between
        // troughs keep their height and the lower walls steepen.
        trough[n] = cell.deepen * (1.0 - smooth(cell.width * 0.6, cell.width, travelled));
        heap.push(FloorCell {
          floor: -level,
          index: neighbour,
          travelled,
          width: cell.width,
          deepen: cell.deepen,
        });
      }
    }
  }

  // The floor is the lower envelope of a cone around every channel cell,
  // which leaves a cusp wherever two cones meet. Blurring how far each
  // cell is cut, rather than the heights, rounds those cusps off without
  // filling the floors.
  let n = size as usize;
  let mut cut: Vec<f64> = heights
    .iter()
    .zip(&floor)
    .zip(&trough)
    .map(|((height, level), deeper)| (height - level).max(0.0) + deeper)
    .collect();
  let mut previous = cut.clone();

  for _ in 0..FLOODPLAIN_BLUR {
    previous.copy_from_slice(&cut);

    for y in 1..n - 1 {
      for x in 1..n - 1 {
        let i = y * n + x;
        let laplacian =
          previous[i - 1] + previous[i + 1] + previous[i - n] + previous[i + n] - 4.0 * previous[i];
        cut[i] = previous[i] + 0.2 * laplacian;
      }
    }
  }

  for (height, cut) in heights.iter_mut().zip(&cut) {
    if *height > 0.0 {
      *height -= cut;
    }
  }
}

/// A floodplain search entry; `floor` is negated so the heap pops the
/// lowest floor first.
#[derive(Clone, Copy, PartialEq)]
struct FloorCell {
  floor: f64,
  index: u32,
  travelled: f64,
  width: f64,
  deepen: f64,
}

impl Eq for FloorCell {}

impl PartialOrd for FloorCell {
  fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
    Some(self.cmp(other))
  }
}

impl Ord for FloorCell {
  fn cmp(&self, other: &Self) -> std::cmp::Ordering {
    // Integer keys order like `f64::total_cmp`, and compare faster.
    (total_order_bits(self.floor), other.index).cmp(&(total_order_bits(other.floor), self.index))
  }
}

fn smooth(edge0: f64, edge1: f64, value: f64) -> f64 {
  let t = ((value - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
  t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::terrain::noise::simplex;

  /// A plane tilted gently away from the y = 0 edge under metres of
  /// roughness, so the network organises itself rather than following the
  /// tilt in parallel strips.
  fn tilted_plane(size: u32, spacing: f64, seed: u32) -> Vec<f64> {
    let n = size as usize;
    let mut heights = vec![0.0; n * n];

    for y in 1..n {
      for x in 0..n {
        let roughness = simplex(seed, x as f32 * 0.37, y as f32 * 0.37) as f64 * 5.0;
        heights[y * n + x] = y as f64 * spacing * 0.001 + 6.0 + roughness;
      }
    }

    heights
  }

  fn options(seed: u64) -> StreamPowerOptions {
    StreamPowerOptions {
      iterations: 60,
      seed,
      rate: None,
      erodibility: 1.0,
      diffusion: 0.02,
      glacial: 0.0,
      snowline: 1e9,
      threshold_slope: 0.8,
      area_exponent: AREA_EXPONENT,
      target_relief: 0.0,
      settling_iterations: SETTLING_ITERATIONS,
      crest_passes: CREST_PASSES,
    }
  }

  #[test]
  fn every_cell_drains_to_an_outlet_afterwards() {
    let size = 64;
    let mut heights = tilted_plane(size, 30.0, 11);
    let uplift = vec![2.0; heights.len()];
    stream_power(size, 30.0, &mut heights, &uplift, &options(3));
    let flood = priority_flood(
      size,
      size,
      &heights,
      0.0,
      edge_or_sea_outlet(size, size, &heights, 0.0),
    );

    for (filled, height) in flood.filled.iter().zip(&heights) {
      assert!(filled - height < 1e-9);
    }
  }

  #[test]
  fn a_tilted_plane_develops_channels_that_follow_hacks_law() {
    // Hack's law: along a main stream, the length of the longest channel
    // above a point grows as drainage area to a power between 0.5 and 0.6
    // in real basins. The main streams of several independent runs are
    // pooled, since one small network gives a noisy estimate.
    let size = 128;
    let spacing = 30.0;
    let n = size as usize;
    let (mut sx, mut sy, mut sxx, mut sxy, mut samples) = (0.0, 0.0, 0.0, 0.0, 0.0);

    for run in 0..8u32 {
      let mut heights = tilted_plane(size, spacing, 11 + run);
      let uplift = vec![2.0; heights.len()];
      let area = stream_power(
        size,
        spacing,
        &mut heights,
        &uplift,
        &options(3 + run as u64),
      );
      let cell_area = (spacing * spacing) as f32;
      let outlet: Vec<bool> = (0..n * n)
        .map(|i| edge_or_sea_outlet(size, size, &heights, 0.0)(i as u32))
        .collect();
      let mut receiver = vec![NO_RECEIVER; n * n];
      steepest_descent(size, &heights, &outlet, &mut receiver);
      let order = stack_order(&receiver);
      let mut length = vec![0.0f64; n * n];
      let mut donors: Vec<Vec<usize>> = vec![Vec::new(); n * n];

      // Donors come after their receivers in the stack, so walk it
      // backwards to finish every upstream length before it is used.
      for index in order.iter().rev() {
        let i = *index as usize;

        if receiver[i] != NO_RECEIVER {
          let r = receiver[i] as usize;
          length[r] = length[r].max(length[i] + cell_distance(size, i, r));
          donors[r].push(i);
        }
      }

      // Channels: a few trunk streams collect most of the area.
      let largest = area.iter().cloned().fold(0.0f32, f32::max) / cell_area;
      assert!(largest > 200.0, "largest catchment {largest} cells");

      for mouth in 0..n * n {
        let drains_out = receiver[mouth] != NO_RECEIVER && outlet[receiver[mouth] as usize];

        if outlet[mouth] || !drains_out || area[mouth] < cell_area * 100.0 {
          continue;
        }

        // Walk up the main stream: always the donor with the most area.
        let mut cell = mouth;

        while area[cell] >= cell_area * 20.0 {
          let x = ((area[cell] / cell_area) as f64).ln();
          let y = length[cell].max(1.0).ln();
          sx += x;
          sy += y;
          sxx += x * x;
          sxy += x * y;
          samples += 1.0;

          match donors[cell]
            .iter()
            .max_by(|a, b| area[**a].total_cmp(&area[**b]))
          {
            Some(donor) => cell = *donor,
            None => break,
          }
        }
      }
    }

    let exponent = (samples * sxy - sx * sy) / (samples * sxx - sx * sx);
    assert!((0.5..=0.6).contains(&exponent), "Hack exponent {exponent}");
  }

  #[test]
  fn glacial_solve_matches_the_quadratic() {
    let height = glacial_solve(100.0, 20.0, 0.01);
    let drop = height - 20.0;
    assert!((drop + 0.01 * drop * drop - 80.0).abs() < 1e-9);
    assert_eq!(glacial_solve(10.0, 20.0, 0.01), 10.0);
  }
}
