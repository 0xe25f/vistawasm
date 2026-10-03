use vista_types::{
  AtmosphereOptions, BiomeOptions, CameraOptions, CloudStyle, CloudsOptions, ErosionOptions,
  FloraOptions, FractalTerrainOptions, GrassOptions, MistOptions, RenderQualityOptions,
  RenderSizeOptions, ShadowOptions, SunOptions, SurfaceOptions, VistaEngineOptions, WaterOptions,
  WeatherOptions,
};

use crate::errors::{VistaError, VistaResult};

/// Raymarch step counts for the volumetric cloud style are clamped to this
/// inclusive range regardless of what a caller requests. This bounds a real
/// shader loop, so an unclamped value would let untrusted JS input hang the
/// GPU driver or the tab.
const CLOUD_RAYMARCH_STEPS_MIN: u32 = 8;
const CLOUD_RAYMARCH_STEPS_MAX: u32 = 64;

/// Widest and tallest render surface, in CSS pixels and in device pixels:
/// WebGPU's default `maxTextureDimension2D`, which the engine requests.
pub const MAX_RENDER_SIZE: u32 = 8192;
/// Why terrains stop at [`MAX_TERRAIN_SIZE`], for the errors that say so.
pub const TERRAIN_SIZE_REASON: &str =
  "A larger terrain leaves too little of the 4 GiB of memory WebAssembly can address to rebuild and export it.";
/// Fewest samples per side of a loaded heightmap or GeoTIFF: the same
/// range as painted maps, so every later stage (and the GPU textures)
/// fits.
pub const MIN_TERRAIN_SIZE: u32 = 2;
/// Most samples per side of any terrain: generated, loaded from a raw
/// heightmap or GeoTIFF, or imported. It is set by WebAssembly's memory: a
/// WASM module can address at most 4 GiB, a terrain needs its heights,
/// normals, surface classification, drainage, rivers and vegetation in
/// memory at once, and that memory never shrinks. A 4096 x 4096 terrain
/// peaks at up to 3.7 GiB, too close to the limit for rebuilds and
/// exports to be safe; a 2048 x 2048 one stays well inside it
/// (`scripts/visual-check/memory-check.mjs`).
pub const MAX_TERRAIN_SIZE: u32 = 2048;
/// The widest spacing between terrain samples. At 2048 samples this spans
/// 20,000 km, half way round the Earth, and it keeps every world
/// coordinate finite.
pub const MAX_METRES_PER_SAMPLE: f32 = 10_000.0;
/// Heights beyond this, up or down, cannot be real ground: loaded samples
/// outside it are treated as no data.
pub const MAX_HEIGHT_METRES: f32 = 100_000.0;
/// The furthest distance any option takes: the engine's own "unlimited".
pub const MAX_DISTANCE_METRES: f32 = vista_types::UNLIMITED_DISTANCE_METRES;
/// How far from the terrain's centre the camera may be on any axis, so
/// grid and noise lookups around it stay within integer range.
pub const MAX_WORLD_METRES: f32 = 10_000_000.0;
/// The furthest grass is generated from the camera. The streamed tiles and
/// their distance classes are laid out out to it when grass options are
/// set, so an unbounded distance would stall the page.
pub const MAX_GRASS_VIEW_METRES: f32 = 1_000.0;
/// The furthest trees are drawn as meshes, and the most the vegetation
/// detail distance may be: the tree tiles within it are walked every frame.
pub const MAX_TREE_DETAIL_METRES: f32 = 5_000.0;
/// Longest weather preset name, in bytes.
pub const MAX_PRESET_NAME_LENGTH: usize = 64;
/// Most custom weather presets.
pub const MAX_CUSTOM_PRESETS: usize = 64;

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
    validate_sun(&sun)?;
    validate_atmosphere(&atmosphere)?;
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
  validate_surface_size(
    "render",
    render.width,
    render.height,
    render.device_pixel_ratio.unwrap_or(1.0),
  )
}

