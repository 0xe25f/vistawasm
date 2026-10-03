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
  /// The GPU reported a validation or out-of-memory error.
  GpuError,
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
      Self::GpuError => "GPU_ERROR",
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerrainHandle {
  /// Stable handle ID for the current engine instance.
  pub id: u32,
  /// Metadata for the active terrain.
  pub metadata: TerrainMetadata,
}

/// Public fractal terrain generation options.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
  /// Billowy detail with a slight step, as in VistaWASM's first releases.
  Classic,
}

/// Fractal noise controls.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ByteOrder {
  /// Little-endian byte order.
  #[default]
  LittleEndian,
  /// Big-endian byte order.
  BigEndian,
}

/// Options for loading raw heightmap bytes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
  /// The landform the heights were generated as, which sets the bedrock's
  /// beds, so a bundle's terrain loads with the rock it was exported
  /// with. Defaults to continental.
  #[serde(default)]
  pub landform: Option<LandformKind>,
}

/// Options for DEM loading.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
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
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
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
  /// How strongly lowland bends migrate by bank erosion, from 0 (rivers
  /// follow their valley without meandering) to 1.
  pub meanders: f32,
  /// How long meanders have been developing, from 0 (young, gentle bends)
  /// to 1 (mature loops, neck cut-offs and oxbow lakes).
  pub meander_maturity: f32,
  /// Braided threads between gravel bars on steep, wide, unconfined
  /// valley floors, from 0 (never) to 1 (wherever slope and discharge call
  /// for them).
  pub braiding: f32,
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
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct RiverInflow {
  /// World x and z in metres. Snaps to the nearest land sample.
  pub position: [f32; 2],
  /// Mean discharge, 0 to 100,000 cubic metres per second.
  pub discharge_cubic_metres_per_second: f32,
}

/// An inflow in use, as `getInflows` reports it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
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
      meander_maturity: 0.5,
      braiding: 1.0,
      waterfalls: true,
      inflow: RiverInflows::default(),
      riparian: 1.0,
    }
  }
}

/// The loudest water sound of one kind near a listener.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
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
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
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
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
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
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct WaterMask {
  /// Samples per row, 2 to 2048 (the largest terrain it is resampled to).
  pub width: u32,
  /// Rows, 2 to 2048.
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
  /// Eddies and vortices in flowing rivers, from 0 (off) to 1. Changing it
  /// does not rebuild rivers.
  #[serde(default = "default_eddies")]
  pub eddies: f32,
  /// Refraction of the bed and caustics in shallow water, from 0 (off) to
  /// 1. Changing it does not rebuild rivers.
  #[serde(default = "default_refraction")]
  pub refraction: f32,
}

fn default_eddies() -> f32 {
  1.0
}

fn default_refraction() -> f32 {
  1.0
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
      eddies: default_eddies(),
      refraction: default_refraction(),
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
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TreeQuality {
  /// A single camera-facing impostor quad per tree (cheapest).
  Billboard,
  /// Two static, crossed impostor quads per tree.
  CrossQuad,
  /// Full 3D meshes near the camera, impostors in the distance (default).
  #[default]
  Mesh,
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

fn default_variants_per_species() -> u32 {
  4
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FloraOptions {
  /// Whether flora is enabled.
  pub enabled: bool,
  /// Tree density, 0 to 4. 1 is the old maximum; 4 is a closed canopy
  /// where the land supports it. Multiplies each biome's own density.
  pub density: f32,
  /// Tree line altitude in metres.
  pub tree_line_metres: f32,
  /// Seed offset for deterministic placement.
  pub seed_offset: u64,
  /// Most trees generated at once: the far set plus the tiles streamed
  /// around the camera.
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
  /// Distinct grown shapes per species, 1 to 4. Each variant grows in four
  /// age classes at two levels of detail, with its own impostors, so
  /// fewer use less GPU memory. Defaults to 4.
  #[serde(default = "default_variants_per_species")]
  pub variants_per_species: u32,
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
      variants_per_species: default_variants_per_species(),
    }
  }
}

/// Grass rendering fidelity.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GrassStyle {
  /// Crossed billboard blade clumps (default once enabled, cheap).
  #[default]
  BillboardBlades,
  /// Denser blade instancing with a shorter view distance (hyper-realistic).
  DenseBlades,
}

/// Grass ground-cover controls. On by default, at density 0.5: a natural
/// meadow near the camera costs well under a millisecond a frame.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GrassOptions {
  /// Whether grass is enabled.
  pub enabled: bool,
  /// Rendering style/tier.
  pub style: GrassStyle,
  /// Grass density, 0 to 4: how a meadow looks near the camera. 0.5 is a
  /// natural, dense meadow covering at least 70 % of the ground, 1 a
  /// lush, taller meadow, and 4 long, dense grass.
  pub density: f32,
  /// Distance from the camera, in metres, at which grass fully fades out.
  pub view_distance_metres: f32,
  /// Seed offset for deterministic placement.
  pub seed_offset: u64,
  /// Most tufts generated around the camera at once.
  pub max_instances: u32,
  /// Ferns and undergrowth replace grass under dense canopy. Defaults to
  /// true.
  #[serde(default = "default_true")]
  pub forest_floor: bool,
}

