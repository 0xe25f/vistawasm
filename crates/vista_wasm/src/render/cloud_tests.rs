//! Tests for the low-cloud bases and the mid-level layer in
//! `shaders/atmosphere.wgsl` and `shaders/common.wgsl`, through CPU ports
//! of their functions and of the noise texture they sample (`gen_noise`
//! in `shaders/texture_gen.wgsl`, at level 0).

use super::pack_ice_tests::{fbm, pcg, remap, unit};

const NOISE_SIZE: usize = 512;

/// `worley2` in `texture_gen.wgsl`: the distance to the nearest feature
/// point, in lattice units.
fn worley(p: [f32; 2], period: i32, seed: u32) -> f32 {
  let (ix, iy) = (p[0].floor() as i32, p[1].floor() as i32);
  let (fx, fy) = (p[0] - ix as f32, p[1] - iy as f32);
  let mut nearest = 8.0_f32;

  for y in -1..=1 {
    for x in -1..=1 {
      let wrap = |v: i32| v.rem_euclid(period) as u32;
      let h = pcg(wrap(ix + x).wrapping_add(pcg(wrap(iy + y).wrapping_add(pcg(seed)))));
      let dx = x as f32 + unit(h) - fx;
      let dy = y as f32 + unit(pcg(h)) - fy;
      nearest = nearest.min((dx * dx + dy * dy).sqrt());
    }
  }

  nearest
}

/// The noise texture's four channels at level 0, quantised as the
/// `rgba8unorm` texture stores them.
struct Noise {
  size: usize,
  texels: Vec<[f32; 4]>,
}

fn byte(v: f32) -> f32 {
  (v * 255.0).round() / 255.0
}

impl Noise {
  fn new() -> Self {
    Self {
      size: NOISE_SIZE,
      texels: (0..NOISE_SIZE * NOISE_SIZE)
        .map(|i| {
          let uv = [
            ((i % NOISE_SIZE) as f32 + 0.5) / NOISE_SIZE as f32,
            ((i / NOISE_SIZE) as f32 + 0.5) / NOISE_SIZE as f32,
          ];
          [
            byte(remap(fbm(uv, 4, 6, 301), 0.22, 0.78)),
            byte(remap(fbm(uv, 8, 5, 302), 0.25, 0.75)),
            byte(1.0 - worley([uv[0] * 6.0, uv[1] * 6.0], 6, 303).clamp(0.0, 1.0)),
            byte(remap(fbm(uv, 16, 4, 304), 0.25, 0.75)),
          ]
        })
        .collect(),
    }
  }

  /// Bilinear, repeating sample, as `linear_sampler` reads it.
  fn sample(&self, uv: [f32; 2]) -> [f32; 4] {
    let size = self.size as i32;
    let x = uv[0] * self.size as f32 - 0.5;
    let y = uv[1] * self.size as f32 - 0.5;
    let (x0, y0) = (x.floor(), y.floor());
    let (tx, ty) = (x - x0, y - y0);
    let at = |dx: i32, dy: i32| {
      let ix = (x0 as i32 + dx).rem_euclid(size) as usize;
      let iy = (y0 as i32 + dy).rem_euclid(size) as usize;
      self.texels[iy * self.size + ix]
    };
    let (a, b, c, d) = (at(0, 0), at(1, 0), at(0, 1), at(1, 1));
    std::array::from_fn(|k| {
      let top = a[k] + (b[k] - a[k]) * tx;
      let bottom = c[k] + (d[k] - c[k]) * tx;
      top + (bottom - top) * ty
    })
  }
}

fn smoothstep(low: f32, high: f32, x: f32) -> f32 {
  let t = ((x - low) / (high - low)).clamp(0.0, 1.0);
  t * t * (3.0 - 2.0 * t)
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
  a + (b - a) * t
}

// --- Low-cloud bases -------------------------------------------------------

/// `cloud_base_lift` in `atmosphere.wgsl`.
fn cloud_base_lift(level: f32, low: f32, variation: f32) -> f32 {
  variation + (level - 0.5) * 2.0 * variation + 0.035 * (1.0 - low)
}

