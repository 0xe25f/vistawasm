//! Seeded 2D gradient noise for terrain generation.
//!
//! Simplex-lattice gradient noise (in the OpenSimplex2 family): each
//! sample sums the contributions of the three corners of its lattice
//! triangle, each a hashed unit gradient dotted with the offset and faded
//! by a radial kernel. Unlike value noise on a square lattice it has no
//! grid-aligned ridges or plateaus, and unlike square-lattice gradient
//! noise it is not pinned to zero on integer coordinates.
//!
//! [`simplex_d`] also returns the analytic gradient, which the detail
//! layer uses to damp new octaves on steep ground.

/// Skew factor from input space to the simplex lattice, `(sqrt(3) - 1) / 2`.
const SKEW: f32 = 0.366_025_42;
/// Unskew factor back to input space, `(3 - sqrt(3)) / 6`.
const UNSKEW: f32 = 0.211_324_87;
/// Radial kernel extent, squared. 0.5 keeps each corner's kernel inside
/// the triangles that share it, so the noise is continuous.
const RADIUS_SQUARED: f32 = 0.5;
/// Scales the kernel sum to fill [-1, 1]. The largest magnitude three unit
/// gradients can produce is about 0.0189, found by the search in the tests.
const NORMALISER: f32 = 52.5;

/// 24 unit gradient directions, 15 degrees apart. Many evenly spread
/// directions keep the noise free of axis bias.
const GRADIENTS: [[f32; 2]; 24] = [
  [1.0, 0.0],
  [0.965_925_8, 0.258_819_04],
  [0.866_025_4, 0.5],
  [0.707_106_77, 0.707_106_77],
  [0.5, 0.866_025_4],
  [0.258_819_04, 0.965_925_8],
  [0.0, 1.0],
  [-0.258_819_04, 0.965_925_8],
  [-0.5, 0.866_025_4],
  [-0.707_106_77, 0.707_106_77],
  [-0.866_025_4, 0.5],
  [-0.965_925_8, 0.258_819_04],
  [-1.0, 0.0],
  [-0.965_925_8, -0.258_819_04],
  [-0.866_025_4, -0.5],
  [-0.707_106_77, -0.707_106_77],
  [-0.5, -0.866_025_4],
  [-0.258_819_04, -0.965_925_8],
  [0.0, -1.0],
  [0.258_819_04, -0.965_925_8],
  [0.5, -0.866_025_4],
  [0.707_106_77, -0.707_106_77],
  [0.866_025_4, -0.5],
  [0.965_925_8, -0.258_819_04],
];

/// Fold a 64-bit seed into the 32-bit seed the noise hashes use.
pub fn noise_seed(seed: u64, stream: u64) -> u32 {
  let mixed = crate::maths::hash_u64(seed ^ stream.wrapping_mul(0x9e37_79b9_7f4a_7c15));
  (mixed ^ (mixed >> 32)) as u32
}

fn gradient(seed: u32, i: i32, j: i32) -> [f32; 2] {
  let mut h = seed ^ (i as u32).wrapping_mul(0x27d4_eb2d) ^ (j as u32).wrapping_mul(0x1656_67b1);
  h ^= h >> 15;
  h = h.wrapping_mul(0x2c1b_3c6d);
  h ^= h >> 12;
  h = h.wrapping_mul(0x297a_2d39);
  h ^= h >> 15;
  GRADIENTS[(h % 24) as usize]
}

/// The lattice corners of the triangle holding `(x, y)`, each as integer
/// lattice coordinates and the offset from the corner in input space.
fn corners(x: f32, y: f32) -> [(i32, i32, f32, f32); 3] {
  let skew = (x + y) * SKEW;
  let i = (x + skew).floor();
  let j = (y + skew).floor();
  let unskew = (i + j) * UNSKEW;
  let x0 = x - (i - unskew);
  let y0 = y - (j - unskew);
  // The lower-left triangle when x0 > y0, the upper-right one otherwise.
  let (i1, j1) = if x0 > y0 { (1, 0) } else { (0, 1) };
  let corners = [
    (0, 0, x0, y0),
    (i1, j1, x0 - i1 as f32 + UNSKEW, y0 - j1 as f32 + UNSKEW),
    (1, 1, x0 - 1.0 + 2.0 * UNSKEW, y0 - 1.0 + 2.0 * UNSKEW),
  ];
  let (ii, jj) = (i as i32, j as i32);
  corners.map(|(ci, cj, ox, oy)| (ii + ci, jj + cj, ox, oy))
}

