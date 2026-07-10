use vista_types::{CameraOptions, Vec3};

use crate::config::validate_camera;
use crate::errors::VistaResult;
use crate::maths::{cross, dot, normalise, sub};

/// Camera state for the VistaWASM projector model.
#[derive(Clone, Debug, PartialEq)]
pub struct CameraProjector {
  /// Public camera options after validation.
  pub options: CameraOptions,
  /// View matrix in column-major order.
  pub view_matrix: [f32; 16],
  /// Projection matrix in column-major order.
  pub projection_matrix: [f32; 16],
}

impl CameraProjector {
  /// Create a validated camera projector.
  pub fn new(options: CameraOptions, aspect: f32) -> VistaResult<Self> {
    validate_camera(&options)?;
    let view_matrix = look_at_matrix(options.position, options.target, [0.0, 1.0, 0.0]);
    let projection_matrix = perspective_matrix(
      options.field_of_view_degrees,
      aspect.max(0.001),
      options.near_metres.unwrap_or(0.5),
      options.far_metres.unwrap_or(120_000.0),
    );

    Ok(Self {
      options,
      view_matrix,
      projection_matrix,
    })
  }

  /// Replace the camera options and recompute matrices.
  pub fn set_camera(&mut self, options: CameraOptions, aspect: f32) -> VistaResult<()> {
    *self = Self::new(options, aspect)?;
    Ok(())
  }
}

/// Build a right-handed look-at matrix.
pub fn look_at_matrix(eye: Vec3, target: Vec3, up: Vec3) -> [f32; 16] {
  let forward = normalise(sub(target, eye));
  let side = normalise(cross(forward, up));
  let up = cross(side, forward);

  [
    side[0],
    up[0],
    -forward[0],
    0.0,
    side[1],
    up[1],
    -forward[1],
    0.0,
    side[2],
    up[2],
    -forward[2],
    0.0,
    -dot(side, eye),
    -dot(up, eye),
    dot(forward, eye),
    1.0,
  ]
}

/// Build a perspective projection matrix.
pub fn perspective_matrix(
  field_of_view_degrees: f32,
  aspect: f32,
  near_metres: f32,
  far_metres: f32,
) -> [f32; 16] {
  let fov_radians = field_of_view_degrees.to_radians();
  let f = 1.0 / (fov_radians * 0.5).tan();
  let range_inv = 1.0 / (near_metres - far_metres);

  [
    f / aspect,
    0.0,
    0.0,
    0.0,
    0.0,
    f,
    0.0,
    0.0,
    0.0,
    0.0,
    far_metres * range_inv,
    -1.0,
    0.0,
    0.0,
    near_metres * far_metres * range_inv,
    0.0,
  ]
}

/// Return a camera that looks from `position` to `target`.
pub fn look_at(position: Vec3, target: Vec3, field_of_view_degrees: f32) -> CameraOptions {
  CameraOptions {
    position,
    target,
    field_of_view_degrees,
    ..CameraOptions::default()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn look_at_moves_eye_to_origin() {
    let matrix = look_at_matrix([0.0, 0.0, 10.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);

    assert!((matrix[14] + 10.0).abs() < 0.001);
  }

  #[test]
  fn camera_rejects_bad_field_of_view() {
    let result = CameraProjector::new(
      CameraOptions {
        field_of_view_degrees: 0.0,
        ..CameraOptions::default()
      },
      1.0,
    );

    assert!(result.is_err());
  }
}