impl Default for GrassOptions {
  fn default() -> Self {
    Self {
      enabled: true,
      style: GrassStyle::default(),
      density: 0.5,
      view_distance_metres: 220.0,
      seed_offset: 7_331,
      max_instances: 200_000,
      forest_floor: true,
    }
  }
}

/// Cloud rendering fidelity.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CloudStyle {
  /// No cloud layer (default).
  #[default]
  Off,
  /// A 2D noise layer blended into the sky dome shader.
  Painted,
  /// A raymarched volumetric layer with sun-facing shading (hyper-realistic).
  Volumetric,
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
  /// How far low-cloud bases vary in height, from cloud to cloud and
  /// within each cloud, from 0 (one flat level) to 0.2 of the layer's
  /// thickness. Volumetric clouds only.
  #[serde(default = "default_base_variation")]
  pub base_variation: f32,
  /// Soft, cotton-wool lumps under low clouds, from 0 (wispy undersides)
  /// to 1. Volumetric clouds only.
  #[serde(default = "default_base_lumpiness")]
  pub base_lumpiness: f32,
  /// Rippled mid-level cloudlets (altocumulus, a "mackerel sky"), from 0
  /// (none) to 1.
  #[serde(default)]
  pub altocumulus: f32,
  /// A grey mid-level veil (altostratus) that dims the sun to a watery
  /// disc, from 0 (none) to 1.
  #[serde(default)]
  pub altostratus: f32,
  /// Height of the mid-level layer in metres, 2000 to 7000. It is kept at
  /// least 300 m above the top of the low clouds and, where there is
  /// room, 500 m below the cirrus.
  #[serde(default = "default_alto_height")]
  pub alto_height_metres: f32,
  /// Drift speed of the mid-level layer, in units of 15 m/s, from 0 to 4.
  /// It drifts with the cloud wind veered 20 degrees.
  #[serde(default = "default_alto_speed")]
  pub alto_speed: f32,
}

fn default_cloud_resolution() -> f32 {
  0.5
}

fn default_base_variation() -> f32 {
  0.07
}

fn default_base_lumpiness() -> f32 {
  0.6
}

fn default_alto_height() -> f32 {
  4_200.0
}

fn default_alto_speed() -> f32 {
  1.0
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
      base_variation: default_base_variation(),
      base_lumpiness: default_base_lumpiness(),
      altocumulus: 0.0,
      altostratus: 0.0,
      alto_height_metres: default_alto_height(),
      alto_speed: default_alto_speed(),
    }
  }
}

/// Mist/ground-fog rendering fidelity.
///
/// This is distinct from `AtmosphereOptions.hazeDistanceMetres`, which is a
/// uniform, distance-only blend to sky colour. Mist is a height-based
/// ground fog that pools in valleys and near water.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MistStyle {
  /// No ground mist (default).
  #[default]
  Off,
  /// A static height-falloff blend (cheap).
  Flat,
  /// Animated density noise drifting across the terrain (hyper-realistic).
  Volumetric,
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
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

/// A weather preset's name: one of the built-in presets (the associated
/// constants) or a custom preset added through `WeatherOptions::presets`.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WeatherKind(std::borrow::Cow<'static, str>);

/// Values by name, in the order they were given: a JavaScript object's
/// entries. A name given twice keeps its last value.
#[derive(Clone, Debug, PartialEq)]
pub struct NamedMap<T>(pub Vec<(String, T)>);

/// Most entries a [`NamedMap`] may be given. Lookups are linear, so the
/// cap keeps parsing an untrusted object from taking quadratic time.
pub const NAMED_MAP_MAX_ENTRIES: usize = 256;

impl<T> Default for NamedMap<T> {
  fn default() -> Self {
    Self(Vec::new())
  }
}

impl<T> NamedMap<T> {
  /// The value of a name.
  pub fn get(&self, name: &str) -> Option<&T> {
    self
      .0
      .iter()
      .find(|(key, _)| key == name)
      .map(|(_, value)| value)
  }

  /// Set a name's value, replacing any it had.
  pub fn insert(&mut self, name: impl Into<String>, value: T) {
    let name = name.into();

    match self.0.iter_mut().find(|(key, _)| *key == name) {
      Some(entry) => entry.1 = value,
      None => self.0.push((name, value)),
    }
  }

  /// Whether no names are set.
  pub fn is_empty(&self) -> bool {
    self.0.is_empty()
  }

