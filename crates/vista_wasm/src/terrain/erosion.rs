//! Stage D of fractal generation: hydraulic and thermal erosion at full
//! resolution. This is the CPU reference for the GPU passes in
//! `shaders/hydraulic_erosion.wgsl` and `shaders/thermal_erosion.wgsl`,
//! used by native builds, tests, and as a fallback when the GPU fails.
//!
//! Hydraulic erosion is the virtual-pipe shallow-water model of Mei,
//! Decaudin and Hu (2007). Every iteration runs six passes, each a
//! "gather" that writes only its own cell:
//!
//! 1. Rain, weighted towards cells with a large drainage area, so the
//!    valleys from stream power keep carving.
//! 2. Outflow flux through four virtual pipes, driven by the difference
//!    in water surface height.
//! 3. Water depth and velocity from the fluxes, plus the terrain slope.
//! 4. Erosion and deposition towards the sediment capacity
//!    `C = Kc * sin(slope) * |v| * limit(depth)`. Water that slows where a
//!    valley opens out drops its load, which builds alluvial fans.
//! 5. Sediment advection: each cell's load moves along its velocity and
//!    is split bilinearly between the four cells around where it lands
//!    (a forward semi-Lagrangian step, gathered so it stays race-free).
//!    The split weights sum to one, so advection conserves sediment.
//! 6. Evaporation.
//!
//! The sea and the map edge are sinks: water and sediment that reach them
//! leave the model.
//!
//! Thermal erosion moves material between neighbours whenever the slope
//! between them exceeds the talus angle, in proportion to the excess. The
//! exchange between two cells is computed the same way from either side,
//! so it is race-free and conserves material. It leaves scree slopes at
//! the talus angle below cliffs.
//!
//! Work is split across two scales: 60 % of the iterations run at half
//! resolution, where large features settle cheaply, and the change is
//! upsampled onto the full-resolution map for the remaining 40 %, which
//! form the fine gullies. All heights are in units of the cell size, so
//! the same constants serve both scales.

use vista_types::{ErosionOptions, ErosionQuality};

use crate::config::EROSION_ITERATIONS_MAX;
use crate::errors::VistaResult;
use crate::terrain::fractal::Progress;
use crate::terrain::heightmap::{update_stats, HeightMap};
use crate::terrain::landforms::Landform;

/// Pipe flux gain per unit of water surface difference (the product of
/// time step, pipe area and gravity over pipe length, in cell units).
pub const PIPE_GAIN: f32 = 0.25;
/// Sediment capacity is never computed from a slope gentler than this
/// sine, so slow water on flats still carries some load.
pub const MIN_TILT: f32 = 0.02;
/// Water depth in metres at which capacity reaches its full value;
/// shallower films carry proportionally less.
pub const FULL_DEPTH_METRES: f32 = 0.25;
/// Dissolving rate: the fraction of the capacity shortfall eroded per
/// iteration.
pub const DISSOLVE_RATE: f32 = 0.3;
/// Deposition rate: the fraction of the excess load dropped per iteration.
pub const DEPOSIT_RATE: f32 = 0.3;
/// Thermal exchange rate per neighbour. At most 1/16 keeps the eight-way
/// exchange stable.
pub const THERMAL_RATE: f32 = 0.06;
/// Converts `ErosionOptions.rainAmount` to metres of rain per iteration.
pub const RAIN_METRES_PER_UNIT: f32 = 0.1;
/// Converts `ErosionOptions.evaporation` to a fraction lost per iteration.
pub const EVAPORATION_PER_UNIT: f32 = 0.04;
/// Converts `ErosionOptions.sedimentCapacity` to the capacity constant.
pub const CAPACITY_PER_UNIT: f32 = 1.5;
/// Fraction of the iterations run at half resolution.
pub const HALF_RESOLUTION_SHARE: f32 = 0.6;

/// Unset `rainAmount`: this times the landform's relative rain.
const DEFAULT_RAIN_AMOUNT: f32 = 0.02;
/// Unset `evaporation`.
const DEFAULT_EVAPORATION: f32 = 0.5;
/// Unset `sedimentCapacity`.
const DEFAULT_SEDIMENT_CAPACITY: f32 = 0.04;

