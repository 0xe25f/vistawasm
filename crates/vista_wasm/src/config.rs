use vista_types::{
  AtmosphereOptions, CameraOptions, CloudStyle, CloudsOptions, FloraOptions, GrassOptions,
  MistOptions, RenderQualityOptions, RenderSizeOptions, SunOptions, VistaEngineOptions,
  WaterOptions,
};

use crate::errors::{VistaError, VistaResult};

/// Raymarch step counts for the volumetric cloud style are clamped to this
/// inclusive range regardless of what a caller requests. This bounds a real
/// shader loop, so an unclamped value would let untrusted JS input hang the
/// GPU driver or the tab — see `docs/environment-upgrade-plan.md` §1.6.
const CLOUD_RAYMARCH_STEPS_MIN: u32 = 8;
const CLOUD_RAYMARCH_STEPS_MAX: u32 = 64;

/// Validated engine configuration.
#[derive(Clone, Debug, PartialEq)]
pub struct VistaEngineConfig {
  /// Initial render dimensions.
  pub render: RenderSizeOptions,
  /// Initial camera controls.
  pub camera: CameraOptions,
  /// Initial sun controls.
  pub sun: SunOptions,
  /// Initial atmosphere controls.
  pub atmosphere: AtmosphereOptions,
  /// Initial water controls.
  pub water: WaterOptions,
  /// Initial flora controls.
  pub flora: FloraOptions,
  /// Initial grass controls.
  pub grass: GrassOptions,
  /// Initial cloud controls.
  pub clouds: CloudsOptions,
  /// Initial mist controls.
  pub mist: MistOptions,
  /// Initial render quality controls.
  pub quality: RenderQualityOptions,
}

impl VistaEngineConfig {
  /// Validate public options and return a complete configuration.
  pub fn from_options(options: VistaEngineOptions) -> VistaResult<Self> {
    let defaults = VistaEngineOptions::default();
    let render = options.render.or(defaults.render).unwrap_or_default();
    let camera = options.camera.or(defaults.camera).unwrap_or_default();
    let sun = options.sun.or(defaults.sun).unwrap_or_default();
    let atmosphere = options
      .atmosphere
      .or(defaults.atmosphere)
      .unwrap_or_default();
    let water = options.water.or(defaults.water).unwrap_or_default();
    let flora = options.flora.or(defaults.flora).unwrap_or_default();
    let grass = options.grass.or(defaults.grass).unwrap_or_default();
    let clouds = options.clouds.or(defaults.clouds).unwrap_or_default();
    let mist = options.mist.or(defaults.mist).unwrap_or_default();
    let quality = options.quality.or(defaults.quality).unwrap_or_default();

    validate_render_size(&render)?;
    validate_camera(&camera)?;
    validate_finite("sun.azimuthDegrees", sun.azimuth_degrees)?;
    validate_finite("sun.elevationDegrees", sun.elevation_degrees)?;
    validate_positive("sun.intensity", sun.intensity)?;
    validate_positive(
      "atmosphere.hazeDistanceMetres",
      atmosphere.haze_distance_metres,
    )?;
    validate_positive("atmosphere.exposure", atmosphere.exposure)?;
    validate_finite("water.seaLevelMetres", water.sea_level_metres)?;
    validate_non_negative("water.waveScale", water.wave_scale)?;
    validate_non_negative("water.reflectivity", water.reflectivity)?;
    validate_non_negative("flora.density", flora.density)?;
    validate_non_negative("flora.speciesVariation", flora.species_variation)?;
    validate_non_negative("flora.windStrength", flora.wind_strength)?;
    validate_grass(&grass)?;
    validate_clouds(&clouds)?;
    validate_mist(&mist)?;

    Ok(Self {
      render,
      camera,
      sun,
      atmosphere,
      water,
      flora,
      grass,
      clouds,
      mist,
      quality,
    })
  }