  /// Names and values, in order.
  pub fn iter(&self) -> impl Iterator<Item = (&String, &T)> {
    self.0.iter().map(|(key, value)| (key, value))
  }
}

impl<T> FromIterator<(String, T)> for NamedMap<T> {
  fn from_iter<I: IntoIterator<Item = (String, T)>>(entries: I) -> Self {
    let mut map = Self::default();

    for (name, value) in entries {
      map.insert(name, value);
    }

    map
  }
}

impl<T: Serialize> Serialize for NamedMap<T> {
  fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeMap;
    let mut map = serializer.serialize_map(Some(self.0.len()))?;

    for (name, value) in &self.0 {
      map.serialize_entry(name, value)?;
    }

    map.end()
  }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for NamedMap<T> {
  fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
    struct Entries<T>(std::marker::PhantomData<T>);

    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Entries<T> {
      type Value = NamedMap<T>;

      fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("an object of values by name")
      }

      fn visit_map<A: serde::de::MapAccess<'de>>(
        self,
        mut access: A,
      ) -> Result<Self::Value, A::Error> {
        let mut map = NamedMap::default();

        while let Some((name, value)) = access.next_entry::<String, T>()? {
          if map.0.len() >= NAMED_MAP_MAX_ENTRIES {
            return Err(serde::de::Error::custom(format!(
              "an object of values by name may have at most {NAMED_MAP_MAX_ENTRIES} entries"
            )));
          }

          map.insert(name, value);
        }

        Ok(map)
      }
    }

    deserializer.deserialize_map(Entries(std::marker::PhantomData))
  }
}

#[allow(non_upper_case_globals)]
impl WeatherKind {
  /// Blue sky, light breeze.
  pub const Clear: Self = Self::built_in("clear");
  /// A few small fair-weather cumulus.
  pub const FewClouds: Self = Self::built_in("fewClouds");
  /// Scattered fair-weather cumulus.
  pub const PartlyCloudy: Self = Self::built_in("partlyCloudy");
  /// Mostly cloudy, with breaks of blue.
  pub const BrokenClouds: Self = Self::built_in("brokenClouds");
  /// Full, grey cloud cover.
  pub const Overcast: Self = Self::built_in("overcast");
  /// Damp, milky air and a thin ground mist.
  pub const Mist: Self = Self::built_in("mist");
  /// Low cloud and thick ground fog.
  pub const Fog: Self = Self::built_in("fog");
  /// Light, steady rain.
  pub const LightRain: Self = Self::built_in("lightRain");
  /// Steady rain under dark cloud.
  pub const Rain: Self = Self::built_in("rain");
  /// A downpour.
  pub const HeavyRain: Self = Self::built_in("heavyRain");
  /// Heavy rain, strong gusting wind, rough water, and lightning.
  pub const Storm: Self = Self::built_in("storm");
  /// Falling snow that settles on the ground and trees.
  pub const Snow: Self = Self::built_in("snow");
  /// Heavy snow in a gale.
  pub const Blizzard: Self = Self::built_in("blizzard");

  const fn built_in(name: &'static str) -> Self {
    Self(std::borrow::Cow::Borrowed(name))
  }

  /// A preset by name, built in or custom.
  pub fn new(name: impl Into<String>) -> Self {
    Self(std::borrow::Cow::Owned(name.into()))
  }

  /// The preset's name.
  pub fn as_str(&self) -> &str {
    &self.0
  }
}

impl Default for WeatherKind {
  fn default() -> Self {
    Self::PartlyCloudy
  }
}

impl std::fmt::Display for WeatherKind {
  fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    formatter.write_str(&self.0)
  }
}

/// The temperatures a preset suits, in °C at the camera. The weather
/// cycle does not move to a preset outside them.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WeatherClimate {
  /// Coldest suitable temperature.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub min_celsius: Option<f32>,
  /// Warmest suitable temperature.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub max_celsius: Option<f32>,
}

/// Number of numeric fields in a weather preset.
pub const WEATHER_PRESET_NUMBERS: usize = 29;

