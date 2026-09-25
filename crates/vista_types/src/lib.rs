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
  /// Character of the land. Defaults to [`LandformKind::Continental`].
  #[serde(default)]
  pub landform: LandformKind,
  /// What happens at the map edge. Defaults to [`TerrainEdges::Coast`].
  #[serde(default)]
  pub edges: TerrainEdges,
}

/// What happens where a generated map meets its square edge.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TerrainEdges {
  /// The land stays inside the map, ringed by sea along a natural coast.
  #[default]
  Coast,
  /// The land runs to the map edge, for tiling several maps.
  Open,
}

/// The character of a generated map: how much of it is land, how the
/// land is raised into ranges, and how water and ice carve it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LandformKind {
  /// Mixed plains, hills and one or two eroded mountain ranges.
  #[default]
  Continental,
  /// High, heavily eroded ranges with deep valleys and glacial troughs.
  Alpine,
  /// Gentle downs and broad vales with no mountain ranges.
  RollingHills,
  /// Many islands of varied size in a shallow sea.
  Archipelago,
  /// Terraced plateaus, buttes and canyons under a dry climate.
  MesaDesert,
  /// Steep coastal ranges cut by flooded, U-shaped glacial valleys.
  Fjords,
  /// A central cone with a caldera, radial gullies and a reef shelf.
  VolcanicIsland,
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
      landform: LandformKind::Continental,
      edges: TerrainEdges::Coast,
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

/// Erosion controls. Unset fields take the landform's defaults, and unset
/// iteration counts follow the quality preset.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
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

fn default_true() -> bool {
  true
}

/// Gerstner wave simulation controls for open water.
///
/// Waves are an analytic sum of directional Gerstner waves spread around
/// `directionDegrees`, so they are deterministic, cost nothing on the CPU,
/// and shoal (shrink and steepen into breaking foam) as the water gets
/// shallower near the shore.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WaveOptions {
  /// Whether vertex-displaced waves are simulated. When `false` the surface
  /// stays flat and only small procedural ripples are shaded.
  pub enabled: bool,
  /// Height of the dominant swell from trough to crest, in metres.
  pub amplitude_metres: f32,
  /// Wavelength of the dominant swell, in metres.
  pub wavelength_metres: f32,
  /// Direction the swell travels towards, in degrees (0 = +Z, 90 = +X).
  pub direction_degrees: f32,
  /// Crest sharpness from 0 (rolling sine waves) to 1 (sharp, choppy
  /// crests).
  pub steepness: f32,
  /// Animation speed multiplier (1 = physically based deep-water speed).
  pub speed: f32,
  /// Spread of the secondary waves around the main direction, from 0
  /// (parallel swell) to 1 (confused, storm-like sea).
  pub directional_spread: f32,
}

impl Default for WaveOptions {
  fn default() -> Self {
    Self {
      enabled: true,
      amplitude_metres: 0.9,
      wavelength_metres: 38.0,
      direction_degrees: 35.0,
      steepness: 0.55,
      speed: 1.0,
      directional_spread: 0.55,
    }
  }
}

/// River and current controls.
///
/// Rivers are extracted from the terrain's own drainage network (flow
/// accumulation over the heightmap), so they always run downhill along
/// valleys towards the sea or a basin. Each river vertex carries its flow
/// direction and speed, which drives the animated current.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RiverOptions {
  /// Whether rivers are generated.
  pub enabled: bool,
  /// Minimum upstream catchment area, in square kilometres, before a
  /// channel is drawn as a river. Smaller values draw more, thinner
  /// streams.
  pub min_catchment_km2: f32,
  /// Multiplier applied to the automatic river width.
  pub width_scale: f32,
  /// Current speed multiplier for river flow animation.
  pub current_speed: f32,
  /// How strongly snow fields and glaciers feed streams: 0 turns
  /// snowmelt off, 1 is a typical melt, 2 doubles it. Streams also start
  /// at glacier snouts and at the lower edge of snowy peaks.
  pub snowmelt: f32,
  /// Whether small springs rise at the foot of steep slopes.
  pub springs: bool,
  /// Lowland meander strength, from 0 (straight) to 1.
  pub meanders: f32,
  /// Whether rivers crossing cliffs become waterfalls.
  pub waterfalls: bool,
  /// Water arriving from beyond the map. [`InflowMode::Auto`] (the
  /// default) adds one inflow where a valley meets an open map edge, sized
  /// from a basin ten times the map's land area; [`InflowMode::None`]
  /// adds nothing; or up to 8 explicit inflows.
  pub inflow: RiverInflows,
  /// Bankside greening and trees, from 0 (off) to 2: moister ground,
  /// greener biomes and more trees within 25 to 400 m of rivers (by
  /// discharge) and 40 m of lakes.
  pub riparian: f32,
}

/// Water arriving from beyond the map (`RiverOptions::inflow`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum RiverInflows {
  /// `"auto"` or `"none"`.
  Mode(InflowMode),
  /// Up to 8 explicit inflows.
  List(Vec<RiverInflow>),
}

impl Default for RiverInflows {
  fn default() -> Self {
    Self::Mode(InflowMode::Auto)
  }
}

// A string or a list, read directly: the untagged derive would buffer
// every value first, which costs far more code.
impl<'de> Deserialize<'de> for RiverInflows {
  fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
    struct Inflows;

    impl<'de> serde::de::Visitor<'de> for Inflows {
      type Value = RiverInflows;

      fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("\"auto\", \"none\" or a list of inflows")
      }

      fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
        match value {
          "auto" => Ok(RiverInflows::Mode(InflowMode::Auto)),
          "none" => Ok(RiverInflows::Mode(InflowMode::None)),
          _ => Err(E::invalid_value(serde::de::Unexpected::Str(value), &self)),
        }
      }

      fn visit_seq<A: serde::de::SeqAccess<'de>>(
        self,
        mut seq: A,
      ) -> Result<Self::Value, A::Error> {
        let mut list = Vec::new();

        while let Some(inflow) = seq.next_element()? {
          list.push(inflow);
        }

        Ok(RiverInflows::List(list))
      }
    }

    deserializer.deserialize_any(Inflows)
  }
}