/// Iteration counts after applying quality defaults and caps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ErosionIterations {
  /// Hydraulic iterations.
  pub hydraulic: u32,
  /// Thermal iterations.
  pub thermal: u32,
}

/// Resolve iteration counts: unset counts take the quality's defaults,
/// and requested counts are capped by the quality's budget.
pub fn erosion_iterations(options: &ErosionOptions) -> ErosionIterations {
  let (hydraulic, thermal, cap) = match options.quality.unwrap_or(ErosionQuality::Preview) {
    ErosionQuality::Preview => (60, 30, 120),
    ErosionQuality::Balanced => (120, 60, 240),
    ErosionQuality::High => (200, 100, 400),
    ErosionQuality::Offline => (400, 200, EROSION_ITERATIONS_MAX),
  };

  ErosionIterations {
    hydraulic: options.hydraulic_iterations.unwrap_or(hydraulic).min(cap),
    thermal: options.thermal_iterations.unwrap_or(thermal).min(cap),
  }
}

/// Erosion constants for one scale, in cell units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ErosionParams {
  /// Rain per iteration.
  pub rain: f32,
  /// Fraction of water evaporated per iteration.
  pub evaporation: f32,
  /// Sediment capacity constant `Kc`.
  pub capacity: f32,
  /// Depth at which capacity reaches its full value.
  pub full_depth: f32,
  /// Tangent of the talus angle.
  pub talus: f32,
}

impl ErosionParams {
  /// Constants for cells `cell_metres` across, from the options with the
  /// landform's defaults for unset fields.
  pub fn new(options: &ErosionOptions, landform: &Landform, cell_metres: f32) -> Self {
    let cell = cell_metres.max(0.001);
    let rain = options
      .rain_amount
      .unwrap_or(DEFAULT_RAIN_AMOUNT * landform.rain)
      .clamp(0.0, 1.0);
    let evaporation = options
      .evaporation
      .unwrap_or(DEFAULT_EVAPORATION)
      .clamp(0.0, 1.0);
    let capacity = options
      .sediment_capacity
      .unwrap_or(DEFAULT_SEDIMENT_CAPACITY)
      .clamp(0.0, 1.0);
    let talus = options
      .talus_angle_degrees
      .unwrap_or(landform.talus_angle_degrees)
      .clamp(1.0, 89.0);

    Self {
      rain: rain * RAIN_METRES_PER_UNIT / cell,
      evaporation: (evaporation * EVAPORATION_PER_UNIT).min(1.0),
      capacity: capacity * CAPACITY_PER_UNIT,
      full_depth: FULL_DEPTH_METRES / cell,
      talus: talus.to_radians().tan(),
    }
  }
}

/// One scale of the multi-scale schedule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ErosionPhase {
  /// Whether this phase runs at half resolution.
  pub half: bool,
  /// Iterations in this phase.
  pub iterations: ErosionIterations,
}

/// Split the iterations between half and full resolution. Maps too small
/// to halve run everything at full resolution.
pub fn erosion_schedule(iterations: ErosionIterations, size: u32) -> Vec<ErosionPhase> {
  let split = |count: u32| ((count as f32 * HALF_RESOLUTION_SHARE).round() as u32).min(count);
  let (half_hydraulic, half_thermal) = if size >= 64 {
    (split(iterations.hydraulic), split(iterations.thermal))
  } else {
    (0, 0)
  };
  let mut phases = Vec::new();

  if half_hydraulic + half_thermal > 0 {
    phases.push(ErosionPhase {
      half: true,
      iterations: ErosionIterations {
        hydraulic: half_hydraulic,
        thermal: half_thermal,
      },
    });
  }

  let full = ErosionIterations {
    hydraulic: iterations.hydraulic - half_hydraulic,
    thermal: iterations.thermal - half_thermal,
  };

  if full.hydraulic + full.thermal > 0 {
    phases.push(ErosionPhase {
      half: false,
      iterations: full,
    });
  }

  phases
}