/// One kind of weather. Every field is optional: a custom preset takes
/// unset fields from the preset it `extends` (or from `partlyCloudy`), and
/// an override of a built-in preset changes only the fields given. Unknown
/// fields are rejected, with the valid ones listed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WeatherPreset {
  /// The preset to inherit unset fields from.
  pub extends: Option<WeatherKind>,
  /// Mean cloud coverage, 0 to 1.
  pub cloud_coverage: Option<f32>,
  /// Cloud density, 0 to 1.
  pub cloud_density: Option<f32>,
  /// Multiplier on the cloud layer's thickness, 0.2 to 3.
  pub cloud_thickness: Option<f32>,
  /// Cloud type, 0 (cumulus) to 1 (flat sheet).
  pub stratiform: Option<f32>,
  /// Towering storm clouds, 0 to 1.
  pub towering: Option<f32>,
  /// Darkening of cloud bases, 0 to 1.
  pub base_darkness: Option<f32>,
  /// Raggedness of cloud bases, 0 to 1.
  pub ragged_base: Option<f32>,
  /// Curtains of rain or snow below the clouds, 0 to 1.
  pub rain_shafts: Option<f32>,
  /// High cirrus, 0 to 1.
  pub cirrus: Option<f32>,
  /// Relative humidity, 0 to 1: damp air whitens and thickens the haze.
  pub humidity: Option<f32>,
  /// Aerosol turbidity, 2 (clean alpine air) to 10 (thick tropical haze).
  pub turbidity: Option<f32>,
  /// Multiplier on the haze distance, 0.05 to 2 (lower is hazier).
  pub haze_distance_scale: Option<f32>,
  /// Ground mist density, 0 to 1.
  pub mist_density: Option<f32>,
  /// Added to the climate's temperature, -30 to 30 °C.
  pub temperature_offset_celsius: Option<f32>,
  /// Mean wind speed, 0 to 60 m/s.
  pub wind_metres_per_second: Option<f32>,
  /// Gust strength relative to the mean wind, 0 to 1.
  pub gustiness: Option<f32>,
  /// Rain intensity, 0 to 2 (above 1, a downpour).
  pub rain: Option<f32>,
  /// Snowfall intensity, 0 to 2.
  pub snow: Option<f32>,
  /// Lightning flashes per minute, 0 to 60.
  pub lightning_per_minute: Option<f32>,
  /// How far coverage varies across the regional map, 0 to 1.
  pub coverage_spread: Option<f32>,
  /// How far precipitation varies under thick cloud, 0 to 1.
  pub precipitation_spread: Option<f32>,
  /// Size of weather cells, 1 to 100 km.
  pub cell_size_km: Option<f32>,
  /// 0 for smooth fields, 1 for discrete storm cells.
  pub cellularity: Option<f32>,
  /// Variation of low-cloud base height, 0 to 0.2 of the layer thickness.
  pub base_variation: Option<f32>,
  /// Cotton-wool lumps under low clouds, 0 to 1.
  pub base_lumpiness: Option<f32>,
  /// Rippled mid-level cloudlets (altocumulus), 0 to 1.
  pub altocumulus: Option<f32>,
  /// Grey mid-level veil (altostratus), 0 to 1.
  pub altostratus: Option<f32>,
  /// Height of the mid-level layer, 2000 to 7000 m.
  pub alto_height_metres: Option<f32>,
  /// Mid-level drift speed in units of 15 m/s, 0 to 4.
  pub alto_speed: Option<f32>,
  /// Weights of the presets that may follow this one when cycling.
  pub next: Option<NamedMap<f32>>,
  /// Shortest time the preset lasts when cycling, in seconds.
  pub min_duration_seconds: Option<f32>,
  /// Longest time the preset lasts when cycling, in seconds.
  pub max_duration_seconds: Option<f32>,
  /// Temperatures the preset suits.
  pub climate: Option<WeatherClimate>,
}

impl WeatherPreset {
  /// The numeric fields' names, in [`Self::numbers`] order.
  pub const NUMBER_NAMES: [&'static str; WEATHER_PRESET_NUMBERS] = [
    "cloudCoverage",
    "cloudDensity",
    "cloudThickness",
    "stratiform",
    "towering",
    "baseDarkness",
    "raggedBase",
    "rainShafts",
    "cirrus",
    "humidity",
    "turbidity",
    "hazeDistanceScale",
    "mistDensity",
    "temperatureOffsetCelsius",
    "windMetresPerSecond",
    "gustiness",
    "rain",
    "snow",
    "lightningPerMinute",
    "coverageSpread",
    "precipitationSpread",
    "cellSizeKm",
    "cellularity",
    "baseVariation",
    "baseLumpiness",
    "altocumulus",
    "altostratus",
    "altoHeightMetres",
    "altoSpeed",
  ];

  /// Every field's name, for messages.
  pub const FIELD_NAMES: &'static str = "extends, cloudCoverage, cloudDensity, cloudThickness, stratiform, towering, baseDarkness, raggedBase, rainShafts, cirrus, humidity, turbidity, hazeDistanceScale, mistDensity, temperatureOffsetCelsius, windMetresPerSecond, gustiness, rain, snow, lightningPerMinute, coverageSpread, precipitationSpread, cellSizeKm, cellularity, baseVariation, baseLumpiness, altocumulus, altostratus, altoHeightMetres, altoSpeed, next, minDurationSeconds, maxDurationSeconds, climate";