/// How inflows are placed when none are listed.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InflowMode {
  /// One inflow where a valley meets an open map edge; none on maps ringed
  /// by sea.
  #[default]
  Auto,
  /// No inflow.
  None,
}

/// One explicit inflow.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RiverInflow {
  /// World x and z in metres. Snaps to the nearest land sample.
  pub position: [f32; 2],
  /// Mean discharge, 0 to 100,000 cubic metres per second.
  pub discharge_cubic_metres_per_second: f32,
}

/// An inflow in use, as `getInflows` reports it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WaterInflow {
  /// Where the water enters, on the land sample it snapped to, in world
  /// metres.
  pub position: Vec3,
  /// Mean discharge in cubic metres per second.
  pub discharge_cubic_metres_per_second: f32,
}

impl Default for RiverOptions {
  fn default() -> Self {
    Self {
      enabled: true,
      min_catchment_km2: 0.15,
      width_scale: 1.0,
      current_speed: 1.0,
      snowmelt: 1.0,
      springs: true,
      meanders: 0.6,
      waterfalls: true,
      inflow: RiverInflows::default(),
      riparian: 1.0,
    }
  }
}

/// The loudest water sound of one kind near a listener.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WaterSound {
  /// Distance from the listener to the source, in metres.
  pub distance_metres: f32,
  /// Loudness from 0 to 1: the source's strength over its distance
  /// squared.
  pub loudness: f32,
  /// Where the sound comes from, in world metres.
  pub position: Vec3,
}

/// The water sounds near a listener. Each is `None` when there is no
/// such source within its search radius.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WaterSounds {
  /// Running water, within 400 m.
  pub river: Option<WaterSound>,
  /// A waterfall, within 1500 m.
  pub waterfall: Option<WaterSound>,
  /// Water lapping on a lake shore, within 400 m.
  pub lake_shore: Option<WaterSound>,
  /// Waves breaking on the coast, within 600 m.
  pub surf: Option<WaterSound>,
}

/// A waterfall.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Waterfall {
  /// Where the water lands, at the river level at the foot of the fall,
  /// in world metres.
  pub position: Vec3,
  /// Height of the drop, in metres.
  pub height_metres: f32,
  /// Width of the falling water, in metres.
  pub width_metres: f32,
  /// Mean discharge, in cubic metres per second.
  pub discharge_cubic_metres_per_second: f32,
}

/// Painted water for `setWaterMask`: one byte per sample, row-major,
/// north row first. 0 is no water, 1 to 127 a river brush of that
/// strength (1 is 1 m wide, 127 is 60 m), and 128 to 255 a lake or pond.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WaterMask {
  /// Samples per row, 2 to 8192.
  pub width: u32,
  /// Rows, 2 to 8192.
  pub height: u32,
  /// `width x height` bytes.
  pub data: Vec<u8>,
}

fn default_shallow_colour() -> Rgb {
  [0.10, 0.52, 0.50]
}

fn default_deep_colour() -> Rgb {
  [0.015, 0.09, 0.16]
}

fn default_clarity_metres() -> f32 {
  6.0
}

fn default_foam() -> f32 {
  0.7
}

fn default_current_direction() -> f32 {
  60.0
}

fn default_current_speed() -> f32 {
  0.35
}

/// Water rendering controls.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WaterOptions {
  /// Whether water is enabled.
  pub enabled: bool,
  /// Sea level in metres.
  pub sea_level_metres: f32,
  /// Small-scale ripple strength multiplier. Large swell is controlled by
  /// `waves`.
  pub wave_scale: f32,
  /// Reflection strength.
  pub reflectivity: f32,
  /// Shoreline blend distance in metres.
  pub shoreline_softness_metres: f32,
  /// Gerstner wave simulation. Defaults to a gentle swell.
  #[serde(default)]
  pub waves: WaveOptions,
  /// Rivers extracted from the terrain drainage network.
  #[serde(default)]
  pub rivers: RiverOptions,
  /// Direction of the open-water surface current, in degrees.
  #[serde(default = "default_current_direction")]
  pub current_direction_degrees: f32,
  /// Speed of the open-water surface current in metres per second. Moves
  /// ripples and foam; set to 0 for still water.
  #[serde(default = "default_current_speed")]
  pub current_speed: f32,
  /// Colour of shallow, sunlit water.
  #[serde(default = "default_shallow_colour")]
  pub shallow_colour: Rgb,
  /// Colour of deep water.
  #[serde(default = "default_deep_colour")]
  pub deep_colour: Rgb,
  /// Depth in metres at which the sea bed stops being visible.
  #[serde(default = "default_clarity_metres")]
  pub clarity_metres: f32,
  /// Foam strength on crests, shorelines, and rapids, from 0 to 1.
  #[serde(default = "default_foam")]
  pub foam: f32,
  /// What water reflects.
  #[serde(default)]
  pub reflections: WaterReflections,
}

/// What water reflects (`WaterOptions::reflections`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WaterReflections {
  /// The scene on screen (terrain, trees, banks), falling back to the sky
  /// and clouds where a reflected ray leaves the screen or finds nothing.
  #[default]
  Screen,
  /// The sky and clouds only.
  Sky,
}

impl Default for WaterOptions {
  fn default() -> Self {
    Self {
      enabled: true,
      sea_level_metres: 0.0,
      wave_scale: 0.8,
      reflectivity: 0.35,
      shoreline_softness_metres: 6.0,
      waves: WaveOptions::default(),
      rivers: RiverOptions::default(),
      current_direction_degrees: default_current_direction(),
      current_speed: default_current_speed(),
      shallow_colour: default_shallow_colour(),
      deep_colour: default_deep_colour(),
      clarity_metres: default_clarity_metres(),
      foam: default_foam(),
      reflections: WaterReflections::Screen,
    }
  }
}