/// Rain weights for a `size` x `size` grid: 0.35 to 1 on land, rising
/// with the logarithm of the drainage area, and -1 marking the sea.
pub fn rain_weights(map: &HeightMap, size: u32) -> Vec<f32> {
  let n = size as usize;
  let full = map.metadata.width as usize;
  let stride = (full / n).max(1);
  let sea = map.metadata.sea_level_metres;
  let cell_area = map.metadata.metres_per_sample * map.metadata.metres_per_sample;
  let aux = map.aux.as_deref();
  let largest = aux
    .map(|aux| aux.drainage_area.iter().cloned().fold(cell_area, f32::max))
    .unwrap_or(cell_area);
  let log_largest = (largest / cell_area).ln().max(1.0);
  let mut weights = vec![0.0; n * n];

  for y in 0..n {
    for x in 0..n {
      let height = map.heights[(y * stride) * full + x * stride];

      weights[y * n + x] = if height <= sea {
        -1.0
      } else {
        let u = x as f32 / (n - 1).max(1) as f32;
        let v = y as f32 / (n - 1).max(1) as f32;
        let area = aux
          .map(|aux| aux.drainage_area_at(u, v))
          .unwrap_or(cell_area);
        let flow = ((area / cell_area).max(1.0).ln() / log_largest).clamp(0.0, 1.0);
        0.35 + 0.65 * flow
      };
    }
  }

  weights
}

/// Average 2 x 2 blocks of a square `size` grid into a `size / 2` grid.
pub fn downsample(values: &[f32], size: usize) -> Vec<f32> {
  let half = size / 2;
  let mut out = vec![0.0; half * half];

  for y in 0..half {
    for x in 0..half {
      let a = values[(2 * y) * size + 2 * x];
      let b = values[(2 * y) * size + 2 * x + 1];
      let c = values[(2 * y + 1) * size + 2 * x];
      let d = values[(2 * y + 1) * size + 2 * x + 1];
      out[y * half + x] = (a + b + c + d) * 0.25;
    }
  }

  out
}

/// Bilinearly upsample a `size / 2` grid made by [`downsample`] back to
/// `size`, aligning block centres.
pub fn upsample(values: &[f32], size: usize) -> Vec<f32> {
  let half = size / 2;
  let mut out = vec![0.0; size * size];
  let at = |x: isize, y: isize| {
    let cx = x.clamp(0, half as isize - 1) as usize;
    let cy = y.clamp(0, half as isize - 1) as usize;
    values[cy * half + cx]
  };

  for y in 0..size {
    for x in 0..size {
      let fx = (x as f32 - 0.5) * 0.5;
      let fy = (y as f32 - 0.5) * 0.5;
      let x0 = fx.floor();
      let y0 = fy.floor();
      let tx = fx - x0;
      let ty = fy - y0;
      let (x0, y0) = (x0 as isize, y0 as isize);
      let top = at(x0, y0) + (at(x0 + 1, y0) - at(x0, y0)) * tx;
      let bottom = at(x0, y0 + 1) + (at(x0 + 1, y0 + 1) - at(x0, y0 + 1)) * tx;
      out[y * size + x] = top + (bottom - top) * ty;
    }
  }

  out
}

/// Totals of material moved, in cell units of height summed over cells.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ErosionTotals {
  /// Material dissolved from the terrain.
  pub eroded: f64,
  /// Material deposited back onto the terrain, including the load that
  /// settles when the water is finally removed.
  pub deposited: f64,
  /// Material carried into the sea or off the map edge.
  pub exported: f64,
  /// Material still in suspension.
  pub suspended: f64,
}

/// The state of the pipe model on one square grid, in cell units.
pub struct ErosionField {
  size: usize,
  /// Terrain height.
  pub terrain: Vec<f32>,
  water: Vec<f32>,
  sediment: Vec<f32>,
  scratch: Vec<f32>,
  flux: Vec<[f32; 4]>,
  /// Velocity x, velocity y, sine of the slope, rain weight.
  velocity: Vec<[f32; 4]>,
  /// Running totals for conservation checks.
  pub totals: ErosionTotals,
}

/// Flux slots: towards -x, +x, -y, +y.
const LEFT: usize = 0;
const RIGHT: usize = 1;
const UP: usize = 2;
const DOWN: usize = 3;