  /// The numeric fields, in [`Self::NUMBER_NAMES`] order.
  pub fn numbers(&self) -> [Option<f32>; WEATHER_PRESET_NUMBERS] {
    [
      self.cloud_coverage,
      self.cloud_density,
      self.cloud_thickness,
      self.stratiform,
      self.towering,
      self.base_darkness,
      self.ragged_base,
      self.rain_shafts,
      self.cirrus,
      self.humidity,
      self.turbidity,
      self.haze_distance_scale,
      self.mist_density,
      self.temperature_offset_celsius,
      self.wind_metres_per_second,
      self.gustiness,
      self.rain,
      self.snow,
      self.lightning_per_minute,
      self.coverage_spread,
      self.precipitation_spread,
      self.cell_size_km,
      self.cellularity,
      self.base_variation,
      self.base_lumpiness,
      self.altocumulus,
      self.altostratus,
      self.alto_height_metres,
      self.alto_speed,
    ]
  }

  /// A numeric field, by its place in [`Self::NUMBER_NAMES`].
  fn number_mut(&mut self, index: usize) -> Option<&mut Option<f32>> {
    Some(match index {
      0 => &mut self.cloud_coverage,
      1 => &mut self.cloud_density,
      2 => &mut self.cloud_thickness,
      3 => &mut self.stratiform,
      4 => &mut self.towering,
      5 => &mut self.base_darkness,
      6 => &mut self.ragged_base,
      7 => &mut self.rain_shafts,
      8 => &mut self.cirrus,
      9 => &mut self.humidity,
      10 => &mut self.turbidity,
      11 => &mut self.haze_distance_scale,
      12 => &mut self.mist_density,
      13 => &mut self.temperature_offset_celsius,
      14 => &mut self.wind_metres_per_second,
      15 => &mut self.gustiness,
      16 => &mut self.rain,
      17 => &mut self.snow,
      18 => &mut self.lightning_per_minute,
      19 => &mut self.coverage_spread,
      20 => &mut self.precipitation_spread,
      21 => &mut self.cell_size_km,
      22 => &mut self.cellularity,
      23 => &mut self.base_variation,
      24 => &mut self.base_lumpiness,
      25 => &mut self.altocumulus,
      26 => &mut self.altostratus,
      27 => &mut self.alto_height_metres,
      28 => &mut self.alto_speed,
      _ => return None,
    })
  }
}

/// A number, or nothing for `null` and `undefined`.
struct Number(Option<f32>);

impl<'de> Deserialize<'de> for Number {
  fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
    struct Value;

    impl serde::de::Visitor<'_> for Value {
      type Value = Number;

      fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a number")
      }

      fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Number, E> {
        Ok(Number(Some(value as f32)))
      }

      fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Number, E> {
        Ok(Number(Some(value as f32)))
      }

      fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Number, E> {
        Ok(Number(Some(value as f32)))
      }

      fn visit_unit<E: serde::de::Error>(self) -> Result<Number, E> {
        Ok(Number(None))
      }

      fn visit_none<E: serde::de::Error>(self) -> Result<Number, E> {
        Ok(Number(None))
      }
    }

    deserializer.deserialize_any(Value)
  }
}

impl<'de> Deserialize<'de> for WeatherPreset {
  fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
    struct Fields;

    impl<'de> serde::de::Visitor<'de> for Fields {
      type Value = WeatherPreset;

      fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a weather preset object")
      }

      fn visit_map<A: serde::de::MapAccess<'de>>(
        self,
        mut access: A,
      ) -> Result<Self::Value, A::Error> {
        let mut preset = WeatherPreset::default();

        while let Some(key) = access.next_key::<String>()? {
          match key.as_str() {
            "extends" => preset.extends = access.next_value()?,
            "next" => preset.next = access.next_value()?,
            "minDurationSeconds" => preset.min_duration_seconds = access.next_value()?,
            "maxDurationSeconds" => preset.max_duration_seconds = access.next_value()?,
            "climate" => preset.climate = access.next_value()?,
            name => {
              let slot = WeatherPreset::NUMBER_NAMES
                .iter()
                .position(|known| *known == name)
                .and_then(|index| preset.number_mut(index))
                .ok_or_else(|| {
                  serde::de::Error::custom(format_args!(
                    "unknown weather preset field `{name}`, expected one of {}",
                    WeatherPreset::FIELD_NAMES
                  ))
                })?;
              *slot = access.next_value::<Number>()?.0;
            }
          }
        }

        Ok(preset)
      }
    }

    deserializer.deserialize_map(Fields)
  }
}

impl Serialize for WeatherPreset {
  fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeMap;
    let mut map = serializer.serialize_map(None)?;

    if let Some(extends) = &self.extends {
      map.serialize_entry("extends", extends)?;
    }

    for (name, value) in WeatherPreset::NUMBER_NAMES.iter().zip(self.numbers()) {
      if let Some(value) = value {
        map.serialize_entry(name, &value)?;
      }
    }

    if let Some(next) = &self.next {
      map.serialize_entry("next", next)?;
    }

    if let Some(value) = self.min_duration_seconds {
      map.serialize_entry("minDurationSeconds", &value)?;
    }

