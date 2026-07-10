use vista_types::CameraOptions;
use vista_wasm::camera::{look_at, CameraProjector};

#[test]
fn look_at_helper_sets_position_and_target() {
  let camera = look_at([1.0, 2.0, 3.0], [4.0, 5.0, 6.0], 45.0);

  assert_eq!(camera.position, [1.0, 2.0, 3.0]);
  assert_eq!(camera.target, [4.0, 5.0, 6.0]);
  assert_eq!(camera.field_of_view_degrees, 45.0);
}

#[test]
fn projector_builds_projection_matrix() {
  let projector = CameraProjector::new(CameraOptions::default(), 16.0 / 9.0).unwrap();

  assert!(projector.projection_matrix[0] > 0.0);
  assert!(projector.projection_matrix[5] > 0.0);
}