/// Tree rendering fidelity.
///
/// Every tier draws the same procedurally modelled tree species (palm,
/// jungle, swamp cypress, pine, oak, spruce, acacia, and shrub). `Mesh`
/// (the default) draws full 3D meshes near the camera and switches to
/// pre-rendered impostors of the very same meshes in the distance.
/// `CrossQuad` draws only the impostors as two crossed quads, and
/// `Billboard` draws them as one camera-facing quad — both are cheaper
/// fallbacks for low-end devices.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TreeQuality {
  /// A single camera-facing impostor quad per tree (cheapest).
  Billboard,
  /// Two static, crossed impostor quads per tree.
  CrossQuad,
  /// Full 3D meshes near the camera, impostors in the distance (default).
  Mesh,
}

impl Default for TreeQuality {
  fn default() -> Self {
    Self::Mesh
  }
}

fn default_species_variation() -> f32 {
  0.6
}

fn default_wind_strength() -> f32 {
  0.3
}

fn default_mesh_distance() -> f32 {
  420.0
}

/// A tree species slot. Each slot has a procedural model that
/// `setTreeModel` can replace.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[repr(u8)]
pub enum TreeSpeciesKind {
  /// Broad, spreading deciduous oak.
  Oak = 0,
  /// Tall pine with a high, irregular crown.
  Pine = 1,
  /// Dense, conical spruce.
  Spruce = 2,
  /// Coconut palm.
  Palm = 3,
  /// Tall buttressed rainforest emergent.
  Jungle = 4,
  /// Bald cypress with hanging moss.
  Cypress = 5,
  /// Flat-topped savannah acacia.
  Acacia = 6,
  /// Low leafy shrub.
  Shrub = 7,
}

impl TreeSpeciesKind {
  /// Every species, in slot order.
  pub const ALL: [TreeSpeciesKind; 8] = [
    Self::Oak,
    Self::Pine,
    Self::Spruce,
    Self::Palm,
    Self::Jungle,
    Self::Cypress,
    Self::Acacia,
    Self::Shrub,
  ];

  /// Slot index, 0 to 7.
  pub fn index(self) -> usize {
    self as usize
  }
}

/// One weighted species choice in a [`FloraRule`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeciesWeight {
  /// The species slot.
  pub species: TreeSpeciesKind,
  /// Relative weight. Weights in a rule do not need to sum to 1.
  pub weight: f32,
}

fn default_rule_density() -> f32 {
  1.0
}

/// Replaces the built-in species mix for one biome.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FloraRule {
  /// The biome this rule applies to.
  pub biome: BiomeKind,
  /// Species to plant, by relative weight. An empty list keeps the biome
  /// treeless.
  pub species: Vec<SpeciesWeight>,
  /// Multiplier on the biome's tree density, 0 to 4.
  #[serde(default = "default_rule_density")]
  pub density: f32,
}

/// Flora placement controls.
///
/// Tree species and density follow the biome under each tree, so jungles
/// fill with jungle trees and palms, swamps with cypress, cold forests
/// with pine and spruce, and so on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FloraOptions {
  /// Whether flora is enabled.
  pub enabled: bool,
  /// Placement density from 0 to 1. Multiplies each biome's own density.
  pub density: f32,
  /// Tree line altitude in metres.
  pub tree_line_metres: f32,
  /// Seed offset for deterministic placement.
  pub seed_offset: u64,
  /// Maximum instance count requested by the host.
  pub max_instances: u32,
  /// Tree rendering fidelity. Defaults to `Mesh`.
  #[serde(default)]
  pub tree_quality: TreeQuality,
  /// Per-tree size, shape, and colour variety strength, from 0 to 1.
  #[serde(default = "default_species_variation")]
  pub species_variation: f32,
  /// Wind sway strength, from 0 to 1.
  #[serde(default = "default_wind_strength")]
  pub wind_strength: f32,
  /// Distance in metres at which `Mesh` quality trees cross-fade from full
  /// 3D meshes to impostors.
  #[serde(default = "default_mesh_distance")]
  pub mesh_distance_metres: f32,
  /// Per-biome species rules that replace the built-in species mix.
  /// Biomes without a rule keep the built-in mix.
  #[serde(default)]
  pub species_rules: Vec<FloraRule>,
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
      wind_strength: default_wind_strength(),
      mesh_distance_metres: default_mesh_distance(),
      species_rules: Vec::new(),
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

fn default_cloud_wind_direction() -> f32 {
  70.0
}

fn default_cloud_evolution() -> f32 {
  0.35
}

fn default_cloud_thickness() -> f32 {
  1_600.0
}

fn default_cloud_density() -> f32 {
  0.6
}