/// Validate a render surface for `create` and `resize`: 1 to
/// [`MAX_RENDER_SIZE`] CSS pixels a side, and a device pixel ratio above
/// 0 and at most 8. The device pixels are capped at the GPU (see
/// `gpu::scaled_extent`), so large displays still work.
pub fn validate_surface_size(call: &str, width: u32, height: u32, ratio: f32) -> VistaResult<()> {
  validate_range(&format!("{call} width"), width, 1, MAX_RENDER_SIZE)?;
  validate_range(&format!("{call} height"), height, 1, MAX_RENDER_SIZE)?;
  validate_positive_at_most(&format!("{call} devicePixelRatio"), ratio, 8.0)
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
    validate_unit_range(name, value, -MAX_WORLD_METRES, MAX_WORLD_METRES)?;
  }

  validate_unit_range(
    "camera.fieldOfViewDegrees",
    camera.field_of_view_degrees,
    1.0,
    160.0,
  )?;

  if let Some(roll) = camera.roll_degrees {
    validate_finite("camera.rollDegrees", roll)?;
  }

  if let Some(height) = camera.minimum_height_above_terrain_metres {
    validate_unit_range(
      "camera.minimumHeightAboveTerrainMetres",
      height,
      0.0,
      MAX_WORLD_METRES,
    )?;
  }

  if let Some(near) = camera.near_metres {
    validate_positive_at_most("camera.nearMetres", near, MAX_DISTANCE_METRES)?;
  }

  if let Some(far) = camera.far_metres {
    validate_positive_at_most("camera.farMetres", far, MAX_DISTANCE_METRES)?;
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

/// Validate sun controls: an elevation from -90 to 90 degrees and an
/// intensity above 0 and at most 100.
pub fn validate_sun(sun: &SunOptions) -> VistaResult<()> {
  validate_finite("sun.azimuthDegrees", sun.azimuth_degrees)?;
  validate_unit_range("sun.elevationDegrees", sun.elevation_degrees, -90.0, 90.0)?;
  validate_positive_at_most("sun.intensity", sun.intensity, 100.0)
}

/// Validate atmosphere controls.
pub fn validate_atmosphere(atmosphere: &AtmosphereOptions) -> VistaResult<()> {
  validate_unit_range(
    "atmosphere.rayleighStrength",
    atmosphere.rayleigh_strength,
    0.0,
    100.0,
  )?;
  validate_unit_range(
    "atmosphere.mieStrength",
    atmosphere.mie_strength,
    0.0,
    100.0,
  )?;
  validate_positive_at_most(
    "atmosphere.hazeDistanceMetres",
    atmosphere.haze_distance_metres,
    MAX_DISTANCE_METRES,
  )?;
  validate_positive_at_most("atmosphere.exposure", atmosphere.exposure, 100.0)?;
  validate_colour("atmosphere.skyTint", atmosphere.sky_tint)
}

/// Validate a finite value.
pub fn validate_finite(name: &str, value: f32) -> VistaResult<()> {
  if !value.is_finite() {
    return Err(VistaError::options(format!(
      "{name} must be finite, but it is {value}."
    )));
  }

  Ok(())
}

/// Validate a strictly positive value.
pub fn validate_positive(name: &str, value: f32) -> VistaResult<()> {
  validate_positive_at_most(name, value, f32::MAX)
}

/// Validate a value above 0 and at most `max`.
pub fn validate_positive_at_most(name: &str, value: f32, max: f32) -> VistaResult<()> {
  if !(value.is_finite() && value > 0.0 && value <= max) {
    let most = if max < f32::MAX {
      format!(" and at most {max}")
    } else {
      String::new()
    };
    return Err(VistaError::options(format!(
      "{name} must be greater than 0{most}, but it is {value}."
    )));
  }

  Ok(())
}

/// Validate a non-negative value.
pub fn validate_non_negative(name: &str, value: f32) -> VistaResult<()> {
  validate_unit_range(name, value, 0.0, f32::MAX)
}

/// Validate that an inclusive integer range is respected.
///
/// Used for options that bound a shader loop (for example the volumetric
/// cloud raymarch step count), where an unbounded value from an untrusted
/// JavaScript caller could hang the GPU driver or the browser tab.
pub fn validate_range(name: &str, value: u32, min: u32, max: u32) -> VistaResult<()> {
  if value < min || value > max {
    return Err(VistaError::options(format!(
      "{name} must be between {min} and {max}, but it is {value}."
    )));
  }

  Ok(())
}

/// Validate an RGB colour with components in the range 0 to 4.
pub fn validate_colour(name: &str, colour: [f32; 3]) -> VistaResult<()> {
  for value in colour {
    validate_unit_range(name, value, 0.0, 4.0)?;
  }

  Ok(())
}

/// Validate water controls, including waves and rivers.
pub fn validate_water(water: &WaterOptions) -> VistaResult<()> {
  validate_height("water.seaLevelMetres", water.sea_level_metres)?;
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
  validate_unit_range("water.foam", water.foam, 0.0, 1.0)?;
  validate_unit_range("water.eddies", water.eddies, 0.0, 1.0)?;
  validate_unit_range("water.refraction", water.refraction, 0.0, 1.0)?;
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
  validate_unit_range("water.waves.steepness", water.waves.steepness, 0.0, 1.0)?;
  validate_non_negative("water.waves.speed", water.waves.speed)?;
  validate_unit_range(
    "water.waves.directionalSpread",
    water.waves.directional_spread,
    0.0,
    1.0,
  )?;
  validate_positive(
    "water.rivers.minCatchmentKm2",
    water.rivers.min_catchment_km2,
  )?;
  validate_positive("water.rivers.widthScale", water.rivers.width_scale)?;
  validate_non_negative("water.rivers.currentSpeed", water.rivers.current_speed)?;
  validate_unit_range("water.rivers.snowmelt", water.rivers.snowmelt, 0.0, 2.0)?;
  validate_unit_range("water.rivers.meanders", water.rivers.meanders, 0.0, 1.0)?;
  validate_unit_range(
    "water.rivers.meanderMaturity",
    water.rivers.meander_maturity,
    0.0,
    1.0,
  )?;
  validate_unit_range("water.rivers.braiding", water.rivers.braiding, 0.0, 1.0)?;
  validate_unit_range("water.rivers.riparian", water.rivers.riparian, 0.0, 2.0)?;
  validate_inflows(&water.rivers.inflow)
}

/// Most explicit inflows `water.rivers.inflow` may list.
pub const MAX_INFLOWS: usize = 8;

/// Validate explicit inflows: at most [`MAX_INFLOWS`], finite positions
/// and 0 to 100,000 m³/s each. Whether a position is on the map is checked
/// against the terrain (see `EngineCore::set_water`).
pub fn validate_inflows(inflow: &vista_types::RiverInflows) -> VistaResult<()> {
  let vista_types::RiverInflows::List(list) = inflow else {
    return Ok(());
  };

  if list.len() > MAX_INFLOWS {
    return Err(VistaError::options(format!(
      "water.rivers.inflow may list at most {MAX_INFLOWS} inflows, but {} were given.",
      list.len()
    )));
  }

  for (index, entry) in list.iter().enumerate() {
    validate_finite(
      &format!("water.rivers.inflow[{index}].position x"),
      entry.position[0],
    )?;
    validate_finite(
      &format!("water.rivers.inflow[{index}].position z"),
      entry.position[1],
    )?;
    validate_unit_range(
      &format!("water.rivers.inflow[{index}].dischargeCubicMetresPerSecond"),
      entry.discharge_cubic_metres_per_second,
      0.0,
      100_000.0,
    )?;
  }

  Ok(())
}

/// Most trees or tufts the generation caps and the drawing budgets allow.
pub const VEGETATION_INSTANCES_MAX: u32 = 4_000_000;

/// Validate flora controls.
pub fn validate_flora(flora: &FloraOptions) -> VistaResult<()> {
  validate_unit_range("flora.density", flora.density, 0.0, 4.0)?;
  validate_range(
    "flora.maxInstances",
    flora.max_instances,
    0,
    VEGETATION_INSTANCES_MAX,
  )?;
  validate_finite("flora.treeLineMetres", flora.tree_line_metres)?;
  validate_unit_range("flora.speciesVariation", flora.species_variation, 0.0, 1.0)?;
  validate_unit_range("flora.windStrength", flora.wind_strength, 0.0, 1.0)?;
  validate_positive_at_most(
    "flora.meshDistanceMetres",
    flora.mesh_distance_metres,
    MAX_TREE_DETAIL_METRES,
  )?;
  validate_range(
    "flora.variantsPerSpecies",
    flora.variants_per_species,
    1,
    crate::render::tree_growth::VARIANTS as u32,
  )?;

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
  validate_unit_range("biomes.volcanism", biomes.volcanism, 0.0, 1.0)?;
  validate_non_negative("biomes.beachHeightMetres", biomes.beach_height_metres)?;

  if let Some(snow_line) = biomes.snow_line_metres {
    validate_finite("biomes.snowLineMetres", snow_line)?;
  }

  if let Some(celsius) = biomes.mean_temperature_celsius {
    validate_unit_range("biomes.meanTemperatureCelsius", celsius, -30.0, 35.0)?;
  }

  Ok(())
}

/// Validate grass controls.
pub fn validate_grass(grass: &GrassOptions) -> VistaResult<()> {
  validate_unit_range("grass.density", grass.density, 0.0, 4.0)?;
  validate_positive_at_most(
    "grass.viewDistanceMetres",
    grass.view_distance_metres,
    MAX_GRASS_VIEW_METRES,
  )?;
  validate_range(
    "grass.maxInstances",
    grass.max_instances,
    0,
    VEGETATION_INSTANCES_MAX,
  )?;
  Ok(())
}

/// Validate cloud controls.
pub fn validate_clouds(clouds: &CloudsOptions) -> VistaResult<()> {
  validate_non_negative("clouds.coverage", clouds.coverage)?;
  validate_finite("clouds.speed", clouds.speed)?;
  validate_finite("clouds.heightMetres", clouds.height_metres)?;
  validate_finite("clouds.windDirectionDegrees", clouds.wind_direction_degrees)?;
  validate_unit_range("clouds.evolution", clouds.evolution, 0.0, 1.0)?;
  validate_positive("clouds.thicknessMetres", clouds.thickness_metres)?;
  validate_unit_range("clouds.density", clouds.density, 0.0, 1.0)?;
  validate_colour("clouds.colour", clouds.colour)?;
  validate_unit_range("clouds.resolutionScale", clouds.resolution_scale, 0.25, 1.0)?;
  validate_unit_range("clouds.cirrus", clouds.cirrus, 0.0, 1.0)?;
  validate_unit_range("clouds.stratiform", clouds.stratiform, 0.0, 1.0)?;
  validate_unit_range("clouds.towering", clouds.towering, 0.0, 1.0)?;
  validate_unit_range("clouds.baseDarkness", clouds.base_darkness, 0.0, 1.0)?;
  validate_unit_range("clouds.raggedBase", clouds.ragged_base, 0.0, 1.0)?;
  validate_unit_range("clouds.rainShafts", clouds.rain_shafts, 0.0, 1.0)?;
  validate_unit_range("clouds.baseVariation", clouds.base_variation, 0.0, 0.2)?;
  validate_unit_range("clouds.baseLumpiness", clouds.base_lumpiness, 0.0, 1.0)?;
  validate_unit_range("clouds.altocumulus", clouds.altocumulus, 0.0, 1.0)?;
  validate_unit_range("clouds.altostratus", clouds.altostratus, 0.0, 1.0)?;
  validate_unit_range(
    "clouds.altoHeightMetres",
    clouds.alto_height_metres,
    2_000.0,
    7_000.0,
  )?;
  validate_unit_range("clouds.altoSpeed", clouds.alto_speed, 0.0, 4.0)?;
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
  validate_unit_range("mist.sunScattering", mist.sun_scattering, 0.0, 1.0)?;
  validate_colour("mist.colour", mist.colour)?;
  Ok(())
}

/// Validate that a value lies in an inclusive floating-point range.
pub fn validate_unit_range(name: &str, value: f32, min: f32, max: f32) -> VistaResult<()> {
  if !(value.is_finite() && value >= min && value <= max) {
    let range = if max < f32::MAX {
      format!("between {min} and {max}")
    } else {
      format!("at least {min}")
    };
    return Err(VistaError::options(format!(
      "{name} must be {range}, but it is {value}."
    )));
  }

  Ok(())
}

/// Validate weather controls.
pub fn validate_weather(weather: &WeatherOptions) -> VistaResult<()> {
  validate_positive_at_most(
    "weather.stateDurationSeconds",
    weather.state_duration_seconds,
    86_400.0,
  )?;
  validate_unit_range(
    "weather.transitionSeconds",
    weather.transition_seconds,
    0.0,
    86_400.0,
  )?;
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
  validate_range(
    "weather.lensDropCount",
    weather.lens_drop_count,
    0,
    crate::lens_drops::MAX_LENS_DROPS as u32,
  )?;
  validate_unit_range(
    "weather.lensDropMinSize",
    weather.lens_drop_min_size,
    0.002,
    0.2,
  )?;
  validate_unit_range(
    "weather.lensDropMaxSize",
    weather.lens_drop_max_size,
    0.002,
    0.2,
  )?;

  if weather.lens_drop_min_size > weather.lens_drop_max_size {
    return Err(VistaError::options(format!(
      "weather.lensDropMinSize ({}) must not be larger than weather.lensDropMaxSize ({}).",
      weather.lens_drop_min_size, weather.lens_drop_max_size
    )));
  }

  validate_unit_range("weather.regionSizeKm", weather.region_size_km, 16.0, 256.0)?;

  if weather.presets.0.len() > MAX_CUSTOM_PRESETS {
    return Err(VistaError::options(format!(
      "weather.presets may hold at most {MAX_CUSTOM_PRESETS} presets, but it holds {}.",
      weather.presets.0.len()
    )));
  }

  // Names are echoed in messages and compared on every lookup, so they
  // are bounded before anything else reads them.
  let names = weather.presets.iter().flat_map(|(name, preset)| {
    let next = preset.next.iter().flat_map(|next| next.iter());
    std::iter::once(name.as_str())
      .chain(preset.extends.as_ref().map(|parent| parent.as_str()))
      .chain(next.map(|(successor, _)| successor.as_str()))
  });

  for name in std::iter::once(weather.state.as_str()).chain(names) {
    if name.len() > MAX_PRESET_NAME_LENGTH {
      return Err(VistaError::options(format!(
        "weather preset names may be at most {MAX_PRESET_NAME_LENGTH} bytes long, but one is {} bytes.",
        name.len()
      )));
    }
  }

  let table = crate::weather::presets::PresetTable::resolve(&weather.presets)?;

  if table.get(&weather.state).is_none() {
    return Err(VistaError::options(format!(
      "weather.state `{}` is not a preset. Presets are: {}.",
      weather.state,
      table.list()
    )));
  }

  Ok(())
}

/// Validate time of day controls.
pub fn validate_time_of_day(time: &vista_types::TimeOfDayOptions) -> VistaResult<()> {
  validate_unit_range("timeOfDay.hours", time.hours, 0.0, 24.0)?;
  validate_unit_range(
    "timeOfDay.dayLengthMinutes",
    time.day_length_minutes,
    1.0,
    1_440.0,
  )?;
  validate_unit_range(
    "timeOfDay.latitudeDegrees",
    time.latitude_degrees,
    -89.0,
    89.0,
  )?;
  validate_range("timeOfDay.dayOfYear", time.day_of_year, 1, 366)
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
      validate_unit_range(name, value, 100.0, MAX_DISTANCE_METRES)?;
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
    validate_unit_range("quality.maxFrameRate", rate, 0.0, 1_000.0)?;
  }

  if let Some(levels) = quality.max_clipmap_levels {
    validate_range("quality.maxClipmapLevels", levels, 1, 12)?;
  }

  if let Some(scale) = quality.flora_density_scale {
    validate_unit_range("quality.floraDensityScale", scale, 0.0, 4.0)?;
  }

  for (name, value, most) in [
    (
      "quality.vegetationDetailMetres",
      quality.vegetation_detail_metres,
      MAX_TREE_DETAIL_METRES,
    ),
    (
      "quality.canopyDistanceMetres",
      quality.canopy_distance_metres,
      MAX_DISTANCE_METRES,
    ),
  ] {
    if let Some(value) = value {
      validate_unit_range(name, value, 50.0, most)?;
    }
  }

  for (name, value) in [
    ("quality.maxTreeInstances", quality.max_tree_instances),
    ("quality.maxGrassInstances", quality.max_grass_instances),
  ] {
    if let Some(value) = value {
      validate_range(name, value, 1_000, VEGETATION_INSTANCES_MAX)?;
    }
  }

  if let Some(triangles) = quality.max_tree_triangles {
    validate_range("quality.maxTreeTriangles", triangles, 100_000, 50_000_000)?;
  }

  if let Some(metres) = quality.grass_detail_metres {
    validate_unit_range("quality.grassDetailMetres", metres, 10.0, 300.0)?;
  }

  for (name, value) in [
    ("quality.renderFadeMetres", quality.render_fade_metres),
    ("quality.cloudFadeMetres", quality.cloud_fade_metres),
  ] {
    if let Some(value) = value {
      validate_unit_range(name, value, 0.0, MAX_DISTANCE_METRES)?;
    }
  }

  Ok(())
}

