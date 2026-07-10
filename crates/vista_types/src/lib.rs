//! Shared serialisable types for VistaWASM.
//!
//! This crate is intentionally small. It contains public option, metadata,
//! event, and error shapes without depending on WebGPU or browser bindings.

use serde::{Deserialize, Serialize};

/// A three component vector in terrain metre space.
pub type Vec3 = [f32; 3];

/// A three component RGB value.
pub type Rgb = [f32; 3];

/// Stable VistaWASM error codes exposed to JavaScript.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum VistaErrorCode {
  /// WebGPU is not available in the current browser.
  WebGpuUnavailable,
  /// The browser refused to create a WebGPU device.
  WebGpuDeviceRequestFailed,
  /// The active WebGPU device was lost.
  WebGpuDeviceLost,
  /// The supplied canvas cannot be used by VistaWASM.
  CanvasInvalid,
  /// Public options failed validation.
  OptionsInvalid,
  /// Terrain generation failed.
  TerrainGenerationFailed,
  /// DEM data could not be fetched by the host wrapper.
  DemFetchFailed,
  /// DEM bytes use a format VistaWASM does not support yet.
  DemFormatUnsupported,
  /// DEM metadata was missing or contradictory.
  DemMetadataMissing,
  /// The request exceeds the active GPU limits.
  GpuLimitExceeded,
  /// The engine was used after disposal.
  EngineDisposed,
  /// An unexpected internal fault occurred.
  InternalError,
}

impl VistaErrorCode {
  /// Return the JavaScript-facing stable code.
  pub const fn as_str(self) -> &'static str {
    match self {
      Self::WebGpuUnavailable => "WEBGPU_UNAVAILABLE",
      Self::WebGpuDeviceRequestFailed => "WEBGPU_DEVICE_REQUEST_FAILED",
      Self::WebGpuDeviceLost => "WEBGPU_DEVICE_LOST",
      Self::CanvasInvalid => "CANVAS_INVALID",
      Self::OptionsInvalid => "OPTIONS_INVALID",
      Self::TerrainGenerationFailed => "TERRAIN_GENERATION_FAILED",
      Self::DemFetchFailed => "DEM_FETCH_FAILED",
      Self::DemFormatUnsupported => "DEM_FORMAT_UNSUPPORTED",
      Self::DemMetadataMissing => "DEM_METADATA_MISSING",
      Self::GpuLimitExceeded => "GPU_LIMIT_EXCEEDED",
      Self::EngineDisposed => "ENGINE_DISPOSED",
      Self::InternalError => "INTERNAL_ERROR",
    }
  }
}

/// Runtime state for one engine instance.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EngineState {
  /// The engine object exists but has not completed GPU setup.
  Created,
  /// The engine is requesting a WebGPU adapter and device.
  InitialisingGpu,
  /// The engine is ready to accept terrain and render requests.
  Ready,
  /// A terrain mutation job is running.
  LoadingTerrain,
  /// The internal frame loop is active.
  Rendering,
  /// The WebGPU device was lost.
  DeviceLost,
  /// The engine has released its resources.
  Disposed,
}

/// Metadata describing real-world map information when it is known.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeospatialMetadata {
  /// Horizontal sample size on the x axis in metres.
  pub metres_per_sample_x: Option<f32>,
  /// Horizontal sample size on the y axis in metres.
  pub metres_per_sample_y: Option<f32>,
  /// Source projection name or authority string when decoded.
  pub projection_name: Option<String>,
  /// The source tie point in `[raster_x, raster_y, raster_z, model_x, model_y, model_z]` order.
  pub model_tiepoint: Option<[f64; 6]>,
  /// The source pixel scale in `[x, y, z]` order.
  pub model_pixel_scale: Option<[f64; 3]>,
}

/// Metadata describing terrain scale and source.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerrainMetadata {
  /// Width of the heightmap in samples.
  pub width: u32,
  /// Height of the heightmap in samples.
  pub height: u32,
  /// Horizontal distance between samples in metres.
  pub metres_per_sample: f32,
  /// Height multiplier applied after loading or generation.
  pub vertical_scale: f32,
  /// Sea level in terrain metres.
  pub sea_level_metres: f32,
  /// Minimum valid height in metres.
  pub min_height_metres: f32,
  /// Maximum valid height in metres.
  pub max_height_metres: f32,
  /// Mean valid height in metres.
  pub mean_height_metres: f32,
  /// Source kind such as `fractal`, `raw-heightmap`, or `geotiff`.
  pub source: String,
  /// Generator version or decoder version used for this terrain.
  pub generator_version: String,
  /// Optional real-world metadata.
  pub geospatial: Option<GeospatialMetadata>,
  /// Warnings that should be surfaced to host applications.
  pub warnings: Vec<String>,
}