/// Cloud layer controls.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudsOptions {
  /// Rendering style/tier. Defaults to `Off`.
  pub style: CloudStyle,
  /// Cloud coverage from 0 (clear) to 1 (overcast).
  pub coverage: f32,
  /// Wind speed multiplier. 1 moves clouds at roughly 15 metres per
  /// second; 0 freezes them in place.
  pub speed: f32,
  /// Altitude of the cloud base in metres.
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
  /// Direction the wind blows clouds towards, in degrees.
  #[serde(default = "default_cloud_wind_direction")]
  pub wind_direction_degrees: f32,
  /// How quickly cloud shapes billow and change while they drift, from 0
  /// (rigid) to 1 (fast-changing).
  #[serde(default = "default_cloud_evolution")]
  pub evolution: f32,
  /// Vertical thickness of the cloud layer in metres.
  #[serde(default = "default_cloud_thickness")]
  pub thickness_metres: f32,
  /// Optical density of the clouds, from 0 (wispy) to 1 (dense cumulus).
  #[serde(default = "default_cloud_density")]
  pub density: f32,
  /// Whether clouds cast moving shadows on the terrain and water.
  #[serde(default = "default_true")]
  pub cast_shadows: bool,
  /// Resolution of the volumetric cloud pass relative to the canvas, from
  /// 0.25 to 1. Clouds are soft, so half resolution (the default) looks
  /// the same as full resolution at a quarter of the cost.
  #[serde(default = "default_cloud_resolution")]
  pub resolution_scale: f32,
  /// Amount of thin, high cirrus above the main cloud layer, from 0 (none)
  /// to 1. Cirrus is drawn with the clouds, so it needs a cloud style other
  /// than `Off`.
  #[serde(default = "default_cirrus")]
  pub cirrus: f32,
  /// Altitude of the cirrus layer in metres. It is always kept above the
  /// top of the main cloud layer.
  #[serde(default = "default_cirrus_height")]
  pub cirrus_height_metres: f32,
  /// Drift speed of the cirrus layer, in units of 15 m/s, separate from
  /// `speed`. The weather system does not change it.
  #[serde(default = "default_cirrus_speed")]
  pub cirrus_speed: f32,
  /// Reuse distant clouds between frames: each frame raymarches one sky
  /// pixel in every 2 x 2 block and reprojects the other three from the
  /// previous frame, cutting the cost of sky clouds to about a quarter.
  /// Volumetric clouds only; switched off automatically while the camera is
  /// in or near the cloud layer.
  #[serde(default)]
  pub temporal: bool,
  /// Cloud type, from 0 (heaped cumulus) to 1 (a flat, layered sheet such
  /// as stratus or nimbostratus). Volumetric clouds only.
  #[serde(default)]
  pub stratiform: f32,
  /// Amount of towering storm clouds (cumulonimbus) with anvil tops, from
  /// 0 to 1. Towers rise well above `thicknessMetres`. Volumetric clouds
  /// only.
  #[serde(default)]
  pub towering: f32,
  /// Extra darkening of cloud bases, as in rain-laden clouds, from 0 to 1.
  #[serde(default)]
  pub base_darkness: f32,
  /// Ragged, uneven cloud bases with loose scraps of cloud below them,
  /// from 0 to 1.
  #[serde(default)]
  pub ragged_base: f32,
  /// Visible curtains of rain or snow hanging below the clouds, from 0 to
  /// 1.
  #[serde(default)]
  pub rain_shafts: f32,
}

fn default_cloud_resolution() -> f32 {
  0.5
}

fn default_cirrus() -> f32 {
  0.35
}

fn default_cirrus_height() -> f32 {
  9_000.0
}

fn default_cirrus_speed() -> f32 {
  0.4
}

impl Default for CloudsOptions {
  fn default() -> Self {
    Self {
      style: CloudStyle::default(),
      coverage: 0.45,
      speed: 1.0,
      height_metres: 1_800.0,
      colour: [1.0, 1.0, 1.0],
      seed_offset: 9_007,
      raymarch_steps: Some(32),
      wind_direction_degrees: default_cloud_wind_direction(),
      evolution: default_cloud_evolution(),
      thickness_metres: default_cloud_thickness(),
      density: default_cloud_density(),
      cast_shadows: true,
      resolution_scale: default_cloud_resolution(),
      cirrus: default_cirrus(),
      cirrus_height_metres: default_cirrus_height(),
      cirrus_speed: default_cirrus_speed(),
      temporal: false,
      stratiform: 0.0,
      towering: 0.0,
      base_darkness: 0.0,
      ragged_base: 0.0,
      rain_shafts: 0.0,
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

fn default_mist_wind_direction() -> f32 {
  70.0
}

fn default_mist_wind_speed() -> f32 {
  2.5
}

fn default_mist_sun_scattering() -> f32 {
  0.6
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
  /// Direction the wind pushes fog banks towards, in degrees.
  #[serde(default = "default_mist_wind_direction")]
  pub wind_direction_degrees: f32,
  /// Speed at which fog banks drift, in metres per second.
  #[serde(default = "default_mist_wind_speed")]
  pub wind_speed_metres_per_second: f32,
  /// How strongly mist glows when looking towards the sun, from 0 to 1.
  #[serde(default = "default_mist_sun_scattering")]
  pub sun_scattering: f32,
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
      wind_direction_degrees: default_mist_wind_direction(),
      wind_speed_metres_per_second: default_mist_wind_speed(),
      sun_scattering: default_mist_sun_scattering(),
    }
  }
}

/// A named biome.
///
/// Biomes are classified per terrain sample from height, slope, and two
/// seeded climate fields (temperature and moisture), plus volcanic
/// hotspots. They drive ground textures, tree species and density, and
/// grass colour.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[repr(u8)]
pub enum BiomeKind {
  /// Open temperate grassland with scattered trees.
  GrassyMeadows = 0,
  /// Scrub and young trees at the edge of forests.
  OuterThicket = 1,
  /// The open fringe of a temperate or boreal forest.
  OuterForest = 2,
  /// Dense forest interior.
  InnerForest = 3,
  /// Rolling uplands below the mountains.
  MountainFoothills = 4,
  /// High, steep, rocky mountains.
  MountainProper = 5,
  /// Ash and basalt slopes around a volcano.
  OuterVolcanic = 6,
  /// The summit caldera of a volcano, with glowing lava.
  CalderaVolcanic = 7,
  /// Hot, dry grassland with sparse acacia.
  SavannahExpanse = 8,
  /// Flat sandy shoreline.
  CoastalBeach = 9,
  /// Steep, rocky shoreline.
  CoastalRocky = 10,
  /// The fringe of a tropical rainforest.
  OuterJungle = 11,
  /// Dense tropical rainforest.
  InnerJungle = 12,
  /// Low, waterlogged ground with cypress trees.
  SwampWetlands = 13,
  /// Terrain below sea level.
  Ocean = 14,
  /// The band just below the snow line: scree, patchy snow, and dwarf
  /// shrubs above the trees.
  AlpineTransition = 15,
  /// Snowfields just above the snow line, broken by rock outcrops.
  LowerSnowyPeaks = 16,
  /// Permanent snow and ice on the highest ground.
  UpperSnowyPeaks = 17,
  /// Glaciers, ice sheets, and the tundra fringe where the ice thins.
  IceArctic = 18,
}

impl BiomeKind {
  /// Every biome, in `repr(u8)` order.
  pub const ALL: [BiomeKind; 19] = [
    Self::GrassyMeadows,
    Self::OuterThicket,
    Self::OuterForest,
    Self::InnerForest,
    Self::MountainFoothills,
    Self::MountainProper,
    Self::OuterVolcanic,
    Self::CalderaVolcanic,
    Self::SavannahExpanse,
    Self::CoastalBeach,
    Self::CoastalRocky,
    Self::OuterJungle,
    Self::InnerJungle,
    Self::SwampWetlands,
    Self::Ocean,
    Self::AlpineTransition,
    Self::LowerSnowyPeaks,
    Self::UpperSnowyPeaks,
    Self::IceArctic,
  ];