impl ErosionField {
  /// A dry field over `terrain` (cell units) with per-cell rain weights.
  pub fn new(size: usize, terrain: Vec<f32>, weights: &[f32]) -> Self {
    let count = size * size;

    Self {
      size,
      terrain,
      water: vec![0.0; count],
      sediment: vec![0.0; count],
      scratch: vec![0.0; count],
      flux: vec![[0.0; 4]; count],
      velocity: weights.iter().map(|w| [0.0, 0.0, 0.0, *w]).collect(),
      totals: ErosionTotals::default(),
    }
  }

  fn is_edge(&self, i: usize) -> bool {
    let (x, y) = (i % self.size, i / self.size);
    x == 0 || y == 0 || x == self.size - 1 || y == self.size - 1
  }

  /// Neighbour in each flux direction, or `None` off the map.
  fn neighbour(&self, i: usize, direction: usize) -> Option<usize> {
    let n = self.size;
    let (x, y) = (i % n, i / n);

    match direction {
      LEFT => (x > 0).then(|| i - 1),
      RIGHT => (x + 1 < n).then(|| i + 1),
      UP => (y > 0).then(|| i - n),
      _ => (y + 1 < n).then(|| i + n),
    }
  }

  /// Run one full hydraulic iteration (passes 1 to 6).
  pub fn hydraulic_step(&mut self, params: &ErosionParams) {
    self.rain(params);
    self.outflow();
    self.update_water();
    self.erode_and_deposit(params);
    self.advect();
    self.evaporate(params);
  }

  /// Pass 1.
  pub fn rain(&mut self, params: &ErosionParams) {
    for (water, velocity) in self.water.iter_mut().zip(&self.velocity) {
      if velocity[3] >= 0.0 {
        *water += params.rain * velocity[3];
      }
    }
  }

  /// Pass 2.
  pub fn outflow(&mut self) {
    for i in 0..self.terrain.len() {
      let surface = self.terrain[i] + self.water[i];
      let mut flux = self.flux[i];
      let mut total = 0.0;

      for (direction, slot) in flux.iter_mut().enumerate() {
        *slot = match self.neighbour(i, direction) {
          Some(j) => (*slot + PIPE_GAIN * (surface - self.terrain[j] - self.water[j])).max(0.0),
          None => 0.0,
        };
        total += *slot;
      }

      // Never let more water leave than the cell holds.
      if total > self.water[i] && total > 0.0 {
        let scale = self.water[i] / total;

        for slot in &mut flux {
          *slot *= scale;
        }
      }

      self.flux[i] = flux;
    }
  }

  fn inflow(&self, i: usize, direction: usize) -> f32 {
    // Water arriving from the neighbour in `direction` is that
    // neighbour's flux in the opposite direction.
    let opposite = [RIGHT, LEFT, DOWN, UP][direction];
    self
      .neighbour(i, direction)
      .map(|j| self.flux[j][opposite])
      .unwrap_or(0.0)
  }

  /// Pass 3.
  pub fn update_water(&mut self) {
    let n = self.size;
    let mut velocity = std::mem::take(&mut self.velocity);

    for (i, cell) in velocity.iter_mut().enumerate() {
      let out = self.flux[i];
      let inflow = [
        self.inflow(i, LEFT),
        self.inflow(i, RIGHT),
        self.inflow(i, UP),
        self.inflow(i, DOWN),
      ];
      let before = self.water[i];
      let mut after = (before + inflow.iter().sum::<f32>() - out.iter().sum::<f32>()).max(0.0);

      if cell[3] < 0.0 || self.is_edge(i) {
        after = 0.0;
      }

      let depth = ((before + after) * 0.5).max(1e-4);
      let flow_x = (inflow[LEFT] - out[LEFT] + out[RIGHT] - inflow[RIGHT]) * 0.5;
      let flow_y = (inflow[UP] - out[UP] + out[DOWN] - inflow[DOWN]) * 0.5;
      let (x, y) = (i % n, i / n);
      let left = self.terrain[y * n + x.saturating_sub(1)];
      let right = self.terrain[y * n + (x + 1).min(n - 1)];
      let up = self.terrain[y.saturating_sub(1) * n + x];
      let down = self.terrain[(y + 1).min(n - 1) * n + x];
      let dx = (right - left) * 0.5;
      let dy = (down - up) * 0.5;
      let tangent_squared = dx * dx + dy * dy;

      self.water[i] = after;
      *cell = [
        (flow_x / depth).clamp(-1.0, 1.0),
        (flow_y / depth).clamp(-1.0, 1.0),
        (tangent_squared / (1.0 + tangent_squared)).sqrt(),
        cell[3],
      ];
    }

    self.velocity = velocity;
  }