impl Default for TerrainMetadata {
  fn default() -> Self {
    Self {
      width: 0,
      height: 0,
      metres_per_sample: 1.0,
      vertical_scale: 1.0,
      sea_level_metres: 0.0,
      min_height_metres: 0.0,
      max_height_metres: 0.0,
      mean_height_metres: 0.0,
      source: "unknown".to_string(),
      generator_version: "0.1.0".to_string(),
      geospatial: None,
      warnings: Vec::new(),
    }
  }
}

/// Handle returned to JavaScript after a terrain load or generation job.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerrainHandle {
  /// Stable handle ID for the current engine instance.
  pub id: u32,
  /// Metadata for the active terrain.
  pub metadata: TerrainMetadata,
}

/// Public fractal terrain generation options.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FractalTerrainOptions {
  /// Deterministic seed used by terrain, erosion, flora, and sky variation.
  pub seed: u64,
  /// Width and height of the square terrain in samples.
  pub size: u32,
  /// Horizontal distance between samples in metres.
  pub horizontal_scale_metres: f32,
  /// Height multiplier for generated terrain.
  pub vertical_scale: f32,
  /// Optional base height added after generation.
  pub base_height_metres: Option<f32>,
  /// Sea level in metres.
  pub sea_level_metres: Option<f32>,
  /// Noise controls.
  pub noise: NoiseOptions,
  /// Optional large-scale terrain shaping.
  pub shape: Option<TerrainShapeOptions>,
  /// Optional erosion controls.
  pub erosion: Option<ErosionOptions>,
}

impl Default for FractalTerrainOptions {
  fn default() -> Self {
    Self {
      seed: 1,
      size: 512,
      horizontal_scale_metres: 10.0,
      vertical_scale: 1.0,
      base_height_metres: Some(0.0),
      sea_level_metres: Some(0.0),
      noise: NoiseOptions::default(),
      shape: None,
      erosion: None,
    }
  }
}

/// Supported procedural noise families.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NoiseKind {
  /// Smooth multi-octave noise.
  Simplex,
  /// Inverted absolute noise for ridges.
  Ridged,
  /// A blend of smooth and ridged noise.
  Hybrid,
  /// Island-like terrain with lower edges.
  Island,
  /// Terrain carved by broad canyon masks.
  Canyon,
  /// Terrain with deterministic crater depressions.
  Cratered,
  /// A CPU-era inspired preset.
  Classic,
}

/// Fractal noise controls.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoiseOptions {
  /// Noise family.
  pub kind: NoiseKind,
  /// Number of octaves to combine.
  pub octaves: u32,
  /// Amplitude multiplier between octaves.
  pub gain: f32,
  /// Frequency multiplier between octaves.
  pub lacunarity: f32,
  /// Domain warp amount.
  pub warp: Option<f32>,
}

impl Default for NoiseOptions {
  fn default() -> Self {
    Self {
      kind: NoiseKind::Ridged,
      octaves: 7,
      gain: 0.5,
      lacunarity: 2.0,
      warp: Some(0.0),
    }
  }
}

/// Large-scale terrain shaping controls.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerrainShapeOptions {
  /// Amount of island falloff to apply.
  pub island: Option<f32>,
  /// Amount of terrace quantisation to apply.
  pub terrace: Option<f32>,
  /// Amount of basin lowering to apply.
  pub basin: Option<f32>,
  /// Amount of canyon carving to apply.
  pub canyon: Option<f32>,
  /// Amount of crater shaping to apply.
  pub crater: Option<f32>,
}

/// Erosion quality preset.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErosionQuality {
  /// A quick pass suitable for interaction.
  Preview,
  /// A balanced pass for normal use.
  Balanced,
  /// A slower pass for visible gullies and scree.
  High,
  /// A maximum budget pass for still renders.
  Offline,
}

