//! Wet and snowy ground that follows the weather where it fell.
//!
//! Each place on the terrain keeps its wetness, the water in its puddles
//! and its snow depth, each 0 to 1. [`step`] advances one place by `dt`
//! seconds; `shaders/surface_weather.wgsl` runs the same maths for every
//! texel of the surface weather map on the GPU, and [`SurfaceMirror`] runs
//! it on a coarse grid on the CPU for `weatherAt`. The rates are
//! zero-order (independent of the current state), so a large `dt` gives
//! the same result as many small ones until a value reaches 0 or 1.

use crate::maths::smoothstep;
use crate::weather::snow_fraction;

/// Wetness gained per second in full rain.
pub const WET_RATE: f32 = 1.0 / 120.0;
/// Wetness lost per second in mild, still, sunlit air, before the factors
/// in [`evaporation`].
pub const EVAPORATION: f32 = 1.0 / 2_400.0;
/// A hollow's puddle fills in about three minutes of full rain.
pub const PUDDLE_FILL: f32 = 1.0 / 180.0;
/// Puddles evaporate at this share of the ground's rate, so they are last
/// to go.
pub const PUDDLE_EVAPORATION: f32 = 0.3;
/// Snow depth gained per second in full snowfall.
pub const SNOW_RATE: f32 = 1.0 / 150.0;
/// Snow depth lost per second per degree above 1 °C in full sun.
pub const MELT_RATE: f32 = 1.0 / 1_800.0;

/// Wetness above which hollows collect water.
pub const PUDDLE_THRESHOLD: f32 = 0.6;
/// Puddles only form on ground flatter than this.
pub const PUDDLE_SLOPE_DEGREES: f32 = 3.0;

/// What the ground is wetted and dried by.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SurfaceInputs {
  /// Rain and snow falling together, before the temperature splits them.
  pub precipitation: f32,
  /// Air temperature in °C.
  pub celsius: f32,
  /// Sunlight reaching the ground, 0 to 1: the sine of its elevation
  /// times the clouds' transmittance.
  pub sun: f32,
  /// Wind speed in metres per second.
  pub wind: f32,
  /// Share of the ground under tree crowns, 0 to 1.
  pub canopy: f32,
  /// Ground slope in degrees.
  pub slope_degrees: f32,
  /// How deep a hollow the place lies in, in metres: minus the height's
  /// Laplacian times the cell area, positive in hollows.
  pub hollow_metres: f32,
  /// A fixed 0 to 1 value per place, so the ground dries in patches.
  pub patch: f32,
}

/// The ground at one place, each 0 to 1.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SurfaceCell {
  /// Wetness.
  pub wetness: f32,
  /// How full a hollow's puddle is (the water is this times
  /// [`puddle_capacity`]).
  pub puddles: f32,
  /// Snow depth.
  pub snow: f32,
}

/// How much a puddle a place can hold, 0 to 1: flat ground that is
/// concave, fully once it lies 5 cm below its neighbours.
pub fn puddle_capacity(slope_degrees: f32, hollow_metres: f32) -> f32 {
  let flat =
    1.0 - smoothstep((slope_degrees - PUDDLE_SLOPE_DEGREES * 0.6) / (PUDDLE_SLOPE_DEGREES * 0.4));
  flat * smoothstep(hollow_metres / 0.05)
}

/// Wetness lost per second: faster with sun, wind and warmth, slower under
/// trees, and faster on slopes that shed their water.
pub fn evaporation(inputs: &SurfaceInputs) -> f32 {
  let warmth = (0.3 + inputs.celsius / 20.0).clamp(0.1, 2.0);
  EVAPORATION
    * (0.3 + inputs.sun.clamp(0.0, 1.0))
    * (0.5 + inputs.wind.max(0.0) / 10.0)
    * warmth
    * (1.0 - inputs.canopy.clamp(0.0, 1.0) * 0.5)
    * (0.6 + 0.8 * inputs.patch)
    * (1.0 + inputs.slope_degrees.max(0.0) / 15.0)
}