    if let Some(value) = self.max_duration_seconds {
      map.serialize_entry("maxDurationSeconds", &value)?;
    }

    if let Some(climate) = &self.climate {
      map.serialize_entry("climate", climate)?;
    }

    map.end()
  }
}

/// A fully resolved preset, as `getWeatherPresets()` reports it: every
/// field of [`WeatherPreset`] but `extends`, set.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResolvedWeatherPreset {
  /// The numeric fields, in [`WeatherPreset::NUMBER_NAMES`] order.
  pub numbers: [f32; WEATHER_PRESET_NUMBERS],
  /// Weights of the presets that may follow this one when cycling.
  pub next: NamedMap<f32>,
  /// Shortest time the preset lasts when cycling, in seconds.
  pub min_duration_seconds: f32,
  /// Longest time the preset lasts when cycling, in seconds.
  pub max_duration_seconds: f32,
  /// Temperatures the preset suits.
  pub climate: WeatherClimate,
}

impl ResolvedWeatherPreset {
  /// A numeric field by its name in [`WeatherPreset::NUMBER_NAMES`].
  pub fn number(&self, name: &str) -> Option<f32> {
    let index = WeatherPreset::NUMBER_NAMES
      .iter()
      .position(|known| *known == name)?;
    Some(self.numbers[index])
  }
}

impl Serialize for ResolvedWeatherPreset {
  fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeMap;
    let mut map = serializer.serialize_map(Some(WEATHER_PRESET_NUMBERS + 4))?;

    for (name, value) in WeatherPreset::NUMBER_NAMES.iter().zip(self.numbers) {
      map.serialize_entry(name, &value)?;
    }

    map.serialize_entry("next", &self.next)?;
    map.serialize_entry("minDurationSeconds", &self.min_duration_seconds)?;
    map.serialize_entry("maxDurationSeconds", &self.max_duration_seconds)?;
    map.serialize_entry("climate", &self.climate)?;
    map.end()
  }
}

/// The weather at one place, as `weatherAt()` reports it. Every value is 0
/// to 1.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalWeather {
  /// Cloud coverage overhead.
  pub coverage: f32,
  /// Rain and snow falling, together (above 1 in a downpour).
  pub precipitation: f32,
  /// How much of a storm cell is overhead.
  pub storminess: f32,
  /// Relative humidity.
  pub humidity: f32,
  /// Wetness of the ground.
  pub wetness: f32,
  /// Puddle water on the ground.
  pub puddles: f32,
  /// Depth of settled snow.
  pub snow_depth: f32,
}

/// Time of day controls.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct TimeOfDayOptions {
  /// Whether the sun follows the time of day.
  pub enabled: bool,
  /// Local solar-zone time, 0 to 24.
  pub hours: f32,
  /// Real minutes for a full day, 1 to 1440.
  pub day_length_minutes: f32,
  /// Latitude, -89 to 89 degrees (north positive).
  pub latitude_degrees: f32,
  /// Day of the year, 1 to 366.
  pub day_of_year: u32,
}

impl Default for TimeOfDayOptions {
  fn default() -> Self {
    Self {
      enabled: false,
      hours: 12.0,
      day_length_minutes: 24.0,
      latitude_degrees: 45.0,
      day_of_year: 172,
    }
  }
}

/// The time of day, as `getTimeOfDay()` reports it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TimeOfDay {
  /// Local time, 0 to 24.
  pub hours: f32,
  /// Sun azimuth in the convention of `SunOptions::azimuth_degrees`.
  pub sun_azimuth_degrees: f32,
  /// Sun elevation above the horizon, with refraction.
  pub sun_elevation_degrees: f32,
  /// Sunrise, or `None` in polar day or night.
  pub sunrise_hours: Option<f32>,
  /// Sunset, or `None` in polar day or night.
  pub sunset_hours: Option<f32>,
}

/// Which systems the weather drives. Anything switched off here keeps its
/// own manual settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
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
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
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
  /// Custom presets and overrides of built-in presets, by name.
  pub presets: NamedMap<WeatherPreset>,
  /// Whether the weather varies across the map. When `false`, every
  /// place has the same weather.
  pub regional: bool,
  /// Size of the regional weather map in km, 16 to 256.
  pub region_size_km: f32,
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
      presets: Default::default(),
      regional: true,
      region_size_km: 64.0,
    }
  }
}

/// The current, blended weather, as reported by `getWeather()`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
  /// Relative humidity at the camera, 0 to 1.
  #[serde(default)]
  pub humidity: f32,
  /// Aerosol turbidity, 2 to 10.
  #[serde(default)]
  pub turbidity: f32,
  /// How much of a storm cell is over the camera, 0 to 1.
  #[serde(default)]
  pub storminess: f32,
  /// Share of direct sunlight the clouds let through towards the camera,
  /// 0 to 1.
  #[serde(default)]
  pub sun_transmittance: f32,
  /// Gust strength relative to the mean wind, 0 to 1.
  #[serde(default)]
  pub gustiness: f32,
  /// Puddle water under the camera, 0 to 1.
  #[serde(default)]
  pub puddles: f32,
}