/// Gradient noise in [-1, 1] at `(x, y)`, with its gradient `(d/dx, d/dy)`.
pub fn simplex_d(seed: u32, x: f32, y: f32) -> (f32, f32, f32) {
  let mut value = 0.0;
  let mut dx = 0.0;
  let mut dy = 0.0;

  for (ci, cj, ox, oy) in corners(x, y) {
    let t = RADIUS_SQUARED - ox * ox - oy * oy;

    if t <= 0.0 {
      continue;
    }

    let g = gradient(seed, ci, cj);
    let dot = g[0] * ox + g[1] * oy;
    let t2 = t * t;
    let t4 = t2 * t2;
    value += t4 * dot;
    // d/dx of t^4 * dot = -8 t^3 x dot + t^4 g.x
    let fade = -8.0 * t2 * t * dot;
    dx += fade * ox + t4 * g[0];
    dy += fade * oy + t4 * g[1];
  }

  (
    (value * NORMALISER).clamp(-1.0, 1.0),
    dx * NORMALISER,
    dy * NORMALISER,
  )
}

/// Gradient noise in [-1, 1] at `(x, y)`.
pub fn simplex(seed: u32, x: f32, y: f32) -> f32 {
  simplex_d(seed, x, y).0
}

/// Fractal sum of `octaves` layers of [`simplex`], normalised to [-1, 1].
/// Each octave uses its own seed so octaves never line up.
pub fn fbm(seed: u32, x: f32, y: f32, octaves: u32, gain: f32, lacunarity: f32) -> f32 {
  let mut amplitude = 1.0;
  let mut frequency = 1.0;
  let mut sum = 0.0;
  let mut normaliser = 0.0;

  for octave in 0..octaves {
    let octave_seed = seed.wrapping_add(octave.wrapping_mul(0x9e37_79b9));
    sum += simplex(octave_seed, x * frequency, y * frequency) * amplitude;
    normaliser += amplitude;
    amplitude *= gain;
    frequency *= lacunarity;
  }

  if normaliser <= f32::EPSILON {
    0.0
  } else {
    sum / normaliser
  }
}

