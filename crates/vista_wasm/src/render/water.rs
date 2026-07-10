use vista_types::WaterOptions;

/// CPU mirror of water uniforms used by shaders.
#[derive(Clone, Debug, PartialEq)]
pub struct WaterUniforms {
  /// Sea level in metres.
  pub sea_level_metres: f32,
  /// Wave scale.
  pub wave_scale: f32,
  /// Reflection strength.
  pub reflectivity: f32,
  /// Shoreline blend distance.
  pub shoreline_softness_metres: f32,
}

impl From<&WaterOptions> for WaterUniforms {
  fn from(options: &WaterOptions) -> Self {
    Self {
      sea_level_metres: options.sea_level_metres,
      wave_scale: options.wave_scale,
      reflectivity: options.reflectivity,
      shoreline_softness_metres: options.shoreline_softness_metres,
    }
  }
}

/// One vertex of the flat water plane mesh.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct WaterVertex {
  /// World position in terrain metres.
  pub position: [f32; 3],
}

/// Build a flat quad covering the terrain footprint at the given sea level.
///
/// Winding is counter-clockwise when viewed from above, matching the
/// renderer's `front_face: Ccw` convention.
pub fn build_water_plane(
  half_width_metres: f32,
  half_height_metres: f32,
  sea_level_metres: f32,
) -> [WaterVertex; 6] {
  let y = sea_level_metres;
  let a = [-half_width_metres, y, -half_height_metres];
  let b = [-half_width_metres, y, half_height_metres];
  let c = [half_width_metres, y, -half_height_metres];
  let d = [half_width_metres, y, half_height_metres];

  [
    WaterVertex { position: a },
    WaterVertex { position: b },
    WaterVertex { position: c },
    WaterVertex { position: b },
    WaterVertex { position: d },
    WaterVertex { position: c },
  ]
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn plane_sits_at_the_requested_sea_level() {
    let plane = build_water_plane(100.0, 50.0, 12.5);

    assert!(plane.iter().all(|vertex| vertex.position[1] == 12.5));
  }

  #[test]
  fn plane_spans_the_requested_footprint() {
    let plane = build_water_plane(100.0, 50.0, 0.0);
    let xs: Vec<f32> = plane.iter().map(|vertex| vertex.position[0]).collect();
    let zs: Vec<f32> = plane.iter().map(|vertex| vertex.position[2]).collect();

    assert!(xs.contains(&-100.0) && xs.contains(&100.0));
    assert!(zs.contains(&-50.0) && zs.contains(&50.0));
  }
}