/// Terrain self-shadowing controls.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
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
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
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
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
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
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct ShadowOptions {
  /// Hills and mountains shadowing the landscape.
  pub terrain: TerrainShadowOptions,
  /// Trees shadowing the ground and each other.
  pub trees: TreeShadowOptions,
  /// Clouds shadowing everything below them.
  pub clouds: CloudShadowOptions,
}

/// Number of terrain surface materials.
pub const MATERIAL_COUNT: usize = 12;

fn default_material_tints() -> [Rgb; MATERIAL_COUNT] {
  [[1.0, 1.0, 1.0]; MATERIAL_COUNT]
}

/// Accept 8 tints, as before ice and tundra were added, 10, as before
/// gravel was added, 11, as before scree was added, or all 12. Missing
/// tints stay white.
fn deserialize_material_tints<'de, D>(deserializer: D) -> Result<[Rgb; MATERIAL_COUNT], D::Error>
where
  D: serde::Deserializer<'de>,
{
  let tints = Vec::<Rgb>::deserialize(deserializer)?;

  if ![8, 10, 11, MATERIAL_COUNT].contains(&tints.len()) {
    return Err(serde::de::Error::custom(format!(
      "materialTints must list 8, 10, 11 or {MATERIAL_COUNT} colours, but {} were given.",
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
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
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
  /// forest floor, sand, rock, snow, mud, volcanic, ice, tundra, gravel,
  /// scree. A list of the first 8, 10 or 11 is also accepted; the
  /// materials left out stay untinted.
  #[serde(
    default = "default_material_tints",
    deserialize_with = "deserialize_material_tints"
  )]
  pub material_tints: [Rgb; MATERIAL_COUNT],
  /// How readily bedrock shows through thin soil, 0 (deep soil
  /// everywhere) to 2 (rocky). 1 is the natural soil depth.
  pub rockiness: f32,
  /// Fallen boulders and talus below rock outcrops.
  pub boulders: bool,
  /// Distance in metres within which boulders are drawn, 50 to 1000.
  /// Beyond it they shrink away, and the scree texture carries the look.
  pub boulder_distance_metres: f32,
}

impl Default for SurfaceOptions {
  fn default() -> Self {
    Self {
      textures: true,
      detail_normals: true,
      texture_scale: 1.0,
      material_tints: default_material_tints(),
      rockiness: 1.0,
      boulders: true,
      boulder_distance_metres: 300.0,
    }
  }
}

/// Render quality preset.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RenderQualityPreset {
  /// Fast preview rendering.
  Preview,
  /// Balanced quality and speed.
  #[default]
  Balanced,
  /// High quality rendering.
  High,
  /// Offline still-render quality.
  Offline,
}

/// Render quality options.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
  /// Radius in metres of full-density vegetation around the camera.
  /// Unset uses the preset's.
  #[serde(default)]
  pub vegetation_detail_metres: Option<f32>,
  /// Distance where individual trees give way to a canopy layer. Unset
  /// uses the preset's.
  #[serde(default)]
  pub canopy_distance_metres: Option<f32>,
  /// Most tree instances drawn per frame. Unset uses the preset's.
  #[serde(default)]
  pub max_tree_instances: Option<u32>,
  /// Most grass tufts drawn per frame. Unset uses the preset's.
  #[serde(default)]
  pub max_grass_instances: Option<u32>,
  /// Most tree triangles drawn per frame, 100,000 to 50,000,000: over
  /// it, the furthest full-mesh trees become impostors. Unset uses the
  /// preset's (`offline` has no limit).
  #[serde(default)]
  pub max_tree_triangles: Option<u32>,
  /// Radius in metres of full-cover grass around the camera, 10 to 300.
  /// Unset uses the preset's.
  #[serde(default)]
  pub grass_detail_metres: Option<f32>,
  /// Time the trees pass as three passes (canopy meshes, understorey
  /// meshes and impostors), reported in `gpuPassTimesMs`. For profiling:
  /// the extra passes cost a little. Defaults to off.
  #[serde(default)]
  pub split_tree_timing: Option<bool>,
}

/// Vegetation distances and budgets after applying the preset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VegetationBudget {
  /// See [`RenderQualityOptions::vegetation_detail_metres`].
  pub detail_metres: f32,
  /// See [`RenderQualityOptions::canopy_distance_metres`].
  pub canopy_metres: f32,
  /// See [`RenderQualityOptions::max_tree_instances`].
  pub max_trees: u32,
  /// See [`RenderQualityOptions::max_grass_instances`].
  pub max_grass: u32,
  /// See [`RenderQualityOptions::max_tree_triangles`]; `u32::MAX` means
  /// no limit.
  pub max_tree_triangles: u32,
  /// See [`RenderQualityOptions::grass_detail_metres`].
  pub grass_detail_metres: f32,
}

