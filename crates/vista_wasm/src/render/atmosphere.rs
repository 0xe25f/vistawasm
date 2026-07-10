use vista_types::AtmosphereOptions;

/// CPU mirror of atmosphere uniforms used by shaders.
#[derive(Clone, Debug, PartialEq)]
pub struct AtmosphereUniforms {
  /// Rayleigh scattering strength.
  pub rayleigh_strength: f32,
  /// Mie scattering strength.
  pub mie_strength: f32,
  /// Haze distance in metres.
  pub haze_distance_metres: f32,
  /// Exposure multiplier.
  pub exposure: f32,
  /// RGB sky tint.
  pub sky_tint: [f32; 3],
}

impl From<&AtmosphereOptions> for AtmosphereUniforms {
  fn from(options: &AtmosphereOptions) -> Self {
    Self {
      rayleigh_strength: options.rayleigh_strength,
      mie_strength: options.mie_strength,
      haze_distance_metres: options.haze_distance_metres,
      exposure: options.exposure,
      sky_tint: options.sky_tint,
    }
  }
}