  #[cfg(target_arch = "wasm32")]
  /// Decode and validate configuration from JavaScript.
  pub fn from_js(value: wasm_bindgen::JsValue) -> Result<Self, wasm_bindgen::JsValue> {
    let options = if value.is_undefined() || value.is_null() {
      VistaEngineOptions::default()
    } else {
      serde_wasm_bindgen::from_value(value)
        .map_err(|error| VistaError::options(error.to_string()).to_js_value())?
    };

    Self::from_options(options).map_err(|error| error.to_js_value())
  }
}

/// Validate render dimensions.
pub fn validate_render_size(render: &RenderSizeOptions) -> VistaResult<()> {
  if render.width == 0 || render.height == 0 {
    return Err(VistaError::options(
      "render width and height must be at least 1.",
    ));
  }

  let ratio = render.device_pixel_ratio.unwrap_or(1.0);

  if !ratio.is_finite() || ratio <= 0.0 || ratio > 8.0 {
    return Err(VistaError::options(
      "render.devicePixelRatio must be a finite value in the range 0 to 8.",
    ));
  }

  Ok(())
}

/// Validate camera controls.
pub fn validate_camera(camera: &CameraOptions) -> VistaResult<()> {
  for (name, value) in [
    ("camera.position[0]", camera.position[0]),
    ("camera.position[1]", camera.position[1]),
    ("camera.position[2]", camera.position[2]),
    ("camera.target[0]", camera.target[0]),
    ("camera.target[1]", camera.target[1]),
    ("camera.target[2]", camera.target[2]),
  ] {
    validate_finite(name, value)?;
  }

  validate_positive("camera.fieldOfViewDegrees", camera.field_of_view_degrees)?;

  if !(1.0..=160.0).contains(&camera.field_of_view_degrees) {
    return Err(VistaError::options(
      "camera.fieldOfViewDegrees must be between 1 and 160 degrees.",
    ));
  }

  if let Some(near) = camera.near_metres {
    validate_positive("camera.nearMetres", near)?;
  }

  if let Some(far) = camera.far_metres {
    validate_positive("camera.farMetres", far)?;
  }

  if let (Some(near), Some(far)) = (camera.near_metres, camera.far_metres) {
    if near >= far {
      return Err(VistaError::options(
        "camera.nearMetres must be smaller than camera.farMetres.",
      ));
    }
  }

  Ok(())
}

/// Validate a finite value.
pub fn validate_finite(name: &str, value: f32) -> VistaResult<()> {
  if !value.is_finite() {
    return Err(VistaError::options(format!("{name} must be finite.")));
  }

  Ok(())
}

/// Validate a strictly positive value.
pub fn validate_positive(name: &str, value: f32) -> VistaResult<()> {
  validate_finite(name, value)?;

  if value <= 0.0 {
    return Err(VistaError::options(format!(
      "{name} must be greater than 0."
    )));
  }

  Ok(())
}

/// Validate a non-negative value.
pub fn validate_non_negative(name: &str, value: f32) -> VistaResult<()> {
  validate_finite(name, value)?;

  if value < 0.0 {
    return Err(VistaError::options(format!("{name} must not be negative.")));
  }

  Ok(())
}

/// Validate that an inclusive integer range is respected.
///
/// Used for options that bound a shader loop (for example the volumetric
/// cloud raymarch step count), where an unbounded value from an untrusted
/// JavaScript caller could hang the GPU driver or the browser tab.
pub fn validate_range(name: &str, value: u32, min: u32, max: u32) -> VistaResult<()> {
  if value < min || value > max {
    return Err(VistaError::options(format!(
      "{name} must be between {min} and {max}."
    )));
  }

  Ok(())
}

/// Validate grass controls.
pub fn validate_grass(grass: &GrassOptions) -> VistaResult<()> {
  validate_non_negative("grass.density", grass.density)?;
  validate_positive("grass.viewDistanceMetres", grass.view_distance_metres)?;
  Ok(())
}