/// Erosion controls.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErosionOptions {
  /// Number of hydraulic erosion iterations.
  pub hydraulic_iterations: Option<u32>,
  /// Number of thermal erosion iterations.
  pub thermal_iterations: Option<u32>,
  /// Rain amount applied per hydraulic step.
  pub rain_amount: Option<f32>,
  /// Evaporation amount applied per hydraulic step.
  pub evaporation: Option<f32>,
  /// Sediment capacity used by the hydraulic model.
  pub sediment_capacity: Option<f32>,
  /// Talus angle used by thermal erosion.
  pub talus_angle_degrees: Option<f32>,
  /// Quality preset used to clamp work.
  pub quality: Option<ErosionQuality>,
}

impl Default for ErosionOptions {
  fn default() -> Self {
    Self {
      hydraulic_iterations: Some(0),
      thermal_iterations: Some(0),
      rain_amount: Some(0.02),
      evaporation: Some(0.5),
      sediment_capacity: Some(0.04),
      talus_angle_degrees: Some(35.0),
      quality: Some(ErosionQuality::Preview),
    }
  }
}

/// Supported raw heightmap sample formats.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RawSampleFormat {
  /// Unsigned 16-bit integer samples.
  Uint16,
  /// Signed 16-bit integer samples.
  Int16,
  /// IEEE 32-bit float samples.
  Float32,
}

/// Byte order for raw heightmap samples.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ByteOrder {
  /// Little-endian byte order.
  LittleEndian,
  /// Big-endian byte order.
  BigEndian,
}

impl Default for ByteOrder {
  fn default() -> Self {
    Self::LittleEndian
  }
}

/// Options for loading raw heightmap bytes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawHeightmapOptions {
  /// Width in samples.
  pub width: u32,
  /// Height in samples.
  pub height: u32,
  /// Sample format.
  pub sample_format: RawSampleFormat,
  /// Byte order of numeric samples.
  pub byte_order: Option<ByteOrder>,
  /// Horizontal metres per sample.
  pub metres_per_sample: f32,
  /// Height scale in metres.
  pub height_scale_metres: f32,
  /// Optional no-data marker.
  pub no_data_value: Option<f32>,
  /// Optional sea level.
  pub sea_level_metres: Option<f32>,
}

/// Options for DEM loading.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DemLoadOptions {
  /// Height multiplier applied after decode.
  pub vertical_scale: Option<f32>,
  /// Whether normals should be regenerated after load.
  pub generate_normals: Option<bool>,
  /// Whether material masks should be regenerated after load.
  pub generate_material_masks: Option<bool>,
}

/// Metadata returned after DEM decode.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DemMetadata {
  /// Width in samples.
  pub width: u32,
  /// Height in samples.
  pub height: u32,
  /// Sample format name.
  pub sample_format: String,
  /// Bits per sample.
  pub bits_per_sample: u16,
  /// Optional x axis metres per sample.
  pub metres_per_sample_x: Option<f32>,
  /// Optional y axis metres per sample.
  pub metres_per_sample_y: Option<f32>,
  /// Minimum valid height in metres.
  pub min_height_metres: f32,
  /// Maximum valid height in metres.
  pub max_height_metres: f32,
  /// Optional no-data marker.
  pub no_data_value: Option<f32>,
  /// Optional projection name.
  pub projection_name: Option<String>,
  /// Decode warnings.
  pub warnings: Vec<String>,
}

/// Camera options for the projector model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraOptions {
  /// Camera position in terrain metres.
  pub position: Vec3,
  /// Target point in terrain metres.
  pub target: Vec3,
  /// Optional roll in degrees.
  pub roll_degrees: Option<f32>,
  /// Field of view in degrees.
  pub field_of_view_degrees: f32,
  /// Near plane in metres.
  pub near_metres: Option<f32>,
  /// Far plane in metres.
  pub far_metres: Option<f32>,
  /// Optional collision-safe minimum height above terrain.
  pub minimum_height_above_terrain_metres: Option<f32>,
  /// Whether the camera may be placed below the terrain.
  pub allow_underground: Option<bool>,
}

impl Default for CameraOptions {
  fn default() -> Self {
    Self {
      position: [0.0, 120.0, 300.0],
      target: [0.0, 0.0, 0.0],
      roll_degrees: Some(0.0),
      field_of_view_degrees: 55.0,
      near_metres: Some(0.5),
      far_metres: Some(120_000.0),
      minimum_height_above_terrain_metres: Some(2.0),
      allow_underground: Some(false),
    }
  }
}

/// Sun lighting controls.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SunOptions {
  /// Sun azimuth in degrees.
  pub azimuth_degrees: f32,
  /// Sun elevation in degrees.
  pub elevation_degrees: f32,
  /// Light intensity multiplier.
  pub intensity: f32,
}