  /// Pass 4.
  pub fn erode_and_deposit(&mut self, params: &ErosionParams) {
    for i in 0..self.terrain.len() {
      let [vx, vy, sine, weight] = self.velocity[i];

      // Load reaching the sea is carried off by currents (dropping it all
      // in the first sea cell would heap bars along every shore), and load
      // reaching the map edge leaves the map.
      if weight < 0.0 || self.is_edge(i) {
        self.totals.exported += self.sediment[i] as f64;
        self.sediment[i] = 0.0;
        continue;
      }

      let speed = (vx * vx + vy * vy).sqrt();
      let depth = (self.water[i] / params.full_depth).clamp(0.0, 1.0);
      let capacity = params.capacity * sine.max(MIN_TILT) * speed * depth;
      let load = self.sediment[i];

      if capacity > load {
        let amount = DISSOLVE_RATE * (capacity - load);
        self.terrain[i] -= amount;
        self.sediment[i] += amount;
        self.totals.eroded += amount as f64;
      } else {
        let amount = DEPOSIT_RATE * (load - capacity);
        self.terrain[i] += amount;
        self.sediment[i] -= amount;
        self.totals.deposited += amount as f64;
      }
    }
  }

  /// Pass 5: forward semi-Lagrangian advection, gathered.
  pub fn advect(&mut self) {
    let n = self.size as isize;

    for y in 0..n {
      for x in 0..n {
        let mut gathered = 0.0;

        for oy in -1..=1 {
          for ox in -1..=1 {
            let (sx, sy) = (x + ox, y + oy);

            if sx < 0 || sy < 0 || sx >= n || sy >= n {
              continue;
            }

            let j = (sy * n + sx) as usize;
            let [vx, vy, _, _] = self.velocity[j];
            let tx = (sx as f32 + vx).clamp(0.0, (n - 1) as f32);
            let ty = (sy as f32 + vy).clamp(0.0, (n - 1) as f32);
            let wx = (1.0 - (tx - x as f32).abs()).max(0.0);
            let wy = (1.0 - (ty - y as f32).abs()).max(0.0);
            gathered += self.sediment[j] * wx * wy;
          }
        }

        self.scratch[(y * n + x) as usize] = gathered;
      }
    }
  }

  /// Pass 6, which also takes the advected load from pass 5.
  pub fn evaporate(&mut self, params: &ErosionParams) {
    for water in &mut self.water {
      *water *= 1.0 - params.evaporation;
    }

    std::mem::swap(&mut self.sediment, &mut self.scratch);
  }

  /// One thermal iteration: exchange, then apply.
  pub fn thermal_step(&mut self, params: &ErosionParams) {
    let n = self.size as isize;

    for y in 0..n {
      for x in 0..n {
        let i = (y * n + x) as usize;
        let mut change = 0.0;

        for (ox, oy) in crate::terrain::drainage::OFFSETS {
          let (nx, ny) = (x + ox as isize, y + oy as isize);

          if nx < 0 || ny < 0 || nx >= n || ny >= n {
            continue;
          }

          let j = (ny * n + nx) as usize;
          let critical = params.talus
            * if ox != 0 && oy != 0 {
              std::f32::consts::SQRT_2
            } else {
              1.0
            };
          let difference = self.terrain[j] - self.terrain[i];
          change +=
            THERMAL_RATE * ((difference - critical).max(0.0) - (-difference - critical).max(0.0));
        }

        self.scratch[i] = change;
      }
    }

    for (terrain, change) in self.terrain.iter_mut().zip(&self.scratch) {
      *terrain += *change;
    }
  }

