//! The regional weather map: how the blended weather varies across the
//! land, so a storm can approach while the coast is in sunshine.
//!
//! The field is a pure function of the blended preset, the seed and the
//! wind drift: [`RegionalField::evaluate`] gives coverage, precipitation,
//! storminess and humidity anywhere. A 128 x 128 grid of it, centred on
//! the terrain, is refreshed 16 rows a frame for the clouds and the wet
//! ground on the GPU; the camera and `weatherAt` evaluate the function
//! itself, so they never wait on the grid.

use crate::maths::smoothstep;
use crate::terrain::noise::{noise_seed, simplex, simplex_d};

use super::presets::{field, Values};

/// Texels along each side of the grid.
pub const GRID: usize = 128;
/// Rows refreshed per frame: the whole grid every eight frames.
pub const ROWS_PER_FRAME: usize = 16;

/// The weather at one place, each 0 to 1 (precipitation above 1 in a
/// downpour).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RegionalSample {
  /// Cloud coverage.
  pub coverage: f32,
  /// Rain and snow together, before the temperature splits them.
  pub precipitation: f32,
  /// How much of a storm cell is overhead.
  pub storminess: f32,
  /// Relative humidity.
  pub humidity: f32,
}

/// What the field is built from: the blended preset, less any spread when
/// the weather is not regional.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldParams {
  /// Mean coverage.
  pub coverage: f32,
  /// Coverage spread.
  pub coverage_spread: f32,
  /// Rain and snow together, scaled by `precipitationScale`.
  pub precipitation: f32,
  /// Precipitation spread.
  pub precipitation_spread: f32,
  /// Storm cell size in metres.
  pub cell_metres: f32,
  /// 0 for smooth fields, 1 for discrete storm cells.
  pub cellularity: f32,
  /// Mean humidity.
  pub humidity: f32,
}

impl FieldParams {
  /// The field of a blended preset. Without `regional`, every spread is
  /// 0, so the field is the same everywhere.
  pub fn of(values: &Values, precipitation_scale: f32, regional: bool) -> Self {
    let spread = if regional { 1.0 } else { 0.0 };
    Self {
      coverage: values.get(field::COVERAGE),
      coverage_spread: values.get(field::COVERAGE_SPREAD) * spread,
      precipitation: (values.get(field::RAIN) + values.get(field::SNOW))
        * precipitation_scale.max(0.0),
      precipitation_spread: values.get(field::PRECIPITATION_SPREAD) * spread,
      cell_metres: values.get(field::CELL_SIZE).max(1.0) * 1_000.0,
      cellularity: values.get(field::CELLULARITY) * spread,
      humidity: values.get(field::HUMIDITY),
    }
  }
}

impl FieldParams {
  /// Whether the field changed too much since `before` for a band of the
  /// grid a frame to keep up.
  pub fn jumps_from(&self, before: &Self) -> bool {
    let near = |a: f32, b: f32| (a - b).abs() <= 0.05;
    !(near(self.coverage, before.coverage)
      && near(self.coverage_spread, before.coverage_spread)
      && near(self.precipitation, before.precipitation)
      && near(self.precipitation_spread, before.precipitation_spread)
      && near(self.cellularity, before.cellularity)
      && near(self.humidity, before.humidity)
      && (self.cell_metres - before.cell_metres).abs() <= before.cell_metres * 0.05)
  }
}

/// The regional field and its grid.
#[derive(Clone, Debug)]
pub struct RegionalField {
  seed: u32,
  /// Where the air that is now at the origin was at time 0, in metres:
  /// the integral of the cloud-layer wind.
  drift: [f64; 2],
  /// Grid side in metres.
  size_metres: f32,
  params: FieldParams,
  /// RGBA half floats: coverage, precipitation, storminess, humidity.
  grid: Vec<[u16; 4]>,
  next_row: usize,
  /// Rows changed since the grid was last taken for upload: the first and
  /// how many, wrapping around.
  dirty: Option<(usize, usize)>,
}

/// Rows of the grid to upload.
#[derive(Debug, PartialEq)]
pub struct GridUpload<'a> {
  /// The first row.
  pub first_row: u32,
  /// Rows.
  pub rows: u32,
  /// RGBA half floats, row-major.
  pub data: &'a [u16],
}