impl Default for SunOptions {
  fn default() -> Self {
    Self {
      azimuth_degrees: 132.0,
      elevation_degrees: 18.0,
      intensity: 1.2,
    }
  }
}

/// Atmosphere and haze controls.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtmosphereOptions {
  /// Rayleigh scattering strength.
  pub rayleigh_strength: f32,
  /// Mie scattering strength.
  pub mie_strength: f32,
  /// Haze distance in metres.
  pub haze_distance_metres: f32,
  /// Exposure multiplier.
  pub exposure: f32,
  /// Public API tint field retained as `skyTint`.
  pub sky_tint: Rgb,
}

impl Default for AtmosphereOptions {
  fn default() -> Self {
    Self {
      rayleigh_strength: 1.0,
      mie_strength: 0.45,
      haze_distance_metres: 60_000.0,
      exposure: 1.1,
      sky_tint: [1.0, 1.0, 1.0],
    }
  }
}

/// Water rendering controls.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WaterOptions {
  /// Whether water is enabled.
  pub enabled: bool,
  /// Sea level in metres.
  pub sea_level_metres: f32,
  /// Procedural wave scale.
  pub wave_scale: f32,
  /// Reflection strength.
  pub reflectivity: f32,
  /// Shoreline blend distance in metres.
  pub shoreline_softness_metres: f32,
}

impl Default for WaterOptions {
  fn default() -> Self {
    Self {
      enabled: true,
      sea_level_metres: 0.0,
      wave_scale: 0.8,
      reflectivity: 0.35,
      shoreline_softness_metres: 6.0,
    }
  }
}

/// Tree rendering fidelity.
///
/// `Billboard` is the original, cheapest rendering (a single camera-facing
/// quad) and stays the default so existing scenes are unaffected. `CrossQuad`
/// renders two static, world-oriented quads per tree (not camera-facing),
/// giving real parallax/volume from any angle. `Mesh` is accepted by the
/// public API but currently falls back to `CrossQuad` rendering — a true
/// instanced 3D tree mesh is tracked as future work in
/// `docs/environment-upgrade-plan.md` ("Trees (Mesh tier)", deliberately the
/// last phase of that plan) rather than shipped partially.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TreeQuality {
  /// A single camera-facing billboard quad (default, cheapest).
  Billboard,
  /// Two static, crossed quads per tree for parallax and volume.
  CrossQuad,
  /// Reserved for a future full instanced mesh tier; renders as `CrossQuad`.
  Mesh,
}

impl Default for TreeQuality {
  fn default() -> Self {
    Self::Billboard
  }
}

fn default_species_variation() -> f32 {
  0.0
}

/// Flora placement controls.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FloraOptions {
  /// Whether flora is enabled.
  pub enabled: bool,
  /// Placement density from 0 to 1.
  pub density: f32,
  /// Tree line altitude in metres.
  pub tree_line_metres: f32,
  /// Seed offset for deterministic placement.
  pub seed_offset: u64,
  /// Maximum instance count requested by the host.
  pub max_instances: u32,
  /// Tree rendering fidelity. Defaults to `Billboard`, today's rendering,
  /// so existing callers see no change unless they opt in.
  #[serde(default)]
  pub tree_quality: TreeQuality,
  /// Canopy silhouette and colour variety strength, from 0 to 1. Defaults
  /// to `0.0` (today's uniform look).
  #[serde(default = "default_species_variation")]
  pub species_variation: f32,
  /// Canopy wind sway strength, from 0 to 1. Defaults to `0.0` (static).
  #[serde(default)]
  pub wind_strength: f32,
}

impl Default for FloraOptions {
  fn default() -> Self {
    Self {
      enabled: true,
      density: 0.35,
      tree_line_metres: 1_800.0,
      seed_offset: 3_001,
      max_instances: 500_000,
      tree_quality: TreeQuality::default(),
      species_variation: default_species_variation(),
      wind_strength: 0.0,
    }
  }
}

/// Grass rendering fidelity.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GrassStyle {
  /// Crossed billboard blade clumps (default once enabled, cheap).
  BillboardBlades,
  /// Denser blade instancing with a shorter view distance (hyper-realistic).
  DenseBlades,
}

impl Default for GrassStyle {
  fn default() -> Self {
    Self::BillboardBlades
  }
}