/// Advance one place by `dt` seconds.
pub fn step(cell: SurfaceCell, inputs: &SurfaceInputs, dt: f32) -> SurfaceCell {
  let frozen = snow_fraction(inputs.celsius);
  let precipitation = inputs.precipitation.max(0.0);
  // Trees catch some of the rain before it reaches the ground.
  let rain = precipitation * (1.0 - frozen) * (1.0 - inputs.canopy.clamp(0.0, 1.0) * 0.3);
  let snowfall = precipitation * frozen;
  let dry = evaporation(inputs);
  let mut snow = cell.snow;

  if inputs.celsius < 0.5 {
    snow += snowfall * SNOW_RATE * dt;
  }

  let mut melt = 0.0;

  if inputs.celsius > 1.0 {
    melt = (MELT_RATE * (inputs.celsius - 1.0) * (0.5 + inputs.sun.clamp(0.0, 1.0)) * dt).min(snow);
    snow -= melt;
  }

  // Melt water runs into the ground and the hollows like light rain.
  let water = rain.min(1.0) + melt / dt.max(1e-3) * 60.0;
  let wetness = cell.wetness + (water * WET_RATE - dry) * dt;
  let filling = if cell.wetness.max(wetness) > PUDDLE_THRESHOLD {
    water * PUDDLE_FILL
  } else {
    0.0
  };
  let puddles = (cell.puddles + (filling - dry * PUDDLE_EVAPORATION) * dt).clamp(0.0, 1.0);
  let capacity = puddle_capacity(inputs.slope_degrees, inputs.hollow_metres);

  SurfaceCell {
    // Standing water keeps the ground around it wet.
    wetness: wetness.clamp(0.0, 1.0).max(puddles * capacity * 0.8),
    puddles,
    snow: snow.clamp(0.0, 1.0),
  }
}

/// A fixed 0 to 1 value per texel, so neighbouring places dry at slightly
/// different rates. The shader hashes texel coordinates the same way.
pub fn patch(x: u32, y: u32) -> f32 {
  let mut h = x.wrapping_mul(0x8da6_b343) ^ y.wrapping_mul(0xd816_3841);
  h ^= h >> 13;
  h = h.wrapping_mul(0x5bd1_e995);
  h ^= h >> 15;
  (h & 0xffff) as f32 / 65_535.0
}

/// The static inputs of one mirror cell.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CellGround {
  /// Temperature in °C before the weather's offset.
  pub celsius: f32,
  /// Share under tree crowns.
  pub canopy: f32,
  /// Slope in degrees.
  pub slope_degrees: f32,
  /// Hollow depth in metres.
  pub hollow_metres: f32,
  /// Snow lying all year, 0 to 1.
  pub permanent_snow: f32,
}

/// The ground's weather on a coarse grid over the terrain, on the CPU.
#[derive(Clone, Debug, Default)]
pub struct SurfaceMirror {
  /// Cells per row and rows.
  pub width: usize,
  /// Rows.
  pub height: usize,
  /// Terrain half extents in metres.
  pub half: [f32; 2],
  /// Static inputs per cell.
  pub ground: Vec<CellGround>,
  /// State per cell.
  pub cells: Vec<SurfaceCell>,
}

impl SurfaceMirror {
  /// A mirror of `ground` cells, `width` by `height`, over a terrain with
  /// half extents `half`, settled into `settled`.
  pub fn new(
    width: usize,
    height: usize,
    half: [f32; 2],
    ground: Vec<CellGround>,
    settled: SurfaceCell,
  ) -> Self {
    let cells = vec![settled; ground.len()];
    Self {
      width,
      height,
      half,
      ground,
      cells,
    }
  }

  /// World position of a cell's centre.
  pub fn centre(&self, x: usize, y: usize) -> [f32; 2] {
    [
      ((x as f32 + 0.5) / self.width as f32 * 2.0 - 1.0) * self.half[0],
      ((y as f32 + 0.5) / self.height as f32 * 2.0 - 1.0) * self.half[1],
    ]
  }