/// Validate cloud controls.
pub fn validate_clouds(clouds: &CloudsOptions) -> VistaResult<()> {
  validate_non_negative("clouds.coverage", clouds.coverage)?;
  validate_finite("clouds.speed", clouds.speed)?;
  validate_finite("clouds.heightMetres", clouds.height_metres)?;

  if clouds.style == CloudStyle::Volumetric {
    let steps = clouds.raymarch_steps.unwrap_or(24);
    validate_range(
      "clouds.raymarchSteps",
      steps,
      CLOUD_RAYMARCH_STEPS_MIN,
      CLOUD_RAYMARCH_STEPS_MAX,
    )?;
  }

  Ok(())
}

/// Validate mist controls.
pub fn validate_mist(mist: &MistOptions) -> VistaResult<()> {
  validate_non_negative("mist.density", mist.density)?;
  validate_finite("mist.baseHeightMetres", mist.base_height_metres)?;
  validate_non_negative("mist.heightFalloffMetres", mist.height_falloff_metres)?;
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn rejects_zero_render_size() {
    let result = validate_render_size(&RenderSizeOptions {
      width: 0,
      height: 1,
      device_pixel_ratio: Some(1.0),
    });

    assert!(result.is_err());
  }

  #[test]
  fn rejects_negative_grass_density() {
    let mut grass = GrassOptions::default();
    grass.density = -0.1;

    assert!(validate_grass(&grass).is_err());
  }

  #[test]
  fn accepts_default_grass_options() {
    assert!(validate_grass(&GrassOptions::default()).is_ok());
  }

  #[test]
  fn painted_clouds_do_not_require_a_raymarch_step_count() {
    let mut clouds = CloudsOptions::default();
    clouds.style = CloudStyle::Painted;
    clouds.raymarch_steps = None;

    assert!(validate_clouds(&clouds).is_ok());
  }

  #[test]
  fn volumetric_clouds_reject_a_raymarch_step_count_below_the_minimum() {
    let mut clouds = CloudsOptions::default();
    clouds.style = CloudStyle::Volumetric;
    clouds.raymarch_steps = Some(CLOUD_RAYMARCH_STEPS_MIN - 1);

    assert!(validate_clouds(&clouds).is_err());
  }

  #[test]
  fn volumetric_clouds_reject_a_raymarch_step_count_above_the_maximum() {
    let mut clouds = CloudsOptions::default();
    clouds.style = CloudStyle::Volumetric;
    clouds.raymarch_steps = Some(CLOUD_RAYMARCH_STEPS_MAX + 1);

    assert!(validate_clouds(&clouds).is_err());
  }

  #[test]
  fn volumetric_clouds_accept_a_raymarch_step_count_within_range() {
    let mut clouds = CloudsOptions::default();
    clouds.style = CloudStyle::Volumetric;
    clouds.raymarch_steps = Some(32);

    assert!(validate_clouds(&clouds).is_ok());
  }

  #[test]
  fn rejects_negative_mist_density() {
    let mut mist = MistOptions::default();
    mist.density = -0.5;

    assert!(validate_mist(&mist).is_err());
  }

  #[test]
  fn accepts_default_mist_options() {
    assert!(validate_mist(&MistOptions::default()).is_ok());
  }

  #[test]
  fn from_options_rejects_an_out_of_range_raymarch_step_count() {
    let mut options = VistaEngineOptions::default();
    let mut clouds = CloudsOptions::default();
    clouds.style = CloudStyle::Volumetric;
    clouds.raymarch_steps = Some(1000);
    options.clouds = Some(clouds);

    assert!(VistaEngineConfig::from_options(options).is_err());
  }

  #[test]
  fn from_options_defaults_grass_clouds_and_mist_to_off() {
    let config = VistaEngineConfig::from_options(VistaEngineOptions::default()).unwrap();

    assert!(!config.grass.enabled);
    assert_eq!(config.clouds.style, CloudStyle::Off);
    assert_eq!(config.mist.style, vista_types::MistStyle::Off);
  }
}