/// Ridged fractal noise in [0, 1]: each octave is `1 - |n|`, so octaves
/// form sharp crests along the zero lines of the underlying noise. Later
/// octaves are weighted by the earlier ones, so detail gathers on crests.
pub fn ridged(seed: u32, x: f32, y: f32, octaves: u32, gain: f32, lacunarity: f32) -> f32 {
  let mut amplitude = 1.0;
  let mut frequency = 1.0;
  let mut sum = 0.0;
  let mut normaliser = 0.0;
  let mut weight = 1.0;

  for octave in 0..octaves {
    let octave_seed = seed.wrapping_add(octave.wrapping_mul(0x9e37_79b9));
    let ridge = 1.0 - simplex(octave_seed, x * frequency, y * frequency).abs();
    let ridge = ridge * ridge * weight;
    weight = ridge.clamp(0.0, 1.0);
    sum += ridge * amplitude;
    normaliser += amplitude;
    amplitude *= gain;
    frequency *= lacunarity;
  }

  if normaliser <= f32::EPSILON {
    0.0
  } else {
    (sum / normaliser).clamp(0.0, 1.0)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::maths::hash_u64;

  fn random_point(index: u64) -> (f32, f32) {
    let a = hash_u64(index * 2 + 1);
    let b = hash_u64(index * 2 + 2);
    (
      (a % 1_000_000) as f32 / 1000.0 - 500.0,
      (b % 1_000_000) as f32 / 1000.0 - 500.0,
    )
  }

  #[test]
  fn output_stays_in_range_and_fills_it() {
    let mut peak: f32 = 0.0;

    for seed in 0..4 {
      for index in 0..50_000 {
        let (x, y) = random_point(index + seed as u64 * 100_000);
        let value = simplex(seed, x, y);
        assert!((-1.0..=1.0).contains(&value), "{value} at {x}, {y}");
        peak = peak.max(value.abs());
      }
    }

    // The normaliser is a strict bound, which random gradients rarely
    // approach, but it should not waste most of the range.
    assert!(peak > 0.45, "peak {peak}");
  }

  #[test]
  fn raw_kernel_sum_never_exceeds_the_normaliser() {
    // Brute-force the worst case: every corner gradient pointing along
    // its offset, searched over one lattice cell.
    let mut worst: f32 = 0.0;

    for sx in 0..300 {
      for sy in 0..300 {
        let mut sum = 0.0;

        for (_, _, ox, oy) in corners(sx as f32 / 150.0, sy as f32 / 150.0) {
          let t = RADIUS_SQUARED - ox * ox - oy * oy;

          if t > 0.0 {
            sum += t.powi(4) * (ox * ox + oy * oy).sqrt();
          }
        }

        worst = worst.max(sum);
      }
    }

    assert!(worst * NORMALISER <= 1.0, "worst {worst}");
  }

  #[test]
  fn noise_is_deterministic_per_seed() {
    for index in 0..1000 {
      let (x, y) = random_point(index);
      assert_eq!(simplex(7, x, y).to_bits(), simplex(7, x, y).to_bits());
    }

    let differs = (0..1000).any(|index| {
      let (x, y) = random_point(index);
      simplex(7, x, y) != simplex(8, x, y)
    });
    assert!(differs);
  }

  #[test]
  fn noise_has_no_lattice_artefacts() {
    // Square-lattice noise is pinned (to zero for gradient noise, to a
    // random corner value for value noise) on integer points, so |n|
    // there has a different mean from |n| elsewhere. Value noise fails
    // this check; see `value_noise_fails_the_lattice_check`.
    let mut lattice = 0.0;
    let mut lattice_count = 0.0;

    for y in -150..150 {
      for x in -150..150 {
        lattice += simplex(3, x as f32, y as f32).abs() as f64;
        lattice_count += 1.0;
      }
    }

    let mut random = 0.0;

    for index in 0..90_000 {
      let (x, y) = random_point(index);
      random += simplex(3, x, y).abs() as f64;
    }

    let difference = (lattice / lattice_count - random / 90_000.0).abs();
    assert!(difference < 0.05, "difference {difference}");
  }

  #[test]
  fn value_noise_fails_the_lattice_check() {
    use crate::maths::value_noise;
    let mut lattice = 0.0;
    let mut random = 0.0;

    for y in -100..100 {
      for x in -100..100 {
        lattice += value_noise(3, x as f32, y as f32).abs() as f64;
      }
    }

    for index in 0..40_000 {
      let (x, y) = random_point(index);
      random += value_noise(3, x, y).abs() as f64;
    }

    assert!((lattice - random).abs() / 40_000.0 >= 0.05);
  }

  #[test]
  fn analytic_gradient_matches_finite_differences() {
    let step = 1e-3;

    for index in 0..500 {
      let (x, y) = random_point(index);
      let (value, dx, dy) = simplex_d(5, x, y);

      if value.abs() > 0.99 {
        continue;
      }

      let fx = (simplex(5, x + step, y) - simplex(5, x - step, y)) / (2.0 * step);
      let fy = (simplex(5, x, y + step) - simplex(5, x, y - step)) / (2.0 * step);
      assert!((fx - dx).abs() < 0.05 * (1.0 + dx.abs()), "{fx} vs {dx}");
      assert!((fy - dy).abs() < 0.05 * (1.0 + dy.abs()), "{fy} vs {dy}");
    }
  }

  #[test]
  fn fractal_sums_stay_in_range() {
    for index in 0..5000 {
      let (x, y) = random_point(index);
      let f = fbm(9, x, y, 5, 0.5, 2.0);
      let r = ridged(9, x, y, 3, 0.5, 2.0);
      assert!((-1.0..=1.0).contains(&f));
      assert!((0.0..=1.0).contains(&r));
    }
  }
}