  /// Convert a stored biome index back to a biome.
  pub fn from_index(index: u8) -> Self {
    Self::ALL
      .get(index as usize)
      .copied()
      .unwrap_or(Self::GrassyMeadows)
  }
}

fn default_climate_scale() -> f32 {
  7_000.0
}

fn default_volcanism() -> f32 {
  0.35
}

fn default_beach_height() -> f32 {
  5.0
}

/// Biome classification controls.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BiomeOptions {
  /// Whether climate-driven biomes are used. When `false`, only height and
  /// slope are used (a temperate world of meadows, forests, and
  /// mountains).
  pub enabled: bool,
  /// Seed offset for the climate noise fields.
  pub seed_offset: u64,
  /// Shifts the whole world colder (negative) or hotter (positive), from
  /// -1 to 1.
  pub temperature_bias: f32,
  /// Shifts the whole world drier (negative) or wetter (positive), from
  /// -1 to 1.
  pub moisture_bias: f32,
  /// Typical size of a climate region in metres.
  #[serde(default = "default_climate_scale")]
  pub climate_scale_metres: f32,
  /// Likelihood and size of volcanic regions around high peaks, from 0
  /// (none) to 1 (many, large).
  #[serde(default = "default_volcanism")]
  pub volcanism: f32,
  /// Height above sea level, in metres, below which flat ground becomes
  /// beach.
  #[serde(default = "default_beach_height")]
  pub beach_height_metres: f32,
  /// Optional snow line in metres. Defaults to 80% of the way from sea
  /// level to the highest peak.
  pub snow_line_metres: Option<f32>,
  /// Mean annual temperature at sea level in °C, from -30 to 35. When
  /// unset, the climate follows `temperature_bias` alone and only the
  /// highest summits of a map hold ice.
  pub mean_temperature_celsius: Option<f32>,
}

impl Default for BiomeOptions {
  fn default() -> Self {
    Self {
      enabled: true,
      seed_offset: 1_733,
      temperature_bias: 0.0,
      moisture_bias: 0.0,
      climate_scale_metres: default_climate_scale(),
      volcanism: default_volcanism(),
      beach_height_metres: default_beach_height(),
      snow_line_metres: None,
      mean_temperature_celsius: None,
    }
  }
}

/// A weather state.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WeatherKind {
  /// Blue sky, light breeze.
  Clear,
  /// Scattered fair-weather cumulus.
  #[default]
  PartlyCloudy,
  /// Full, grey cloud cover.
  Overcast,
  /// Low cloud and thick ground fog.
  Fog,
  /// Steady rain under dark cloud.
  Rain,
  /// Heavy rain, strong gusting wind, rough water, and lightning.
  Storm,
  /// Falling snow that settles on the ground and trees.
  Snow,
}

/// Which systems the weather drives. Anything switched off here keeps its
/// own manual settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WeatherEffects {
  /// Cloud coverage, density, and thickness.
  pub clouds: bool,
  /// Ground mist and haze.
  pub mist: bool,
  /// Wind for trees, grass, clouds, mist, and water.
  pub wind: bool,
  /// Wave height, choppiness, and current.
  pub water: bool,
  /// Rain and snow falling.
  pub precipitation: bool,
  /// Wet ground, puddles, and settled snow.
  pub ground: bool,
  /// Lightning flashes during storms.
  pub lightning: bool,
}

impl Default for WeatherEffects {
  fn default() -> Self {
    Self {
      clouds: true,
      mist: true,
      wind: true,
      water: true,
      precipitation: true,
      ground: true,
      lightning: true,
    }
  }
}