  /// Advance every cell by `dt` seconds. `weather` gives the precipitation
  /// at a world position; the rest of the inputs are shared.
  pub fn step(
    &mut self,
    dt: f32,
    shared: &SurfaceInputs,
    mut precipitation: impl FnMut([f32; 2]) -> f32,
  ) {
    for y in 0..self.height {
      for x in 0..self.width {
        let index = y * self.width + x;
        let ground = self.ground[index];
        let inputs = SurfaceInputs {
          precipitation: precipitation(self.centre(x, y)),
          celsius: ground.celsius + shared.celsius,
          canopy: ground.canopy,
          slope_degrees: ground.slope_degrees,
          hollow_metres: ground.hollow_metres,
          patch: patch(x as u32, y as u32),
          ..*shared
        };
        self.cells[index] = step(self.cells[index], &inputs, dt);
      }
    }
  }

  /// Settle every cell into `cell`, the ground under `mean`
  /// precipitation, in proportion to the precipitation `precipitation`
  /// gives at the cell.
  pub fn settle(
    &mut self,
    cell: SurfaceCell,
    mean: f32,
    mut precipitation: impl FnMut([f32; 2]) -> f32,
  ) {
    for y in 0..self.height {
      for x in 0..self.width {
        let share = if mean > 0.001 {
          (precipitation(self.centre(x, y)) / mean.max(0.01)).clamp(0.0, 1.0)
        } else {
          0.0
        };
        self.cells[y * self.width + x] = SurfaceCell {
          wetness: cell.wetness * share,
          puddles: cell.puddles * share,
          snow: cell.snow * share,
        };
      }
    }
  }