/// The base level `cloud_weather_level` reads at a point, with the
/// clouds' wind offset at zero.
fn base_level(noise: &Noise, x: f32, z: f32) -> f32 {
  noise.sample([x / 26_000.0, z / 26_000.0])[3]
}

#[test]
fn cloud_bases_vary_between_clouds_but_stay_mostly_flat() {
  let noise = Noise::new();
  let variation = 0.07;
  // A 20 x 20 km grid at 100 m, at cloud cores (`low` 1, no edge lift).
  let steps = 201;
  let at = |i: usize, j: usize| {
    let (x, z) = (i as f32 * 100.0 - 10_000.0, j as f32 * 100.0 - 10_000.0);
    cloud_base_lift(base_level(&noise, x, z), 1.0, variation)
  };
  let mut lowest = f32::MAX;
  let mut highest = f32::MIN;
  let mut steepest = 0.0_f32;
  let mut slope = 0.0;
  let mut apart = 0.0;
  let mut pairs = 0;

  for j in 0..steps {
    for i in 0..steps {
      // The offset about the mean, `(n - 0.5) x 2 x variation`.
      let offset = at(i, j) - variation;
      lowest = lowest.min(offset);
      highest = highest.max(offset);

      if i + 1 < steps {
        let step = (at(i + 1, j) - at(i, j)).abs();
        steepest = steepest.max(step);
        slope += step;
      }

      // Clouds 2 km apart.
      if i + 20 < steps {
        apart += (at(i + 20, j) - at(i, j)).abs();
        pairs += 1;
      }
    }
  }

  let apart = apart / pairs as f32;
  let slope = slope / (steps * (steps - 1)) as f32;
  assert!(
    highest >= 0.6 * variation && lowest <= -0.6 * variation,
    "offsets span {lowest} to {highest}, not 60 % of ±{variation}"
  );
  // Never below the slab floor, never above twice the variation.
  assert!(lowest + variation >= 0.0 && highest + variation <= 2.0 * variation + 1e-6);
  assert!(
    apart >= 0.02,
    "clouds 2 km apart differ by {apart} of the thickness on average"
  );
  // Mostly flat, with no spikes or columns: at 1 600 m thick, the base
  // rises or falls under 12 m in 100 m on average (about 10 m, a slope
  // of 6 degrees), and never 60 m (the worst is about 48 m).
  assert!(slope * 1_600.0 < 12.0, "the base slopes {slope} per 100 m");
  assert!(
    steepest * 1_600.0 < 60.0,
    "the base jumps {steepest} in 100 m"
  );
}

#[test]
fn cloud_bases_curl_up_gently_at_their_rims() {
  // Inside clouds the coarse field runs from about 0.5 at the rim to 1 in
  // the core.
  let mut previous = f32::MAX;

  for step in 0..=10 {
    let low = 0.5 + step as f32 * 0.05;
    let lift = cloud_base_lift(0.5, low, 0.07);
    assert!(lift <= previous, "the rim is lower than the core");
    previous = lift;
  }

  let rim = cloud_base_lift(0.5, 0.5, 0.07) - cloud_base_lift(0.5, 1.0, 0.07);
  assert!((rim - 0.0175).abs() < 1e-6, "rims lift {rim}");
  // No variation: every base on one level, as before, bar the rims.
  assert_eq!(cloud_base_lift(0.1, 1.0, 0.0), 0.0);
  assert_eq!(cloud_base_lift(0.9, 1.0, 0.0), 0.0);
}

// --- The mid-level layer ---------------------------------------------------

const ALTOCUMULUS_THICKNESS: f32 = 250.0;
const ALTOSTRATUS_THICKNESS: f32 = 600.0;
const ALTO_EXTINCTION: f32 = 0.00495;
const ALTO_RIPPLE: f32 = 900.0;
const ALTO_CELL_TILE: f32 = 1_080.0;

/// The layer's wind (`alto_wind`): a cloud wind towards +z, veered.
const WIND: [f32; 2] = [0.342_020_1, 0.939_692_6];

/// `alto_ripple` in `common.wgsl`.
fn alto_ripple(q: [f32; 2], warp: f32) -> f32 {
  let across = [-WIND[1], WIND[0]];
  0.5
    + 0.5
      * ((q[0] * across[0] + q[1] * across[1]) * (std::f32::consts::TAU / ALTO_RIPPLE) + warp).sin()
}