/// Weather pattern controls.
///
/// The weather system blends between weather states over time and drives
/// clouds, mist, wind, water, precipitation, ground wetness, snow cover,
/// and lightning from one place. With `autoCycle`, it moves through a
/// plausible sequence of states (clear skies cloud over, rain, clear
/// again) that is deterministic for a given seed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WeatherOptions {
  /// Whether the weather system is active. When `false`, clouds, mist,
  /// wind, and water use their own settings unchanged.
  pub enabled: bool,
  /// The weather to move towards (or start from, with `autoCycle`).
  pub state: WeatherKind,
  /// Automatically move on to new weather over time.
  pub auto_cycle: bool,
  /// Average time each state lasts when cycling, in seconds.
  pub state_duration_seconds: f32,
  /// Time taken to blend from one state to the next, in seconds.
  pub transition_seconds: f32,
  /// Whether snow may occur when cycling.
  pub allow_snow: bool,
  /// Seed for the weather sequence and gusts.
  pub seed_offset: u64,
  /// Prevailing wind direction in degrees (the direction the wind blows
  /// towards).
  pub wind_direction_degrees: f32,
  /// Multiplier on each state's wind speed.
  pub wind_scale: f32,
  /// Multiplier on rain and snow intensity. Above 1, rain becomes a
  /// downpour: denser, longer streaks and a grey veil that cuts visibility.
  pub precipitation_scale: f32,
  /// Raindrops that land on the camera lens, bead, and run down the
  /// screen while it rains.
  pub lens_drops: bool,
  /// Drops on the lens at once in full rain, 0 to 512. It scales with the
  /// rain's intensity.
  pub lens_drop_count: u32,
  /// Smallest lens drop diameter, as a fraction of the canvas height,
  /// 0.002 to 0.2.
  pub lens_drop_min_size: f32,
  /// Largest lens drop diameter, as a fraction of the canvas height,
  /// 0.002 to 0.2, and at least `lens_drop_min_size`.
  pub lens_drop_max_size: f32,
  /// Which systems the weather drives.
  pub effects: WeatherEffects,
}

impl Default for WeatherOptions {
  fn default() -> Self {
    Self {
      enabled: false,
      state: WeatherKind::default(),
      auto_cycle: false,
      state_duration_seconds: 240.0,
      transition_seconds: 30.0,
      allow_snow: false,
      seed_offset: 4_111,
      wind_direction_degrees: 70.0,
      wind_scale: 1.0,
      precipitation_scale: 1.0,
      lens_drops: false,
      lens_drop_count: 60,
      lens_drop_min_size: 0.008,
      lens_drop_max_size: 0.05,
      effects: WeatherEffects::default(),
    }
  }
}

/// The current, blended weather, as reported by `getWeather()`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WeatherState {
  /// The state being left.
  pub from: WeatherKind,
  /// The state being approached.
  pub to: WeatherKind,
  /// Blend from `from` (0) to `to` (1).
  pub blend: f32,
  /// Cloud coverage, 0 to 1.
  pub cloud_coverage: f32,
  /// Cloud density, 0 to 1.
  pub cloud_density: f32,
  /// Ground mist density, 0 to 1.
  pub mist_density: f32,
  /// Wind speed in metres per second, including gusts.
  pub wind_speed_metres_per_second: f32,
  /// Wind direction in degrees.
  pub wind_direction_degrees: f32,
  /// Rain intensity, 0 to 1.
  pub rain: f32,
  /// Snowfall intensity, 0 to 1.
  pub snow: f32,
  /// Ground wetness, 0 (dry) to 1 (puddles); lags behind the rain.
  pub wetness: f32,
  /// Settled snow, 0 to 1; builds up and melts slowly.
  pub snow_cover: f32,
  /// Current lightning flash brightness, 0 to 1.
  pub lightning: f32,
  /// Cloud type, 0 (cumulus) to 1 (flat sheet).
  #[serde(default)]
  pub stratiform: f32,
  /// Amount of towering storm clouds, 0 to 1.
  #[serde(default)]
  pub towering: f32,
  /// Darkening of cloud bases, 0 to 1.
  #[serde(default)]
  pub base_darkness: f32,
  /// Raggedness of cloud bases, 0 to 1.
  #[serde(default)]
  pub ragged_base: f32,
  /// Visible rain or snow curtains below the clouds, 0 to 1.
  #[serde(default)]
  pub rain_shafts: f32,
}

/// Terrain self-shadowing controls.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TerrainShadowOptions {
  /// Whether hills and mountains cast shadows.
  pub enabled: bool,
  /// Shadow darkness, 0 to 1.
  pub strength: f32,
  /// Penumbra width, 0 (hard) to 1 (very soft).
  pub softness: f32,
}

impl Default for TerrainShadowOptions {
  fn default() -> Self {
    Self {
      enabled: true,
      strength: 0.9,
      softness: 0.35,
    }
  }
}

/// Tree shadow controls.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TreeShadowOptions {
  /// Whether trees cast shadows on the ground, grass, water, and each
  /// other.
  pub enabled: bool,
  /// Radius around the camera, in metres, within which trees cast
  /// shadows.
  pub distance_metres: f32,
  /// Shadow map resolution: 512, 1024, 2048, or 4096.
  pub resolution: u32,
  /// Shadow darkness, 0 to 1.
  pub strength: f32,
  /// Filter width, 0 (sharp) to 1 (soft).
  pub softness: f32,
}

impl Default for TreeShadowOptions {
  fn default() -> Self {
    Self {
      enabled: true,
      distance_metres: 260.0,
      resolution: 2048,
      strength: 0.8,
      softness: 0.5,
    }
  }
}

/// Cloud shadow controls. Cloud shadows also require
/// `CloudsOptions.castShadows`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CloudShadowOptions {
  /// Whether clouds cast shadows.
  pub enabled: bool,
  /// Shadow darkness, 0 to 1.
  pub strength: f32,
}

impl Default for CloudShadowOptions {
  fn default() -> Self {
    Self {
      enabled: true,
      strength: 0.78,
    }
  }
}

/// Shadow controls for terrain, trees, and clouds.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ShadowOptions {
  /// Hills and mountains shadowing the landscape.
  pub terrain: TerrainShadowOptions,
  /// Trees shadowing the ground and each other.
  pub trees: TreeShadowOptions,
  /// Clouds shadowing everything below them.
  pub clouds: CloudShadowOptions,
}

/// Number of terrain surface materials.
pub const MATERIAL_COUNT: usize = 11;

fn default_material_tints() -> [Rgb; MATERIAL_COUNT] {
  [[1.0, 1.0, 1.0]; MATERIAL_COUNT]
}

