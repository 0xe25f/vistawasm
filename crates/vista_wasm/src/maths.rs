use vista_types::Vec3;

/// Clamp a value to an inclusive range while treating NaN as the lower bound.
pub fn clamp_f32(value: f32, min: f32, max: f32) -> f32 {
  if value.is_nan() {
    return min;
  }

  value.max(min).min(max)
}

/// Linearly interpolate between two values.
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
  a + (b - a) * t
}

/// Length of a 2D vector. Cheaper in code size than `f32::hypot`, which
/// pulls in a careful overflow-safe routine these distances never need.
pub fn length2(x: f32, y: f32) -> f32 {
  (x * x + y * y).sqrt()
}

/// Smooth interpolation curve used by value noise.
pub fn smoothstep(t: f32) -> f32 {
  let t = clamp_f32(t, 0.0, 1.0);
  t * t * (3.0 - 2.0 * t)
}

/// Return a deterministic 64-bit hash.
pub fn hash_u64(mut value: u64) -> u64 {
  value ^= value >> 30;
  value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
  value ^= value >> 27;
  value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
  value ^ (value >> 31)
}

/// Return a deterministic signed noise value in the range -1 to 1.
pub fn hash_noise(seed: u64, x: i32, y: i32) -> f32 {
  let mut value = seed;
  value ^= (x as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
  value ^= (y as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f);
  let hashed = hash_u64(value);
  let unit = (hashed as f64 / u64::MAX as f64) as f32;
  unit * 2.0 - 1.0
}

/// Two-dimensional deterministic value noise.
pub fn value_noise(seed: u64, x: f32, y: f32) -> f32 {
  // Coordinates beyond `i32` saturate; wrapping the neighbours keeps such
  // lookups a hash instead of an overflow.
  let xi = x.floor() as i32;
  let yi = y.floor() as i32;
  let tx = smoothstep(x - xi as f32);
  let ty = smoothstep(y - yi as f32);
  let (xn, yn) = (xi.wrapping_add(1), yi.wrapping_add(1));

  let a = hash_noise(seed, xi, yi);
  let b = hash_noise(seed, xn, yi);
  let c = hash_noise(seed, xi, yn);
  let d = hash_noise(seed, xn, yn);
  let ab = lerp(a, b, tx);
  let cd = lerp(c, d, tx);

  lerp(ab, cd, ty)
}

/// Normalise a vector.
pub fn normalise(vec: Vec3) -> Vec3 {
  let length = dot(vec, vec).sqrt();

  if length <= f32::EPSILON {
    return [0.0, 1.0, 0.0];
  }

  [vec[0] / length, vec[1] / length, vec[2] / length]
}

/// Return the vector cross product.
pub fn cross(a: Vec3, b: Vec3) -> Vec3 {
  [
    a[1] * b[2] - a[2] * b[1],
    a[2] * b[0] - a[0] * b[2],
    a[0] * b[1] - a[1] * b[0],
  ]
}

/// Return the vector dot product.
pub fn dot(a: Vec3, b: Vec3) -> f32 {
  a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Subtract vector `b` from vector `a`.
pub fn sub(a: Vec3, b: Vec3) -> Vec3 {
  [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// Add two vectors.
pub fn add(a: Vec3, b: Vec3) -> Vec3 {
  [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// Multiply a vector by a scalar.
pub fn mul(vec: Vec3, scalar: f32) -> Vec3 {
  [vec[0] * scalar, vec[1] * scalar, vec[2] * scalar]
}

/// Multiply two column-major 4x4 matrices, returning `a * b`.
///
/// Matrices use the same column-major layout produced by
/// [`crate::camera::look_at_matrix`] and [`crate::camera::perspective_matrix`],
/// where element `(row, col)` is stored at index `col * 4 + row`.
pub fn mat4_multiply(a: [f32; 16], b: [f32; 16]) -> [f32; 16] {
  let mut result = [0.0_f32; 16];

  for col in 0..4 {
    for row in 0..4 {
      let mut sum = 0.0;

      for k in 0..4 {
        sum += a[k * 4 + row] * b[col * 4 + k];
      }

      result[col * 4 + row] = sum;
    }
  }

  result
}

/// Return a unit vector pointing from the terrain towards the sun.
///
/// `azimuth_degrees` is measured around the horizontal plane and
/// `elevation_degrees` is measured above the horizon.
pub fn sun_direction_vector(azimuth_degrees: f32, elevation_degrees: f32) -> Vec3 {
  let azimuth = azimuth_degrees.to_radians();
  let elevation = elevation_degrees.to_radians();
  let horizontal = elevation.cos();

  normalise([
    azimuth.cos() * horizontal,
    elevation.sin(),
    azimuth.sin() * horizontal,
  ])
}

/// Extract normalised frustum planes (`ax + by + cz + d >= 0` inside) from
/// a column-major view-projection matrix with a 0..1 depth range.
pub fn frustum_planes(m: &[f32; 16]) -> [[f32; 4]; 6] {
  let row = |i: usize| [m[i], m[4 + i], m[8 + i], m[12 + i]];
  let (r0, r1, r2, r3) = (row(0), row(1), row(2), row(3));
  let combine = |a: [f32; 4], b: [f32; 4], sign: f32| {
    [
      a[0] + sign * b[0],
      a[1] + sign * b[1],
      a[2] + sign * b[2],
      a[3] + sign * b[3],
    ]
  };
  let planes = [
    combine(r3, r0, 1.0),
    combine(r3, r0, -1.0),
    combine(r3, r1, 1.0),
    combine(r3, r1, -1.0),
    r2,
    combine(r3, r2, -1.0),
  ];

  planes.map(|plane| {
    let length = (plane[0] * plane[0] + plane[1] * plane[1] + plane[2] * plane[2])
      .sqrt()
      .max(1e-6);
    [
      plane[0] / length,
      plane[1] / length,
      plane[2] / length,
      plane[3] / length,
    ]
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn hash_noise_is_repeatable() {
    assert_eq!(hash_noise(42, 10, 12), hash_noise(42, 10, 12));
  }

  #[test]
  fn normalise_handles_zero() {
    assert_eq!(normalise([0.0, 0.0, 0.0]), [0.0, 1.0, 0.0]);
  }

  #[test]
  fn mat4_multiply_identity_is_a_no_op() {
    let identity = [
      1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    let translation = [
      1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 5.0, 6.0, 7.0, 1.0,
    ];

    assert_eq!(mat4_multiply(identity, translation), translation);
  }

  #[test]
  fn sun_direction_at_zero_elevation_is_horizontal() {
    let direction = sun_direction_vector(0.0, 0.0);

    assert!((direction[1]).abs() < 0.0001);
    assert!((direction[0] - 1.0).abs() < 0.0001);
  }
}