impl RenderQualityOptions {
  /// The vegetation distances and budgets in effect: each explicit value,
  /// or else the preset's.
  pub fn vegetation(&self) -> VegetationBudget {
    let (detail, canopy, trees, grass, triangles, grass_detail) = match self.preset {
      RenderQualityPreset::Preview => (120.0, 1_500.0, 40_000, 150_000, 1_000_000, 25.0),
      RenderQualityPreset::Balanced => (250.0, 2_500.0, 120_000, 400_000, 2_500_000, 45.0),
      RenderQualityPreset::High => (400.0, 4_000.0, 250_000, 800_000, 5_000_000, 70.0),
      RenderQualityPreset::Offline => (600.0, 8_000.0, 1_000_000, 2_000_000, u32::MAX, 120.0),
    };

    VegetationBudget {
      detail_metres: self.vegetation_detail_metres.unwrap_or(detail),
      canopy_metres: self.canopy_distance_metres.unwrap_or(canopy),
      max_trees: self.max_tree_instances.unwrap_or(trees),
      max_grass: self.max_grass_instances.unwrap_or(grass),
      max_tree_triangles: self.max_tree_triangles.unwrap_or(triangles),
      grass_detail_metres: self.grass_detail_metres.unwrap_or(grass_detail),
    }
  }
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
      vegetation_detail_metres: None,
      canopy_distance_metres: None,
      max_tree_instances: None,
      max_grass_instances: None,
      max_tree_triangles: None,
      grass_detail_metres: None,
      split_tree_timing: None,
    }
  }
}

/// Debug overlay mode.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DebugView {
  /// Normal rendering.
  #[default]
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

/// Render statistics exposed to JavaScript.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
  /// Tree triangles drawn (meshes, impostors and shadow casters), read
  /// back from the GPU a frame or two late.
  #[serde(default)]
  pub tree_triangles: u32,
  /// Milliseconds spent growing the trees in WASM: the first variant of
  /// every species before the first frame, and the rest after it.
  #[serde(default)]
  pub tree_growth_ms: [f32; 2],
  /// Milliseconds from submitting the tree impostor bakes until the GPU
  /// finished them, in all.
  #[serde(default)]
  pub tree_bake_ms: f32,
}

fn default_render_scale() -> f32 {
  1.0
}

/// GPU time spent in each render pass, in milliseconds. A pass that did not run reports 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
  /// Vegetation streaming: filling tree, grass and boulder tiles near the
  /// camera, culling the streamed grass and boulders, and grounding them
  /// again when the terrain mesh recentres.
  #[serde(default)]
  pub generation: f32,
  /// With `splitTreeTiming`, the part of `trees` spent on canopy meshes;
  /// 0 otherwise.
  #[serde(default)]
  pub tree_meshes: f32,
  /// With `splitTreeTiming`, the part of `trees` spent on understorey
  /// meshes; 0 otherwise.
  #[serde(default)]
  pub understorey: f32,
  /// With `splitTreeTiming`, the part of `trees` spent on impostors; 0
  /// otherwise.
  #[serde(default)]
  pub tree_impostors: f32,
  /// Boulders and talus below rock outcrops, drawn with their shadow
  /// casters.
  #[serde(default)]
  pub boulders: f32,
  /// Stepping the wet ground, puddles and snow under the weather. It runs
  /// four times a second, so most frames report 0.
  #[serde(default)]
  pub surface_weather: f32,
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
      tree_triangles: 0,
      tree_growth_ms: [0.0; 2],
      tree_bake_ms: 0.0,
    }
  }
}

/// Engine creation options.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportHeightmapOptions {
  /// Export format. The current implementation supports `float32-le`.
  pub format: Option<String>,
}

/// Options for exporting a canvas snapshot.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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
  fn material_tints_accept_eight_ten_eleven_or_twelve_colours() {
    let eight = tints(8).unwrap();
    assert_eq!(eight[7], [0.5, 0.25, 2.0]);
    assert_eq!(eight[8], [1.0, 1.0, 1.0]);
    assert_eq!(eight[9], [1.0, 1.0, 1.0]);

    assert_eq!(tints(10).unwrap()[9], [0.5, 0.25, 2.0]);
    assert_eq!(tints(10).unwrap()[10], [1.0, 1.0, 1.0]);
    assert_eq!(tints(11).unwrap()[10], [0.5, 0.25, 2.0]);
    assert_eq!(tints(11).unwrap()[11], [1.0, 1.0, 1.0]);
    assert_eq!(tints(12).unwrap()[11], [0.5, 0.25, 2.0]);
    assert!(tints(9).is_err());
    assert!(tints(13).is_err());
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