/// `alto_coarse` in `common.wgsl`, without a regional map.
fn alto_coarse(noise: &Noise, q: [f32; 2], amounts: [f32; 2]) -> [f32; 3] {
  let across = [-WIND[1], WIND[0]];
  let local = [
    (q[0] * WIND[0] + q[1] * WIND[1]) / 2.5,
    q[0] * across[0] + q[1] * across[1],
  ];
  let n = noise.sample([local[0] / 48_000.0 + 0.29, local[1] / 48_000.0 + 0.53]);
  [
    (amounts[0] * mix(0.3, 1.7, n[0])).clamp(0.0, 1.0),
    amounts[1] * (0.7 + 0.3 * n[1]),
    (n[3] - 0.5) * 7.5,
  ]
}

/// The optical depth `alto_along` finds looking straight up, with the
/// pixel footprint well inside one texel.
fn alto_depth_overhead(noise: &Noise, q: [f32; 2], amounts: [f32; 2]) -> f32 {
  let coarse = alto_coarse(noise, q, amounts);
  let cumulus = cloudlets(noise, q, amounts[0], Some(ALTO_SMALLEST));
  ALTO_EXTINCTION * (cumulus * ALTOCUMULUS_THICKNESS + coarse[1] * ALTOSTRATUS_THICKNESS)
}

fn transmittance_overhead(noise: &Noise, q: [f32; 2], amounts: [f32; 2]) -> f32 {
  (-alto_depth_overhead(noise, q, amounts)).exp()
}

/// Points on a 20 x 20 km grid at 250 m.
fn grid() -> impl Iterator<Item = [f32; 2]> {
  (0..81)
    .flat_map(|j| (0..81).map(move |i| [i as f32 * 250.0 - 10_000.0, j as f32 * 250.0 - 10_000.0]))
}

#[test]
fn full_altostratus_lets_through_about_eight_percent_overhead() {
  let noise = Noise::new();
  let mut total = 0.0;
  let mut count = 0;

  for q in grid() {
    let through = transmittance_overhead(&noise, q, [0.0, 1.0]);
    // Its fibres vary it a little, never to a hole or a black patch.
    assert!((0.045..=0.13).contains(&through), "{through} at {q:?}");
    total += through;
    count += 1;
  }

  let mean = total / count as f32;
  assert!((mean - 0.08).abs() <= 0.02, "a mean of {mean}");
}

#[test]
fn no_alto_cloud_lets_every_ray_through() {
  let noise = Noise::new();

  for q in grid() {
    assert_eq!(transmittance_overhead(&noise, q, [0.0, 0.0]), 1.0);
  }
}

#[test]
fn alto_transmittance_falls_as_the_amounts_rise() {
  let noise = Noise::new();

  for q in grid().step_by(7) {
    for mix_of in [[1.0, 0.0], [0.0, 1.0], [1.0, 1.0], [0.7, 0.3]] {
      let mut previous = 1.0;

      for step in 0..=20 {
        let amount = step as f32 / 20.0;
        let through = transmittance_overhead(&noise, q, [mix_of[0] * amount, mix_of[1] * amount]);
        assert!(
          through <= previous + 1e-6,
          "{through} after {previous} at {amount} of {mix_of:?}, {q:?}"
        );
        previous = through;
      }
    }
  }
}

#[test]
fn a_mackerel_sky_has_rows_of_cloudlets_and_clear_lanes() {
  let noise = Noise::new();
  let mut cloudy = 0;
  let mut clear = 0;
  let mut count = 0;

  for q in grid() {
    let through = transmittance_overhead(&noise, q, [0.7, 0.0]);
    cloudy += usize::from(through < 0.85);
    clear += usize::from(through > 0.97);
    count += 1;
  }

  // Patches come and go, so neither cloud nor sky fills the sky.
  let (cloudy, clear) = (cloudy as f32 / count as f32, clear as f32 / count as f32);
  assert!(
    cloudy > 0.15 && clear > 0.2,
    "cloudy {cloudy}, clear {clear}"
  );
}