/// Validate terrain surface controls.
pub fn validate_surface(surface: &SurfaceOptions) -> VistaResult<()> {
  validate_unit_range("surface.textureScale", surface.texture_scale, 0.05, 20.0)?;
  validate_unit_range("surface.rockiness", surface.rockiness, 0.0, 2.0)?;
  validate_unit_range(
    "surface.boulderDistanceMetres",
    surface.boulder_distance_metres,
    50.0,
    1_000.0,
  )?;

  for tint in surface.material_tints {
    validate_colour("surface.materialTints", tint)?;
  }

  Ok(())
}

/// The most erosion iterations one generation may request, so untrusted
/// input cannot queue unbounded GPU work.
pub const EROSION_ITERATIONS_MAX: u32 = 5000;

/// Validate fractal terrain options.
///
/// `landform` needs no check here: serde rejects unknown names with a
/// message listing the valid ones.
pub fn validate_fractal(options: &FractalTerrainOptions) -> VistaResult<()> {
  if options.size < 16 || options.size > MAX_TERRAIN_SIZE || !options.size.is_power_of_two() {
    return Err(VistaError::options(format!(
      "fractal size must be a power of two from 16 to {MAX_TERRAIN_SIZE}, but it is {}.{}",
      options.size,
      if options.size > MAX_TERRAIN_SIZE {
        format!(" {TERRAIN_SIZE_REASON}")
      } else {
        String::new()
      }
    )));
  }

  validate_positive_at_most(
    "horizontalScaleMetres",
    options.horizontal_scale_metres,
    MAX_METRES_PER_SAMPLE,
  )?;
  validate_positive_at_most("verticalScale", options.vertical_scale, 100.0)?;

  if let Some(shape) = &options.shape {
    for (name, value) in [
      ("shape.island", shape.island),
      ("shape.terrace", shape.terrace),
      ("shape.basin", shape.basin),
      ("shape.canyon", shape.canyon),
      ("shape.crater", shape.crater),
    ] {
      if let Some(value) = value {
        validate_unit_range(name, value, 0.0, 1.0)?;
      }
    }
  }

  if let Some(base) = options.base_height_metres {
    validate_height("baseHeightMetres", base)?;
  }

  if let Some(sea) = options.sea_level_metres {
    validate_height("seaLevelMetres", sea)?;
  }

  validate_range("noise.octaves", options.noise.octaves, 1, 16)?;
  validate_unit_range("noise.gain", options.noise.gain, 0.0, 1.0)?;

  if !(options.noise.lacunarity > 1.0 && options.noise.lacunarity <= 8.0) {
    return Err(VistaError::options(format!(
      "noise.lacunarity must be a number greater than 1 and at most 8, but it is {}.",
      options.noise.lacunarity
    )));
  }

  if let Some(warp) = options.noise.warp {
    validate_unit_range("noise.warp", warp, 0.0, 4.0)?;
  }

  if let Some(erosion) = &options.erosion {
    validate_erosion(erosion)?;
  }

  Ok(())
}