/// Accept 8 tints, as before ice and tundra were added, 10, as before
/// gravel was added, or all 11. Missing tints stay white.
fn deserialize_material_tints<'de, D>(deserializer: D) -> Result<[Rgb; MATERIAL_COUNT], D::Error>
where
  D: serde::Deserializer<'de>,
{
  let tints = Vec::<Rgb>::deserialize(deserializer)?;

  if ![8, 10, MATERIAL_COUNT].contains(&tints.len()) {
    return Err(serde::de::Error::custom(format!(
      "materialTints must list 8, 10 or {MATERIAL_COUNT} colours, but {} were given.",
      tints.len()
    )));
  }

  let mut out = default_material_tints();
  out[..tints.len()].copy_from_slice(&tints);
  Ok(out)
}

/// A baked texture array that host textures can replace, one layer at a
/// time.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextureTarget {
  /// Terrain material colour (rgb, sRGB) and height (a), one layer per
  /// material in [`SurfaceOptions::material_tints`] order.
  TerrainAlbedo,
  /// Terrain material normal (rg, tangent space), occlusion (b), and
  /// roughness (a).
  TerrainNormal,
  /// Bark and foliage colour (rgb, sRGB) with alpha coverage.
  Flora,
}

/// Terrain surface shading controls.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SurfaceOptions {
  /// Use the detailed, textured materials. When `false`, each material is
  /// a flat colour, which is cheaper on low-end GPUs.
  pub textures: bool,
  /// Use detail normal maps.
  pub detail_normals: bool,
  /// Multiplier on every material's texture size (larger values stretch
  /// textures over more ground).
  pub texture_scale: f32,
  /// Colour multiplier per material, in the order lush grass, dry grass,
  /// forest floor, sand, rock, snow, mud, volcanic, ice, tundra. A list of
  /// the first 8 is also accepted; ice and tundra then stay untinted.
  #[serde(
    default = "default_material_tints",
    deserialize_with = "deserialize_material_tints"
  )]
  pub material_tints: [Rgb; MATERIAL_COUNT],
}