/// The smallest cloudlet the mackerel pattern draws, as a radius in
/// Worley cells (`ALTO_SMALLEST` in `common.wgsl`): 0.3 of a 180 m cell,
/// about 110 m across.
const ALTO_SMALLEST: f32 = 0.3;

/// The altocumulus density `alto_along` finds at a point, with the pixel
/// footprint well inside one texel; `smallest` is the smallest cloudlet
/// drawn, or `None` for the first shape, which had none.
fn cloudlets(noise: &Noise, q: [f32; 2], amount: f32, smallest: Option<f32>) -> f32 {
  let coarse = alto_coarse(noise, q, [amount, 0.0]);

  if coarse[0] <= 0.001 {
    return 0.0;
  }

  let warp = noise.sample([q[0] / 7_000.0 + 0.71, q[1] / 7_000.0 + 0.07]);
  let warp = [warp[0] - 0.5, warp[1] - 0.5];
  let cells = noise.sample([
    (q[0] + warp[0] * 500.0) / ALTO_CELL_TILE,
    (q[1] + warp[1] * 500.0) / ALTO_CELL_TILE,
  ])[2];
  let ripple = alto_ripple(q, coarse[2] + warp[0] * 5.0);
  let threshold = 0.95 - 0.6 * coarse[0];
  let rise = mix(0.6, 1.0, ripple);
  // A cloudlet's edge, in the cells' units: where they rise above the
  // threshold. The first shape drew every cloudlet that crossed it, however
  // small; the smallest are drawn at the smallest size instead, and
  // fade out below it, so none is a speck.
  let mut edge = threshold / rise;
  let mut presence = 1.0;

  if let Some(smallest) = smallest {
    presence = smoothstep(0.5 * smallest, smallest, 1.0 - edge);
    edge = edge.min(1.0 - smallest);
  }

  presence
    * smoothstep(edge, edge + 0.35 / rise, cells)
    * (0.55 + 0.45 * smoothstep(edge, 1.0 / rise, cells))
}

/// Connected patches of `density` above `floor` on a square grid of
/// `side` points, leaving out those that touch its edge: each patch's
/// peak density and its footprint, the points above a tenth of it.
fn patches(density: &[f32], side: usize, floor: f32) -> Vec<(f32, usize)> {
  let mut seen = vec![false; density.len()];
  let mut found = Vec::new();

  for start in 0..density.len() {
    if seen[start] || density[start] <= floor {
      continue;
    }

    let (mut points, mut edge, mut stack) = (Vec::new(), false, vec![start]);
    seen[start] = true;

    while let Some(at) = stack.pop() {
      points.push(at);
      let (x, y) = (at % side, at / side);
      edge |= x == 0 || y == 0 || x + 1 == side || y + 1 == side;
      let neighbours = [
        (x > 0).then(|| at - 1),
        (x + 1 < side).then_some(at + 1),
        (y > 0).then(|| at - side),
        (y + 1 < side).then_some(at + side),
      ];

      for next in neighbours.into_iter().flatten() {
        if !seen[next] && density[next] > floor {
          seen[next] = true;
          stack.push(next);
        }
      }
    }

    if !edge {
      let peak = points.iter().map(|at| density[*at]).fold(0.0, f32::max);
      let core = points
        .iter()
        .filter(|at| density[**at] >= 0.1 * peak)
        .count();
      found.push((peak, core));
    }
  }

  found
}