impl RegionalField {
  /// A field for a seed over a square `size_metres` across.
  pub fn new(seed: u64, size_metres: f32, params: FieldParams) -> Self {
    let mut field = Self {
      seed: noise_seed(seed, 0x7e47_4e52),
      drift: [0.0; 2],
      size_metres,
      params,
      grid: vec![[0; 4]; GRID * GRID],
      next_row: 0,
      dirty: None,
    };
    field.refresh_rows(GRID);
    field
  }

  /// Change the region's size, refilling the grid.
  pub fn set_size(&mut self, size_metres: f32) {
    if size_metres != self.size_metres {
      self.size_metres = size_metres;
      self.refresh_rows(GRID);
    }
  }

  /// Grid side in metres.
  pub fn size_metres(&self) -> f32 {
    self.size_metres
  }

  /// The wind drift so far, in metres.
  pub fn drift(&self) -> [f64; 2] {
    self.drift
  }

  /// The parameters the field is built from.
  pub fn params(&self) -> &FieldParams {
    &self.params
  }

  /// Carry the field downwind by `offset` metres and take new parameters.
  pub fn advance(&mut self, offset: [f32; 2], params: FieldParams) {
    self.drift[0] += offset[0] as f64;
    self.drift[1] += offset[1] as f64;
    self.params = params;
  }

  /// The field at a world position.
  pub fn evaluate(&self, x: f32, z: f32) -> RegionalSample {
    evaluate(
      self.seed,
      &self.params,
      [x as f64 - self.drift[0], z as f64 - self.drift[1]],
      &mut Cells::default(),
    )
  }

  /// Refresh the next `rows` rows of the grid.
  pub fn refresh_rows(&mut self, rows: usize) {
    let start = self.next_row;
    let rows = rows.min(GRID);

    for _ in 0..rows {
      let row = self.next_row;
      self.refresh_row(row);
      self.next_row = (row + 1) % GRID;
    }

    self.mark(start, rows);
  }

  /// Refresh the rows that cross a band of world z, from `min_z` to
  /// `max_z` metres.
  pub fn refresh_band(&mut self, min_z: f32, max_z: f32) {
    let texel = self.size_metres / GRID as f32;
    let row_of = |z: f32| {
      ((z + self.size_metres * 0.5) / texel)
        .floor()
        .clamp(0.0, GRID as f32 - 1.0) as usize
    };
    let (first, last) = (row_of(min_z), row_of(max_z));

    for row in first..=last {
      self.refresh_row(row);
    }

    self.mark(first, last + 1 - first);
  }

  fn refresh_row(&mut self, row: usize) {
    let texel = self.size_metres / GRID as f32;
    let origin = -self.size_metres * 0.5;
    let z = origin + (row as f32 + 0.5) * texel;
    // A storm cell spans many texels, so neighbours share its points.
    let mut cells = Cells::default();
    let mut sample_at = |column: usize| {
      let x = origin + (column as f32 + 0.5) * texel;
      let p = [x as f64 - self.drift[0], z as f64 - self.drift[1]];
      let sample = evaluate(self.seed, &self.params, p, &mut cells);
      [
        sample.coverage,
        sample.precipitation,
        sample.storminess,
        sample.humidity,
      ]
    };
    let mut values = [[0.0; 4]; GRID];

    // The field's features span several kilometres, so every other texel
    // is evaluated and the ones between are the mean of their neighbours:
    // half the cost, and no visible difference at 128 texels across.
    for column in (0..GRID).step_by(2).chain([GRID - 1]) {
      values[column] = sample_at(column);
    }

    for column in (1..GRID - 1).step_by(2) {
      values[column] =
        std::array::from_fn(|k| (values[column - 1][k] + values[column + 1][k]) * 0.5);
    }

    for (texel, value) in self.grid[row * GRID..(row + 1) * GRID]
      .iter_mut()
      .zip(values)
    {
      *texel = value.map(half);
    }
  }

  fn mark(&mut self, start: usize, rows: usize) {
    self.dirty = Some(match self.dirty {
      None => (start, rows),
      // Contiguous with what is already waiting (the usual case, a band
      // per frame): extend it.
      Some((first, count)) if (first + count) % GRID == start => (first, (count + rows).min(GRID)),
      Some(_) => (0, GRID),
    });
  }