impl Default for SurfaceOptions {
  fn default() -> Self {
    Self {
      textures: true,
      detail_normals: true,
      texture_scale: 1.0,
      material_tints: default_material_tints(),
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
  /// Quality preset. It sets any of the distances below that are left
  /// unset.
  pub preset: RenderQualityPreset,
  /// Optional maximum clipmap levels.
  pub max_clipmap_levels: Option<u32>,
  /// Optional flora density multiplier.
  pub flora_density_scale: Option<f32>,
  /// Render distance in metres: terrain, trees, and water beyond it are not
  /// drawn, hidden by horizon-coloured fog that is complete at this
  /// distance. Unset uses the preset's distance.
  #[serde(default)]
  pub render_distance_metres: Option<f32>,
  /// Length in metres of the fog band before the render distance, over
  /// which the fog thickens from none to complete. Unset uses a third of
  /// the render distance.
  #[serde(default)]
  pub render_fade_metres: Option<f32>,
  /// Terrain detail distance in metres: beyond it, terrain uses one
  /// far-scale texture sample per material instead of up to eight. Unset
  /// uses the preset's distance.
  #[serde(default)]
  pub detail_distance_metres: Option<f32>,
  /// Cloud render distance in metres: clouds and rain curtains are
  /// raymarched only this far, fading out before it. Unset uses the
  /// preset's distance.
  #[serde(default)]
  pub cloud_distance_metres: Option<f32>,
  /// Length in metres of the band before the cloud distance over which
  /// clouds thin out to nothing. Unset uses 30 % of the cloud distance.
  #[serde(default)]
  pub cloud_fade_metres: Option<f32>,
  /// Highest frame rate the built-in render loop draws at, evenly paced;
  /// 0 removes the cap. Defaults to 60.
  #[serde(default)]
  pub max_frame_rate: Option<f32>,
  /// Fraction of the canvas resolution the scene is rendered at, 0.25 to 1;
  /// a final pass upscales and sharpens it. With `dynamicResolution`, the
  /// highest scale used. Defaults to 1.
  #[serde(default)]
  pub render_scale: Option<f32>,
  /// Lower the render scale automatically while frames arrive late, and
  /// raise it again when there is room, to hold the frame rate. Defaults
  /// to on.
  #[serde(default)]
  pub dynamic_resolution: Option<bool>,
  /// Lowest render scale `dynamicResolution` may use, 0.25 to 1. Defaults
  /// to 0.5.
  #[serde(default)]
  pub min_render_scale: Option<f32>,
}

impl RenderQualityOptions {
  /// The frame-rate cap in effect; 0 means uncapped.
  pub fn frame_rate_cap(&self) -> f32 {
    self.max_frame_rate.unwrap_or(60.0).max(0.0)
  }

  /// The highest and lowest render scales in effect.
  pub fn render_scale_range(&self) -> (f32, f32) {
    let max = self.render_scale.unwrap_or(1.0).clamp(0.25, 1.0);
    let min = if self.dynamic_resolution.unwrap_or(true) {
      self.min_render_scale.unwrap_or(0.5).clamp(0.25, max)
    } else {
      max
    };
    (max, min)
  }
}

/// Distances used when none is set; far enough to change nothing.
pub const UNLIMITED_DISTANCE_METRES: f32 = 1.0e9;

/// Render, detail, and cloud distances after applying the preset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderDistances {
  /// See [`RenderQualityOptions::render_distance_metres`].
  pub render_metres: f32,
  /// See [`RenderQualityOptions::detail_distance_metres`].
  pub detail_metres: f32,
  /// See [`RenderQualityOptions::cloud_distance_metres`].
  pub cloud_metres: f32,
  /// See [`RenderQualityOptions::render_fade_metres`]; never longer than
  /// the render distance.
  pub render_fade_metres: f32,
  /// See [`RenderQualityOptions::cloud_fade_metres`]; never longer than
  /// the cloud distance.
  pub cloud_fade_metres: f32,
}

impl RenderQualityOptions {
  /// The distances in effect: each explicit value, or else the preset's.
  pub fn distances(&self) -> RenderDistances {
    let (render, detail, cloud) = match self.preset {
      RenderQualityPreset::Preview => (6_000.0, 400.0, 12_000.0),
      RenderQualityPreset::Balanced => (UNLIMITED_DISTANCE_METRES, 2_000.0, 60_000.0),
      RenderQualityPreset::High => (UNLIMITED_DISTANCE_METRES, 5_000.0, 90_000.0),
      RenderQualityPreset::Offline => (
        UNLIMITED_DISTANCE_METRES,
        UNLIMITED_DISTANCE_METRES,
        90_000.0,
      ),
    };

    let render_metres = self.render_distance_metres.unwrap_or(render);
    let cloud_metres = self.cloud_distance_metres.unwrap_or(cloud);

    RenderDistances {
      render_metres,
      detail_metres: self.detail_distance_metres.unwrap_or(detail),
      cloud_metres,
      render_fade_metres: self
        .render_fade_metres
        .unwrap_or(render_metres / 3.0)
        .min(render_metres),
      cloud_fade_metres: self
        .cloud_fade_metres
        .unwrap_or(cloud_metres * 0.3)
        .min(cloud_metres),
    }
  }
}

impl Default for RenderQualityOptions {
  fn default() -> Self {
    Self {
      preset: RenderQualityPreset::Balanced,
      max_clipmap_levels: Some(7),
      flora_density_scale: Some(1.0),
      render_distance_metres: None,
      render_fade_metres: None,
      detail_distance_metres: None,
      cloud_distance_metres: None,
      cloud_fade_metres: None,
      max_frame_rate: None,
      render_scale: None,
      dynamic_resolution: None,
      min_render_scale: None,
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
  /// Biome map overlay.
  Biomes,
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
  /// The weather state currently dominating, or `None` when the weather
  /// system is disabled.
  #[serde(default)]
  pub weather: Option<WeatherKind>,
  /// GPU time per pass, when the browser supports timestamp queries.
  /// Measured a few frames behind, without stalling rendering.
  #[serde(default)]
  pub gpu_pass_times_ms: Option<GpuPassTimes>,
  /// Fraction of the canvas resolution rendered this frame.
  #[serde(default = "default_render_scale")]
  pub render_scale: f32,
}

fn default_render_scale() -> f32 {
  1.0
}

/// GPU time spent in each render pass, in milliseconds. A pass that did not run reports 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuPassTimes {
  /// Terrain shadow bake and tree shadow map.
  pub shadows: f32,
  /// GPU tree culling.
  pub tree_culling: f32,
  /// Terrain.
  pub terrain: f32,
  /// Trees: near meshes and distant impostors.
  pub trees: f32,
  /// Grass.
  pub grass: f32,
  /// Cloud raymarching, including rain curtains.
  pub clouds: f32,
  /// Sky, fog, mist, falling rain and snow, and tone mapping.
  pub sky_and_fog: f32,
  /// Ocean, rivers, and lakes.
  pub water: f32,
  /// Upscaling to the canvas and raindrops on the lens, when either is on.
  pub present: f32,
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
      weather: None,
      gpu_pass_times_ms: None,
      render_scale: 1.0,
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
  /// Optional initial biome controls.
  #[serde(default)]
  pub biomes: Option<BiomeOptions>,
  /// Optional initial weather controls.
  #[serde(default)]
  pub weather: Option<WeatherOptions>,
  /// Optional initial shadow controls.
  #[serde(default)]
  pub shadows: Option<ShadowOptions>,
  /// Optional initial terrain surface controls.
  #[serde(default)]
  pub surface: Option<SurfaceOptions>,
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
      biomes: Some(BiomeOptions::default()),
      weather: Some(WeatherOptions::default()),
      shadows: Some(ShadowOptions::default()),
      surface: Some(SurfaceOptions::default()),
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

#[cfg(test)]
mod tests {
  use super::*;
  use serde::de::value::{Error, SeqDeserializer};

  fn tints(count: usize) -> Result<[Rgb; MATERIAL_COUNT], Error> {
    let colours = vec![vec![0.5f32, 0.25, 2.0]; count];
    deserialize_material_tints(SeqDeserializer::<_, Error>::new(colours.into_iter()))
  }

  #[test]
  fn material_tints_accept_eight_ten_or_eleven_colours() {
    let eight = tints(8).unwrap();
    assert_eq!(eight[7], [0.5, 0.25, 2.0]);
    assert_eq!(eight[8], [1.0, 1.0, 1.0]);
    assert_eq!(eight[9], [1.0, 1.0, 1.0]);

    assert_eq!(tints(10).unwrap()[9], [0.5, 0.25, 2.0]);
    assert_eq!(tints(10).unwrap()[10], [1.0, 1.0, 1.0]);
    assert_eq!(tints(11).unwrap()[10], [0.5, 0.25, 2.0]);
    assert!(tints(9).is_err());
    assert!(tints(12).is_err());
  }

  #[test]
  fn inflows_read_a_mode_or_a_list() {
    use serde::de::value::StrDeserializer;
    let mode = |text: &str| RiverInflows::deserialize(StrDeserializer::<Error>::new(text));

    assert_eq!(mode("auto").unwrap(), RiverInflows::Mode(InflowMode::Auto));
    assert_eq!(mode("none").unwrap(), RiverInflows::Mode(InflowMode::None));
    let error = mode("every edge").unwrap_err().to_string();
    assert!(error.contains("\"auto\", \"none\" or a list"), "{error}");
    let empty: Vec<f32> = Vec::new();
    let list = RiverInflows::deserialize(SeqDeserializer::<_, Error>::new(empty.into_iter()));
    assert_eq!(list.unwrap(), RiverInflows::List(Vec::new()));
  }
}
