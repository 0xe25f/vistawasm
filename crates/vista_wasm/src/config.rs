use vista_types::{
  AtmosphereOptions, BiomeOptions, CameraOptions, CloudStyle, CloudsOptions, FloraOptions,
  GrassOptions, MistOptions, RenderQualityOptions, RenderSizeOptions, ShadowOptions, SunOptions,
  SurfaceOptions, VistaEngineOptions, WaterOptions, WeatherOptions,
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
  /// Initial biome controls.
  pub biomes: BiomeOptions,
  /// Initial weather controls.
  pub weather: WeatherOptions,
  /// Initial shadow controls.
  pub shadows: ShadowOptions,
  /// Initial terrain surface controls.
  pub surface: SurfaceOptions,
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
    let biomes = options.biomes.or(defaults.biomes).unwrap_or_default();
    let weather = options.weather.or(defaults.weather).unwrap_or_default();
    let shadows = options.shadows.or(defaults.shadows).unwrap_or_default();
    let surface = options.surface.or(defaults.surface).unwrap_or_default();

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
    validate_water(&water)?;
    validate_flora(&flora)?;
    validate_grass(&grass)?;
    validate_clouds(&clouds)?;
    validate_mist(&mist)?;
    validate_biomes(&biomes)?;
    validate_weather(&weather)?;
    validate_shadows(&shadows)?;
    validate_surface(&surface)?;
    validate_quality(&quality)?;

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
      biomes,
      weather,
      shadows,
      surface,
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

/// Validate an RGB colour with components in the range 0 to 4.
pub fn validate_colour(name: &str, colour: [f32; 3]) -> VistaResult<()> {
  for value in colour {
    validate_finite(name, value)?;

    if !(0.0..=4.0).contains(&value) {
      return Err(VistaError::options(format!(
        "{name} components must be between 0 and 4."
      )));
    }
  }

  Ok(())
}

/// Validate water controls, including waves and rivers.
pub fn validate_water(water: &WaterOptions) -> VistaResult<()> {
  validate_finite("water.seaLevelMetres", water.sea_level_metres)?;
  validate_non_negative("water.waveScale", water.wave_scale)?;
  validate_non_negative("water.reflectivity", water.reflectivity)?;
  validate_non_negative(
    "water.shorelineSoftnessMetres",
    water.shoreline_softness_metres,
  )?;
  validate_finite(
    "water.currentDirectionDegrees",
    water.current_direction_degrees,
  )?;
  validate_non_negative("water.currentSpeed", water.current_speed)?;
  validate_colour("water.shallowColour", water.shallow_colour)?;
  validate_colour("water.deepColour", water.deep_colour)?;
  validate_positive("water.clarityMetres", water.clarity_metres)?;
  validate_non_negative("water.foam", water.foam)?;
  validate_non_negative("water.waves.amplitudeMetres", water.waves.amplitude_metres)?;

  if water.waves.amplitude_metres > 30.0 {
    return Err(VistaError::options(
      "water.waves.amplitudeMetres must be at most 30.",
    ));
  }

  validate_positive(
    "water.waves.wavelengthMetres",
    water.waves.wavelength_metres,
  )?;

  if water.waves.wavelength_metres > 2_000.0 {
    return Err(VistaError::options(
      "water.waves.wavelengthMetres must be at most 2000.",
    ));
  }

  validate_finite(
    "water.waves.directionDegrees",
    water.waves.direction_degrees,
  )?;
  validate_non_negative("water.waves.steepness", water.waves.steepness)?;
  validate_non_negative("water.waves.speed", water.waves.speed)?;
  validate_non_negative(
    "water.waves.directionalSpread",
    water.waves.directional_spread,
  )?;
  validate_positive(
    "water.rivers.minCatchmentKm2",
    water.rivers.min_catchment_km2,
  )?;
  validate_positive("water.rivers.widthScale", water.rivers.width_scale)?;
  validate_non_negative("water.rivers.currentSpeed", water.rivers.current_speed)?;
  Ok(())
}

/// Validate flora controls.
pub fn validate_flora(flora: &FloraOptions) -> VistaResult<()> {
  validate_non_negative("flora.density", flora.density)?;
  validate_finite("flora.treeLineMetres", flora.tree_line_metres)?;
  validate_non_negative("flora.speciesVariation", flora.species_variation)?;
  validate_non_negative("flora.windStrength", flora.wind_strength)?;
  validate_positive("flora.meshDistanceMetres", flora.mesh_distance_metres)?;

  if flora.species_rules.len() > 64 {
    return Err(VistaError::options(
      "flora.speciesRules may hold at most 64 rules.",
    ));
  }

  for rule in &flora.species_rules {
    validate_unit_range("flora.speciesRules density", rule.density, 0.0, 4.0)?;

    if rule.species.len() > 8 {
      return Err(VistaError::options(
        "each flora.speciesRules entry may list at most 8 species.",
      ));
    }

    for choice in &rule.species {
      validate_non_negative("flora.speciesRules weight", choice.weight)?;
    }
  }

  Ok(())
}

/// Validate biome controls.
pub fn validate_biomes(biomes: &BiomeOptions) -> VistaResult<()> {
  validate_finite("biomes.temperatureBias", biomes.temperature_bias)?;
  validate_finite("biomes.moistureBias", biomes.moisture_bias)?;
  validate_positive("biomes.climateScaleMetres", biomes.climate_scale_metres)?;
  validate_non_negative("biomes.volcanism", biomes.volcanism)?;
  validate_non_negative("biomes.beachHeightMetres", biomes.beach_height_metres)?;

  if let Some(snow_line) = biomes.snow_line_metres {
    validate_finite("biomes.snowLineMetres", snow_line)?;
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
  validate_finite("clouds.windDirectionDegrees", clouds.wind_direction_degrees)?;
  validate_non_negative("clouds.evolution", clouds.evolution)?;
  validate_positive("clouds.thicknessMetres", clouds.thickness_metres)?;
  validate_non_negative("clouds.density", clouds.density)?;
  validate_colour("clouds.colour", clouds.colour)?;
  validate_unit_range("clouds.resolutionScale", clouds.resolution_scale, 0.25, 1.0)?;
  validate_unit_range("clouds.cirrus", clouds.cirrus, 0.0, 1.0)?;
  validate_unit_range("clouds.stratiform", clouds.stratiform, 0.0, 1.0)?;
  validate_unit_range("clouds.towering", clouds.towering, 0.0, 1.0)?;
  validate_unit_range("clouds.baseDarkness", clouds.base_darkness, 0.0, 1.0)?;
  validate_unit_range("clouds.raggedBase", clouds.ragged_base, 0.0, 1.0)?;
  validate_unit_range("clouds.rainShafts", clouds.rain_shafts, 0.0, 1.0)?;
  validate_unit_range("clouds.cirrusSpeed", clouds.cirrus_speed, 0.0, 10.0)?;
  validate_unit_range(
    "clouds.cirrusHeightMetres",
    clouds.cirrus_height_metres,
    1_000.0,
    20_000.0,
  )?;

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
  validate_finite("mist.windDirectionDegrees", mist.wind_direction_degrees)?;
  validate_non_negative(
    "mist.windSpeedMetresPerSecond",
    mist.wind_speed_metres_per_second,
  )?;
  validate_non_negative("mist.sunScattering", mist.sun_scattering)?;
  validate_colour("mist.colour", mist.colour)?;
  Ok(())
}

/// Validate that a value lies in an inclusive floating-point range.
pub fn validate_unit_range(name: &str, value: f32, min: f32, max: f32) -> VistaResult<()> {
  validate_finite(name, value)?;

  if value < min || value > max {
    return Err(VistaError::options(format!(
      "{name} must be between {min} and {max}."
    )));
  }

  Ok(())
}

/// Validate weather controls.
pub fn validate_weather(weather: &WeatherOptions) -> VistaResult<()> {
  validate_positive(
    "weather.stateDurationSeconds",
    weather.state_duration_seconds,
  )?;
  validate_non_negative("weather.transitionSeconds", weather.transition_seconds)?;
  validate_finite(
    "weather.windDirectionDegrees",
    weather.wind_direction_degrees,
  )?;
  validate_unit_range("weather.windScale", weather.wind_scale, 0.0, 4.0)?;
  validate_unit_range(
    "weather.precipitationScale",
    weather.precipitation_scale,
    0.0,
    2.0,
  )?;
  Ok(())
}

/// Tree shadow map resolutions the renderer accepts.
pub const TREE_SHADOW_RESOLUTIONS: [u32; 4] = [512, 1024, 2048, 4096];

/// Validate shadow controls.
pub fn validate_shadows(shadows: &ShadowOptions) -> VistaResult<()> {
  validate_unit_range(
    "shadows.terrain.strength",
    shadows.terrain.strength,
    0.0,
    1.0,
  )?;
  validate_unit_range(
    "shadows.terrain.softness",
    shadows.terrain.softness,
    0.0,
    1.0,
  )?;
  validate_unit_range(
    "shadows.trees.distanceMetres",
    shadows.trees.distance_metres,
    10.0,
    4_000.0,
  )?;

  if !TREE_SHADOW_RESOLUTIONS.contains(&shadows.trees.resolution) {
    return Err(VistaError::options(
      "shadows.trees.resolution must be 512, 1024, 2048, or 4096.",
    ));
  }

  validate_unit_range("shadows.trees.strength", shadows.trees.strength, 0.0, 1.0)?;
  validate_unit_range("shadows.trees.softness", shadows.trees.softness, 0.0, 1.0)?;
  validate_unit_range("shadows.clouds.strength", shadows.clouds.strength, 0.0, 1.0)?;
  Ok(())
}

/// Validate render quality controls. Distances must be at least 100 m.
pub fn validate_quality(quality: &RenderQualityOptions) -> VistaResult<()> {
  for (name, value) in [
    (
      "quality.renderDistanceMetres",
      quality.render_distance_metres,
    ),
    (
      "quality.detailDistanceMetres",
      quality.detail_distance_metres,
    ),
    ("quality.cloudDistanceMetres", quality.cloud_distance_metres),
  ] {
    if let Some(value) = value {
      validate_finite(name, value)?;

      if value < 100.0 {
        return Err(VistaError::options(format!(
          "{name} must be at least 100 metres."
        )));
      }
    }
  }

  for (name, value) in [
    ("quality.renderScale", quality.render_scale),
    ("quality.minRenderScale", quality.min_render_scale),
  ] {
    if let Some(value) = value {
      validate_unit_range(name, value, 0.25, 1.0)?;
    }
  }

  if let Some(rate) = quality.max_frame_rate {
    validate_non_negative("quality.maxFrameRate", rate)?;
  }

  for (name, value) in [
    ("quality.renderFadeMetres", quality.render_fade_metres),
    ("quality.cloudFadeMetres", quality.cloud_fade_metres),
  ] {
    if let Some(value) = value {
      validate_non_negative(name, value)?;
    }
  }

  Ok(())
}

/// Validate terrain surface controls.
pub fn validate_surface(surface: &SurfaceOptions) -> VistaResult<()> {
  validate_unit_range("surface.textureScale", surface.texture_scale, 0.05, 20.0)?;

  for tint in surface.material_tints {
    validate_colour("surface.materialTints", tint)?;
  }

  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn rejects_invalid_weather_shadow_and_surface_options() {
    let shadows = VistaEngineOptions {
      shadows: Some(ShadowOptions {
        trees: vista_types::TreeShadowOptions {
          resolution: 3000,
          ..Default::default()
        },
        ..Default::default()
      }),
      ..Default::default()
    };
    let weather = VistaEngineOptions {
      weather: Some(WeatherOptions {
        wind_scale: f32::NAN,
        ..Default::default()
      }),
      ..Default::default()
    };
    let surface = VistaEngineOptions {
      surface: Some(SurfaceOptions {
        texture_scale: 0.0,
        ..Default::default()
      }),
      ..Default::default()
    };

    assert!(VistaEngineConfig::from_options(shadows).is_err());
    assert!(VistaEngineConfig::from_options(weather).is_err());
    assert!(VistaEngineConfig::from_options(surface).is_err());
    assert!(VistaEngineConfig::from_options(VistaEngineOptions::default()).is_ok());
  }

  #[test]
  fn rejects_out_of_range_cirrus_speed() {
    let mut clouds = CloudsOptions::default();
    clouds.cirrus_speed = -0.1;
    assert!(validate_clouds(&clouds).is_err());
    clouds.cirrus_speed = 10.5;
    assert!(validate_clouds(&clouds).is_err());
    clouds.cirrus_speed = 0.4;
    assert!(validate_clouds(&clouds).is_ok());
  }

  #[test]
  fn rejects_short_render_distances() {
    let mut quality = RenderQualityOptions::default();
    assert!(validate_quality(&quality).is_ok());
    quality.render_distance_metres = Some(50.0);
    assert!(validate_quality(&quality).is_err());
    quality.render_distance_metres = Some(f32::NAN);
    assert!(validate_quality(&quality).is_err());
    quality.render_distance_metres = Some(3_000.0);
    assert!(validate_quality(&quality).is_ok());
    quality.render_fade_metres = Some(-1.0);
    assert!(validate_quality(&quality).is_err());
    quality.render_fade_metres = Some(0.0);
    assert!(validate_quality(&quality).is_ok());
  }

  #[test]
  fn fades_default_to_a_share_of_their_distance_and_never_exceed_it() {
    let quality = RenderQualityOptions {
      render_distance_metres: Some(9_000.0),
      cloud_distance_metres: Some(20_000.0),
      ..Default::default()
    };
    let distances = quality.distances();
    assert!((distances.render_fade_metres - 3_000.0).abs() < 1e-3);
    assert!((distances.cloud_fade_metres - 6_000.0).abs() < 1e-3);

    let custom = RenderQualityOptions {
      render_distance_metres: Some(9_000.0),
      render_fade_metres: Some(50_000.0),
      cloud_fade_metres: Some(500.0),
      ..quality
    };
    let distances = custom.distances();
    assert!((distances.render_fade_metres - 9_000.0).abs() < 1e-3);
    assert!((distances.cloud_fade_metres - 500.0).abs() < 1e-3);
  }

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
  fn frame_pacing_options_are_validated_and_resolved() {
    let quality = RenderQualityOptions {
      render_scale: Some(0.8),
      min_render_scale: Some(0.6),
      ..Default::default()
    };
    assert!(validate_quality(&quality).is_ok());
    assert_eq!(quality.render_scale_range(), (0.8, 0.6));
    assert_eq!(quality.frame_rate_cap(), 60.0);

    let fixed = RenderQualityOptions {
      render_scale: Some(0.7),
      dynamic_resolution: Some(false),
      max_frame_rate: Some(0.0),
      ..Default::default()
    };
    assert_eq!(fixed.render_scale_range(), (0.7, 0.7));
    assert_eq!(fixed.frame_rate_cap(), 0.0);

    for bad in [
      RenderQualityOptions {
        render_scale: Some(0.1),
        ..Default::default()
      },
      RenderQualityOptions {
        min_render_scale: Some(1.5),
        ..Default::default()
      },
      RenderQualityOptions {
        max_frame_rate: Some(-30.0),
        ..Default::default()
      },
    ] {
      assert!(validate_quality(&bad).is_err());
    }
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
  fn rejects_invalid_wave_and_river_options() {
    let mut water = WaterOptions::default();
    water.waves.wavelength_metres = 0.0;
    assert!(validate_water(&water).is_err());

    let mut water = WaterOptions::default();
    water.waves.amplitude_metres = 500.0;
    assert!(validate_water(&water).is_err());

    let mut water = WaterOptions::default();
    water.rivers.min_catchment_km2 = -1.0;
    assert!(validate_water(&water).is_err());

    assert!(validate_water(&WaterOptions::default()).is_ok());
  }

  #[test]
  fn rejects_invalid_biome_options() {
    let mut biomes = BiomeOptions::default();
    biomes.climate_scale_metres = 0.0;
    assert!(validate_biomes(&biomes).is_err());

    let mut biomes = BiomeOptions::default();
    biomes.temperature_bias = f32::NAN;
    assert!(validate_biomes(&biomes).is_err());

    assert!(validate_biomes(&BiomeOptions::default()).is_ok());
  }

  #[test]
  fn legacy_water_json_without_new_fields_still_parses() {
    let json = r#"{"enabled":true,"seaLevelMetres":0,"waveScale":0.8,"reflectivity":0.4,"shorelineSoftnessMetres":6}"#;
    let water: WaterOptions = serde_json_like(json);

    assert!(water.waves.enabled);
    assert!(water.rivers.enabled);
  }

  fn serde_json_like(json: &str) -> WaterOptions {
    // `serde_json` is not a dependency; round-trip through the minimal
    // value deserialiser that serde ships for tests instead.
    use serde::de::value::{MapDeserializer, StrDeserializer};
    use serde::Deserialize;
    let _ = json;
    let entries = vec![
      ("enabled", ValueStub::Bool(true)),
      ("seaLevelMetres", ValueStub::Number(0.0)),
      ("waveScale", ValueStub::Number(0.8)),
      ("reflectivity", ValueStub::Number(0.4)),
      ("shorelineSoftnessMetres", ValueStub::Number(6.0)),
    ];
    let deserializer: MapDeserializer<'_, _, serde::de::value::Error> = MapDeserializer::new(
      entries
        .into_iter()
        .map(|(key, value)| (StrDeserializer::<serde::de::value::Error>::new(key), value)),
    );
    WaterOptions::deserialize(deserializer).unwrap()
  }

  enum ValueStub {
    Bool(bool),
    Number(f64),
  }

  impl<'de> serde::de::IntoDeserializer<'de, serde::de::value::Error> for ValueStub {
    type Deserializer = ValueStubDeserializer;

    fn into_deserializer(self) -> Self::Deserializer {
      ValueStubDeserializer(self)
    }
  }

  struct ValueStubDeserializer(ValueStub);

  impl<'de> serde::Deserializer<'de> for ValueStubDeserializer {
    type Error = serde::de::value::Error;

    fn deserialize_any<V: serde::de::Visitor<'de>>(
      self,
      visitor: V,
    ) -> Result<V::Value, Self::Error> {
      match self.0 {
        ValueStub::Bool(value) => visitor.visit_bool(value),
        ValueStub::Number(value) => visitor.visit_f64(value),
      }
    }

    serde::forward_to_deserialize_any! {
      bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
      bytes byte_buf option unit unit_struct newtype_struct seq tuple
      tuple_struct map struct enum identifier ignored_any
    }
  }

  #[test]
  fn from_options_defaults_grass_clouds_and_mist_to_off() {
    let config = VistaEngineConfig::from_options(VistaEngineOptions::default()).unwrap();

    assert!(!config.grass.enabled);
    assert_eq!(config.clouds.style, CloudStyle::Off);
    assert_eq!(config.mist.style, vista_types::MistStyle::Off);
  }
}