  /// Rows changed since the last call, for an `rgba16float` texture.
  pub fn take_upload(&mut self) -> Option<GridUpload<'_>> {
    let (first, count) = self.dirty.take()?;
    // A band that wraps past the last row goes up whole.
    let (first, count) = if first + count > GRID {
      (0, GRID)
    } else {
      (first, count)
    };
    let texels: &[u16] = bytemuck::cast_slice(&self.grid[first * GRID..(first + count) * GRID]);
    Some(GridUpload {
      first_row: first as u32,
      rows: count as u32,
      data: texels,
    })
  }

  /// Precipitation from the grid, bilinearly, at a world position: what
  /// the GPU samples.
  pub fn grid_precipitation(&self, x: f32, z: f32) -> f32 {
    let texel = self.size_metres / GRID as f32;
    let u = ((x + self.size_metres * 0.5) / texel - 0.5).clamp(0.0, GRID as f32 - 1.0);
    let v = ((z + self.size_metres * 0.5) / texel - 0.5).clamp(0.0, GRID as f32 - 1.0);
    let (x0, y0) = (u.floor() as usize, v.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(GRID - 1), (y0 + 1).min(GRID - 1));
    let (fx, fy) = (u - x0 as f32, v - y0 as f32);
    let at = |x: usize, y: usize| from_half(self.grid[y * GRID + x][1]);
    let top = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * fx;
    let bottom = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * fx;
    top + (bottom - top) * fy
  }

  /// Share of direct sunlight the clouds let through towards `position`,
  /// from four samples of the field along the sun's direction through the
  /// cloud layer: 1 under a clear sky, down to 0.1 under a thick deck.
  pub fn sun_transmittance(
    &self,
    position: [f32; 3],
    sun: [f32; 3],
    cloud_base: f32,
    cloud_thickness: f32,
  ) -> f32 {
    let mut coverage = 0.0;

    for step in 0..4 {
      let height = cloud_base + cloud_thickness * (step as f32 + 0.5) * 0.25 - position[1];
      let travel = height.max(0.0) / sun[1].max(0.08);
      coverage += self
        .evaluate(position[0] + sun[0] * travel, position[2] + sun[2] * travel)
        .coverage
        * 0.25;
    }

    transmittance_of_coverage(coverage)
  }
}

/// Direct sunlight through a cloud layer of mean coverage `coverage`: as
/// before the weather varied across the map, a deck thicker than 60 %
/// starts to hide the sun and a full one leaves a tenth of it.
pub fn transmittance_of_coverage(coverage: f32) -> f32 {
  1.0 - 0.9 * ((coverage - 0.6) / 0.35).clamp(0.0, 1.0)
}

fn evaluate(
  seed: u32,
  params: &FieldParams,
  p: [f64; 2],
  cells_cache: &mut Cells,
) -> RegionalSample {
  // Large-scale structure spans about three cells: a broad octave, and a
  // finer one warped by the broad one's gradient so fronts curl instead of
  // lying in blobs. Two noise lookups a texel keep 16 rows a frame cheap.
  let scale = params.cell_metres * 3.0;
  let q = [(p[0] / scale as f64) as f32, (p[1] / scale as f64) as f32];
  let mut n = 0.0;
  let mut humidity_noise = 0.0;

  if params.coverage_spread > 0.0 || params.precipitation_spread > 0.0 {
    let (warp, dx, dy) = simplex_d(seed, q[0] * 0.5, q[1] * 0.5);
    let fine = simplex(seed ^ 0x51, q[0] + dx * 0.12, q[1] + dy * 0.12);
    n = (fine * 0.7 + warp * 0.45) * 1.3;
    humidity_noise = warp;
  }

  let mut storminess = 0.0;
  let mut cells = 0.0;

  if params.cellularity > 0.0 {
    let c = cells_cache.worley(
      seed ^ 0xce11,
      (p[0] / params.cell_metres as f64) as f32 + n * 0.25,
      (p[1] / params.cell_metres as f64) as f32,
    );
    storminess = params.cellularity * smoothstep((c - 0.25) / 0.55);
    // Cells build cloud at their cores and clear the air between them.
    cells = params.cellularity * (smoothstep((c - 0.2) / 0.6) - 0.4) * 0.9;
  }

  let coverage = (params.coverage + params.coverage_spread * n + cells).clamp(0.0, 1.0);
  let mut precipitation = 0.0;

  if params.precipitation > 0.0 {
    let variation = if params.precipitation_spread > 0.0 {
      // The two octaves the other way round: heavier and lighter rain
      // within the cloud, not following its edge.
      (humidity_noise * 0.7 - n * 0.4) * params.precipitation_spread * 1.5
    } else {
      0.0
    };
    // Rain and snow fall only from thick cloud, and hardest in storm cores.
    precipitation = params.precipitation
      * smoothstep((coverage - 0.55) / 0.25)
      * (1.0 + variation + storminess * 0.5).clamp(0.0, 2.0);
  }

  RegionalSample {
    coverage,
    precipitation,
    storminess,
    humidity: (params.humidity + 0.1 * humidity_noise).clamp(0.0, 1.0),
  }
}