/// Grass ground-cover controls.
///
/// Grass is a brand-new visual element with no prior equivalent, so unlike
/// the other environmental options it defaults fully `enabled: false`
/// rather than merely defaulting to a cheap tier.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrassOptions {
  /// Whether grass is enabled.
  pub enabled: bool,
  /// Rendering style/tier.
  pub style: GrassStyle,
  /// Placement density from 0 to 1.
  pub density: f32,
  /// Distance from the camera, in metres, at which grass fully fades out.
  pub view_distance_metres: f32,
  /// Seed offset for deterministic placement.
  pub seed_offset: u64,
  /// Maximum instance count requested by the host.
  pub max_instances: u32,
}

impl Default for GrassOptions {
  fn default() -> Self {
    Self {
      enabled: false,
      style: GrassStyle::default(),
      density: 0.5,
      view_distance_metres: 220.0,
      seed_offset: 7_331,
      max_instances: 200_000,
    }
  }
}

/// Cloud rendering fidelity.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CloudStyle {
  /// No cloud layer (default).
  Off,
  /// A 2D noise layer blended into the sky dome shader.
  Painted,
  /// A raymarched volumetric layer with sun-facing shading (hyper-realistic).
  Volumetric,
}

impl Default for CloudStyle {
  fn default() -> Self {
    Self::Off
  }
}

/// Cloud layer controls.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudsOptions {
  /// Rendering style/tier. Defaults to `Off`.
  pub style: CloudStyle,
  /// Cloud coverage from 0 (clear) to 1 (overcast).
  pub coverage: f32,
  /// Drift speed. Larger values drift faster.
  pub speed: f32,
  /// Cloud layer altitude in metres.
  pub height_metres: f32,
  /// Base cloud tint, blended with sun/sky colour.
  pub colour: Rgb,
  /// Seed offset for deterministic cloud noise.
  pub seed_offset: u64,
  /// Raymarch step count for the `Volumetric` style only. Always clamped
  /// to a safe range server-side regardless of the requested value (see
  /// `VistaEngineConfig::from_options`), since this value bounds a shader
  /// loop and untrusted callers must not be able to request an unbounded
  /// one.
  pub raymarch_steps: Option<u32>,
}

impl Default for CloudsOptions {
  fn default() -> Self {
    Self {
      style: CloudStyle::default(),
      coverage: 0.45,
      speed: 1.0,
      height_metres: 4_000.0,
      colour: [1.0, 1.0, 1.0],
      seed_offset: 9_007,
      raymarch_steps: Some(24),
    }
  }
}

/// Mist/ground-fog rendering fidelity.
///
/// This is distinct from `AtmosphereOptions.hazeDistanceMetres`, which is a
/// uniform, distance-only blend to sky colour. Mist is a height-based
/// ground fog that pools in valleys and near water.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MistStyle {
  /// No ground mist (default).
  Off,
  /// A static height-falloff blend (cheap).
  Flat,
  /// Animated density noise drifting across the terrain (hyper-realistic).
  Volumetric,
}

impl Default for MistStyle {
  fn default() -> Self {
    Self::Off
  }
}

/// Mist/ground-fog controls.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MistOptions {
  /// Rendering style/tier. Defaults to `Off`.
  pub style: MistStyle,
  /// Mist density from 0 to 1.
  pub density: f32,
  /// Altitude, in metres, mist is thickest at.
  pub base_height_metres: f32,
  /// How quickly mist thins out with altitude, in metres.
  pub height_falloff_metres: f32,
  /// Mist tint.
  pub colour: Rgb,
  /// Adds extra mist near `WaterOptions.seaLevelMetres`, regardless of
  /// `baseHeightMetres`, for a "mist rising off the water" look.
  pub rise_above_water: bool,
  /// Seed offset for deterministic mist noise (`Volumetric` style only).
  pub seed_offset: u64,
}

impl Default for MistOptions {
  fn default() -> Self {
    Self {
      style: MistStyle::default(),
      density: 0.5,
      base_height_metres: 40.0,
      height_falloff_metres: 120.0,
      colour: [0.82, 0.85, 0.88],
      rise_above_water: true,
      seed_offset: 5_303,
    }
  }
}

/// Render quality preset.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RenderQualityPreset {
  /// Fast preview rendering.
  Preview,
  /// Balanced quality and speed.
  Balanced,
  /// High quality rendering.
  High,
  /// Offline still-render quality.
  Offline,
}

impl Default for RenderQualityPreset {
  fn default() -> Self {
    Self::Balanced
  }
}