  /// Remove the water and settle the remaining load where it is.
  pub fn settle(&mut self) {
    for (terrain, sediment) in self.terrain.iter_mut().zip(self.sediment.iter_mut()) {
      *terrain += *sediment;
      self.totals.deposited += *sediment as f64;
      *sediment = 0.0;
    }

    self.water.iter_mut().for_each(|w| *w = 0.0);
    self.flux.iter_mut().for_each(|f| *f = [0.0; 4]);
  }

  /// Update the suspended total from the current load.
  pub fn measure_suspended(&mut self) {
    self.totals.suspended = self.sediment.iter().map(|s| *s as f64).sum();
  }

  /// Run `iterations`, interleaving thermal steps with hydraulic ones,
  /// then settle. `tick` is called after every iteration.
  pub fn run(
    &mut self,
    params: &ErosionParams,
    iterations: ErosionIterations,
    tick: &mut dyn FnMut(),
  ) {
    for step in 0..iterations.hydraulic.max(iterations.thermal) {
      if step < iterations.hydraulic {
        self.hydraulic_step(params);
      }

      if step < iterations.thermal {
        self.thermal_step(params);
      }

      tick();
    }

    self.settle();
  }
}

/// Apply budgeted CPU reference erosion to a heightmap, with the
/// landform's defaults for unset options, reporting progress as the
/// `"erosion"` phase.
pub fn apply_erosion(
  map: &mut HeightMap,
  options: &ErosionOptions,
  landform: &Landform,
  progress: Progress<'_>,
) -> VistaResult<()> {
  let size = map.metadata.width as usize;

  if size != map.metadata.height as usize || size < 4 {
    return Ok(());
  }

  let iterations = erosion_iterations(options);
  let total = (iterations.hydraulic.max(iterations.thermal)).max(1);
  let mut done = 0u32;
  let mut reported = 0.0f32;
  progress("erosion", 0.0);
  let mut tick = || {
    done += 1;
    let fraction = done as f32 / total as f32;

    if fraction - reported >= 0.1 || done == total {
      reported = fraction;
      progress("erosion", fraction.min(1.0));
    }
  };

  let metres = map.metadata.metres_per_sample;

  for phase in erosion_schedule(iterations, size as u32) {
    if phase.half {
      let half = size / 2;
      let cell = metres * 2.0;
      let low = downsample(&map.heights, size);
      let terrain: Vec<f32> = low.iter().map(|h| h / cell).collect();
      let mut field = ErosionField::new(half, terrain, &rain_weights(map, half as u32));
      field.run(
        &ErosionParams::new(options, landform, cell),
        phase.iterations,
        &mut tick,
      );
      let change: Vec<f32> = field
        .terrain
        .iter()
        .zip(&low)
        .map(|(eroded, before)| eroded * cell - before)
        .collect();

      for (height, delta) in map.heights.iter_mut().zip(upsample(&change, size)) {
        *height += delta;
      }
    } else {
      let terrain: Vec<f32> = map.heights.iter().map(|h| h / metres).collect();
      let mut field = ErosionField::new(size, terrain, &rain_weights(map, size as u32));
      field.run(
        &ErosionParams::new(options, landform, metres),
        phase.iterations,
        &mut tick,
      );

      for (height, eroded) in map.heights.iter_mut().zip(&field.terrain) {
        *height = eroded * metres;
      }
    }
  }

  update_stats(&map.heights, &map.no_data, &mut map.metadata);
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use vista_types::LandformKind;

  fn params() -> ErosionParams {
    ErosionParams::new(
      &ErosionOptions::default(),
      &Landform::preset(LandformKind::Continental),
      10.0,
    )
  }

  /// A V-shaped valley sloping down along +y, opening onto a flat plain
  /// in the last fifth of the map. Heights in cell units.
  fn valley(size: usize) -> Vec<f32> {
    let mut terrain = vec![0.0; size * size];
    let plain = size * 4 / 5;

    for y in 0..size {
      for x in 0..size {
        let across = (x as f32 - size as f32 * 0.5).abs() * 0.35;
        let along = (plain as f32 - y as f32).max(0.0) * 0.25;
        terrain[y * size + x] = 2.0 + along + if y < plain { across } else { 0.0 };
      }
    }

    terrain
  }

  #[test]
  fn sediment_is_conserved() {
    let size = 48;
    let weights = vec![1.0; size * size];
    let mut field = ErosionField::new(size, valley(size), &weights);
    let before: f64 = field.terrain.iter().map(|t| *t as f64).sum();

    for _ in 0..80 {
      field.hydraulic_step(&params());
    }

    field.measure_suspended();
    let totals = field.totals;
    let balance = totals.eroded - totals.deposited - totals.exported - totals.suspended;
    assert!(totals.eroded > 1.0, "{totals:?}");
    assert!(balance.abs() < 0.01 * totals.eroded, "{totals:?}");

    field.settle();
    let after: f64 = field.terrain.iter().map(|t| *t as f64).sum();
    let lost = before - after - field.totals.exported;
    assert!(lost.abs() < 0.01 * field.totals.eroded, "lost {lost}");
  }

  #[test]
  fn thermal_erosion_conserves_material_and_relaxes_to_the_talus_angle() {
    let size = 16;
    let mut terrain = vec![0.0; size * size];
    terrain[8 * size + 8] = 20.0;
    let weights = vec![1.0; size * size];
    let mut field = ErosionField::new(size, terrain, &weights);
    let params = params();

    for _ in 0..400 {
      field.thermal_step(&params);
    }

    let total: f32 = field.terrain.iter().sum();
    assert!((total - 20.0).abs() < 1e-3);

    for y in 0..size {
      for x in 1..size {
        let step = (field.terrain[y * size + x] - field.terrain[y * size + x - 1]).abs();
        assert!(step <= params.talus * 1.05 + 1e-3, "step {step}");
      }
    }
  }

  #[test]
  fn an_open_valley_builds_a_fan_on_the_plain() {
    let size = 48;
    let weights = vec![1.0; size * size];
    let original = valley(size);
    let mut field = ErosionField::new(size, original.clone(), &weights);
    field.run(
      &params(),
      ErosionIterations {
        hydraulic: 150,
        thermal: 0,
      },
      &mut || {},
    );

    // The lowest 20 % of the interior cells, where the valley opens out.
    let mut interior: Vec<usize> = (0..size * size)
      .filter(|i| {
        let (x, y) = (i % size, i / size);
        x > 0 && y > 0 && x < size - 1 && y < size - 1
      })
      .collect();
    interior.sort_by(|a, b| original[*a].total_cmp(&original[*b]).then(a.cmp(b)));
    let lowest = &interior[..interior.len() / 5];
    let net: f32 = lowest
      .iter()
      .map(|i| field.terrain[*i] - original[*i])
      .sum();
    let upper = &interior[interior.len() / 2..];
    let carved: f32 = upper.iter().map(|i| field.terrain[*i] - original[*i]).sum();

    assert!(net > 0.0, "net change on the plain {net}");
    assert!(carved < 0.0, "net change in the valley {carved}");
  }

  #[test]
  fn schedule_splits_sixty_forty_and_quality_caps_requests() {
    let iterations = erosion_iterations(&ErosionOptions {
      quality: Some(ErosionQuality::High),
      ..ErosionOptions::default()
    });
    assert_eq!(
      iterations,
      ErosionIterations {
        hydraulic: 200,
        thermal: 100
      }
    );

    let phases = erosion_schedule(iterations, 512);
    assert_eq!(phases.len(), 2);
    assert!(phases[0].half);
    assert_eq!(phases[0].iterations.hydraulic, 120);
    assert_eq!(phases[1].iterations.hydraulic, 80);

    let capped = erosion_iterations(&ErosionOptions {
      hydraulic_iterations: Some(1000),
      ..ErosionOptions::default()
    });
    assert_eq!(capped.hydraulic, 120);
    assert_eq!(erosion_schedule(capped, 32).len(), 1);
  }

  #[test]
  fn downsample_and_upsample_round_trip_smooth_fields() {
    let size = 32;
    let field: Vec<f32> = (0..size * size).map(|i| (i % size) as f32 * 2.0).collect();
    let back = upsample(&downsample(&field, size), size);

    for y in 0..size {
      for x in 1..size - 1 {
        assert!((back[y * size + x] - field[y * size + x]).abs() < 1e-3);
      }
    }
  }
}