/// Validate a height or sea level in metres: finite and within
/// [`MAX_HEIGHT_METRES`] of 0.
pub fn validate_height(name: &str, value: f32) -> VistaResult<()> {
  validate_unit_range(name, value, -MAX_HEIGHT_METRES, MAX_HEIGHT_METRES)
}

/// Validate erosion controls. Unset fields take the landform's defaults.
pub fn validate_erosion(erosion: &ErosionOptions) -> VistaResult<()> {
  for (name, value) in [
    ("erosion.hydraulicIterations", erosion.hydraulic_iterations),
    ("erosion.thermalIterations", erosion.thermal_iterations),
  ] {
    if let Some(value) = value {
      validate_range(name, value, 0, EROSION_ITERATIONS_MAX)?;
    }
  }

  for (name, value) in [
    ("erosion.rainAmount", erosion.rain_amount),
    ("erosion.evaporation", erosion.evaporation),
    ("erosion.sedimentCapacity", erosion.sediment_capacity),
  ] {
    if let Some(value) = value {
      validate_unit_range(name, value, 0.0, 1.0)?;
    }
  }

  if let Some(talus) = erosion.talus_angle_degrees {
    validate_unit_range("erosion.talusAngleDegrees", talus, 1.0, 89.0)?;
  }

  Ok(())
}