  /// The cell state at a world position, bilinearly, or `None` off the
  /// terrain.
  pub fn at(&self, x: f32, z: f32) -> Option<(SurfaceCell, CellGround)> {
    if self.cells.is_empty() || x.abs() > self.half[0] || z.abs() > self.half[1] {
      return None;
    }

    let u = ((x / self.half[0] + 1.0) * 0.5 * self.width as f32 - 0.5)
      .clamp(0.0, self.width as f32 - 1.0);
    let v = ((z / self.half[1] + 1.0) * 0.5 * self.height as f32 - 0.5)
      .clamp(0.0, self.height as f32 - 1.0);
    let (x0, y0) = (u.floor() as usize, v.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
    let (fx, fy) = (u - x0 as f32, v - y0 as f32);
    let weights = [
      (x0, y0, (1.0 - fx) * (1.0 - fy)),
      (x1, y0, fx * (1.0 - fy)),
      (x0, y1, (1.0 - fx) * fy),
      (x1, y1, fx * fy),
    ];
    let mut cell = SurfaceCell::default();
    let mut ground = CellGround::default();

    for (cx, cy, weight) in weights {
      let index = cy * self.width + cx;
      let state = self.cells[index];
      let static_ground = self.ground[index];
      cell.wetness += state.wetness * weight;
      cell.puddles += state.puddles * weight;
      cell.snow += state.snow * weight;
      ground.celsius += static_ground.celsius * weight;
      ground.canopy += static_ground.canopy * weight;
      ground.permanent_snow += static_ground.permanent_snow * weight;
    }

    Some((cell, ground))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn open_ground() -> SurfaceInputs {
    SurfaceInputs {
      celsius: 15.0,
      wind: 3.0,
      sun: 0.6,
      patch: 0.5,
      ..Default::default()
    }
  }

  fn run(mut cell: SurfaceCell, inputs: &SurfaceInputs, seconds: u32) -> SurfaceCell {
    for _ in 0..seconds {
      cell = step(cell, inputs, 1.0);
    }

    cell
  }

  #[test]
  fn wetness_rises_under_rain_and_dries_faster_in_sun_and_wind() {
    let rain = SurfaceInputs {
      precipitation: 0.65,
      sun: 0.0,
      ..open_ground()
    };
    let wet = run(SurfaceCell::default(), &rain, 180);
    assert!(wet.wetness > 0.7, "{wet:?}");

    let still_and_dull = SurfaceInputs {
      sun: 0.0,
      wind: 0.0,
      ..open_ground()
    };
    let sunny_and_windy = SurfaceInputs {
      sun: 1.0,
      wind: 12.0,
      ..open_ground()
    };
    let slow = run(wet, &still_and_dull, 600);
    let fast = run(wet, &sunny_and_windy, 600);

    assert!(fast.wetness < slow.wetness - 0.2, "{fast:?} {slow:?}");
    assert!(slow.wetness < wet.wetness);

    // Ground under trees stays damp longer.
    let shaded = run(
      wet,
      &SurfaceInputs {
        canopy: 1.0,
        ..sunny_and_windy
      },
      600,
    );
    assert!(shaded.wetness > fast.wetness);
  }

  #[test]
  fn puddles_form_only_in_flat_hollows_and_dry_last() {
    assert_eq!(puddle_capacity(5.0, 1.0), 0.0);
    assert_eq!(puddle_capacity(1.0, -0.2), 0.0);
    assert!(puddle_capacity(1.0, 0.3) > 0.9);
    assert!(puddle_capacity(2.0, 0.3) > 0.5);

    let rain = SurfaceInputs {
      precipitation: 1.0,
      ..open_ground()
    };
    let soaked = run(SurfaceCell::default(), &rain, 600);
    assert!(soaked.puddles > 0.5, "{soaked:?}");

    // Standing water keeps a hollow wet; open ground is not held wet.
    let hollow = SurfaceInputs {
      slope_degrees: 1.0,
      hollow_metres: 0.4,
      ..open_ground()
    };
    assert!(step(soaked, &hollow, 1.0).wetness >= soaked.puddles * 0.79);

    // Drying: the ground goes before the puddles do.
    let mut cell = soaked;
    let mut ground_dry_at = None;
    let mut puddles_dry_at = None;

    for second in 0..40_000 {
      cell = step(cell, &open_ground(), 1.0);

      if ground_dry_at.is_none() && cell.wetness < 0.05 {
        ground_dry_at = Some(second);
      }

      if puddles_dry_at.is_none() && cell.puddles == 0.0 {
        puddles_dry_at = Some(second);
      }
    }

    assert!(ground_dry_at.unwrap() < puddles_dry_at.unwrap());
  }

  #[test]
  fn snow_builds_below_half_a_degree_and_melts_above_one() {
    let snowing = |celsius| SurfaceInputs {
      precipitation: 0.8,
      celsius,
      sun: 0.3,
      ..open_ground()
    };
    let deep = run(SurfaceCell::default(), &snowing(-4.0), 300);
    assert!(deep.snow > 0.9, "{deep:?}");

    // At 0.8 °C it neither settles nor melts.
    let thaw_point = run(deep, &snowing(0.8), 300);
    assert!((thaw_point.snow - deep.snow).abs() < 1e-6);

    let melting = run(
      deep,
      &SurfaceInputs {
        celsius: 8.0,
        ..open_ground()
      },
      600,
    );
    assert!(melting.snow < deep.snow - 0.5, "{melting:?}");
    assert!(melting.wetness > 0.2, "melt water wets the ground");

    // Warmer and sunnier melts faster.
    let slow = run(
      deep,
      &SurfaceInputs {
        celsius: 3.0,
        sun: 0.0,
        ..open_ground()
      },
      300,
    );
    let fast = run(
      deep,
      &SurfaceInputs {
        celsius: 6.0,
        sun: 1.0,
        ..open_ground()
      },
      300,
    );
    assert!(fast.snow < slow.snow);
  }

  #[test]
  fn large_steps_match_small_ones_while_nothing_saturates() {
    let drying = open_ground();
    let start = SurfaceCell {
      wetness: 0.5,
      puddles: 0.0,
      snow: 0.0,
    };
    let fine = run(start, &drying, 60);
    let coarse = step(start, &drying, 60.0);
    assert!((fine.wetness - coarse.wetness).abs() < 1e-4);
  }
}