/// The jittered points of the 3×3 cells around the last cell asked for,
/// kept while the next lookup falls in the same cell.
#[derive(Default)]
struct Cells {
  key: Option<(u32, i32, i32)>,
  points: [[f32; 2]; 9],
}

impl Cells {
  /// Cellular noise: 1 at the centre of a jittered cell, falling to 0 a
  /// cell width away.
  fn worley(&mut self, seed: u32, x: f32, y: f32) -> f32 {
    let (cx, cy) = (x.floor() as i32, y.floor() as i32);

    if self.key != Some((seed, cx, cy)) {
      self.key = Some((seed, cx, cy));

      for (k, point) in self.points.iter_mut().enumerate() {
        let (gx, gy) = (cx + k as i32 % 3 - 1, cy + k as i32 / 3 - 1);
        let mut h =
          seed ^ (gx as u32).wrapping_mul(0x27d4_eb2d) ^ (gy as u32).wrapping_mul(0x1656_67b1);
        h ^= h >> 15;
        h = h.wrapping_mul(0x2c1b_3c6d);
        h ^= h >> 12;
        *point = [
          gx as f32 + (h & 0xffff) as f32 / 65_535.0,
          gy as f32 + (h >> 16 & 0xffff) as f32 / 65_535.0,
        ];
      }
    }

    let nearest = self
      .points
      .iter()
      .map(|[px, py]| (px - x) * (px - x) + (py - y) * (py - y))
      .fold(f32::MAX, f32::min);
    (1.0 - nearest.sqrt()).clamp(0.0, 1.0)
  }
}

/// IEEE half-float bits as an `f32`, for the field's non-negative values.
fn from_half(bits: u16) -> f32 {
  let exponent = ((bits >> 10) & 0x1f) as i32;
  let mantissa = (bits & 0x3ff) as f32 / 1_024.0;

  if exponent == 0 {
    return mantissa * 2f32.powi(-14);
  }

  (1.0 + mantissa) * 2f32.powi(exponent - 15)
}