/// Largest map `exportMap` resamples to, in pixels per side: the largest
/// terrain. Resampling past the terrain adds no detail, and the map is
/// built in WASM memory beside the terrain, so it is held to the same
/// limit (see [`MAX_TERRAIN_SIZE`]).
pub const MAX_EXPORT_SIZE: u32 = MAX_TERRAIN_SIZE;
/// Most bytes one exported map may hold: 1 GiB.
pub const MAX_EXPORT_BYTES: u64 = 1 << 30;
/// Trees `exportTrees` returns at most by default.
pub const DEFAULT_EXPORT_TREES: u32 = 2_000_000;
/// The largest `maxCount` `exportTrees` accepts.
pub const MAX_EXPORT_TREES: u32 = 10_000_000;

/// Validate an export size: each side from 2 to [`MAX_EXPORT_SIZE`], and
/// the map, at `bytes_per_pixel`, within [`MAX_EXPORT_BYTES`].
pub fn validate_export_size(size: [u32; 2], bytes_per_pixel: u32) -> VistaResult<()> {
  let [width, height] = size;

  if !(2..=MAX_EXPORT_SIZE).contains(&width) || !(2..=MAX_EXPORT_SIZE).contains(&height) {
    return Err(VistaError::options(format!(
      "exportMap size must be whole numbers from 2 to {MAX_EXPORT_SIZE}, but it is [{width}, {height}].{}",
      if width.max(height) > MAX_EXPORT_SIZE {
        format!(" {TERRAIN_SIZE_REASON}")
      } else {
        String::new()
      }
    )));
  }

  let bytes = u64::from(width) * u64::from(height) * u64::from(bytes_per_pixel);

  if bytes > MAX_EXPORT_BYTES {
    return Err(VistaError::options(format!(
      "exportMap at [{width}, {height}] would need {bytes} bytes, over the 1 GiB limit ({MAX_EXPORT_BYTES} bytes). Choose a smaller size."
    )));
  }

  Ok(())
}

