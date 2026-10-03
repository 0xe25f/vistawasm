//! Light-space maths for the tree shadow map.

use crate::camera::look_at_matrix;
use crate::maths::{add, mat4_multiply, mul, normalise};
use vista_types::Vec3;

/// A light-space transform and the ground area it covers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowFrame {
  /// World to shadow-map clip space (column-major, WebGPU 0..1 depth).
  pub view_proj: [f32; 16],
  /// Centre of the shadowed area (x, z).
  pub centre: [f32; 2],
  /// Radius of the shadowed area in metres.
  pub radius: f32,
}

fn orthographic(half_extent: f32, near: f32, far: f32) -> [f32; 16] {
  let depth = (far - near).max(0.001);

  [
    1.0 / half_extent,
    0.0,
    0.0,
    0.0,
    0.0,
    1.0 / half_extent,
    0.0,
    0.0,
    0.0,
    0.0,
    -1.0 / depth,
    0.0,
    0.0,
    0.0,
    -near / depth,
    1.0,
  ]
}

/// Fit an orthographic sun-space frame around the camera.
///
/// The area is pushed ahead of the camera (where shadows are seen) and the
/// matrix is snapped to whole shadow-map texels, so shadows do not shimmer
/// as the camera moves.
pub fn tree_shadow_frame(
  camera_position: Vec3,
  camera_forward: Vec3,
  sun_direction: Vec3,
  radius: f32,
  resolution: u32,
  height_range: (f32, f32),
) -> ShadowFrame {
  let radius = radius.max(1.0);
  let flat = normalise([camera_forward[0], 0.0, camera_forward[2]]);
  let ahead = if camera_forward[0].abs() + camera_forward[2].abs() < 1e-4 {
    [0.0, 0.0, 0.0]
  } else {
    mul(flat, radius * 0.35)
  };
  let mid_height = (height_range.0 + height_range.1) * 0.5;
  let centre = add([camera_position[0], mid_height, camera_position[2]], ahead);
  let sun = normalise(sun_direction);
  let span = (height_range.1 - height_range.0).abs() + 200.0;
  let distance = radius * 2.0 + span;
  let eye = add(centre, mul(sun, distance));
  let up = if sun[1].abs() > 0.99 {
    [0.0, 0.0, 1.0]
  } else {
    [0.0, 1.0, 0.0]
  };
  let view = look_at_matrix(eye, centre, up);
  let projection = orthographic(radius, 0.0, distance * 2.0);
  let mut view_proj = mat4_multiply(projection, view);

  // Snap the world origin to a whole texel in light space.
  let texel = 2.0 / resolution.max(1) as f32;
  let origin_x = view_proj[12];
  let origin_y = view_proj[13];
  view_proj[12] += (origin_x / texel).round() * texel - origin_x;
  view_proj[13] += (origin_y / texel).round() * texel - origin_y;

  ShadowFrame {
    view_proj,
    centre: [centre[0], centre[2]],
    radius,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn project(m: &[f32; 16], p: Vec3) -> [f32; 3] {
    let x = m[0] * p[0] + m[4] * p[1] + m[8] * p[2] + m[12];
    let y = m[1] * p[0] + m[5] * p[1] + m[9] * p[2] + m[13];
    let z = m[2] * p[0] + m[6] * p[1] + m[10] * p[2] + m[14];
    let w = m[3] * p[0] + m[7] * p[1] + m[11] * p[2] + m[15];
    [x / w, y / w, z / w]
  }

  #[test]
  fn the_area_around_the_camera_is_inside_the_map() {
    let frame = tree_shadow_frame(
      [100.0, 50.0, -40.0],
      [0.0, 0.0, -1.0],
      normalise([0.4, 0.8, 0.2]),
      200.0,
      2048,
      (0.0, 300.0),
    );
    let p = project(&frame.view_proj, [100.0, 60.0, -80.0]);

    assert!(p[0].abs() < 1.0 && p[1].abs() < 1.0);
    assert!(p[2] > 0.0 && p[2] < 1.0);
  }

  #[test]
  fn points_nearer_the_sun_have_smaller_depth() {
    let sun = normalise([0.3, 0.9, 0.1]);
    let frame = tree_shadow_frame(
      [0.0, 0.0, 0.0],
      [1.0, 0.0, 0.0],
      sun,
      150.0,
      1024,
      (0.0, 100.0),
    );
    let low = project(&frame.view_proj, [0.0, 10.0, 0.0]);
    let high = project(&frame.view_proj, [0.0, 60.0, 0.0]);

    assert!(high[2] < low[2]);
  }

  #[test]
  fn frames_are_texel_snapped() {
    let make = |x: f32| {
      tree_shadow_frame(
        [x, 0.0, 0.0],
        [0.0, 0.0, 1.0],
        normalise([0.5, 0.7, 0.0]),
        128.0,
        512,
        (0.0, 50.0),
      )
    };
    let texel = 2.0 / 512.0;
    let frame = make(10.013);

    assert!(((frame.view_proj[12] / texel).round() * texel - frame.view_proj[12]).abs() < 1e-4);
    assert!(
      ((make(10.0).view_proj[13] / texel).round() * texel - make(10.0).view_proj[13]).abs() < 1e-4
    );
  }
}