#[test]
fn sparse_altocumulus_leaves_no_specks() {
  let noise = Noise::new();
  // A 20 x 20 km sky every 10 m.
  let (side, step) = (2_000, 10.0);
  let density = |amount: f32, smallest: Option<f32>| -> Vec<f32> {
    (0..side * side)
      .map(|i| {
        let q = [
          (i % side) as f32 * step - 10_000.0,
          (i / side) as f32 * step - 10_000.0,
        ];
        cloudlets(&noise, q, amount, smallest)
      })
      .collect()
  };
  // The smallest cloudlet the pattern means to draw covers 1,000 m² (ten
  // points) above a tenth of its peak, about 36 m across: a smallest
  // cloudlet, squeezed where the warp packs the cells tighter. A patch
  // shows where it blocks some 2 % of the light (a density of 0.05).
  let smallest_points = 10;
  let specks = |field: &[f32]| {
    patches(field, side, 0.005)
      .iter()
      .filter(|(peak, points)| *peak > 0.05 && *points < smallest_points)
      .count()
  };
  let mut before = 0;

  for twentieths in 1..=6 {
    let amount = twentieths as f32 * 0.05;
    before += specks(&density(amount, None));
    let after = specks(&density(amount, Some(ALTO_SMALLEST)));
    assert_eq!(after, 0, "{after} specks at {amount}");
  }

  // The first shape left hundreds.
  assert!(before > 1_000, "{before} specks before");

  // A full mackerel sky keeps its cover, within 5 % of the first shape's.
  let cover =
    |field: &[f32]| field.iter().filter(|d| **d > 0.1).count() as f32 / field.len() as f32;
  let (old, new) = (
    cover(&density(0.7, None)),
    cover(&density(0.7, Some(ALTO_SMALLEST))),
  );
  assert!(
    (new / old - 1.0).abs() <= 0.05,
    "cover {old} before, {new} after"
  );
}

/// The veil's transmittance the composite takes along a ray at
/// `elevation` (its sine) towards the layer point `q` (the sun-disc
/// branch in `atmosphere.wgsl`: the altostratus of `alto_coarse` there),
/// for altostratus `amount`.
fn veil_along(noise: &Noise, q: [f32; 2], amount: f32, elevation: f32) -> f32 {
  let depth = alto_depth_overhead(noise, q, [0.0, amount]) / elevation.max(0.05);
  1.0 - (1.0 - (-depth).exp()) * smoothstep(0.01, 0.12, elevation)
}

/// The first veil for the disc: the mean depth of altostratus at `amount`
/// (a density of 0.85 of it) towards a sun at `elevation`, the same over
/// the whole disc.
fn mean_veil(amount: f32, elevation: f32) -> f32 {
  let depth = ALTO_EXTINCTION * amount * 0.85 * ALTOSTRATUS_THICKNESS / elevation.max(0.01);
  1.0 - (1.0 - (-depth).exp()) * smoothstep(0.01, 0.12, elevation)
}

#[test]
fn the_watery_sun_shows_the_veils_fibres() {
  let noise = Noise::new();
  let (amount, elevation) = (0.7, 35f32.to_radians().sin());
  // The disc and its corona span 6 degrees; the veil 4,200 m up lies
  // about 7.3 km away towards a sun at 35 degrees. Where the drifting
  // veil's depth differs from its mean by at least 10 %, so does the
  // disc's brightness through it, which the mean veil never showed.
  let mut checked = 0;

  for drift in 0..400 {
    let centre = [drift as f32 * 211.0 - 40_000.0, drift as f32 * 97.0];

    for across in [-380.0f32, 0.0, 380.0] {
      let q = [centre[0] + across, centre[1]];
      let depth = alto_depth_overhead(&noise, q, [0.0, amount]);
      let mean_depth = ALTO_EXTINCTION * amount * 0.85 * ALTOSTRATUS_THICKNESS;

      if (depth / mean_depth - 1.0).abs() >= 0.1 {
        let (pixel, mean) = (
          veil_along(&noise, q, amount, elevation),
          mean_veil(amount, elevation),
        );
        assert!(
          (pixel / mean - 1.0).abs() >= 0.1,
          "{pixel} against {mean} at {q:?}"
        );
        checked += 1;
      }
    }
  }

  assert!(checked > 100, "only {checked} rays through fibres");

  // Under an even veil (its fibres at their mean everywhere) the disc is
  // as bright as the mean veil made it.
  let mut even = Noise::new();

  for texel in &mut even.texels {
    texel[1] = 0.5;
  }

  for amount in [0.2, 0.5, 0.7, 1.0] {
    for degrees in [10.0f32, 35.0, 70.0] {
      let elevation = degrees.to_radians().sin();

      for q in grid().step_by(97) {
        let (pixel, mean) = (
          veil_along(&even, q, amount, elevation),
          mean_veil(amount, elevation),
        );
        assert!((pixel / mean - 1.0).abs() <= 0.02, "{pixel} against {mean}");
      }
    }
  }
}