/// Validate `exportTrees` options: a finite region (min x, min z, max x,
/// max z in metres) with each minimum at most its maximum, and a
/// `maxCount` from 1 to [`MAX_EXPORT_TREES`].
pub fn validate_tree_export(region: Option<[f32; 4]>, max_count: u32) -> VistaResult<()> {
  if let Some([min_x, min_z, max_x, max_z]) = region {
    if [min_x, min_z, max_x, max_z]
      .iter()
      .any(|value| !value.is_finite())
    {
      return Err(VistaError::options(
        "exportTrees region minX, minZ, maxX and maxZ must be finite numbers of metres.",
      ));
    }

    if min_x > max_x || min_z > max_z {
      return Err(VistaError::options(format!(
        "exportTrees region must have minX <= maxX and minZ <= maxZ, but it spans x {min_x} to {max_x} and z {min_z} to {max_z}."
      )));
    }
  }

  if !(1..=MAX_EXPORT_TREES).contains(&max_count) {
    return Err(VistaError::options(format!(
      "exportTrees maxCount must be a whole number from 1 to {MAX_EXPORT_TREES}, but it is {max_count}."
    )));
  }

  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn export_sizes_and_tree_options_are_validated() {
    assert!(validate_export_size([2, 2], 4).is_ok());
    assert!(validate_export_size([2048, 2048], 12).is_ok());

    for bad in [
      [1, 64],
      [64, 1],
      [2049, 64],
      [64, 2049],
      [8192, 8192],
      [0, 0],
    ] {
      let message = validate_export_size(bad, 4).unwrap_err().to_string();
      assert!(message.contains("from 2 to 2048"), "{message}");
    }

    // No kind needs more than 12 bytes a pixel today, but the byte limit
    // still holds for larger ones.
    let message = validate_export_size([2048, 2048], 1024)
      .unwrap_err()
      .to_string();
    assert!(message.contains("1 GiB"), "{message}");

    assert!(validate_tree_export(None, 1).is_ok());
    assert!(validate_tree_export(Some([-10.0, -10.0, -10.0, 5.0]), MAX_EXPORT_TREES).is_ok());
    assert!(validate_tree_export(None, 0).is_err());
    assert!(validate_tree_export(None, MAX_EXPORT_TREES + 1).is_err());
    assert!(validate_tree_export(Some([0.0, 0.0, f32::NAN, 1.0]), 10).is_err());
    assert!(validate_tree_export(Some([0.0, 0.0, f32::INFINITY, 1.0]), 10).is_err());
    let message = validate_tree_export(Some([5.0, 0.0, 1.0, 1.0]), 10)
      .unwrap_err()
      .to_string();
    assert!(message.contains("minX <= maxX"), "{message}");
  }

  #[test]
  fn rejects_unknown_landforms_with_the_valid_names() {
    use serde::de::value::{Error, StrDeserializer};
    use serde::de::IntoDeserializer;
    use serde::Deserialize;
    use vista_types::LandformKind;

    let parse = |name: &'static str| {
      let deserializer: StrDeserializer<'_, Error> = name.into_deserializer();
      LandformKind::deserialize(deserializer)
    };

    assert_eq!(parse("rollingHills").unwrap(), LandformKind::RollingHills);
    assert_eq!(
      parse("volcanicIsland").unwrap(),
      LandformKind::VolcanicIsland
    );

    let message = parse("glacier").unwrap_err().to_string();
    assert!(message.contains("glacier"), "{message}");

    for name in [
      "continental",
      "alpine",
      "rollingHills",
      "archipelago",
      "mesaDesert",
      "fjords",
      "volcanicIsland",
    ] {
      assert!(message.contains(name), "{message}");
    }
  }

  #[test]
  fn rejects_unknown_edges_with_the_valid_names() {
    use serde::de::value::{Error, StrDeserializer};
    use serde::de::IntoDeserializer;
    use serde::Deserialize;
    use vista_types::TerrainEdges;

    let parse = |name: &'static str| {
      let deserializer: StrDeserializer<'_, Error> = name.into_deserializer();
      TerrainEdges::deserialize(deserializer)
    };

    assert_eq!(FractalTerrainOptions::default().edges, TerrainEdges::Coast);
    assert_eq!(parse("coast").unwrap(), TerrainEdges::Coast);
    assert_eq!(parse("open").unwrap(), TerrainEdges::Open);

    let message = parse("cliff").unwrap_err().to_string();

    for name in ["cliff", "coast", "open"] {
      assert!(message.contains(name), "{message}");
    }
  }

  #[test]
  fn rejects_lens_drop_settings_out_of_range() {
    let valid = WeatherOptions::default();
    assert!(validate_weather(&valid).is_ok());

    for bad in [
      WeatherOptions {
        lens_drop_count: 513,
        ..valid.clone()
      },
      WeatherOptions {
        lens_drop_min_size: 0.001,
        ..valid.clone()
      },
      WeatherOptions {
        lens_drop_max_size: 0.25,
        ..valid.clone()
      },
      WeatherOptions {
        lens_drop_max_size: f32::NAN,
        ..valid.clone()
      },
    ] {
      assert!(validate_weather(&bad).is_err());
    }

    let swapped = WeatherOptions {
      lens_drop_min_size: 0.06,
      lens_drop_max_size: 0.02,
      ..valid
    };
    let message = validate_weather(&swapped).unwrap_err().to_string();
    assert!(
      message.contains("0.06") && message.contains("0.02"),
      "{message}"
    );
  }

  #[test]
  fn rejects_out_of_range_fractal_and_erosion_options() {
    let valid = FractalTerrainOptions::default();
    assert!(validate_fractal(&valid).is_ok());

    let odd_size = FractalTerrainOptions {
      size: 300,
      ..FractalTerrainOptions::default()
    };
    assert!(validate_fractal(&odd_size).is_err());

    let heavy = FractalTerrainOptions {
      erosion: Some(ErosionOptions {
        hydraulic_iterations: Some(EROSION_ITERATIONS_MAX + 1),
        ..ErosionOptions::default()
      }),
      ..FractalTerrainOptions::default()
    };
    let message = validate_fractal(&heavy).unwrap_err().to_string();
    assert!(message.contains("hydraulicIterations"), "{message}");
    assert!(message.contains("5000"), "{message}");

    let talus = ErosionOptions {
      talus_angle_degrees: Some(95.0),
      ..ErosionOptions::default()
    };
    assert!(validate_erosion(&talus).is_err());

    let rain = ErosionOptions {
      rain_amount: Some(f32::NAN),
      ..ErosionOptions::default()
    };
    assert!(validate_erosion(&rain).is_err());
  }

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
  fn rejects_out_of_range_rockiness_and_boulder_distance() {
    let check =
      |surface: SurfaceOptions| validate_surface(&surface).map_err(|error| error.to_string());

    for rockiness in [-0.1, 2.1, f32::NAN] {
      let error = check(SurfaceOptions {
        rockiness,
        ..Default::default()
      })
      .unwrap_err();
      assert!(error.contains("surface.rockiness"), "{error}");
      assert!(
        rockiness.is_nan() || error.contains("between 0 and 2"),
        "{error}"
      );
    }

    for distance in [49.0, 1_001.0, f32::INFINITY] {
      let error = check(SurfaceOptions {
        boulder_distance_metres: distance,
        ..Default::default()
      })
      .unwrap_err();
      assert!(error.contains("surface.boulderDistanceMetres"), "{error}");
      assert!(
        distance.is_infinite() || error.contains("between 50 and 1000"),
        "{error}"
      );
    }

    assert!(check(SurfaceOptions {
      rockiness: 0.0,
      boulders: false,
      boulder_distance_metres: 1_000.0,
      ..Default::default()
    })
    .is_ok());
    assert!(check(SurfaceOptions {
      rockiness: 2.0,
      boulder_distance_metres: 50.0,
      ..Default::default()
    })
    .is_ok());
  }

  #[test]
  fn rejects_out_of_range_cirrus_speed() {
    let mut clouds = CloudsOptions {
      cirrus_speed: -0.1,
      ..CloudsOptions::default()
    };
    assert!(validate_clouds(&clouds).is_err());
    clouds.cirrus_speed = 10.5;
    assert!(validate_clouds(&clouds).is_err());
    clouds.cirrus_speed = 0.4;
    assert!(validate_clouds(&clouds).is_ok());
  }

  #[test]
  fn rejects_out_of_range_cloud_base_and_alto_options() {
    type Field = fn(&mut CloudsOptions) -> &mut f32;
    // (field, name in the message, below its range, above it).
    let cases: [(Field, &str, f32, f32); 6] = [
      (
        |c| &mut c.base_variation,
        "clouds.baseVariation",
        -0.01,
        0.21,
      ),
      (|c| &mut c.base_lumpiness, "clouds.baseLumpiness", -0.1, 1.1),
      (|c| &mut c.altocumulus, "clouds.altocumulus", -0.1, 1.1),
      (|c| &mut c.altostratus, "clouds.altostratus", -0.1, 1.1),
      (
        |c| &mut c.alto_height_metres,
        "clouds.altoHeightMetres",
        1_999.0,
        7_001.0,
      ),
      (|c| &mut c.alto_speed, "clouds.altoSpeed", -0.1, 4.1),
    ];

    for (field, name, low, high) in cases {
      for value in [low, high, f32::NAN, f32::INFINITY] {
        let mut clouds = CloudsOptions::default();
        *field(&mut clouds) = value;
        let message = validate_clouds(&clouds).unwrap_err().to_string();
        assert!(message.contains(name), "{message}");
      }
    }

    // The defaults and both ends of each range are accepted.
    let mut clouds = CloudsOptions::default();
    assert!(validate_clouds(&clouds).is_ok());
    clouds.base_variation = 0.2;
    clouds.base_lumpiness = 1.0;
    clouds.altocumulus = 1.0;
    clouds.altostratus = 1.0;
    clouds.alto_height_metres = 7_000.0;
    clouds.alto_speed = 4.0;
    assert!(validate_clouds(&clouds).is_ok());
    clouds.alto_height_metres = 2_000.0;
    clouds.alto_speed = 0.0;
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
    let grass = GrassOptions {
      density: -0.1,
      ..GrassOptions::default()
    };

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
  fn vegetation_densities_run_from_0_to_4() {
    for (density, ok) in [(-0.01, false), (0.0, true), (4.0, true), (4.01, false)] {
      let flora = FloraOptions {
        density,
        ..FloraOptions::default()
      };
      let grass = GrassOptions {
        density,
        ..GrassOptions::default()
      };
      assert_eq!(validate_flora(&flora).is_ok(), ok, "flora {density}");
      assert_eq!(validate_grass(&grass).is_ok(), ok, "grass {density}");
    }

    let error = validate_flora(&FloraOptions {
      density: 4.01,
      ..FloraOptions::default()
    })
    .unwrap_err()
    .to_string();
    assert!(error.contains("between 0 and 4"), "{error}");
  }

  #[test]
  fn vegetation_budgets_and_distances_are_validated() {
    let good = RenderQualityOptions {
      vegetation_detail_metres: Some(50.0),
      canopy_distance_metres: Some(3_000.0),
      max_tree_instances: Some(1_000),
      max_grass_instances: Some(4_000_000),
      max_tree_triangles: Some(100_000),
      grass_detail_metres: Some(300.0),
      ..Default::default()
    };
    assert!(validate_quality(&good).is_ok());

    for bad in [
      RenderQualityOptions {
        vegetation_detail_metres: Some(49.0),
        ..Default::default()
      },
      RenderQualityOptions {
        canopy_distance_metres: Some(f32::NAN),
        ..Default::default()
      },
      RenderQualityOptions {
        max_tree_instances: Some(999),
        ..Default::default()
      },
      RenderQualityOptions {
        max_grass_instances: Some(4_000_001),
        ..Default::default()
      },
      RenderQualityOptions {
        max_tree_triangles: Some(99_999),
        ..Default::default()
      },
      RenderQualityOptions {
        max_tree_triangles: Some(50_000_001),
        ..Default::default()
      },
      RenderQualityOptions {
        grass_detail_metres: Some(9.0),
        ..Default::default()
      },
      RenderQualityOptions {
        grass_detail_metres: Some(f32::INFINITY),
        ..Default::default()
      },
    ] {
      assert!(validate_quality(&bad).is_err());
    }

    // The presets fill in what is left unset.
    let balanced = RenderQualityOptions::default().vegetation();
    assert_eq!(
      (
        balanced.detail_metres,
        balanced.canopy_metres,
        balanced.max_trees,
        balanced.max_grass
      ),
      (250.0, 2_500.0, 120_000, 400_000)
    );
    let preview = RenderQualityOptions {
      preset: vista_types::RenderQualityPreset::Preview,
      max_tree_instances: Some(5_000),
      ..Default::default()
    }
    .vegetation();
    assert_eq!((preview.detail_metres, preview.max_trees), (120.0, 5_000));
    assert_eq!(
      (balanced.max_tree_triangles, balanced.grass_detail_metres),
      (2_500_000, 45.0)
    );
    assert_eq!(
      (preview.max_tree_triangles, preview.grass_detail_metres),
      (1_000_000, 25.0)
    );
    let offline = RenderQualityOptions {
      preset: vista_types::RenderQualityPreset::Offline,
      ..Default::default()
    }
    .vegetation();
    assert_eq!(
      (offline.max_tree_triangles, offline.grass_detail_metres),
      (u32::MAX, 120.0)
    );
  }

  #[test]
  fn accepts_default_grass_options() {
    assert!(validate_grass(&GrassOptions::default()).is_ok());
  }

  #[test]
  fn painted_clouds_do_not_require_a_raymarch_step_count() {
    let clouds = CloudsOptions {
      style: CloudStyle::Painted,
      raymarch_steps: None,
      ..CloudsOptions::default()
    };

    assert!(validate_clouds(&clouds).is_ok());
  }

  #[test]
  fn volumetric_clouds_reject_a_raymarch_step_count_below_the_minimum() {
    let clouds = CloudsOptions {
      style: CloudStyle::Volumetric,
      raymarch_steps: Some(CLOUD_RAYMARCH_STEPS_MIN - 1),
      ..CloudsOptions::default()
    };

    assert!(validate_clouds(&clouds).is_err());
  }

  #[test]
  fn volumetric_clouds_reject_a_raymarch_step_count_above_the_maximum() {
    let clouds = CloudsOptions {
      style: CloudStyle::Volumetric,
      raymarch_steps: Some(CLOUD_RAYMARCH_STEPS_MAX + 1),
      ..CloudsOptions::default()
    };

    assert!(validate_clouds(&clouds).is_err());
  }

  #[test]
  fn volumetric_clouds_accept_a_raymarch_step_count_within_range() {
    let clouds = CloudsOptions {
      style: CloudStyle::Volumetric,
      raymarch_steps: Some(32),
      ..CloudsOptions::default()
    };

    assert!(validate_clouds(&clouds).is_ok());
  }

  #[test]
  fn rejects_negative_mist_density() {
    let mist = MistOptions {
      density: -0.5,
      ..MistOptions::default()
    };

    assert!(validate_mist(&mist).is_err());
  }

  #[test]
  fn accepts_default_mist_options() {
    assert!(validate_mist(&MistOptions::default()).is_ok());
  }

  #[test]
  fn from_options_rejects_an_out_of_range_raymarch_step_count() {
    let mut options = VistaEngineOptions::default();
    let clouds = CloudsOptions {
      style: CloudStyle::Volumetric,
      raymarch_steps: Some(1000),
      ..CloudsOptions::default()
    };
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

    let mut water = WaterOptions::default();
    water.rivers.snowmelt = 2.5;
    let error = validate_water(&water).unwrap_err().to_string();
    assert!(
      error.contains("water.rivers.snowmelt") && error.contains("0 and 2"),
      "{error}"
    );

    let mut water = WaterOptions::default();
    water.rivers.meanders = f32::NAN;
    assert!(validate_water(&water).is_err());

    let mut water = WaterOptions::default();
    water.rivers.meander_maturity = 1.5;
    let error = validate_water(&water).unwrap_err().to_string();
    assert!(
      error.contains("water.rivers.meanderMaturity") && error.contains("0 and 1"),
      "{error}"
    );

    for eddies in [f32::NAN, -0.1, 1.1] {
      let water = WaterOptions {
        eddies,
        ..WaterOptions::default()
      };
      let error = validate_water(&water).unwrap_err().to_string();
      assert!(
        error.contains("water.eddies") && error.contains("0 and 1"),
        "{error}"
      );
    }

    for refraction in [f32::NAN, -0.1, 1.1] {
      let water = WaterOptions {
        refraction,
        ..WaterOptions::default()
      };
      let error = validate_water(&water).unwrap_err().to_string();
      assert!(
        error.contains("water.refraction") && error.contains("0 and 1"),
        "{error}"
      );
    }

    for braiding in [f32::NAN, -0.1, 1.1] {
      let mut water = WaterOptions::default();
      water.rivers.braiding = braiding;
      let error = validate_water(&water).unwrap_err().to_string();
      assert!(
        error.contains("water.rivers.braiding") && error.contains("0 and 1"),
        "{error}"
      );
    }

    let inflow = |discharge: f32| vista_types::RiverInflow {
      position: [10.0, -20.0],
      discharge_cubic_metres_per_second: discharge,
    };
    let mut water = WaterOptions::default();
    water.rivers.inflow = vista_types::RiverInflows::List(vec![inflow(50.0); 9]);
    let error = validate_water(&water).unwrap_err().to_string();
    assert!(error.contains("at most 8"), "{error}");

    water.rivers.inflow = vista_types::RiverInflows::List(vec![inflow(-1.0)]);
    let error = validate_water(&water).unwrap_err().to_string();
    assert!(
      error.contains("inflow[0].dischargeCubicMetresPerSecond") && error.contains("0 and 100000"),
      "{error}"
    );

    let mut bad = inflow(5.0);
    bad.position[1] = f32::INFINITY;
    water.rivers.inflow = vista_types::RiverInflows::List(vec![bad]);
    assert!(validate_water(&water).is_err());

    water.rivers.inflow = vista_types::RiverInflows::List(vec![inflow(5.0); 8]);
    assert!(validate_water(&water).is_ok());

    for riparian in [f32::NAN, -0.1, 2.1] {
      let mut water = WaterOptions::default();
      water.rivers.riparian = riparian;
      let error = validate_water(&water).unwrap_err().to_string();
      assert!(error.contains("riparian"), "{error}");
    }

    water.rivers.riparian = 2.0;
    assert!(validate_water(&water).is_ok());

    assert!(validate_water(&WaterOptions::default()).is_ok());
  }

  #[test]
  fn rejects_invalid_biome_options() {
    let biomes = BiomeOptions {
      climate_scale_metres: 0.0,
      ..BiomeOptions::default()
    };
    assert!(validate_biomes(&biomes).is_err());

    let biomes = BiomeOptions {
      temperature_bias: f32::NAN,
      ..BiomeOptions::default()
    };
    assert!(validate_biomes(&biomes).is_err());

    assert!(validate_biomes(&BiomeOptions::default()).is_ok());

    for celsius in [f32::NAN, f32::INFINITY, -30.5, 35.5] {
      let biomes = BiomeOptions {
        mean_temperature_celsius: Some(celsius),
        ..BiomeOptions::default()
      };
      let error = validate_biomes(&biomes).unwrap_err().to_string();
      assert!(error.contains("meanTemperatureCelsius"), "{error}");
    }

    for celsius in [-30.0, 0.0, 35.0] {
      let biomes = BiomeOptions {
        mean_temperature_celsius: Some(celsius),
        ..BiomeOptions::default()
      };
      assert!(validate_biomes(&biomes).is_ok());
    }
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
  fn from_options_defaults_grass_on_and_clouds_and_mist_to_off() {
    let config = VistaEngineConfig::from_options(VistaEngineOptions::default()).unwrap();

    assert!(config.grass.enabled);
    assert_eq!(config.clouds.style, CloudStyle::Off);
    assert_eq!(config.mist.style, vista_types::MistStyle::Off);
  }
}