/// An `f32` as IEEE half-float bits, rounding to nearest. The field's
/// values are small and never negative zero or NaN.
fn half(value: f32) -> u16 {
  let bits = value.to_bits();
  let sign = ((bits >> 16) & 0x8000) as u16;
  let exponent = ((bits >> 23) & 0xff) as i32 - 127 + 15;
  let mantissa = bits & 0x7f_ffff;

  if exponent <= 0 {
    return sign;
  }

  if exponent >= 31 {
    return sign | 0x7c00;
  }

  let rounded = (exponent as u32) << 10 | (mantissa + 0x1000) >> 13;
  sign | rounded.min(0x7bff) as u16
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::weather::presets::PresetTable;
  use vista_types::WeatherKind;

  fn storm(regional: bool) -> FieldParams {
    let values = PresetTable::default().values(&WeatherKind::Storm);
    FieldParams::of(&values, 1.0, regional)
  }

  #[test]
  fn the_field_is_deterministic() {
    let a = RegionalField::new(7, 64_000.0, storm(true));
    let b = RegionalField::new(7, 64_000.0, storm(true));
    let c = RegionalField::new(8, 64_000.0, storm(true));

    assert_eq!(a.grid, b.grid);
    assert_ne!(a.grid, c.grid);
    assert_eq!(a.evaluate(1234.0, -987.0), b.evaluate(1234.0, -987.0));
  }

  #[test]
  fn the_field_is_carried_by_the_wind() {
    let start = RegionalField::new(3, 64_000.0, storm(true));
    let mut later = start.clone();

    for _ in 0..300 {
      later.advance([12.5, -4.0], storm(true));
    }

    let drift = later.drift();

    for (x, z) in [(0.0, 0.0), (5_000.0, -2_000.0), (-9_000.0, 13_000.0)] {
      let moved = later.evaluate(x + drift[0] as f32, z + drift[1] as f32);
      let original = start.evaluate(x, z);
      assert!((moved.coverage - original.coverage).abs() < 1e-3);
      assert!((moved.precipitation - original.precipitation).abs() < 1e-3);
      assert!((moved.storminess - original.storminess).abs() < 1e-3);
    }
  }

  #[test]
  fn a_uniform_field_has_no_spread() {
    let field = RegionalField::new(3, 64_000.0, storm(false));
    let first = field.evaluate(0.0, 0.0);

    for (x, z) in [(3_000.0, 1_000.0), (-20_000.0, 7_000.0)] {
      assert_eq!(field.evaluate(x, z).coverage, first.coverage);
      assert_eq!(field.evaluate(x, z).precipitation, first.precipitation);
    }

    assert_eq!(field.params().coverage_spread, 0.0);
    assert_eq!(field.params().cellularity, 0.0);
  }

  #[test]
  fn precipitation_falls_only_under_thick_cloud() {
    let field = RegionalField::new(11, 64_000.0, storm(true));
    let mut wet = 0;
    let mut thin = 0;

    for j in 0..80 {
      for i in 0..80 {
        let sample = field.evaluate(i as f32 * 700.0 - 28_000.0, j as f32 * 700.0 - 28_000.0);

        if sample.coverage < 0.55 {
          thin += 1;
          assert_eq!(sample.precipitation, 0.0, "{sample:?}");
        } else if sample.precipitation > 0.5 {
          wet += 1;
        }
      }
    }

    // A storm has both: cells of rain and breaks between them.
    assert!(wet > 100 && thin > 100, "wet {wet}, thin {thin}");
  }

  #[test]
  fn uploads_carry_only_the_rows_that_changed() {
    let mut field = RegionalField::new(3, 64_000.0, storm(true));
    assert_eq!(
      field.take_upload().map(|upload| upload.rows),
      Some(GRID as u32)
    );
    assert!(field.take_upload().is_none());

    field.refresh_rows(ROWS_PER_FRAME);
    field.refresh_rows(ROWS_PER_FRAME);
    let upload = field.take_upload().unwrap();
    assert_eq!(
      (upload.first_row, upload.rows),
      (0, 2 * ROWS_PER_FRAME as u32)
    );
    assert_eq!(upload.data.len(), 2 * ROWS_PER_FRAME * GRID * 4);

    // The grid's bilinear precipitation agrees with the field at texel
    // centres.
    let texel = 64_000.0 / GRID as f32;
    let (x, z) = (-32_000.0 + 40.5 * texel, -32_000.0 + 70.5 * texel);
    field.refresh_band(z, z);
    let exact = field.evaluate(x, z).precipitation;
    assert!((field.grid_precipitation(x, z) - exact).abs() < 0.01);
  }

  #[test]
  fn only_a_sudden_change_refreshes_the_whole_grid() {
    let storm = storm(true);
    let nudged = FieldParams {
      coverage: storm.coverage + 0.01,
      ..storm
    };
    let cleared = FieldParams {
      coverage: 0.08,
      precipitation: 0.0,
      ..storm
    };

    assert!(!nudged.jumps_from(&storm));
    assert!(cleared.jumps_from(&storm));
  }

  #[test]
  fn half_floats_round_trip_the_useful_range() {
    for value in [0.0f32, 0.25, 0.5, 0.999, 1.0, 1.6, 2.0] {
      let back = from_half(half(value));
      assert!((back - value).abs() < 0.002, "{value} -> {back}");
    }
  }
}