/// Render quality options.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderQualityOptions {
  /// Quality preset.
  pub preset: RenderQualityPreset,
  /// Optional maximum clipmap levels.
  pub max_clipmap_levels: Option<u32>,
  /// Optional flora density multiplier.
  pub flora_density_scale: Option<f32>,
}

impl Default for RenderQualityOptions {
  fn default() -> Self {
    Self {
      preset: RenderQualityPreset::Balanced,
      max_clipmap_levels: Some(7),
      flora_density_scale: Some(1.0),
    }
  }
}

/// Debug overlay mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DebugView {
  /// Normal rendering.
  None,
  /// Height overlay.
  Height,
  /// Slope overlay.
  Slope,
  /// Normal vector overlay.
  Normals,
  /// Clipmap level overlay.
  Lod,
  /// Erosion flow overlay.
  Flow,
  /// Material mask overlay.
  Materials,
  /// DEM no-data overlay.
  NoData,
}

impl Default for DebugView {
  fn default() -> Self {
    Self::None
  }
}

/// Render statistics exposed to JavaScript.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderStats {
  /// Monotonic frame index.
  pub frame_index: u64,
  /// CPU frame time in milliseconds.
  pub frame_time_ms: f32,
  /// Optional GPU frame time in milliseconds.
  pub gpu_frame_time_ms: Option<f32>,
  /// Number of terrain triangles submitted.
  pub terrain_triangles: u32,
  /// Number of flora instances submitted.
  pub flora_instances: u32,
  /// Number of grass instances submitted.
  pub grass_instances: u32,
  /// Number of active clipmap levels.
  pub clipmap_levels: u32,
  /// Optional active GPU memory estimate.
  pub active_gpu_memory_bytes: Option<u64>,
}

impl Default for RenderStats {
  fn default() -> Self {
    Self {
      frame_index: 0,
      frame_time_ms: 0.0,
      gpu_frame_time_ms: None,
      terrain_triangles: 0,
      flora_instances: 0,
      grass_instances: 0,
      clipmap_levels: 0,
      active_gpu_memory_bytes: None,
    }
  }
}

/// Engine creation options.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VistaEngineOptions {
  /// Optional render dimensions.
  pub render: Option<RenderSizeOptions>,
  /// Optional initial camera.
  pub camera: Option<CameraOptions>,
  /// Optional initial sun controls.
  pub sun: Option<SunOptions>,
  /// Optional initial atmosphere controls.
  pub atmosphere: Option<AtmosphereOptions>,
  /// Optional initial water controls.
  pub water: Option<WaterOptions>,
  /// Optional initial flora controls.
  pub flora: Option<FloraOptions>,
  /// Optional initial grass controls.
  pub grass: Option<GrassOptions>,
  /// Optional initial cloud controls.
  pub clouds: Option<CloudsOptions>,
  /// Optional initial mist controls.
  pub mist: Option<MistOptions>,
  /// Optional quality controls.
  pub quality: Option<RenderQualityOptions>,
}

impl Default for VistaEngineOptions {
  fn default() -> Self {
    Self {
      render: Some(RenderSizeOptions::default()),
      camera: Some(CameraOptions::default()),
      sun: Some(SunOptions::default()),
      atmosphere: Some(AtmosphereOptions::default()),
      water: Some(WaterOptions::default()),
      flora: Some(FloraOptions::default()),
      grass: Some(GrassOptions::default()),
      clouds: Some(CloudsOptions::default()),
      mist: Some(MistOptions::default()),
      quality: Some(RenderQualityOptions::default()),
    }
  }
}

/// Render dimensions supplied by the host.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderSizeOptions {
  /// Canvas width in CSS pixels.
  pub width: u32,
  /// Canvas height in CSS pixels.
  pub height: u32,
  /// Device pixel ratio.
  pub device_pixel_ratio: Option<f32>,
}

impl Default for RenderSizeOptions {
  fn default() -> Self {
    Self {
      width: 1,
      height: 1,
      device_pixel_ratio: Some(1.0),
    }
  }
}

/// Options for exporting height data.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportHeightmapOptions {
  /// Export format. The current implementation supports `float32-le`.
  pub format: Option<String>,
}

/// Options for exporting a canvas snapshot.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotOptions {
  /// MIME type requested from the browser canvas.
  pub mime_type: Option<String>,
  /// Optional quality for lossy formats.
  pub quality: Option<f32>,
}
