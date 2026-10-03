use vista_types::{
  AtmosphereOptions, BiomeKind, BiomeOptions, CameraOptions, CloudsOptions, DebugView,
  DemLoadOptions, EngineState, FloraOptions, FractalTerrainOptions, GrassOptions, MistOptions,
  RawHeightmapOptions, RenderQualityOptions, RenderStats, RiverOptions, ShadowOptions, SunOptions,
  SurfaceOptions, TerrainHandle, TextureTarget, TreeSpeciesKind, WaterOptions, WeatherOptions,
  WeatherState,
};

use crate::camera::CameraProjector;
use crate::config::VistaEngineConfig;
use crate::dem::{decode_geotiff, decode_raw_heightmap};
use crate::errors::{VistaError, VistaResult};
use crate::maths::smoothstep;
#[cfg(target_arch = "wasm32")]
use crate::maths::{cross, normalise, sub};
use crate::render::flora::TreeInstance;
use crate::render::pipelines::Needs;
use crate::render::tree_models::{layers, mesh_from_arrays, TreeMesh};
use crate::render::water::{build_river_network, restore_carving, RiverNetwork, RiverSources};
#[cfg(target_arch = "wasm32")]
use crate::terrain::biomes::celsius_to_unit;
use crate::terrain::biomes::sea_level_celsius;
use crate::terrain::biomes::SurfaceSample;
use crate::terrain::channels::CarveRecord;
#[cfg(not(target_arch = "wasm32"))]
use crate::terrain::clipmap::clipmap_levels;
use crate::terrain::fractal::Progress;
#[cfg(not(target_arch = "wasm32"))]
use crate::terrain::generate_fractal_heightmap_with_progress;
use crate::terrain::glaciers::{restore_glaciers, shape_painted_glaciers};
use crate::terrain::HeightMap;
use crate::weather::WeatherSystem;

/// Edge length, in texels, of every replaceable texture layer.
pub const TEXTURE_LAYER_SIZE: u32 = 512;
/// Number of terrain material texture layers.
pub const TERRAIN_TEXTURE_LAYERS: u32 = 12;
/// Largest custom tree instance list accepted by `set_tree_instances`.
pub const MAX_CUSTOM_TREES: usize = 1_000_000;
/// Longest skip `advance_weather` accepts: a day.
pub const MAX_WEATHER_ADVANCE_SECONDS: f32 = 86_400.0;
/// Cells along the long side of the CPU mirror of the surface weather.
const SURFACE_MIRROR_CELLS: usize = 64;
/// Weather seconds between steps of the wet and snowy ground.
const SURFACE_WEATHER_INTERVAL: f32 = 0.25;
/// The largest terrain, in square kilometres, trees, grass and boulders are
/// placed over: 100 km square, a 2048-sample map at 49 m. Their lattices
/// and tile grids cover the whole map, so a larger one (a few samples at
/// kilometres apart, say) would take minutes and gigabytes.
pub const MAX_VEGETATION_KM2: f32 = 10_000.0;
/// Sea colder than this, in °C, starts to freeze over (see `water.wgsl`).
const SEA_ICE_CELSIUS: f32 = -1.5;

/// Core engine state owned by the browser-facing wrapper.
pub struct EngineCore {
  state: EngineState,
  terrain: Option<HeightMap>,
  next_terrain_id: u32,
  active_terrain_id: Option<u32>,
  camera: CameraProjector,
  sun: SunOptions,
  atmosphere: AtmosphereOptions,
  water: WaterOptions,
  flora: FloraOptions,
  grass: GrassOptions,
  clouds: CloudsOptions,
  mist: MistOptions,
  quality: RenderQualityOptions,
  biomes: BiomeOptions,
  weather: WeatherSystem,
  /// The time of day, when it drives the sun.
  day: crate::weather::sun::DayClock,
  /// Wet ground, puddles and snow on a coarse grid over the terrain, for
  /// `weather_at`; the GPU keeps the same at full resolution.
  surface_weather: crate::weather::surface::SurfaceMirror,
  /// Weather seconds since the surface weather last stepped.
  surface_weather_pending: f32,
  /// Whether the surface weather must be settled into the current
  /// weather before its next step (a new terrain, or the weather just
  /// switched on).
  surface_weather_settle: bool,
  /// Where the waves are heading, in degrees; it turns towards the wind.
  wave_heading: Option<f32>,
  shadows: ShadowOptions,
  surface_options: SurfaceOptions,
  /// Host-supplied trees that replace procedural placement, if any.
  custom_trees: Option<Vec<TreeInstance>>,
  /// The procedural trees last placed. Browser builds upload them to the
  /// GPU instead of keeping a copy.
  #[cfg(not(target_arch = "wasm32"))]
  placed_trees: Vec<TreeInstance>,
  /// Lowest and highest terrain heights, for fitting the shadow map.
  height_range: (f32, f32),
  /// Time of the previous frame in milliseconds, for the weather clock.
  last_frame_ms: Option<f64>,
  frame_clock: crate::pacing::FrameClock,
  /// Raindrops on the lens.
  lens_drops: crate::lens_drops::LensDrops,
  resolution: crate::pacing::ResolutionController,
  frame_seconds: f32,
  debug_view: DebugView,
  stats: RenderStats,
  render_width: u32,
  render_height: u32,
  device_pixel_ratio: f32,
  /// Per-sample biome and surface material data for the active terrain,
  /// baked once per terrain (and again when biome or river settings
  /// change). Drives the terrain shader, tree species, grass, and
  /// `biome_at`.
  surface: Vec<SurfaceSample>,
  /// Rivers and lakes carved into the active terrain.
  rivers: RiverNetwork,
  /// River options the current carving was built with, or `None` when no
  /// rivers are carved.
  applied_rivers: Option<RiverOptions>,
  /// Original heights of the samples raised into glacier surfaces.
  glaciers: Vec<(usize, f32)>,
  /// D8 drainage on the final heights while no rivers are carved, so
  /// valleys still hold denser forests (see `DrainageArea::d8`).
  drainage: crate::terrain::drainage::DrainageArea,
  /// What the tree and grass generators read about the ground: the same
  /// numbers as the per-terrain textures.
  ground: crate::render::vegetation::GroundData,
  /// The drawn channels, binned for the generators.
  channel_bins: Vec<u32>,
  /// Streamed trees and grass, and the radii their budgets allow.
  streams: crate::render::vegetation::Streams,
  /// Whether a frame has been drawn: the first is drawn without grass,
  /// which streams in after it.
  first_frame_drawn: bool,
  /// Seed for springs, meanders and deltas, from the terrain's heights.
  terrain_seed: u64,
  /// The landform the terrain was generated as, for its bedrock's beds
  /// (`soil::Strata`); continental for loaded heightmaps.
  landform: vista_types::LandformKind,
  /// Each boulder mesh variant's height per metre across, for sinking
  /// boulders into the ground.
  boulder_heights: [f32; crate::render::boulders::VARIANTS],
  /// The painted water mask, resampled to the terrain, if one is set.
  water_mask: Option<Vec<u8>>,
  /// The painted biome map, if one is set.
  biome_map: Option<crate::terrain::painted::PaintedBiomes>,
  /// The tree and grass density masks, resampled to the terrain, if set
  /// (see `painted::density_multiplier`).
  vegetation_masks: [Option<Vec<u8>>; 2],
  /// Where water can be heard.
  sounds: crate::water_sounds::SoundMap,
  /// Whether any sea on the terrain is cold enough to freeze.
  sea_ice_possible: bool,
  /// Terrain materials the ground uses, one bit per material.
  terrain_materials: u32,
  /// Root radius per tree species at scale 1, in metres, for grounding
  /// exported trees: the procedural models' or a replacement model's.
  /// Browser builds read the GPU's own.
  #[cfg(not(target_arch = "wasm32"))]
  tree_roots: [f32; crate::render::tree_models::SPECIES_COUNT],
  /// Cached per-sample normals for the active terrain, computed once when
  /// the terrain is installed and reused by every LOD mesh rebuild so the
  /// camera can recentre the mesh without repeating a full-heightmap pass.
  #[cfg(target_arch = "wasm32")]
  terrain_normals: Vec<vista_types::Vec3>,
  /// Heightmap-sample coordinates the LOD mesh was last centred on. `None`
  /// until a terrain is installed.
  #[cfg(target_arch = "wasm32")]
  mesh_centre_sample: Option<(f32, f32)>,
  /// The next camera-centred mesh, while it is being built a few rows per
  /// frame.
  #[cfg(target_arch = "wasm32")]
  mesh_stream: Option<MeshStream>,
  /// Camera position (heightmap samples) last frame, and its smoothed
  /// velocity in samples per second, for building the next mesh ahead of
  /// the camera.
  #[cfg(target_arch = "wasm32")]
  camera_track: Option<(f32, f32)>,
  #[cfg(target_arch = "wasm32")]
  camera_velocity: (f32, f32),
  #[cfg(target_arch = "wasm32")]
  gpu: crate::render::gpu::GpuContext,
}

/// Rows of the camera-centred terrain mesh built and uploaded per frame
/// while the next mesh streams in: about 33,000 vertices, so a rebuild is
/// spread over eight frames instead of stalling one.
#[cfg(target_arch = "wasm32")]
const MESH_ROWS_PER_FRAME: u32 = 64;

/// The next camera-centred terrain mesh, built a slice at a time.
#[cfg(target_arch = "wasm32")]
struct MeshStream {
  centre: (f32, f32),
  next_row: u32,
  scratch: Vec<crate::render::terrain_mesh::TerrainVertex>,
}

/// Weather values for the shaders, resolved from the weather state and
/// the enabled effects. The default leaves shading as it is without
/// weather.
#[derive(Clone, Copy, Debug, PartialEq)]
struct FrameWeatherValues {
  rain: f32,
  snow: f32,
  wetness: f32,
  snow_cover: f32,
  lightning: f32,
  overcast: f32,
  wind: [f32; 2],
  lightning_position: [f32; 2],
  heaviness: f32,
  lens_drops: bool,
  /// Low drifting snow, 0 to 1.
  blowing_snow: f32,
  /// The regional weather map: xy its corner, z 1 / its size in metres,
  /// w 1 when the clouds and wet ground read it.
  regional: [f32; 4],
  /// Aerosols: rgb the Mie colour (bluer in dry air), w the Mie phase
  /// asymmetry.
  air: [f32; 4],
  /// Light under cloud: x direct sun, y shadow strength, z indirect light,
  /// w how far sky light is the flat light under a cloud deck.
  light: [f32; 4],
  /// The gust front: x distance travelled, y gustiness, z mean wind in
  /// m/s, w 1 when on.
  gust: [f32; 4],
  /// x 1 when the surface weather map is read, y whitecap coverage, z
  /// blown spray, w unused.
  sea: [f32; 4],
  /// Added to the terrain shadows' softness under cloud.
  shadow_softening: f32,
}

impl Default for FrameWeatherValues {
  fn default() -> Self {
    Self {
      rain: 0.0,
      snow: 0.0,
      wetness: 0.0,
      snow_cover: 0.0,
      lightning: 0.0,
      overcast: 0.0,
      wind: [0.0; 2],
      lightning_position: [0.0; 2],
      heaviness: 0.0,
      lens_drops: false,
      blowing_snow: 0.0,
      regional: [0.0; 4],
      air: [1.0, 1.0, 1.0, 0.76],
      light: [1.0, 1.0, 1.0, 0.0],
      gust: [0.0; 4],
      sea: [0.0; 4],
      shadow_softening: 0.0,
    }
  }
}

/// Options after the weather has been applied.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
struct Weathered {
  atmosphere: AtmosphereOptions,
  water: WaterOptions,
  mist: MistOptions,
  clouds: CloudsOptions,
  flora: FloraOptions,
  weather: FrameWeatherValues,
}

/// A copy of `water` for one frame, without explicit river inflows, which
/// no frame reads: cloning their list would allocate every frame.
fn frame_water(water: &vista_types::WaterOptions) -> vista_types::WaterOptions {
  let rivers = &water.rivers;
  vista_types::WaterOptions {
    enabled: water.enabled,
    sea_level_metres: water.sea_level_metres,
    wave_scale: water.wave_scale,
    reflectivity: water.reflectivity,
    shoreline_softness_metres: water.shoreline_softness_metres,
    waves: water.waves.clone(),
    rivers: vista_types::RiverOptions {
      enabled: rivers.enabled,
      min_catchment_km2: rivers.min_catchment_km2,
      width_scale: rivers.width_scale,
      current_speed: rivers.current_speed,
      snowmelt: rivers.snowmelt,
      springs: rivers.springs,
      meanders: rivers.meanders,
      meander_maturity: rivers.meander_maturity,
      braiding: rivers.braiding,
      waterfalls: rivers.waterfalls,
      inflow: vista_types::RiverInflows::default(),
      riparian: rivers.riparian,
    },
    current_direction_degrees: water.current_direction_degrees,
    current_speed: water.current_speed,
    shallow_colour: water.shallow_colour,
    deep_colour: water.deep_colour,
    clarity_metres: water.clarity_metres,
    foam: water.foam,
    reflections: water.reflections,
    eddies: water.eddies,
    refraction: water.refraction,
  }
}

/// A copy of `flora` for one frame, without its species rules, which only
/// placement reads.
fn frame_flora(flora: &vista_types::FloraOptions) -> vista_types::FloraOptions {
  vista_types::FloraOptions {
    enabled: flora.enabled,
    density: flora.density,
    tree_line_metres: flora.tree_line_metres,
    seed_offset: flora.seed_offset,
    max_instances: flora.max_instances,
    tree_quality: flora.tree_quality,
    species_variation: flora.species_variation,
    wind_strength: flora.wind_strength,
    mesh_distance_metres: flora.mesh_distance_metres,
    species_rules: Vec::new(),
    variants_per_species: flora.variants_per_species,
  }
}

impl EngineCore {
  /// Create an engine core for tests without creating WebGPU resources.
  #[cfg(not(target_arch = "wasm32"))]
  pub fn new_for_tests(options: vista_types::VistaEngineOptions) -> VistaResult<Self> {
    let config = VistaEngineConfig::from_options(options)?;
    Self::from_config(config)
  }

  #[cfg(not(target_arch = "wasm32"))]
  fn from_config(config: VistaEngineConfig) -> VistaResult<Self> {
    let aspect = config.render.width as f32 / config.render.height as f32;
    let camera = CameraProjector::new(config.camera.clone(), aspect)?;

    Ok(Self {
      state: EngineState::Ready,
      terrain: None,
      next_terrain_id: 1,
      active_terrain_id: None,
      camera,
      sun: config.sun,
      atmosphere: config.atmosphere,
      water: config.water,
      flora: config.flora,
      grass: config.grass,
      clouds: config.clouds,
      mist: config.mist,
      quality: config.quality,
      biomes: config.biomes,
      lens_drops: crate::lens_drops::LensDrops::new(config.weather.seed_offset),
      weather: WeatherSystem::new(config.weather),
      day: crate::weather::sun::DayClock::new(Default::default()),
      surface_weather: Default::default(),
      surface_weather_pending: 0.0,
      surface_weather_settle: true,
      wave_heading: None,
      shadows: config.shadows,
      surface_options: config.surface,
      custom_trees: None,
      #[cfg(not(target_arch = "wasm32"))]
      placed_trees: Vec::new(),
      height_range: (0.0, 0.0),
      last_frame_ms: None,
      frame_clock: Default::default(),
      resolution: Default::default(),
      frame_seconds: 0.0,
      debug_view: DebugView::None,
      stats: RenderStats::default(),
      render_width: config.render.width,
      render_height: config.render.height,
      device_pixel_ratio: config.render.device_pixel_ratio.unwrap_or(1.0),
      surface: Vec::new(),
      rivers: RiverNetwork::default(),
      applied_rivers: None,
      glaciers: Vec::new(),
      drainage: Default::default(),
      ground: Default::default(),
      channel_bins: Vec::new(),
      streams: Default::default(),
      first_frame_drawn: false,
      terrain_seed: 0,
      landform: Default::default(),
      boulder_heights: crate::render::boulders::BoulderMeshes::build().heights,
      water_mask: None,
      biome_map: None,
      vegetation_masks: [None, None],
      sounds: Default::default(),
      sea_ice_possible: false,
      terrain_materials: 0,
      tree_roots: crate::render::tree_models::TreeSpecies::ALL
        .map(crate::render::flora::species_root_radius),
    })
  }

  #[cfg(target_arch = "wasm32")]
  /// Create an engine core for a browser canvas.
  pub async fn new(
    canvas: web_sys::HtmlCanvasElement,
    config: VistaEngineConfig,
  ) -> VistaResult<Self> {
    let aspect = config.render.width as f32 / config.render.height as f32;
    let camera = CameraProjector::new(config.camera.clone(), aspect)?;
    let gpu = crate::render::gpu::GpuContext::new(
      canvas,
      config.render.width,
      config.render.height,
      config.render.device_pixel_ratio.unwrap_or(1.0),
      config.shadows.trees.resolution,
      config.flora.variants_per_species as usize,
    )
    .await?;

    let mut core = Self {
      state: EngineState::Ready,
      terrain: None,
      next_terrain_id: 1,
      active_terrain_id: None,
      camera,
      sun: config.sun,
      atmosphere: config.atmosphere,
      water: config.water,
      flora: config.flora,
      grass: config.grass,
      clouds: config.clouds,
      mist: config.mist,
      quality: config.quality,
      biomes: config.biomes,
      lens_drops: crate::lens_drops::LensDrops::new(config.weather.seed_offset),
      weather: WeatherSystem::new(config.weather),
      day: crate::weather::sun::DayClock::new(Default::default()),
      surface_weather: Default::default(),
      surface_weather_pending: 0.0,
      surface_weather_settle: true,
      wave_heading: None,
      shadows: config.shadows,
      surface_options: config.surface,
      custom_trees: None,
      #[cfg(not(target_arch = "wasm32"))]
      placed_trees: Vec::new(),
      height_range: (0.0, 0.0),
      last_frame_ms: None,
      frame_clock: Default::default(),
      resolution: Default::default(),
      frame_seconds: 0.0,
      debug_view: DebugView::None,
      stats: RenderStats::default(),
      render_width: config.render.width,
      render_height: config.render.height,
      device_pixel_ratio: config.render.device_pixel_ratio.unwrap_or(1.0),
      surface: Vec::new(),
      rivers: RiverNetwork::default(),
      applied_rivers: None,
      glaciers: Vec::new(),
      drainage: Default::default(),
      ground: Default::default(),
      channel_bins: Vec::new(),
      streams: Default::default(),
      first_frame_drawn: false,
      terrain_seed: 0,
      landform: Default::default(),
      boulder_heights: crate::render::boulders::BoulderMeshes::build().heights,
      water_mask: None,
      biome_map: None,
      vegetation_masks: [None, None],
      sounds: Default::default(),
      sea_ice_possible: false,
      terrain_materials: 0,
      terrain_normals: Vec::new(),
      mesh_centre_sample: None,
      mesh_stream: None,
      camera_track: None,
      camera_velocity: (0.0, 0.0),
      gpu,
    };
    core
      .gpu
      .set_material_tints(&core.surface_options.material_tints);
    Ok(core)
  }

  /// Generate deterministic fractal terrain.
  ///
  /// Browser builds run erosion as GPU compute passes for performance;
  /// native builds (and any error recovering from a GPU erosion pass) use
  /// the CPU reference erosion in `terrain::erosion`.
  pub async fn generate_fractal(
    &mut self,
    options: FractalTerrainOptions,
  ) -> VistaResult<TerrainHandle> {
    self
      .generate_fractal_with_progress(options, &mut |_, _| {})
      .await
  }

  /// [`Self::generate_fractal`], reporting `(phase, progress)` as each
  /// generation stage advances. Phases are `"tectonics"`, `"drainage"`,
  /// `"detail"`, `"erosion"` (when erosion is requested), `"finishing"`
  /// (conditioning the map and building flora and the terrain mesh), and
  /// within it `"rivers"` (routing water and shaping channels, lakes and
  /// waterfalls).
  pub async fn generate_fractal_with_progress(
    &mut self,
    options: FractalTerrainOptions,
    progress: Progress<'_>,
  ) -> VistaResult<TerrainHandle> {
    self.ensure_live()?;
    let previous = self.begin_loading();
    let map = match self.generate_fractal_map(&options, progress).await {
      Ok(map) => map,
      Err(error) => return Err(self.end_loading(previous, error)),
    };
    self
      .install_loaded(map, options.landform, previous, progress)
      .await
  }

  #[cfg(target_arch = "wasm32")]
  async fn generate_fractal_map(
    &mut self,
    options: &FractalTerrainOptions,
    progress: Progress<'_>,
  ) -> VistaResult<HeightMap> {
    let mut map = crate::terrain::generate_fractal_heightmap_base_with_progress(options, progress)?;

    if let Some(erosion) = &options.erosion {
      let landform = crate::terrain::fractal::fractal_landform(options);

      match self
        .gpu
        .run_erosion(&map, erosion, &landform, progress)
        .await
      {
        Ok(eroded) => {
          map.heights = eroded;
        }
        Err(error) => {
          crate::terrain::erosion::apply_erosion(&mut map, erosion, &landform, progress)?;
          map.metadata.warnings.push(format!(
            "GPU erosion failed, so erosion ran on the CPU instead: {error}"
          ));
        }
      }
    }

    progress("finishing", 0.0);
    crate::terrain::finish_fractal_heightmap(&mut map, options);
    Ok(map)
  }

  #[cfg(not(target_arch = "wasm32"))]
  async fn generate_fractal_map(
    &mut self,
    options: &FractalTerrainOptions,
    progress: Progress<'_>,
  ) -> VistaResult<HeightMap> {
    generate_fractal_heightmap_with_progress(options, progress)
  }

  /// Decode an uncompressed GeoTIFF from bytes.
  pub async fn load_dem_from_array_buffer(
    &mut self,
    bytes: &[u8],
    options: DemLoadOptions,
  ) -> VistaResult<TerrainHandle> {
    self.ensure_live()?;
    let previous = self.begin_loading();
    let map = match decode_geotiff(bytes, &options) {
      Ok(map) => map,
      Err(error) => return Err(self.end_loading(previous, error)),
    };
    self
      .install_loaded(map, Default::default(), previous, &mut |_, _| {})
      .await
  }

  /// Decode a raw heightmap from bytes.
  pub async fn load_raw_heightmap<B: crate::dem::RawBytes + ?Sized>(
    &mut self,
    bytes: &B,
    options: RawHeightmapOptions,
  ) -> VistaResult<TerrainHandle> {
    self.ensure_live()?;
    let previous = self.begin_loading();
    let map = match decode_raw_heightmap(bytes, &options) {
      Ok(map) => map,
      Err(error) => return Err(self.end_loading(previous, error)),
    };
    self
      .install_loaded(
        map,
        options.landform.unwrap_or_default(),
        previous,
        &mut |_, _| {},
      )
      .await
  }

  /// Mark the engine as loading terrain, and return the state to restore
  /// if the load fails.
  fn begin_loading(&mut self) -> EngineState {
    std::mem::replace(&mut self.state, EngineState::LoadingTerrain)
  }

  /// Leave the loading state after `error`, as the engine was before:
  /// a failed load changes nothing, so the engine stays usable.
  fn end_loading(&mut self, previous: EngineState, error: VistaError) -> VistaError {
    self.state = previous;
    error
  }

  /// The shared end of every load: wait for the GPU, then make `map` the
  /// active terrain. Nothing has changed until the wait succeeds.
  async fn install_loaded(
    &mut self,
    map: HeightMap,
    landform: vista_types::LandformKind,
    previous: EngineState,
    progress: Progress<'_>,
  ) -> VistaResult<TerrainHandle> {
    if let Err(error) = self.finish_gpu_work().await {
      return Err(self.end_loading(previous, error));
    }

    // The upload creates the terrain's largest GPU resources, so errors
    // there are reported as `"gpuError"` events rather than reach only
    // the console. They arrive once the GPU has caught up, after the
    // call: waiting for them would hold every terrain call back.
    #[cfg(target_arch = "wasm32")]
    let scopes = self.gpu.begin_error_scopes();
    self.landform = landform;
    let handle = self.install_terrain(map, progress);
    progress("finishing", 1.0);
    self.state = EngineState::Ready;
    #[cfg(target_arch = "wasm32")]
    self.gpu.end_error_scopes_later(scopes);
    Ok(handle)
  }

  /// GPU errors no error scope caught since the last call, and why the
  /// device was lost, once (see `GpuContext::take_events`). Native builds
  /// have no GPU.
  pub fn take_gpu_events(&self) -> (Vec<String>, Option<String>) {
    #[cfg(target_arch = "wasm32")]
    return self.gpu.take_events();
    #[cfg(not(target_arch = "wasm32"))]
    (Vec::new(), None)
  }

  /// Wait for the GPU to finish the work submitted so far (engine
  /// start-up, erosion), so the terrain upload that follows does not
  /// freeze the page while it waits. Native builds have no GPU.
  async fn finish_gpu_work(&self) -> VistaResult<()> {
    #[cfg(target_arch = "wasm32")]
    self.gpu.finish_submitted_work().await?;

    Ok(())
  }

  /// Replace the active camera.
  pub fn set_camera(&mut self, camera: CameraOptions) -> VistaResult<()> {
    self.ensure_live()?;
    let aspect = self.render_width as f32 / self.render_height.max(1) as f32;
    self.camera.set_camera(camera, aspect)
  }

  /// Replace sun controls.
  pub fn set_sun(&mut self, sun: SunOptions) -> VistaResult<()> {
    self.ensure_live()?;
    crate::config::validate_sun(&sun)?;
    use crate::render::flora::poleward_of_sun;
    // Shaded slopes keep more moisture and trees; they only change sides
    // when the sun's mean position crosses to the other half of the sky.
    let flipped = poleward_of_sun(sun.azimuth_degrees) != poleward_of_sun(self.sun.azimuth_degrees);
    self.sun = sun;

    if flipped {
      self.refresh_flora();
    }

    Ok(())
  }

  /// Replace atmosphere controls.
  pub fn set_atmosphere(&mut self, atmosphere: AtmosphereOptions) -> VistaResult<()> {
    self.ensure_live()?;
    crate::config::validate_atmosphere(&atmosphere)?;
    self.atmosphere = atmosphere;
    Ok(())
  }

  /// Replace water controls.
  ///
  /// Wave, colour, and current changes only update shader parameters.
  /// Changing river settings (or toggling water) re-extracts and re-carves
  /// the river network, which re-bakes terrain shading.
  pub fn set_water(&mut self, water: WaterOptions) -> VistaResult<()> {
    self.ensure_live()?;
    crate::config::validate_water(&water)?;

    if let (Some(terrain), vista_types::RiverInflows::List(list)) =
      (self.terrain.as_ref(), &water.rivers.inflow)
    {
      let metres = terrain.metadata.metres_per_sample.max(0.001);
      let half_x = (terrain.metadata.width as f32 - 1.0) * metres * 0.5;
      let half_z = (terrain.metadata.height as f32 - 1.0) * metres * 0.5;

      for (index, inflow) in list.iter().enumerate() {
        let [x, z] = inflow.position;

        if x.abs() > half_x || z.abs() > half_z {
          return Err(VistaError::options(format!(
            "water.rivers.inflow[{index}].position ({x}, {z}) is off the map, which spans -{half_x} to {half_x} m in x and -{half_z} to {half_z} m in z."
          )));
        }
      }
    }
    let rivers_changed = self.wanted_rivers(&water) != self.applied_rivers;
    self.water = water;

    if rivers_changed {
      self.rebuild_world();
    } else {
      self.refresh_water();
    }

    Ok(())
  }

  /// Replace biome controls and re-bake terrain shading, trees, and grass.
  /// Glaciers reshape the ground they cover, so the terrain (and the rivers
  /// carved into it) is rebuilt too.
  pub fn set_biomes(&mut self, biomes: BiomeOptions) -> VistaResult<()> {
    self.ensure_live()?;
    crate::config::validate_biomes(&biomes)?;

    if self.biomes == biomes {
      return Ok(());
    }

    self.biomes = biomes;
    self.rebuild_world();
    Ok(())
  }

  /// Return the biome at a world-space position, or `None` when there is
  /// no terrain or the position is outside it.
  pub fn biome_at(&self, world_x: f32, world_z: f32) -> Option<BiomeKind> {
    self
      .surface_at(world_x, world_z)
      .map(|sample| sample.biome_kind())
  }

  /// Return the mean annual temperature in °C at a world-space position,
  /// or `None` when there is no terrain or the position is outside it.
  pub fn celsius_at(&self, world_x: f32, world_z: f32) -> Option<f32> {
    self
      .surface_at(world_x, world_z)
      .map(|sample| sample.celsius())
  }

  fn surface_at(&self, world_x: f32, world_z: f32) -> Option<&SurfaceSample> {
    let terrain = self.terrain.as_ref()?;
    let metres_per_sample = terrain.metadata.metres_per_sample.max(0.001);
    let sample_x = world_x / metres_per_sample + (terrain.metadata.width as f32 - 1.0) * 0.5;
    let sample_z = world_z / metres_per_sample + (terrain.metadata.height as f32 - 1.0) * 0.5;

    if !sample_x.is_finite()
      || !sample_z.is_finite()
      || sample_x < -0.5
      || sample_z < -0.5
      || sample_x > terrain.metadata.width as f32 - 0.5
      || sample_z > terrain.metadata.height as f32 - 0.5
    {
      return None;
    }

    let x = (sample_x.round() as u32).min(terrain.metadata.width - 1);
    let z = (sample_z.round() as u32).min(terrain.metadata.height - 1);
    self.surface.get((z * terrain.metadata.width + x) as usize)
  }

  /// Replace flora controls.
  pub fn set_flora(&mut self, flora: FloraOptions) -> VistaResult<()> {
    self.ensure_live()?;
    crate::config::validate_flora(&flora)?;
    #[cfg(target_arch = "wasm32")]
    self
      .gpu
      .set_tree_variants(flora.variants_per_species as usize);
    self.flora = flora;
    self.refresh_flora();
    // The forest floor follows the trees' canopy.
    self.refresh_grass();
    Ok(())
  }

  /// Replace grass controls.
  pub fn set_grass(&mut self, grass: GrassOptions) -> VistaResult<()> {
    self.ensure_live()?;
    crate::config::validate_grass(&grass)?;
    self.grass = grass;
    self.refresh_grass();
    Ok(())
  }

  /// Replace cloud controls. Clouds have no terrain-dependent placement,
  /// so this only replaces state — the next `render_once()` picks up the
  /// new values, mirroring `set_sun`/`set_atmosphere`. Cloud drift and
  /// billowing are animated entirely on the GPU from the frame clock.
  pub fn set_clouds(&mut self, clouds: CloudsOptions) -> VistaResult<()> {
    self.ensure_live()?;
    crate::config::validate_clouds(&clouds)?;
    self.clouds = clouds;
    Ok(())
  }

  /// Replace mist/ground-fog controls. Like clouds, mist has no
  /// terrain-dependent placement.
  pub fn set_mist(&mut self, mist: MistOptions) -> VistaResult<()> {
    self.ensure_live()?;
    crate::config::validate_mist(&mist)?;
    self.mist = mist;
    Ok(())
  }

  /// Replace render quality controls.
  pub fn set_render_quality(&mut self, quality: RenderQualityOptions) -> VistaResult<()> {
    self.ensure_live()?;
    crate::config::validate_quality(&quality)?;
    self.quality = quality;
    self.refresh_flora();
    self.refresh_grass();
    Ok(())
  }

  /// Replace the debug overlay mode.
  pub fn set_debug_view(&mut self, debug_view: DebugView) -> VistaResult<()> {
    self.ensure_live()?;
    self.debug_view = debug_view;
    Ok(())
  }

  /// Replace weather controls. Changing `state` blends towards the new
  /// weather over `transitionSeconds` instead of jumping.
  pub fn set_weather(&mut self, weather: WeatherOptions) -> VistaResult<()> {
    self.ensure_live()?;
    crate::config::validate_weather(&weather)?;
    self.weather.set_options(weather);
    Ok(())
  }

  /// Return the current blended weather, or `None` when the weather system
  /// is off.
  pub fn weather(&self) -> Option<WeatherState> {
    if self.weather.options().enabled {
      Some(self.weather.state().clone())
    } else {
      None
    }
  }

  /// Every resolved weather preset, built in and custom, by name.
  pub fn weather_presets(&self) -> vista_types::NamedMap<vista_types::ResolvedWeatherPreset> {
    self
      .weather
      .table()
      .public(self.weather.options().state_duration_seconds)
  }

  /// The weather at a world position, for gameplay and audio, or `None`
  /// with no terrain. Beyond the terrain the ground values are 0. With the
  /// weather off, only the coverage is set, from the clouds' own settings.
  pub fn weather_at(&self, x: f32, z: f32) -> Option<vista_types::LocalWeather> {
    let terrain = self.terrain.as_ref()?;

    if !x.is_finite() || !z.is_finite() {
      return None;
    }

    if !self.weather.options().enabled {
      let coverage = if self.clouds.style == vista_types::CloudStyle::Off {
        0.0
      } else {
        self.clouds.coverage.clamp(0.0, 1.0)
      };
      return Some(vista_types::LocalWeather {
        coverage,
        ..Default::default()
      });
    }

    let sample = self.weather.regional().evaluate(x, z);
    let mut local = vista_types::LocalWeather {
      coverage: sample.coverage,
      precipitation: sample.precipitation,
      storminess: sample.storminess,
      humidity: sample.humidity,
      ..Default::default()
    };

    if self.weather.options().effects.ground {
      if let Some((cell, ground)) = self.surface_weather.at(x, z) {
        let (slope, hollow) = crate::weather::surface_relief(terrain, x, z);
        local.wetness = cell.wetness;
        local.puddles = cell.puddles * crate::weather::surface::puddle_capacity(slope, hollow);
        local.snow_depth = cell.snow.max(ground.permanent_snow);
      }
    }

    Some(local)
  }

  /// Run the weather forward by `seconds` at once, in one-second steps:
  /// the weather cycle, the time of day, and the wet and snowy ground.
  pub fn advance_weather(&mut self, seconds: f32) -> VistaResult<()> {
    self.ensure_live()?;

    if !seconds.is_finite() || !(0.0..=MAX_WEATHER_ADVANCE_SECONDS).contains(&seconds) {
      return Err(VistaError::options(format!(
        "advanceWeather seconds must be a finite number from 0 to {MAX_WEATHER_ADVANCE_SECONDS}, but it is {seconds}."
      )));
    }

    // The ground steps in chunks, each with the rain of its own time; at
    // most 240 of them, so a whole day stays quick.
    let chunk = (seconds / 240.0).max(60.0);
    let mut remaining = seconds;
    self.weather.set_refresh_grid(false);

    while remaining > 0.0 {
      let step = remaining.min(1.0);
      self.step_weather(step);
      remaining -= step;

      if self.surface_weather_pending >= chunk || remaining <= 0.0 {
        self.weather.refresh_terrain_rows(self.terrain_half());

        #[cfg(target_arch = "wasm32")]
        if let Some(upload) = self.weather.regional_mut().take_upload() {
          self.gpu.upload_regional_weather(upload);
        }

        self.step_surface_weather(true);
      }
    }

    self.weather.set_refresh_grid(true);
    self.weather.refresh_all_rows();
    Ok(())
  }

  /// Replace the time of day controls. While the time of day is enabled
  /// it sets the sun's azimuth and elevation, and `setSun` only its
  /// intensity.
  pub fn set_time_of_day(&mut self, options: vista_types::TimeOfDayOptions) -> VistaResult<()> {
    self.ensure_live()?;
    crate::config::validate_time_of_day(&options)?;
    self.day.set_options(options);
    Ok(())
  }

  /// The time of day and where it puts the sun.
  pub fn time_of_day(&self) -> vista_types::TimeOfDay {
    self.day.report()
  }

  /// The sun as it lights the scene: the time of day's position when it
  /// runs, otherwise `setSun`'s.
  fn sun_now(&self) -> SunOptions {
    if !self.day.enabled() {
      return self.sun.clone();
    }

    let sun = self.day.sun();
    SunOptions {
      azimuth_degrees: crate::weather::sun::engine_azimuth(sun.azimuth_degrees),
      elevation_degrees: sun.elevation_degrees,
      intensity: self.sun.intensity,
    }
  }

  /// Unit vector towards the sun.
  fn sun_vector(&self) -> [f32; 3] {
    let sun = self.sun_now();
    crate::maths::sun_direction_vector(sun.azimuth_degrees, sun.elevation_degrees)
  }

  fn terrain_half(&self) -> [f32; 2] {
    self.terrain.as_ref().map_or([0.0; 2], |terrain| {
      let metres = terrain.metadata.metres_per_sample.max(0.001);
      [
        (terrain.metadata.width as f32 - 1.0) * metres * 0.5,
        (terrain.metadata.height as f32 - 1.0) * metres * 0.5,
      ]
    })
  }

  /// Advance the time of day and the weather by `dt` seconds.
  fn step_weather(&mut self, dt: f32) {
    self.day.advance(dt);

    if !self.weather.options().enabled {
      self.stats.weather = None;
      self.surface_weather_settle = true;
      return;
    }

    let camera = self.camera.options.position;
    self
      .weather
      .set_celsius(self.celsius_at(camera[0], camera[2]));
    self.weather.set_camera(camera);
    self.weather.set_sky(crate::weather::Sky {
      sun: self.sun_vector(),
      cloud_base: self.clouds.height_metres,
      cloud_thickness: self.clouds.thickness_metres * self.weather.cloud_thickness_scale(),
    });
    let sunset = self.day.sunrise_sunset().map(|(_, set)| set);
    let day = self.day.enabled().then(|| crate::weather::DayInfo {
      hours: self.day.hours(),
      sunset,
      hours_per_second: self.day.hours_per_second(),
    });
    self.weather.set_day(day);
    self.weather.advance(dt);
    self.stats.weather = Some(self.weather.dominant());
    self.surface_weather_pending += dt;
    let wind_direction = self.weather.state().wind_direction_degrees;
    self.wave_heading = Some(match self.wave_heading {
      Some(heading) => crate::weather::wind::turn_waves(heading, wind_direction, dt),
      None => wind_direction,
    });

    // The ground reports what the surface map holds under the camera.
    if let (Some(terrain), Some((cell, _))) = (
      self.terrain.as_ref(),
      self.surface_weather.at(camera[0], camera[2]),
    ) {
      let (slope, hollow) = crate::weather::surface_relief(terrain, camera[0], camera[2]);
      let capacity = crate::weather::surface::puddle_capacity(slope, hollow);
      self.weather.report_ground(cell, capacity);
    }
  }

  /// The open water upwind of the camera, in metres, up to the longest
  /// fetch the sea state counts: the sea beyond the map is open.
  fn upwind_fetch(&self, wind_direction_degrees: f32) -> f32 {
    use crate::weather::wind::MAX_FETCH_METRES;
    let Some(terrain) = self.terrain.as_ref() else {
      return MAX_FETCH_METRES;
    };
    let metres = terrain.metadata.metres_per_sample.max(0.001);
    let sea = terrain.metadata.sea_level_metres;
    let radians = wind_direction_degrees.to_radians();
    let upwind = [-radians.sin(), -radians.cos()];
    let camera = self.camera.options.position;
    let step = (metres * 4.0).max(25.0);
    let mut travelled = 0.0;
    let mut over_water = 0.0;

    while travelled < MAX_FETCH_METRES * 2.0 {
      let x = camera[0] + upwind[0] * travelled;
      let z = camera[2] + upwind[1] * travelled;

      match world_height(terrain, x, z) {
        None => return (over_water + MAX_FETCH_METRES).min(MAX_FETCH_METRES),
        Some(height) if height <= sea => over_water += step,
        // Land ends the fetch, once the camera is over water.
        Some(_) if over_water > 0.0 => break,
        Some(_) => {}
      }

      if over_water >= MAX_FETCH_METRES {
        break;
      }

      travelled += step;
    }

    over_water.min(MAX_FETCH_METRES)
  }

  /// Step the wet and snowy ground by the weather time since its last
  /// step, on the CPU mirror and, in the browser, on the GPU.
  fn step_surface_weather(&mut self, now: bool) {
    let dt = std::mem::take(&mut self.surface_weather_pending);
    let options = self.weather.options();

    if !options.enabled || !options.effects.ground || dt <= 0.0 {
      return;
    }

    let settle = std::mem::take(&mut self.surface_weather_settle);
    let shared = self.surface_inputs();
    let regional = self.weather.options().regional;
    let uniform = self.weather.local().precipitation;
    let mean = self.weather.regional().params().precipitation;
    let field = self.weather.regional();
    let precipitation = |[x, z]: [f32; 2]| {
      if regional {
        field.grid_precipitation(x, z)
      } else {
        uniform
      }
    };

    if settle {
      let ground = self.weather.settled_ground();
      self.surface_weather.settle(ground, mean, precipitation);
    }

    self.surface_weather.step(dt, &shared, precipitation);

    #[cfg(target_arch = "wasm32")]
    self.gpu.step_surface_weather(
      &crate::render::gpu::SurfaceWeatherStep {
        dt,
        settle: settle.then(|| self.weather.settled_ground()),
        sun: shared.sun,
        wind: shared.wind,
        celsius_offset: shared.celsius,
        precipitation: uniform,
        mean_precipitation: mean,
        regional: regional.then(|| self.weather.regional().size_metres()),
        canopy: crate::render::lattice::canopy_density(self.tree_density()),
      },
      now,
    );
    #[cfg(not(target_arch = "wasm32"))]
    let _ = now;
  }

  /// What every place on the ground shares this step: the sunlight, the
  /// mean wind and the weather's temperature offset.
  fn surface_inputs(&self) -> crate::weather::surface::SurfaceInputs {
    use crate::weather::presets::field;
    let sun = self.sun_vector();
    crate::weather::surface::SurfaceInputs {
      sun: (sun[1].max(0.0) * self.weather.sun_transmittance()).min(1.0),
      wind: self.weather.mean_wind(),
      celsius: self.weather.values().get(field::TEMPERATURE),
      ..Default::default()
    }
  }

  /// Rebuild the CPU mirror of the surface weather for the active
  /// terrain, keeping its state when the grid is unchanged.
  fn rebuild_surface_weather(&mut self) {
    let Some(terrain) = self.terrain.as_ref() else {
      self.surface_weather = Default::default();
      return;
    };
    let ground = &self.ground;

    if ground.width == 0 || ground.surface.len() != (ground.width * ground.height) as usize {
      return;
    }

    let longest = ground.width.max(ground.height) as usize;
    let scale = (SURFACE_MIRROR_CELLS as f32 / longest as f32).min(1.0);
    let width = ((ground.width as f32 * scale).round() as usize).max(1);
    let height = ((ground.height as f32 * scale).round() as usize).max(1);
    let canopy = crate::render::lattice::canopy_density(self.tree_density());
    let half = self.terrain_half();
    let mut cells = Vec::with_capacity(width * height);

    for y in 0..height {
      for x in 0..width {
        let tx = ((x as f32 + 0.5) / width as f32 * ground.width as f32) as usize;
        let ty = ((y as f32 + 0.5) / height as f32 * ground.height as f32) as usize;
        let index = ty.min(ground.height as usize - 1) * ground.width as usize
          + tx.min(ground.width as usize - 1);
        let surface = ground.surface[index];
        let red = ground.cover.get(index).map_or(0.0, |cover| cover[0] as f32);
        let world_x = ((x as f32 + 0.5) / width as f32 * 2.0 - 1.0) * half[0];
        let world_z = ((y as f32 + 0.5) / height as f32 * 2.0 - 1.0) * half[1];
        let (slope_degrees, hollow_metres) =
          crate::weather::surface_relief(terrain, world_x, world_z);
        cells.push(crate::weather::surface::CellGround {
          celsius: crate::terrain::biomes::unit_to_celsius(surface[0] as f32 / 255.0),
          canopy: crate::render::lattice::canopy_cover(red, canopy),
          slope_degrees,
          hollow_metres,
          permanent_snow: surface[2] as f32 / 255.0,
        });
      }
    }

    let mirror = &mut self.surface_weather;

    if mirror.width == width && mirror.height == height && mirror.half == half {
      mirror.ground = cells;
    } else {
      *mirror =
        crate::weather::surface::SurfaceMirror::new(width, height, half, cells, Default::default());
      self.surface_weather_settle = true;
    }
  }

  /// Replace shadow controls.
  pub fn set_shadows(&mut self, shadows: ShadowOptions) -> VistaResult<()> {
    self.ensure_live()?;
    crate::config::validate_shadows(&shadows)?;
    self.shadows = shadows;
    Ok(())
  }

  /// Replace terrain surface controls.
  pub fn set_surface(&mut self, surface: SurfaceOptions) -> VistaResult<()> {
    self.ensure_live()?;
    crate::config::validate_surface(&surface)?;

    #[cfg(target_arch = "wasm32")]
    self.gpu.set_material_tints(&surface.material_tints);

    // Rockiness changes the soil, so the ground and everything growing on
    // it are baked again. Trees and grass leave room for boulders only
    // while there are boulders.
    let rebake = surface.rockiness != self.surface_options.rockiness;
    let boulders = surface.boulders != self.surface_options.boulders;
    let distance = surface.boulder_distance_metres != self.surface_options.boulder_distance_metres;
    self.surface_options = surface;

    if rebake {
      self.rebake_surface(None);
    } else if boulders {
      self.refresh_flora();
      self.refresh_grass();
      self.refresh_boulders();
    } else if distance {
      self.refresh_boulders();
    }

    Ok(())
  }

  /// Replace the procedural trees with host-supplied instances, or restore
  /// procedural placement with `None`. Custom trees stay in place when the
  /// terrain, biomes, or flora options change, until they are cleared.
  pub fn set_tree_instances(&mut self, trees: Option<Vec<TreeInstance>>) -> VistaResult<()> {
    self.ensure_live()?;

    if let Some(trees) = &trees {
      validate_tree_instances(trees)?;
    }

    self.custom_trees = trees;
    self.refresh_flora();
    self.refresh_grass();
    Ok(())
  }

  /// Replace one species' model with a host-supplied mesh. See
  /// [`mesh_from_arrays`] for the array layout.
  #[allow(clippy::too_many_arguments)]
  pub fn set_tree_model(
    &mut self,
    species: TreeSpeciesKind,
    positions: &[f32],
    normals: &[f32],
    uvs: &[f32],
    indices: &[u32],
    texture_layers: Option<&[f32]>,
    wind: Option<&[f32]>,
  ) -> VistaResult<()> {
    self.ensure_live()?;
    let mesh = mesh_from_arrays(positions, normals, uvs, indices, texture_layers, wind)
      .map_err(|message| VistaError::options(format!("setTreeModel: {message}")))?;
    self.install_tree_model(species, Some(mesh));
    Ok(())
  }

  /// Restore the procedural model for one species.
  pub fn reset_tree_model(&mut self, species: TreeSpeciesKind) -> VistaResult<()> {
    self.ensure_live()?;
    self.install_tree_model(species, None);
    Ok(())
  }

  fn install_tree_model(&mut self, species: TreeSpeciesKind, mesh: Option<TreeMesh>) {
    #[cfg(target_arch = "wasm32")]
    self.gpu.set_tree_model(species.index(), mesh);

    // As `build_library` roots them for the GPU.
    #[cfg(not(target_arch = "wasm32"))]
    {
      use crate::render::tree_models::TreeSpecies;
      self.tree_roots[species.index()] = mesh.as_ref().map_or(
        crate::render::flora::species_root_radius(TreeSpecies::ALL[species.index()]),
        |mesh| mesh.trunk_radius() * crate::render::flora::ROOT_RADII,
      );
    }
  }

  /// Replace one layer of a baked texture array with RGBA8 texels
  /// (`TEXTURE_LAYER_SIZE` square, row-major, top row first).
  pub fn replace_texture(
    &mut self,
    target: TextureTarget,
    layer: u32,
    rgba: &[u8],
  ) -> VistaResult<()> {
    self.ensure_live()?;
    let layers = texture_layers(target);

    if layer >= layers {
      return Err(VistaError::options(format!(
        "texture layer must be between 0 and {}.",
        layers - 1
      )));
    }

    let expected = (TEXTURE_LAYER_SIZE * TEXTURE_LAYER_SIZE * 4) as usize;

    if rgba.len() != expected {
      return Err(VistaError::options(format!(
        "texture data must be {TEXTURE_LAYER_SIZE} x {TEXTURE_LAYER_SIZE} RGBA ({expected} bytes), but {} bytes were given.",
        rgba.len()
      )));
    }

    #[cfg(target_arch = "wasm32")]
    self.gpu.replace_texture_layer(target, layer, rgba);

    Ok(())
  }

  /// Discard every replaced texture layer and restore the procedural
  /// textures.
  pub fn reset_textures(&mut self) -> VistaResult<()> {
    self.ensure_live()?;

    #[cfg(target_arch = "wasm32")]
    self.gpu.reset_textures();

    Ok(())
  }

  /// Render one frame. The stats are borrowed: copying them would copy a
  /// custom weather preset's name every frame.
  pub fn render_once(&mut self) -> VistaResult<&RenderStats> {
    self.ensure_live()?;

    // While the GPU is still drawing earlier frames, skip this one instead
    // of queueing it. The unchanged `frame_index` tells the caller that
    // nothing was drawn.
    #[cfg(target_arch = "wasm32")]
    if self.gpu.is_busy() {
      return Ok(&self.stats);
    }

    self.stats.frame_index = self.stats.frame_index.saturating_add(1);
    let interval = self.frame_delta_seconds();
    let dt = self.frame_clock.step(interval);
    self.frame_seconds = dt;
    self.stats.render_scale = self.resolution.update(
      interval,
      self.quality.frame_rate_cap(),
      self.quality.render_scale_range(),
    );

    self.step_weather(dt);

    if self.surface_weather_pending >= SURFACE_WEATHER_INTERVAL {
      self.step_surface_weather(false);
    }

    #[cfg(target_arch = "wasm32")]
    if let Some(upload) = self.weather.regional_mut().take_upload() {
      self.gpu.upload_regional_weather(upload);
    }

    // Drops land while it rains and keep running off or evaporating after.
    let options = self.weather.options();
    // Only rain adds drops: snow and sleet do not bead on the lens.
    let intensity = if options.enabled && options.lens_drops && options.effects.precipitation {
      self.weather.state().rain * self.weather.precipitation_heaviness().max(1.0)
    } else {
      0.0
    };
    self.lens_drops.advance(
      dt,
      intensity,
      self.render_width as f32 / self.render_height.max(1) as f32,
      &crate::lens_drops::LensDropSettings {
        count: options.lens_drop_count,
        min_size: options.lens_drop_min_size,
        max_size: options.lens_drop_max_size,
      },
    );

    // Native builds have no GPU mesh to measure, so report a theoretical
    // estimate based on the configured clipmap level budget. Browser
    // builds report the real uploaded mesh below instead.
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(terrain) = &self.terrain {
      let levels = clipmap_levels(
        terrain.metadata.width,
        terrain.metadata.metres_per_sample,
        self.quality.max_clipmap_levels.unwrap_or(7),
      );
      (self.stats.clipmap_levels, self.stats.terrain_triangles) = levels
        .fold((0, 0), |(count, triangles), level| {
          (count + 1, triangles + level.index_count / 3)
        });
    }

    #[cfg(target_arch = "wasm32")]
    {
      self.recentre_terrain_mesh_if_needed(dt);
      let view_proj =
        crate::maths::mat4_multiply(self.camera.projection_matrix, self.camera.view_matrix);
      let budget = self.quality.vegetation();
      let tree_style = self.frame_tree_style();

      (self.stats.tree_growth_ms, self.stats.tree_bake_ms) = self.gpu.tree_timings();

      if let Some(draw) = self.gpu.tree_draw(tree_style) {
        self.stats.tree_triangles = draw.triangles().min(u64::from(u32::MAX)) as u32;
        self.streams.triangles.observe(
          &draw,
          budget.max_tree_triangles,
          self.flora.mesh_distance_metres,
          self.shadows.trees.distance_metres,
          dt,
        );
      }

      // With vegetation to shed, detail gives way before resolution.
      self.resolution.shed_detail_first(
        self.streams.trees.is_some()
          || self.streams.grass.is_some()
          || self.streams.boulders.is_some(),
      );
      self.streams.waiting = self.gpu.generators_ready().map(|ready| !ready);
      let vegetation = self.streams.update(
        &self.ground,
        self.camera.options.position,
        &crate::maths::frustum_planes(&view_proj),
        &budget,
        self.resolution.detail_pressure(),
        dt,
        self.grass.view_distance_metres,
        self.flora.mesh_distance_metres,
        self.shadows.trees.distance_metres,
        self
          .pipeline_needs()
          .0
          .boulders
          .then_some(self.surface_options.boulder_distance_metres),
      );
      self.stats.flora_instances = match &self.custom_trees {
        Some(custom) => custom.len() as u32,
        None => vegetation.trees_drawn,
      };
      self.stats.grass_instances = vegetation.grass_drawn;
      let mut params = self.frame_params();
      params.vegetation = vegetation;
      self.gpu.render_once(&params)?;
      self.stats.gpu_pass_times_ms = self.gpu.pass_times();
      self.stats.gpu_frame_time_ms = self.stats.gpu_pass_times_ms.map(|times| {
        times.shadows
          + times.tree_culling
          + times.terrain
          + times.trees
          + times.grass
          + times.boulders
          + times.clouds
          + times.sky_and_fog
          + times.water
          + times.present
          + times.surface_weather
      });
    }

    self.first_frame_drawn = true;
    Ok(&self.stats)
  }

  /// Resize the renderer.
  pub fn resize(
    &mut self,
    width: u32,
    height: u32,
    device_pixel_ratio: Option<f32>,
  ) -> VistaResult<()> {
    self.ensure_live()?;
    let ratio = device_pixel_ratio.unwrap_or(self.device_pixel_ratio);
    crate::config::validate_surface_size("resize", width, height, ratio)?;
    self.render_width = width;
    self.render_height = height;
    self.device_pixel_ratio = ratio;

    #[cfg(target_arch = "wasm32")]
    self.gpu.resize(width, height, ratio)?;

    let aspect = width as f32 / height.max(1) as f32;
    self.camera = CameraProjector::new(self.camera.options.clone(), aspect)?;
    Ok(())
  }

  /// Export the active heightmap as little-endian `f32` bytes.
  pub fn export_heightmap(&self) -> VistaResult<Vec<u8>> {
    self.ensure_live()?;
    let terrain = self.terrain.as_ref().ok_or_else(|| {
      VistaError::TerrainGenerationFailed("no active terrain exists.".to_string())
    })?;

    Ok(terrain.export_f32_le())
  }

  /// Export one of the maps the renderer builds (see `export.rs`), at the
  /// terrain's own size or resampled to `size`.
  pub fn export_map(
    &self,
    kind: crate::export::MapKind,
    size: Option<[u32; 2]>,
  ) -> VistaResult<crate::export::ExportedMap> {
    use crate::export::{MapData, MapEncoding, MapKind};

    self.ensure_live()?;
    let terrain = self.terrain.as_ref().ok_or_else(|| {
      VistaError::options("exportMap needs a terrain: generate or load one first.")
    })?;
    let (width, height) = (terrain.metadata.width, terrain.metadata.height);
    let count = (width * height) as usize;
    let channels = kind.channels() as usize;
    let bytes = if kind.is_float() { 4 } else { 1 } * kind.channels();
    let [out_width, out_height] = size.unwrap_or([width, height]);

    match size {
      Some(size) => crate::config::validate_export_size(size, bytes)?,
      None
        if u64::from(width) * u64::from(height) * u64::from(bytes)
          > crate::config::MAX_EXPORT_BYTES =>
      {
        return Err(VistaError::options(format!(
          "exportMap at the terrain's own size ({width} x {height}) would need over 1 GiB. Pass a smaller size."
        )));
      }
      None => {}
    }

    if width < 2 || height < 2 || self.surface.len() != count {
      return Err(VistaError::internal(
        "the terrain's surface data is missing, so its maps cannot be exported.",
      ));
    }

    let metres = terrain.metadata.metres_per_sample.max(0.001);
    let half = [
      (width as f32 - 1.0) * metres * 0.5,
      (height as f32 - 1.0) * metres * 0.5,
    ];
    let at = |index: usize| (index as u32 % width, index as u32 / width);
    let world = |index: usize| {
      let (x, z) = at(index);
      (x as f32 * metres - half[0], z as f32 * metres - half[1])
    };
    let surface = &self.surface;
    let (water, depth) =
      if matches!(kind, MapKind::Water | MapKind::WaterDepth) && self.water.enabled {
        crate::export::water_layers(terrain, &self.rivers, self.water_mask.as_deref())
      } else {
        (Vec::new(), Vec::new())
      };
    let discharge = if kind == MapKind::Discharge {
      self.discharge_grid(terrain)
    } else {
      Default::default()
    };
    // The heights before carving and glaciers: `restore_carving` and
    // then `restore_glaciers`, on a copy of the heights alone, without
    // the map's no-data mask or the statistics they would recompute
    // twice.
    let source = (kind == MapKind::SourceHeight).then(|| {
      let mut heights = terrain.heights.clone();

      for (index, height) in self.rivers.carved.iter().chain(&self.glaciers) {
        if let Some(slot) = heights.get_mut(*index) {
          *slot = *height;
        }
      }

      heights
    });
    let drainage = self.soil().drainage;
    let square_km = metres * metres / 1.0e6;
    let tree_scale = crate::render::lattice::target_density(self.tree_density());
    let grass_rules =
      crate::render::grass::GrassRules::new(&self.grass, self.grass_density(), self.tree_density());
    let byte = |value: u8| f32::from(value);
    let read = |index: usize, out: &mut [f32]| match kind {
      MapKind::Height => out[0] = terrain.heights[index],
      MapKind::SourceHeight => out[0] = source.as_ref().map_or(0.0, |heights| heights[index]),
      MapKind::Biome => out[0] = byte(surface[index].biome),
      MapKind::Water => out[0] = water.get(index).map_or(0.0, |value| byte(*value)),
      MapKind::WaterDepth => out[0] = depth.get(index).copied().unwrap_or(0.0),
      MapKind::Flow => {
        let (x, z) = at(index);
        out[0] = drainage.map_or(1.0, |area| area.at(x, z)) * square_km;
      }
      MapKind::Discharge => {
        let (x, z) = at(index);
        let grid = &discharge;
        out[0] = if grid.cells.is_empty() {
          0.0
        } else {
          let cx = ((x + grid.stride / 2) / grid.stride).min(grid.width - 1);
          let cz = ((z + grid.stride / 2) / grid.stride).min(grid.height - 1);
          grid.cells[(cz * grid.width + cx) as usize]
        };
      }
      MapKind::Materials => {
        for (value, weight) in out.iter_mut().zip(surface[index].materials) {
          *value = byte(weight);
        }
      }
      MapKind::Slope | MapKind::Normals => {
        let (x, z) = at(index);
        let normal = crate::terrain::normals::normal_at(terrain, x, z);

        if kind == MapKind::Slope {
          out[0] = normal[1].clamp(-1.0, 1.0).acos().to_degrees();
        } else {
          out.copy_from_slice(&normal);
        }
      }
      MapKind::Occlusion => out[0] = byte(surface[index].occlusion),
      MapKind::Temperature => out[0] = surface[index].celsius(),
      MapKind::Moisture => out[0] = byte(surface[index].moisture),
      MapKind::TreeDensity => {
        let (x, z) = world(index);
        let red = self
          .ground
          .cover
          .get(self.ground.nearest(x, z))
          .map_or(0, |texel| texel[0]);
        out[0] = tree_scale * crate::render::flora::cover_share(red) / 4.0;
      }
      MapKind::GrassDensity => {
        let (x, z) = world(index);
        out[0] = if grass_rules.probability > 0.0 && !self.ground.grass.is_empty() {
          255.0
            * crate::render::grass::grass_cover_at(
              &self.ground,
              &self.channel_bins,
              &grass_rules,
              x,
              z,
            )
        } else {
          0.0
        };
      }
    };

    let resampled = [out_width, out_height] != [width, height];
    let values = out_width as usize * out_height as usize * channels;
    let mut data = if kind.is_float() {
      MapData::F32(vec![0.0; values])
    } else {
      MapData::U8(vec![0; values])
    };
    let (low, high) = match kind {
      MapKind::WaterDepth | MapKind::Flow | MapKind::Discharge => (0.0, f32::MAX),
      MapKind::Slope => (0.0, 90.0),
      _ => (f32::MIN, f32::MAX),
    };
    let mut at = 0;

    crate::export::resample(
      (width as usize, height as usize),
      channels,
      (out_width as usize, out_height as usize),
      kind.filter(),
      &read,
      &mut |row| {
        // Blends of weights and of directions are normalised again.
        if resampled && kind == MapKind::Materials {
          row
            .chunks_exact_mut(channels)
            .for_each(crate::export::normalise_weights);
        } else if resampled && kind == MapKind::Normals {
          for normal in row.chunks_exact_mut(3) {
            normal.copy_from_slice(&crate::maths::normalise([normal[0], normal[1], normal[2]]));
          }
        }

        match &mut data {
          MapData::F32(out) => {
            for (out, value) in out[at..at + row.len()].iter_mut().zip(row.iter()) {
              *out = value.clamp(low, high);
            }
          }
          MapData::U8(out) => {
            // Rounding by truncation, as `f32::round` is a library call
            // in WASM and this runs for every value.
            for (out, value) in out[at..at + row.len()].iter_mut().zip(row.iter()) {
              *out = (value.clamp(0.0, 255.0) + 0.5) as u8;
            }
          }
        }

        at += row.len();
      },
    );

    let mut range = None;

    if let MapData::F32(values) = &data {
      let [mut low, mut high] = [f32::MAX, f32::MIN];

      for value in values {
        (low, high) = (low.min(*value), high.max(*value));
      }

      range = (low <= high).then_some([low, high]);
    }
    let (units, scale, legend) = crate::export::describe(kind);

    Ok(crate::export::ExportedMap {
      width: out_width,
      height: out_height,
      channels: kind.channels(),
      data,
      encoding: MapEncoding {
        units,
        scale,
        range,
        legend,
        metres_per_pixel: [
          2.0 * half[0] / (out_width - 1).max(1) as f32,
          2.0 * half[1] / (out_height - 1).max(1) as f32,
        ],
        sea_level_metres: terrain.metadata.sea_level_metres,
        generator: terrain.metadata.generator_version.clone(),
      },
    })
  }

  /// Mean discharge in cubic metres per second on a coarse grid (`cells`
  /// holding the discharge, not an area): the carved rivers' own, or,
  /// with none carved, what the river model routes over the ground now.
  fn discharge_grid(&self, terrain: &HeightMap) -> crate::terrain::drainage::DrainageArea {
    if !self.rivers.discharge.is_empty() {
      let area = &self.rivers.drainage;
      return crate::terrain::drainage::DrainageArea {
        width: area.width,
        height: area.height,
        stride: area.stride,
        cells: self.rivers.discharge.clone(),
      };
    }

    // As `rebuild_world_with` feeds the hydrology: the default climate
    // when biomes are off.
    let climate = (!self.biomes.enabled).then(|| {
      crate::render::terrain_mesh::bake_terrain_shading(terrain, &BiomeOptions::default(), None).1
    });
    let hydrology = crate::terrain::hydrology::build_hydrology(
      terrain,
      climate.as_deref().unwrap_or(&self.surface),
      &self.water.rivers,
      self.terrain_seed,
    );
    crate::terrain::drainage::DrainageArea {
      width: hydrology.width,
      height: hydrology.height,
      stride: hydrology.stride,
      cells: hydrology.discharge,
    }
  }

  /// Every tree on the map, or in `region` (min x, min z, max x, max z in
  /// metres), packed [`crate::export::TREE_RECORD_FLOATS`] floats a tree:
  /// x, y, z, species index, variant, scale, rotation, tint, dryness and
  /// 1 for a hand-placed tree. Procedural trees come from the whole
  /// lattice, the far set and every tile the renderer streams, at the
  /// current density; hand-placed trees replace them, as they do on
  /// screen. Grounded trees stand on the ground at full detail by their
  /// roots. More than `max_count` trees (default
  /// [`crate::config::DEFAULT_EXPORT_TREES`]) is an error.
  pub fn export_trees(
    &self,
    region: Option<[f32; 4]>,
    max_count: Option<u32>,
  ) -> VistaResult<Vec<f32>> {
    self.ensure_live()?;
    let max = max_count.unwrap_or(crate::config::DEFAULT_EXPORT_TREES);
    crate::config::validate_tree_export(region, max)?;
    let terrain = self.terrain.as_ref().ok_or_else(|| {
      VistaError::options("exportTrees needs a terrain: generate or load one first.")
    })?;
    let region = region.unwrap_or([f32::MIN, f32::MIN, f32::MAX, f32::MAX]);
    let too_many = |max: usize| {
      VistaError::options(format!(
        "exportTrees found more than maxCount ({max}) trees. Pass a region to export part of the map, or raise maxCount (up to {}).",
        crate::config::MAX_EXPORT_TREES
      ))
    };
    let hand_placed = self.custom_trees.is_some();
    let trees = match &self.custom_trees {
      Some(custom) => {
        let inside: Vec<&TreeInstance> = custom
          .iter()
          .filter(|tree| {
            let [x, _, z] = tree.position;
            x >= region[0] && x <= region[2] && z >= region[1] && z <= region[3]
          })
          .take(max as usize + 1)
          .collect();

        if inside.len() > max as usize {
          return Err(too_many(max as usize));
        }

        inside.into_iter().copied().collect()
      }
      None if self.tree_density() > 0.0 => {
        let mut rules = crate::render::vegetation::TreeRules::new(&self.flora, self.tree_density());
        rules.boulders = self.boulder_rules().map(|boulders| boulders.seed);
        crate::render::vegetation::region_trees(
          &self.ground,
          &self.channel_bins,
          &rules,
          region,
          max as usize,
        )
        .map_err(too_many)?
      }
      None => Vec::new(),
    };
    let variants = self.flora.variants_per_species.max(1);
    #[cfg(target_arch = "wasm32")]
    let roots = self.gpu.tree_roots();
    #[cfg(not(target_arch = "wasm32"))]
    let roots = self.tree_roots;
    let mut packed = Vec::with_capacity(trees.len() * crate::export::TREE_RECORD_FLOATS);

    for tree in &trees {
      let species = (tree.species_index() as usize).min(roots.len() - 1);
      let [x, y, z] = tree.position;
      let y = if tree.grounded() {
        crate::render::flora::grounded_full_detail(terrain, x, z, roots[species] * tree.scale)
      } else {
        y
      };
      packed.extend_from_slice(&[
        x,
        y,
        z,
        species as f32,
        (tree.variant() % variants) as f32,
        tree.scale,
        tree.rotation,
        tree.tint,
        tree.dryness,
        f32::from(u8::from(hand_placed)),
      ]);
    }

    Ok(packed)
  }

  /// Return current render statistics.
  pub fn stats(&self) -> RenderStats {
    self.stats.clone()
  }

  /// Return the current engine state.
  pub fn state(&self) -> EngineState {
    self.state
  }

  /// Dispose the engine. This operation is idempotent.
  pub fn dispose(&mut self) -> VistaResult<()> {
    if self.state == EngineState::Disposed {
      return Ok(());
    }

    self.terrain = None;
    self.active_terrain_id = None;
    self.state = EngineState::Disposed;

    self.surface = Vec::new();
    self.rivers = RiverNetwork::default();
    self.applied_rivers = None;
    self.glaciers = Vec::new();
    self.drainage = Default::default();
    self.water_mask = None;
    self.biome_map = None;
    self.vegetation_masks = [None, None];
    self.sounds = Default::default();

    #[cfg(target_arch = "wasm32")]
    {
      self.terrain_normals = Vec::new();
      self.mesh_centre_sample = None;
      self.mesh_stream = None;
    }

    Ok(())
  }

  fn install_terrain(&mut self, map: HeightMap, progress: Progress<'_>) -> TerrainHandle {
    let id = self.next_terrain_id;
    self.next_terrain_id = self.next_terrain_id.saturating_add(1);

    // The previous terrain's carving belongs to a different heightmap, and
    // its world must go before the new one is built (see `release_world`).
    self.release_world();
    self.terrain = None;
    self.applied_rivers = None;
    self.terrain_seed = terrain_seed(&map);
    // Masks belong to the terrain they were painted for.
    self.water_mask = None;
    self.biome_map = None;
    self.vegetation_masks = [None, None];
    // A new terrain starts with ground that matches the weather.
    self.surface_weather_settle = true;
    self.terrain = Some(map);
    self.active_terrain_id = Some(id);

    #[cfg(target_arch = "wasm32")]
    {
      self.mesh_centre_sample = None;
      self.mesh_stream = None;
    }

    self.rebuild_world_with(progress);
    let mut metadata = self
      .terrain
      .as_ref()
      .map(|terrain| terrain.metadata.clone())
      .unwrap_or_default();

    if !self.vegetation_fits() {
      metadata.warnings.push(format!(
        "The terrain covers {} km², more than the {MAX_VEGETATION_KM2} km² trees, grass and boulders are placed over, so none are drawn.",
        // Whole square kilometres, without fixed-precision float
        // formatting, which no other release code needs.
        self.terrain_km2().round() as u64
      ));
    }

    TerrainHandle { id, metadata }
  }

  /// Paint rivers and lakes into the terrain, or remove the painted water
  /// with `None`, which restores the terrain exactly. The mask is
  /// resampled to the terrain's size; the returned warning says when it
  /// had to be. It stays through `set_water` and is cleared when a new
  /// terrain loads.
  pub fn set_water_mask(
    &mut self,
    mask: Option<vista_types::WaterMask>,
  ) -> VistaResult<Option<String>> {
    self.ensure_live()?;
    let mut warning = None;

    self.water_mask = match mask {
      None => None,
      Some(mask) => {
        crate::terrain::water_mask::validate(&mask)?;
        let terrain = self.terrain.as_ref().ok_or_else(|| {
          VistaError::options("setWaterMask needs a terrain: generate or load one first.")
        })?;
        let (width, height) = (terrain.metadata.width, terrain.metadata.height);

        warning = crate::terrain::painted::resample_warning(
          "water mask",
          (mask.width, mask.height),
          (width, height),
        );

        Some(crate::terrain::water_mask::resample(&mask, width, height))
      }
    };
    self.rebuild_world();
    Ok(warning)
  }

  /// Paint biomes onto the terrain (one `BiomeKind` index a sample, 255
  /// where the engine classifies the ground itself), or clear them with
  /// `None`. The map is resampled (nearest) to the terrain's size; it
  /// stays through option changes and clears when a new terrain loads.
  /// Returns warnings: when the map was resampled, and when painted ocean
  /// lies above sea level, where it is classified as usual.
  pub fn set_biome_map(
    &mut self,
    map: Option<(u32, u32, &[u8])>,
    border: u32,
  ) -> VistaResult<Vec<String>> {
    self.ensure_live()?;
    let mut warnings = Vec::new();

    self.biome_map = match map {
      None => None,
      Some(map) => {
        let terrain = self.terrain.as_ref().ok_or_else(|| {
          VistaError::options("setBiomeMap needs a terrain: generate or load one first.")
        })?;
        let size = (terrain.metadata.width, terrain.metadata.height);
        let (painted, warning) =
          crate::terrain::painted::PaintedBiomes::new(map, border, size, self.terrain_seed)?;
        warnings.extend(warning);
        Some(painted)
      }
    };
    self.rebuild_world();

    if let Some(painted) = &self.biome_map {
      let ocean = BiomeKind::Ocean as u8;
      let lost = self
        .surface
        .iter()
        .enumerate()
        .filter(|(index, sample)| {
          let (x, y) = (*index as u32 % painted.width, *index as u32 / painted.width);
          sample.biome != ocean && painted.at(x, y) == Some(BiomeKind::Ocean)
        })
        .count();

      if lost > 0 {
        warnings.push(format!(
          "{lost} samples painted as ocean lie above sea level, where the sea cannot reach, so the engine classified them itself. Paint lakes and rivers with the water mask instead."
        ));
      }
    }

    Ok(warnings)
  }

  /// Scale the trees (`grass` false) or the grass with a density mask
  /// (see `painted::density_multiplier`), or clear it with `None`. The
  /// mask is resampled (bilinear) to the terrain's size, stays through
  /// option changes and clears when a new terrain loads. Returns a
  /// warning when it had to be resampled.
  pub fn set_vegetation_mask(
    &mut self,
    grass: bool,
    mask: Option<(u32, u32, &[u8])>,
  ) -> VistaResult<Option<String>> {
    self.ensure_live()?;
    let mut warning = None;
    let name = if grass { "grass" } else { "tree" };

    self.vegetation_masks[usize::from(grass)] = match mask {
      None => None,
      Some(mask) => {
        let terrain = self.terrain.as_ref().ok_or_else(|| {
          VistaError::options("setVegetationMasks needs a terrain: generate or load one first.")
        })?;
        let size = (terrain.metadata.width, terrain.metadata.height);
        let (data, resampled) = crate::terrain::painted::density_mask(name, mask, size)?;
        warning = resampled;
        Some(data)
      }
    };

    if !grass {
      // Riparian scrub follows the tree mask.
      if let Some(terrain) = &self.terrain {
        if !self.channel_bins.is_empty() {
          crate::render::vegetation::mask_scrub(
            &mut self.channel_bins,
            terrain,
            self.vegetation_masks[0].as_deref(),
          );
          #[cfg(target_arch = "wasm32")]
          self.gpu.upload_surface(&self.ground, &self.channel_bins);
        }
      }

      self.refresh_flora();
    }

    self.refresh_grass();
    Ok(warning)
  }

  /// The loudest river, waterfall, lake shore and surf near a position,
  /// for hosts that play their own audio. Reads only the grid cells
  /// around the position.
  pub fn water_sounds(&self, x: f32, y: f32, z: f32) -> vista_types::WaterSounds {
    if !self.water.enabled || !x.is_finite() || !y.is_finite() || !z.is_finite() {
      return Default::default();
    }

    // The surf is as loud as the sea the weather raises.
    let water = self.weathered_options().water;
    let waves = &water.waves;
    let wave_height = if waves.enabled {
      waves.amplitude_metres * 2.0
    } else {
      0.2
    };
    self.sounds.query([x, y, z], wave_height)
  }

  /// Every waterfall, where its water lands.
  pub fn waterfalls(&self) -> Vec<vista_types::Waterfall> {
    let Some(terrain) = self.terrain.as_ref() else {
      return Vec::new();
    };
    let metres = terrain.metadata.metres_per_sample.max(0.001);
    let half_x = (terrain.metadata.width as f32 - 1.0) * metres * 0.5;
    let half_z = (terrain.metadata.height as f32 - 1.0) * metres * 0.5;

    self
      .rivers
      .falls
      .iter()
      .map(|fall| vista_types::Waterfall {
        position: [
          fall.foot[0] * metres - half_x,
          fall.foot_level,
          fall.foot[1] * metres - half_z,
        ],
        height_metres: fall.height(),
        width_metres: fall.width,
        discharge_cubic_metres_per_second: fall.discharge,
      })
      .collect()
  }

  /// The water entering from beyond the map, including the inflow
  /// `"auto"` placed, where it enters.
  pub fn inflows(&self) -> Vec<vista_types::WaterInflow> {
    let Some(terrain) = self.terrain.as_ref() else {
      return Vec::new();
    };
    let metres = terrain.metadata.metres_per_sample.max(0.001);
    let half_x = (terrain.metadata.width as f32 - 1.0) * metres * 0.5;
    let half_z = (terrain.metadata.height as f32 - 1.0) * metres * 0.5;

    self
      .rivers
      .inflows
      .iter()
      .map(|inflow| vista_types::WaterInflow {
        position: [
          inflow.position[0] * metres - half_x,
          inflow.level,
          inflow.position[1] * metres - half_z,
        ],
        discharge_cubic_metres_per_second: inflow.discharge,
      })
      .collect()
  }

  /// River options that should currently be carved, if any: rivers need
  /// water, and either rivers switched on or painted water.
  fn wanted_rivers(&self, water: &WaterOptions) -> Option<RiverOptions> {
    if water.enabled && (water.rivers.enabled || self.water_mask.is_some()) {
      Some(water.rivers.clone())
    } else {
      None
    }
  }

  /// Re-shape glaciers and re-extract rivers (restoring any previous
  /// shaping and carving first), then re-bake surface shading and every
  /// terrain-dependent layer.
  fn rebuild_world(&mut self) {
    self.rebuild_world_with(&mut |_, _| {});
  }

  /// [`Self::rebuild_world`], reporting the `"rivers"` phase while the
  /// river network is built.
  fn rebuild_world_with(&mut self, progress: Progress<'_>) {
    let wanted = self.wanted_rivers(&self.water);
    let mut before_rivers = None;

    if let Some(terrain) = self.terrain.as_mut() {
      // Undo in the reverse order of shaping: rivers were carved into the
      // glacier surface.
      restore_carving(terrain, &self.rivers.carved);
      restore_glaciers(terrain, &self.glaciers);
    }

    self.release_world();

    if let Some(terrain) = self.terrain.as_mut() {
      self.glaciers = shape_painted_glaciers(terrain, &self.biomes, self.biome_map.as_ref());
      self.rivers = match &wanted {
        Some(options) => {
          // Rain, snow and temperature for the hydrology, from the ground
          // before any channel is cut. This is the terrain's own surface
          // bake, done here instead of afterwards: it is patched where the
          // rivers change the ground, so it is not part of the river build.
          let normals = crate::terrain::normals::generate_normals(terrain);
          let surface =
            crate::terrain::biomes::classify_surface(terrain, &normals, None, &[], &self.biomes);
          // With biomes switched off the ground is not shaded by climate,
          // but rain still falls: rivers follow the default climate. The
          // normals are the same either way, so they are made once.
          let default_climate = (!self.biomes.enabled).then(|| {
            crate::terrain::biomes::classify_surface(
              terrain,
              &normals,
              None,
              &[],
              &BiomeOptions::default(),
            )
          });
          let relief = SurfaceRelief::of(terrain, &self.biomes);
          progress("rivers", 0.0);
          let mut record = CarveRecord::new(terrain.heights.len());
          let painted = self.water_mask.as_ref().map_or(Vec::new(), |mask| {
            crate::terrain::water_mask::apply(terrain, mask, &mut record)
          });
          let network = build_river_network(
            terrain,
            options,
            RiverSources {
              surface: default_climate.as_deref().unwrap_or(&surface),
              seed: self.terrain_seed,
              painted,
              record,
            },
          );
          progress("rivers", 1.0);
          // Back to the rest of finishing, so the phase log times the
          // river build alone.
          progress("finishing", 0.5);
          before_rivers = Some((surface, relief));
          network
        }
        None => RiverNetwork {
          mask: vec![false; terrain.heights.len()],
          ..RiverNetwork::default()
        },
      };
      self.drainage = match &wanted {
        Some(_) => Default::default(),
        None => crate::terrain::drainage::DrainageArea::d8(
          terrain.metadata.width,
          terrain.metadata.height,
          &terrain.heights,
        ),
      };
    } else {
      self.rivers = RiverNetwork::default();
      self.glaciers = Vec::new();
      self.drainage = Default::default();
    }

    self.applied_rivers = wanted;
    self.ground = self.terrain.as_ref().map_or(
      Default::default(),
      crate::render::vegetation::GroundData::of,
    );
    self.sounds = self.terrain.as_ref().map_or(Default::default(), |terrain| {
      crate::water_sounds::SoundMap::build(terrain, &self.rivers)
    });
    self.height_range = self
      .terrain
      .as_ref()
      .map_or((0.0, 0.0), terrain_height_range);

    #[cfg(target_arch = "wasm32")]
    if let Some(terrain) = self.terrain.as_ref() {
      self.gpu.upload_heightmap(terrain, &self.ground);
      self
        .gpu
        .upload_rivers(&self.rivers.vertices, &self.rivers.indices);
      self
        .gpu
        .upload_falls(&self.rivers.fall_vertices, &self.rivers.fall_indices);
      self
        .gpu
        .upload_bank_strips(&self.rivers.bank_vertices, &self.rivers.bank_indices);
      self.gpu.upload_channel_field(&self.rivers.field);
    }

    self.rebake_surface(before_rivers);
  }

  /// Drop everything derived from the terrain before a new world is built
  /// from it. A large world takes much of the 4 GiB WebAssembly can
  /// address, and that memory never shrinks, so holding the old one while
  /// building the new one would raise the peak for good. Everything dropped is rebuilt from the heights, the options
  /// and the painted maps, which stay. Call it only once the rivers'
  /// carving and the glaciers are undone, as it drops their records.
  fn release_world(&mut self) {
    self.rivers = RiverNetwork::default();
    self.glaciers = Vec::new();
    self.drainage = Default::default();
    self.ground = Default::default();
    self.sounds = Default::default();
    self.surface = Vec::new();
    self.channel_bins = Vec::new();

    #[cfg(not(target_arch = "wasm32"))]
    {
      self.placed_trees = Vec::new();
    }

    #[cfg(target_arch = "wasm32")]
    {
      self.terrain_normals = Vec::new();
      // A half-built next mesh has the old heights and colours.
      self.mesh_stream = None;
    }
  }

  /// Re-bake normals and biome/surface data, rebuild the terrain mesh, and
  /// refresh trees, grass, and water. `before_rivers` is the surface
  /// classified just before the rivers were built: only the samples the
  /// rivers changed are classified again.
  fn rebake_surface(&mut self, before_rivers: Option<(Vec<SurfaceSample>, SurfaceRelief)>) {
    match self.terrain.as_ref() {
      Some(terrain) => {
        let mask = if self.rivers.mask.len() == terrain.heights.len() {
          Some(self.rivers.mask.as_slice())
        } else {
          None
        };
        let normals = crate::terrain::normals::generate_normals(terrain);
        let soil = self.soil();
        let mut surface = match before_rivers {
          Some((mut samples, relief)) if relief == SurfaceRelief::of(terrain, &self.biomes) => {
            let touched = touched_samples(terrain, &self.rivers);
            crate::terrain::biomes::reclassify_surface(
              terrain,
              &normals,
              mask,
              &self.rivers.riparian,
              &self.biomes,
              &soil,
              &mut samples,
              &touched,
            );
            samples
          }
          _ => crate::terrain::biomes::classify_surface_with(
            terrain,
            &normals,
            mask,
            &self.rivers.riparian,
            &self.biomes,
            &soil,
          ),
        };
        crate::terrain::biomes::apply_bed_materials(&mut surface, &self.rivers.bed);
        self.surface = surface;
        self.ground.set_surface(
          terrain,
          &self.surface,
          &self.rivers.wet,
          &self.rivers.riparian,
        );
        self.channel_bins = crate::render::vegetation::channel_bins(terrain, Some(&self.rivers));
        crate::render::vegetation::mask_scrub(
          &mut self.channel_bins,
          terrain,
          self.vegetation_masks[0].as_deref(),
        );
        let ocean = BiomeKind::Ocean as u8;
        self.sea_ice_possible = self
          .surface
          .iter()
          .any(|sample| sample.biome == ocean && sample.celsius() < SEA_ICE_CELSIUS);
        self.terrain_materials = terrain_materials(&self.surface);

        // Bank strips and the channel field's beds shade mud, sand or
        // gravel whatever the ground is.
        if !self.rivers.bank_vertices.is_empty() || !self.rivers.field.tiles.is_empty() {
          use crate::terrain::biomes::{MAT_GRAVEL, MAT_MUD, MAT_SAND};
          self.terrain_materials |= (1 << MAT_MUD) | (1 << MAT_GRAVEL) | (1 << MAT_SAND);
        }

        #[cfg(target_arch = "wasm32")]
        {
          let (centre_sample_x, centre_sample_z) = self.mesh_centre_sample.unwrap_or((
            (terrain.metadata.width as f32 - 1.0) * 0.5,
            (terrain.metadata.height as f32 - 1.0) * 0.5,
          ));
          let mesh = crate::render::terrain_mesh::build_terrain_mesh_centred(
            terrain,
            &normals,
            &self.surface,
            centre_sample_x,
            centre_sample_z,
            crate::render::terrain_mesh::CENTRED_MESH_SAMPLES_PER_SIDE,
          );
          self.gpu.upload_terrain(&mesh);
          self.gpu.upload_surface(&self.ground, &self.channel_bins);
          self.terrain_normals = normals;
          self.mesh_centre_sample = Some((centre_sample_x, centre_sample_z));
          // A half-built next mesh has the old heights and colours.
          self.mesh_stream = None;
        }

        #[cfg(not(target_arch = "wasm32"))]
        drop(normals);
      }
      None => {
        self.surface = Vec::new();
        self.sea_ice_possible = false;
        self.terrain_materials = 0;
        self.channel_bins = Vec::new();
      }
    }

    self.refresh_flora();
    self.refresh_grass();
    self.refresh_boulders();
    self.refresh_water();
  }

  /// The soil model's inputs for the active terrain: the rockiness, the
  /// landform's beds and the drainage (the rivers' own, when carved).
  fn soil(&self) -> crate::terrain::biomes::Soil<'_> {
    crate::terrain::biomes::Soil {
      options: crate::terrain::soil::SoilOptions {
        rockiness: self.surface_options.rockiness,
        strata: crate::terrain::soil::Strata::new(self.landform, self.terrain_seed),
      },
      drainage: Some(if self.rivers.drainage.cells.is_empty() {
        &self.drainage
      } else {
        &self.rivers.drainage
      }),
      painted: self.biome_map.as_ref(),
    }
  }

  /// The rock shading's uniforms: the beds' dip and spacing, and the side
  /// the sun mostly shines on, where lichen grows.
  #[cfg(target_arch = "wasm32")]
  fn rock_frame(&self) -> [f32; 4] {
    let strata = crate::terrain::soil::Strata::new(self.landform, self.terrain_seed);
    [
      strata.dip[0],
      strata.dip[1],
      strata.period,
      self.sun.azimuth_degrees.to_radians(),
    ]
  }

  /// The effective grass density, 0 to 4: the slider times the quality
  /// preset's flora density scale, or 0 while grass is off.
  fn grass_density(&self) -> f32 {
    if !self.grass.enabled {
      return 0.0;
    }

    (self.grass.density * self.quality.flora_density_scale.unwrap_or(1.0)).clamp(0.0, 4.0)
  }

  /// The effective tree density, 0 to 4: the slider times the quality
  /// preset's flora density scale, or 0 without procedural trees.
  fn tree_density(&self) -> f32 {
    if !self.flora.enabled || self.custom_trees.is_some() {
      return 0.0;
    }

    (self.flora.density * self.quality.flora_density_scale.unwrap_or(1.0)).clamp(0.0, 4.0)
  }

  /// Bake the cover texture and place the static trees for the active
  /// terrain and current flora and quality settings: on browser builds
  /// the far set (every tree while the forest is small), uploaded with
  /// the tile pool the GPU streams the rest into; on native builds, which
  /// draw nothing, every tree.
  /// The terrain's area in square kilometres.
  fn terrain_km2(&self) -> f32 {
    4.0 * self.ground.half[0] * self.ground.half[1] / 1e6
  }

  /// Whether the terrain is small enough to place vegetation over: its
  /// lattices and tile grids span the whole map, so their work and memory
  /// grow with its area, whatever its sample count.
  fn vegetation_fits(&self) -> bool {
    self.terrain_km2() <= MAX_VEGETATION_KM2
  }

  fn refresh_flora(&mut self) {
    self.place_trees();
    // Ground under trees stays damp longer.
    self.rebuild_surface_weather();
  }

  fn place_trees(&mut self) {
    self.streams.trees = None;
    self.streams.far_trees = 0;

    if let Some(custom) = &self.custom_trees {
      self.stats.flora_instances = custom.len() as u32;
      self.ground.cover = vec![[0; 4]; self.ground.heights.len()];

      #[cfg(target_arch = "wasm32")]
      {
        self.gpu.upload_cover(&self.ground);
        self.gpu.upload_trees(custom, None);
      }

      return;
    }

    let density = if self.vegetation_fits() {
      self.tree_density()
    } else {
      0.0
    };
    let mut trees = Vec::new();

    if let Some(terrain) = &self.terrain {
      let placement = crate::render::flora::Placement {
        rivers: Some(&self.rivers),
        poleward: crate::render::flora::poleward_of_sun(self.sun.azimuth_degrees),
        drainage: Some(&self.drainage),
      };
      let mask = self.vegetation_masks[0]
        .as_deref()
        .map(|mask| capped_mask(mask, crate::render::lattice::target_density, density));
      self.ground.cover = crate::render::flora::bake_cover(
        terrain,
        &self.surface,
        &self.flora,
        &placement,
        self.ground.stride,
        mask.as_deref(),
      );
      let mut rules = crate::render::vegetation::TreeRules::new(&self.flora, density);
      rules.boulders = self.boulder_rules().map(|boulders| boulders.seed);

      if density > 0.0 {
        let scrub = crate::render::vegetation::scrub_mass(
          &self.ground,
          &self.rivers.channels,
          &self.rivers.bands,
        );
        let mass = crate::render::vegetation::TileMass::trees(&self.ground, &rules, &scrub);
        // Native builds have no tiles to stream, so the far set is every
        // tree.
        rules.far_keep = if cfg!(target_arch = "wasm32") {
          crate::render::lattice::far_keep(mass.total())
        } else {
          1.0
        };
        trees = crate::render::flora::stand_lattice_trees(
          terrain,
          &self.ground,
          &self.channel_bins,
          &rules,
          rules.far_keep,
          self.flora.max_instances as usize,
        );
        let room = self.flora.max_instances.saturating_sub(trees.len() as u32);
        self.streams.far_trees = trees.len() as u32;
        self.streams.trees = (rules.far_keep < 1.0 && mass.most() > 0.0).then(|| {
          crate::render::vegetation::tree_stream(
            mass,
            self.quality.vegetation().detail_metres,
            room,
            rules.far_keep,
          )
        });
      }

      #[cfg(target_arch = "wasm32")]
      self.gpu.upload_cover(&self.ground);

      #[cfg(target_arch = "wasm32")]
      {
        let species = crate::render::vegetation::cover_species(&self.ground.cover);
        self.gpu.upload_trees(
          &trees,
          self
            .streams
            .trees
            .as_ref()
            .map(|stream| (&stream.layout, rules, species)),
        );
      }
    }

    self.stats.flora_instances = trees.len() as u32;

    #[cfg(not(target_arch = "wasm32"))]
    {
      self.placed_trees = trees;
    }
  }

  /// The procedural trees last placed, on native builds (browser builds
  /// keep them only on the GPU).
  #[cfg(not(target_arch = "wasm32"))]
  pub fn placed_trees(&self) -> &[TreeInstance] {
    &self.placed_trees
  }

  /// Bake the grass texture and place the reeds for the active terrain
  /// and current grass and quality settings; on browser builds, upload
  /// them with the tile pool the GPU streams the tufts into. Native builds
  /// report every reed and tuft the map holds.
  fn refresh_grass(&mut self) {
    self.streams.grass = None;
    let scale = self.quality.flora_density_scale.unwrap_or(1.0);
    let density = self.grass_density();
    let mut reeds = Vec::new();

    if let Some(terrain) = &self.terrain {
      self.ground.grass = crate::render::grass::bake_grass(terrain, &self.surface, &self.ground);
      self.ground.grass_mask = self.vegetation_masks[1]
        .as_deref()
        .map_or(Vec::new(), |mask| {
          let texels: Vec<u8> = (0..self.ground.grass.len())
            .map(|texel| mask[self.ground.sample_of(terrain, texel)])
            .collect();
          capped_mask(&texels, crate::render::lattice::grass_probability, density)
        });
      #[cfg(target_arch = "wasm32")]
      self.gpu.upload_grass_mask(&self.ground);
      // Reeds are grass too, and on a terrain past the vegetation cap
      // their candidates alone took gigabytes.
      if self.vegetation_fits() {
        reeds = crate::render::grass::build_reed_instances(
          terrain,
          &self.surface,
          Some(&self.rivers.wet),
          &self.rivers.brooks,
          &self.grass,
          scale,
        );
      }
      let rules = crate::render::grass::GrassRules {
        boulders: self.boulder_rules().map(|boulders| boulders.seed),
        ..crate::render::grass::GrassRules::new(&self.grass, density, self.tree_density())
      };

      if density > 0.0 && self.vegetation_fits() {
        let riparian = crate::render::grass::riparian_mass(
          &self.ground,
          &self.rivers.channels,
          &self.rivers.bands,
        );
        let mass = crate::render::vegetation::TileMass::grass(&self.ground, &rules, &riparian);
        let view = self.grass.view_distance_metres;
        self.streams.grass = (mass.most() > 0.0).then(|| {
          crate::render::vegetation::grass_stream(
            mass,
            crate::render::vegetation::grass_radius(&self.quality.vegetation(), view),
            view,
            self.grass.max_instances,
          )
        });
      }

      #[cfg(target_arch = "wasm32")]
      self.gpu.upload_grass(
        &self.ground,
        &reeds,
        self
          .streams
          .grass
          .as_ref()
          .map(|stream| (&stream.layout, rules)),
      );
    }

    self.streams.reeds = reeds.len() as u32;
    self.stats.grass_instances = reeds.len() as u32
      + self.streams.grass.as_ref().map_or(0, |stream| {
        stream.mass.tiles().map(|(_, mass)| mass).sum::<f32>() as u32
      });
  }

  /// The boulders' lattice rules, when boulders are on and the ground has
  /// talus or streams have stones for them.
  fn boulder_rules(&self) -> Option<crate::render::boulders::BoulderRules> {
    (self.surface_options.boulders
      && self.terrain.is_some()
      && self.vegetation_fits()
      && (self.ground.banks.iter().any(|texel| texel[3] > 0)
        || self
          .rivers
          .stones
          .iter()
          .flatten()
          .any(|stone| stone[1] > 0.0)))
    .then(|| crate::render::boulders::BoulderRules::new(self.terrain_seed, self.boulder_heights))
  }

  /// Plan the boulder tiles for the active terrain and surface options;
  /// on browser builds, upload the tile pool the GPU streams them into.
  fn refresh_boulders(&mut self) {
    let rules = self.boulder_rules();
    self.streams.boulders = rules.and_then(|_| {
      let stones = crate::render::boulders::stone_mass(
        &self.ground,
        &self.rivers.channels,
        &self.rivers.stones,
      );
      let mass = crate::render::boulders::boulder_mass(&self.ground, &stones);
      (mass.most() > 0.0).then(|| {
        crate::render::boulders::boulder_stream(mass, self.surface_options.boulder_distance_metres)
      })
    });

    #[cfg(target_arch = "wasm32")]
    self.gpu.upload_boulders(
      self
        .streams
        .boulders
        .as_ref()
        .zip(rules)
        .map(|(stream, rules)| (&stream.layout, rules)),
    );
  }

  /// Update water visibility for the active terrain.
  fn refresh_water(&mut self) {
    #[cfg(target_arch = "wasm32")]
    self
      .gpu
      .set_water_visible(self.water.enabled && self.terrain.is_some());
  }

  /// Resolve the options the weather drives this frame. Systems the
  /// weather does not drive keep their manual settings.
  #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
  fn weathered_options(&self) -> Weathered {
    let mut out = Weathered {
      atmosphere: self.atmosphere.clone(),
      water: frame_water(&self.water),
      mist: self.mist.clone(),
      clouds: self.clouds.clone(),
      flora: frame_flora(&self.flora),
      weather: FrameWeatherValues::default(),
    };
    let camera = self.camera.options.position;
    let camera_surface = self.surface_at(camera[0], camera[2]).copied();

    // Cold air holds little moisture or haze, so it is clear and crisp.
    if let Some(sample) = camera_surface {
      let crisp = smoothstep(-sample.celsius() / 3.0);
      out.atmosphere.haze_distance_metres *= 1.0 + 0.4 * crisp;
      out.atmosphere.mie_strength *= 1.0 - 0.3 * crisp;
    }

    let options = self.weather.options();

    if !options.enabled {
      return out;
    }

    let state = self.weather.state();
    let values = self.weather.values();
    let effects = &options.effects;
    let wind = state.wind_speed_metres_per_second;
    let local = self.weather.local();
    let regional_on = options.regional;

    if effects.clouds {
      use crate::weather::presets::field;
      // Weather without clouds would look wrong, so switch them on.
      if out.clouds.style == vista_types::CloudStyle::Off {
        out.clouds.style = vista_types::CloudStyle::Volumetric;
      }

      // The mean; the regional map varies it across the sky.
      out.clouds.coverage = values.get(field::COVERAGE);
      out.clouds.density = state.cloud_density;
      out.clouds.thickness_metres *= self.weather.cloud_thickness_scale();
      out.clouds.stratiform = state.stratiform;
      out.clouds.towering = state.towering;
      out.clouds.base_darkness = state.base_darkness;
      out.clouds.ragged_base = state.ragged_base;
      out.clouds.rain_shafts = state.rain_shafts;
      out.clouds.cirrus = values.get(field::CIRRUS);
      out.clouds.base_variation = values.get(field::BASE_VARIATION);
      out.clouds.base_lumpiness = values.get(field::BASE_LUMPINESS);
      out.clouds.altocumulus = values.get(field::ALTOCUMULUS);
      out.clouds.altostratus = values.get(field::ALTOSTRATUS);
      out.clouds.alto_height_metres = values.get(field::ALTO_HEIGHT);
      out.clouds.alto_speed = values.get(field::ALTO_SPEED);
      // The sky greys with full cover and darkens further under
      // rain-laden cloud. A near-complete deck (rain's 97 % cover) is a full
      // overcast: no direct sun, no sharp shadows, no glint on the water.
      // Heavy rain (or the sleet and snow it turns to in the cold) means a
      // full deck overhead too, even between storm cells.
      let laden = (state.base_darkness * 0.9).max((state.rain + state.snow).min(1.0));
      out.weather.overcast = ((state.cloud_coverage - 0.6) / 0.35)
        .clamp(0.0, 1.0)
        .max(laden);
      let overcast = out.weather.overcast;
      // Direct sun through the clouds between the camera and the sun, and
      // the softer, dimmer, flatter light under a deck. Over a uniform
      // deck the direct term is exactly `1 - 0.9 x overcast`. The sky
      // model's grey deck already dims the sky light by 40 % at full
      // cover, so the indirect term takes the rest of the 60 %.
      out.weather.light = [
        self.weather.sun_transmittance().min(1.0 - 0.9 * laden),
        1.0 - 0.85 * overcast,
        1.0 - 0.35 * smoothstep((state.cloud_coverage - 0.5) / 0.5),
        smoothstep((state.cloud_coverage - 0.4) / 0.55),
      ];
      out.weather.shadow_softening = 0.6 * overcast;

      if regional_on {
        let size = self.weather.regional().size_metres();
        out.weather.regional = [-size * 0.5, -size * 0.5, 1.0 / size, 1.0];
      }
    }

    if effects.mist {
      if out.mist.style == vista_types::MistStyle::Off && state.mist_density > 0.01 {
        out.mist.style = vista_types::MistStyle::Volumetric;
      }

      // Rain-cooled mist hangs where the rain falls, not in the sunny
      // gaps between storm cells.
      let rain_share = {
        use crate::weather::presets::field;
        let mean = (values.get(field::RAIN) + values.get(field::SNOW))
          * self.weather.options().precipitation_scale.max(0.0);

        if mean > 0.01 {
          0.3 + 0.7 * (local.precipitation / mean).clamp(0.0, 1.0)
        } else {
          1.0
        }
      };
      out.mist.density = state.mist_density * rain_share;
      out.atmosphere.haze_distance_metres *= self.weather.haze_scale();
      // Turbid air scatters more; damp air swells the aerosols, which
      // whitens the haze and sharpens its glow around the sun.
      out.atmosphere.mie_strength *= state.turbidity / 3.0;
      let dry = ((0.5 - local.humidity) / 0.4).clamp(0.0, 1.0);
      out.weather.air = [
        1.0 - 0.2 * dry,
        1.0 - 0.05 * dry,
        1.0 + 0.25 * dry,
        0.76 + 0.18 * (local.humidity - 0.5).max(0.0),
      ];
    }

    if effects.wind {
      out.flora.wind_strength = (wind / 14.0).clamp(0.05, 1.0);
      out.mist.wind_direction_degrees = state.wind_direction_degrees;
      out.mist.wind_speed_metres_per_second = wind * 0.6;
      out.clouds.wind_direction_degrees = state.wind_direction_degrees;
      // Winds aloft are roughly twice the surface wind; `speed` is in
      // units of 15 m/s.
      out.clouds.speed = wind * 2.0 / 15.0;
      let radians = state.wind_direction_degrees.to_radians();
      out.weather.wind = [radians.sin() * wind, radians.cos() * wind];
      out.weather.gust = [
        self.weather.gust_offset(),
        state.gustiness,
        self.weather.mean_wind(),
        1.0,
      ];
    }

    if effects.water {
      use crate::weather::wind::{reference_wave_height, sea_state};
      let fetch = self.upwind_fetch(state.wind_direction_degrees);
      let sea = sea_state(self.weather.mean_wind(), fetch);
      // The configured waves are those of a light breeze over open water;
      // the wind scales them.
      let sea_scale = (sea.wave_height_metres / reference_wave_height()).clamp(0.3, 8.0);
      let waves = &mut out.water.waves;
      waves.amplitude_metres = (waves.amplitude_metres * sea_scale).min(30.0);
      waves.steepness = (waves.steepness * (0.7 + wind / 25.0)).min(1.0);
      waves.direction_degrees = self.wave_heading.unwrap_or(state.wind_direction_degrees);
      out.water.foam = out
        .water
        .foam
        .max((sea.whitecaps / 0.3).sqrt())
        .clamp(0.0, 1.0);
      out.water.current_direction_degrees = state.wind_direction_degrees;
      out.water.current_speed *= 0.6 + wind / 12.0;
      out.weather.sea[1] = sea.whitecaps;
      out.weather.sea[2] = sea.spray;
    }

    if effects.precipitation {
      out.weather.rain = state.rain;
      out.weather.snow = state.snow;
      out.weather.heaviness = self.weather.precipitation_heaviness();
      out.weather.lens_drops = self.weather.options().lens_drops;
    }

    if effects.ground {
      out.weather.wetness = state.wetness;
      out.weather.snow_cover = state.snow_cover;
      out.weather.sea[0] = if self.terrain.is_some() { 1.0 } else { 0.0 };
    }

    // Snow lifted off the ground by a strong wind, over settled snow or
    // snow that lies all year.
    let lying = camera_surface.map_or(0.0, |sample| sample.permanent_snow_unit());
    let cover = out.weather.snow_cover.max(lying);
    let gale = (out.weather.wind[0].powi(2) + out.weather.wind[1].powi(2)).sqrt();
    out.weather.blowing_snow = smoothstep((cover - 0.5) / 0.2) * smoothstep((gale - 8.0) / 4.0);

    if effects.lightning {
      out.weather.lightning = state.lightning;
      let offset = self.weather.lightning_offset();
      let camera = self.camera.options.position;
      out.weather.lightning_position = [camera[0] + offset[0], camera[2] + offset[1]];
    }

    out
  }

  /// What the scene draws now, and what it is likely to draw soon: the
  /// renderer creates the pipelines for the first before each frame, and
  /// warms up those for the second one per frame after the first frame.
  pub fn pipeline_needs(&self) -> (Needs, Needs) {
    let Weathered {
      water,
      mist,
      clouds,
      flora,
      weather,
      ..
    } = self.weathered_options();
    let open_sea = sea_level_celsius(&self.biomes);
    let clouds_on = clouds.style != vista_types::CloudStyle::Off
      && (clouds.coverage > 0.001 || clouds.altocumulus.max(clouds.altostratus) > 0.001);
    // Grass streams in over the first frames anyway, so the first frame
    // is drawn without it, and its pipelines are warmed up after.
    let grass = self.streams.reeds > 0 || self.streams.grass.is_some();
    let after_first = self.first_frame_drawn;
    let needs = Needs {
      terrain_materials: self.terrain_materials,
      terrain_shadows: self.terrain.is_some() && self.shadows.terrain.enabled,
      surface_weather: self.terrain.is_some()
        && self.weather.options().enabled
        && self.weather.options().effects.ground,
      trees: self.stats.flora_instances > 0 || self.streams.trees.is_some(),
      tree_meshes: flora.tree_quality == vista_types::TreeQuality::Mesh,
      // The light meshes' own pipeline comes after the first frame.
      tree_light_meshes: after_first,
      tree_shadows: self.shadows.trees.enabled,
      grass: grass && after_first,
      tree_tiles: self.streams.trees.is_some(),
      grass_tiles: self.streams.grass.is_some() && after_first,
      canopy: self.streams.trees.is_some() && self.canopy_in_view(),
      boulders: after_first && self.boulders_in_view(),
      clouds: clouds_on,
      cloud_noise: clouds_on || mist.style == vista_types::MistStyle::Volumetric,
      cloud_reuse: clouds.temporal && clouds.style == vista_types::CloudStyle::Volumetric,
      water: water.enabled && self.terrain.is_some(),
      sea_ice: self.sea_ice_possible || open_sea < SEA_ICE_CELSIUS,
      sea_near_freezing: false,
      inland_water: !self.rivers.vertices.is_empty(),
      falls: !self.rivers.fall_vertices.is_empty(),
      bank_strips: !self.rivers.bank_vertices.is_empty(),
      reflections: water.reflections == vista_types::WaterReflections::Screen,
      present: !self.lens_drops.is_empty() || self.stats.render_scale < 0.999,
    };
    let options = self.weather.options();
    // Weather that can reach rain brings lens drops and clouds.
    let rain_possible = options.enabled && self.weather.rain_possible();
    let (_, min_scale) = self.quality.render_scale_range();
    let likely = Needs {
      clouds: needs.clouds || (options.enabled && options.effects.clouds),
      cloud_noise: needs.cloud_noise || (options.enabled && options.effects.clouds),
      sea_near_freezing: open_sea < SEA_ICE_CELSIUS + 6.0 || weather.snow_cover > 0.0,
      present: needs.present
        || min_scale < 0.999
        || (rain_possible && options.lens_drops && options.effects.precipitation),
      grass,
      grass_tiles: self.streams.grass.is_some(),
      boulders: self.boulders_in_view(),
      tree_light_meshes: true,
      ..needs
    };
    (needs, likely)
  }

  /// The weather's temperature offset in °C, or 0 without weather.
  #[cfg(target_arch = "wasm32")]
  fn weather_temperature_offset(&self) -> f32 {
    if self.weather.options().enabled {
      self
        .weather
        .values()
        .get(crate::weather::presets::field::TEMPERATURE)
    } else {
      0.0
    }
  }

  /// Whether tiles with boulders lie within the boulder distance of the
  /// camera and in its view. Like grass, they stream in after the first
  /// frame, so their pipelines come after it too.
  fn boulders_in_view(&self) -> bool {
    self.streams.boulders.as_ref().is_some_and(|stream| {
      let view_proj =
        crate::maths::mat4_multiply(self.camera.projection_matrix, self.camera.view_matrix);
      let reach = self.surface_options.boulder_distance_metres;
      let planes = crate::maths::frustum_planes(&view_proj);
      stream.estimate(
        &self.ground,
        self.camera.options.position,
        &planes,
        reach,
        reach,
        f32::MAX,
        false,
      ) > 0
    })
  }

  /// Whether the view reaches the canopy layer: past 0.8 of the canopy
  /// distance, within the far plane and the render distance.
  fn canopy_in_view(&self) -> bool {
    let reach = self
      .camera
      .options
      .far_metres
      .unwrap_or(120_000.0)
      .min(self.quality.distances().render_metres);
    reach > 0.8 * self.quality.vegetation().canopy_metres
  }

  /// Tree style: 0 billboard, 1 cross-quad, 2 mesh.
  #[cfg(target_arch = "wasm32")]
  fn frame_tree_style(&self) -> u32 {
    match self.flora.tree_quality {
      vista_types::TreeQuality::Billboard => 0,
      vista_types::TreeQuality::CrossQuad => 1,
      vista_types::TreeQuality::Mesh => 2,
    }
  }

  /// Collect every per-frame shading parameter for the GPU.
  #[cfg(target_arch = "wasm32")]
  fn frame_params(&self) -> crate::render::gpu::FrameParams {
    let view_proj =
      crate::maths::mat4_multiply(self.camera.projection_matrix, self.camera.view_matrix);
    let sun_direction = self.sun_vector();
    let camera_forward = normalise(sub(
      self.camera.options.target,
      self.camera.options.position,
    ));
    let camera_right = normalise(cross(camera_forward, [0.0, 1.0, 0.0]));
    let camera_up = cross(camera_right, camera_forward);
    let aspect_ratio = self.render_width as f32 / self.render_height.max(1) as f32;
    let (needs, likely) = self.pipeline_needs();
    let Weathered {
      atmosphere,
      water,
      mist,
      clouds,
      flora,
      weather,
    } = self.weathered_options();

    let (mist_density, mist_noise_strength) = match mist.style {
      vista_types::MistStyle::Off => (0.0, 0.0),
      vista_types::MistStyle::Flat => (mist.density, 0.0),
      vista_types::MistStyle::Volumetric => (mist.density, 1.0),
    };
    // Only feed a real sea level into the shader's rise-above-water term
    // when it is actually requested; otherwise push a sentinel height far
    // from any terrain so that term always evaluates to zero.
    let mist_water_level_metres = if mist.rise_above_water && water.enabled {
      water.sea_level_metres
    } else {
      mist.base_height_metres - 1_000_000.0
    };
    let (cloud_coverage, cloud_raymarch_steps) = match clouds.style {
      vista_types::CloudStyle::Off => (0.0, 0),
      vista_types::CloudStyle::Painted => (clouds.coverage, 0),
      vista_types::CloudStyle::Volumetric => {
        // Clamped again here: this bounds a shader loop.
        (
          clouds.coverage,
          clouds.raymarch_steps.unwrap_or(32).clamp(8, 64),
        )
      }
    };
    let alto_amounts = if clouds.style == vista_types::CloudStyle::Off {
      [0.0; 2]
    } else {
      [clouds.altocumulus, clouds.altostratus]
    };
    let tree_style = self.frame_tree_style();

    crate::render::gpu::FrameParams {
      view_proj,
      camera_position: self.camera.options.position,
      camera_forward,
      camera_right,
      camera_up,
      field_of_view_degrees: self.camera.options.field_of_view_degrees,
      aspect_ratio,
      near_metres: self.camera.options.near_metres.unwrap_or(0.5),
      far_metres: self.camera.options.far_metres.unwrap_or(120_000.0),
      sun_direction,
      sun_intensity: self.sun.intensity,
      atmosphere,
      water,
      mist_density,
      mist_noise_strength,
      mist_water_level_metres,
      mist,
      cloud_coverage,
      cloud_raymarch_steps,
      alto_amounts,
      clouds,
      tree_style,
      split_tree_timing: self.quality.split_tree_timing.unwrap_or(false),
      flora,
      grass_view_distance_metres: self.grass.view_distance_metres,
      grass_cover: crate::render::grass::meadow_cover(self.grass_density()),
      grass_height: crate::render::lattice::grass_height(self.grass_density()),
      vegetation: Default::default(),
      canopy: crate::render::gpu::CanopyFrame {
        density: crate::render::lattice::canopy_density(self.tree_density()),
        distance: self.quality.vegetation().canopy_metres,
        drawn: needs.canopy,
      },
      rock: self.rock_frame(),
      debug_view: debug_view_index(self.debug_view),
      // Both are plain values: cloning them copies, without allocating.
      shadows: self.shadows.clone(),
      surface: self.surface_options.clone(),
      weather: crate::render::gpu::FrameWeather {
        rain: weather.rain,
        snow: weather.snow,
        wetness: weather.wetness,
        snow_cover: weather.snow_cover,
        lightning: weather.lightning,
        overcast: weather.overcast,
        wind: weather.wind,
        lightning_position: weather.lightning_position,
        heaviness: weather.heaviness.max(1.0),
        lens_drops: self.lens_drops.packed(),
        blowing_snow: weather.blowing_snow,
        regional: weather.regional,
        air: weather.air,
        light: weather.light,
        gust: weather.gust,
        sea: weather.sea,
        shadow_softening: weather.shadow_softening,
      },
      sea_ice: crate::render::gpu::SeaIce {
        possible: self.sea_ice_possible || sea_level_celsius(&self.biomes) < SEA_ICE_CELSIUS,
        open_sea_unit: celsius_to_unit(sea_level_celsius(&self.biomes)).clamp(0.0, 1.0),
      },
      rivers: crate::render::gpu::RiverFrame {
        // The weather's warm spells and cold snaps swell and shrink the
        // snowmelt.
        melt: melt_factor(
          self
            .celsius_at(
              self.camera.options.position[0],
              self.camera.options.position[2],
            )
            .map(|celsius| celsius + self.weather_temperature_offset()),
        ),
        freezing: self.rivers.freezing,
        falls: !self.rivers.falls.is_empty(),
        wet_banks: !self.rivers.wet.distance.is_empty(),
        eddies: self.water.eddies,
        refraction: self.water.refraction,
        mouths: crate::render::water::plume_mouths(
          &self.rivers.mouths,
          [
            self.camera.options.position[0],
            self.camera.options.position[2],
          ],
        ),
        stones: crate::render::lattice::lattice_seed(self.terrain_seed)
          ^ crate::render::boulders::STONE_SALT,
      },
      mesh_ground: self.terrain.as_ref().map_or([0.0; 4], |terrain| {
        crate::render::terrain_mesh::mesh_ground_uniform(terrain, self.mesh_centre_sample)
      }),
      height_range: self.height_range,
      render_scale: self.stats.render_scale,
      frame_seconds: self.frame_seconds,
      distances: self.quality.distances(),
      needs,
      likely,
    }
  }

  /// Rebuild and upload the camera-centred LOD terrain mesh if the camera
  /// has drifted far enough from where the mesh is currently centred.
  ///
  /// Also refreshes `terrain_triangles` and `clipmap_levels` stats to
  /// reflect the real uploaded mesh rather than a theoretical estimate.
  #[cfg(target_arch = "wasm32")]
  fn recentre_terrain_mesh_if_needed(&mut self, dt: f32) {
    use crate::render::terrain_mesh::{
      build_centred_mesh_rows, next_mesh_centre, world_to_sample_coordinates,
      RECENTRE_LIMIT_SAMPLES,
    };

    let samples_per_side = crate::render::terrain_mesh::CENTRED_MESH_SAMPLES_PER_SIDE;
    self.stats.clipmap_levels = crate::render::terrain_mesh::band_count(samples_per_side / 2);
    self.stats.terrain_triangles = (samples_per_side - 1) * (samples_per_side - 1) * 2;

    let Some(terrain) = self.terrain.as_ref() else {
      return;
    };

    let camera = world_to_sample_coordinates(
      terrain,
      self.camera.options.position[0],
      self.camera.options.position[2],
    );

    // Smoothed camera velocity, so the next mesh is centred where the
    // camera is heading rather than where it was.
    if let Some(last) = self.camera_track {
      if dt > 0.0 {
        let blend = (dt * 4.0).min(1.0);
        let measured = ((camera.0 - last.0) / dt, (camera.1 - last.1) / dt);
        self.camera_velocity.0 += (measured.0 - self.camera_velocity.0) * blend;
        self.camera_velocity.1 += (measured.1 - self.camera_velocity.1) * blend;
      }
    }

    self.camera_track = Some(camera);

    let Some(displayed) = self.mesh_centre_sample else {
      // No mesh yet: build one at once.
      let mesh = crate::render::terrain_mesh::build_terrain_mesh_centred(
        terrain,
        &self.terrain_normals,
        &self.surface,
        camera.0,
        camera.1,
        samples_per_side,
      );
      self.gpu.upload_terrain(&mesh);
      self.mesh_centre_sample = Some(camera);
      return;
    };

    if self.mesh_stream.is_none() {
      let Some(centre) = next_mesh_centre(camera, self.camera_velocity, displayed) else {
        return;
      };

      self.mesh_stream = Some(MeshStream {
        centre,
        next_row: 0,
        scratch: Vec::with_capacity((MESH_ROWS_PER_FRAME * samples_per_side) as usize),
      });
    }

    let Some(stream) = self.mesh_stream.as_mut() else {
      return;
    };

    // If the camera outruns the stream (a teleport, or very fast flight),
    // finish now rather than leave it outside the detailed band.
    let drift = (camera.0 - displayed.0)
      .abs()
      .max((camera.1 - displayed.1).abs());
    let rows = if drift > RECENTRE_LIMIT_SAMPLES {
      samples_per_side - stream.next_row
    } else {
      MESH_ROWS_PER_FRAME
    };
    let end = (stream.next_row + rows).min(samples_per_side);
    stream.scratch.clear();
    build_centred_mesh_rows(
      terrain,
      &self.terrain_normals,
      &self.surface,
      stream.centre,
      samples_per_side,
      stream.next_row..end,
      &mut stream.scratch,
    );

    if !self
      .gpu
      .write_next_terrain_vertices(stream.next_row * samples_per_side, &stream.scratch)
    {
      self.mesh_stream = None;
      return;
    }

    stream.next_row = end;

    if end >= samples_per_side {
      let centre = stream.centre;
      self.gpu.show_next_terrain();
      self.mesh_centre_sample = Some(centre);
      self.mesh_stream = None;
    }
  }

  /// Seconds since the previous frame. Native builds (tests) step a fixed
  /// sixtieth of a second so weather runs deterministically.
  fn frame_delta_seconds(&mut self) -> f32 {
    #[cfg(target_arch = "wasm32")]
    let now = crate::render::gpu::now_millis();
    #[cfg(not(target_arch = "wasm32"))]
    let now = self.last_frame_ms.map_or(0.0, |last| last + 1_000.0 / 60.0);
    let dt = self
      .last_frame_ms
      .map_or(0.0, |last| ((now - last) / 1_000.0) as f32);
    self.last_frame_ms = Some(now);
    dt.max(0.0)
  }

  fn ensure_live(&self) -> VistaResult<()> {
    if self.state == EngineState::Disposed {
      return Err(VistaError::EngineDisposed);
    }

    Ok(())
  }
}

/// Number of layers in a replaceable texture array.
pub fn texture_layers(target: TextureTarget) -> u32 {
  match target {
    TextureTarget::TerrainAlbedo | TextureTarget::TerrainNormal => TERRAIN_TEXTURE_LAYERS,
    TextureTarget::Flora => layers::COUNT,
  }
}

/// Validate host-supplied tree instances.
pub fn validate_tree_instances(trees: &[TreeInstance]) -> VistaResult<()> {
  if trees.len() > MAX_CUSTOM_TREES {
    return Err(VistaError::options(format!(
      "setTreeInstances accepts at most {MAX_CUSTOM_TREES} trees, but {} were given.",
      trees.len()
    )));
  }

  for (index, tree) in trees.iter().enumerate() {
    let valid = tree
      .position
      .iter()
      .all(|value| value.abs() <= crate::config::MAX_WORLD_METRES)
      && tree.rotation.is_finite()
      && tree.scale > 0.0
      && tree.scale <= 20.0
      && (0.0..=1.0).contains(&tree.tint)
      && (0.0..=1.0).contains(&tree.dryness);

    if !valid {
      return Err(VistaError::options(format!(
        "tree {index} must have a position within {} m, a finite rotation, a scale above 0 and at most 20, and a tint and dryness from 0 to 1.",
        crate::config::MAX_WORLD_METRES
      )));
    }

    if tree.species_index() as usize >= TreeSpeciesKind::ALL.len() {
      return Err(VistaError::options(format!(
        "tree {index} has species {}, but species must be 0 to 7.",
        tree.species_index()
      )));
    }
  }

  Ok(())
}

/// How full snowmelt makes the rivers, from the mean temperature where
/// the camera is: 1 at 10 °C, down to 0.4 at -2 °C and below, and up to
/// 1.4 at 18 °C and above. Rivers far from the camera share its season.
pub fn melt_factor(celsius: Option<f32>) -> f32 {
  (1.0 + (celsius.unwrap_or(10.0) - 10.0) / 20.0).clamp(0.4, 1.4)
}

/// The map-wide inputs of surface classification that river carving
/// could change. When they are unchanged, reclassifying just the touched
/// samples gives exactly a full classification.
#[derive(Clone, Debug, PartialEq)]
struct SurfaceRelief {
  max_height: f32,
  volcanoes: Vec<crate::terrain::biomes::Volcano>,
}

impl SurfaceRelief {
  fn of(map: &HeightMap, biomes: &BiomeOptions) -> Self {
    Self {
      max_height: map.metadata.max_height_metres,
      volcanoes: crate::terrain::biomes::find_volcanoes(map, biomes),
    }
  }
}

/// Samples whose classification the rivers may have changed: every
/// changed or masked sample and those within two samples of it (normals
/// and occlusion read that far).
fn touched_samples(map: &HeightMap, rivers: &RiverNetwork) -> Vec<usize> {
  let width = map.metadata.width as i32;
  let height = map.metadata.height as i32;
  let mut touched = vec![false; map.heights.len()];
  let mut list = Vec::new();
  let masked = rivers
    .mask
    .iter()
    .enumerate()
    .filter(|(_, masked)| **masked)
    .map(|(index, _)| index);

  // Greening reaches far beyond the channels, and classification reads
  // only the sample itself, so those samples are listed as they are.
  for (index, _) in rivers
    .riparian
    .iter()
    .enumerate()
    .filter(|(_, value)| **value > 0)
  {
    if !touched[index] {
      touched[index] = true;
      list.push(index);
    }
  }

  for index in rivers.carved.iter().map(|(index, _)| *index).chain(masked) {
    let x = (index as i32) % width;
    let y = (index as i32) / width;

    for ny in (y - 2).max(0)..=(y + 2).min(height - 1) {
      for nx in (x - 2).max(0)..=(x + 2).min(width - 1) {
        let n = (ny * width + nx) as usize;

        if !touched[n] {
          touched[n] = true;
          list.push(n);
        }
      }
    }
  }

  list
}

/// A seed that follows the terrain: a hash of its heights, so every
/// terrain places its springs, meanders and deltas its own way, and the
/// same terrain always places them the same way.
/// The terrain materials a surface uses, one bit per material, plus rock
/// and sand, which the skirt beyond the map adds.
fn terrain_materials(surface: &[SurfaceSample]) -> u32 {
  use crate::terrain::biomes::{MAT_ROCK, MAT_SAND};
  let mut mask = (1 << MAT_ROCK) | (1 << MAT_SAND);

  for sample in surface {
    for (material, weight) in sample.materials.iter().enumerate() {
      if *weight > 0 {
        mask |= 1 << material;
      }
    }
  }

  mask
}

/// The height of the sample nearest a world position, or `None` off the
/// map.
fn world_height(terrain: &HeightMap, x: f32, z: f32) -> Option<f32> {
  let metres = terrain.metadata.metres_per_sample.max(0.001);
  let column = (x / metres + (terrain.metadata.width as f32 - 1.0) * 0.5).round();
  let row = (z / metres + (terrain.metadata.height as f32 - 1.0) * 0.5).round();

  if !(column >= 0.0 && row >= 0.0) {
    return None;
  }

  terrain.height_at(column as u32, row as u32)
}

/// A density mask whose multipliers are capped so the vegetation is at
/// most as dense as at density 4: `density_of` maps a density to plants
/// per unit area, and `density` is the effective density.
fn capped_mask(mask: &[u8], density_of: fn(f32) -> f32, density: f32) -> Vec<u8> {
  use crate::terrain::painted::{density_byte, density_multiplier};
  let most = density_of(4.0) / density_of(density).max(1e-6);
  mask
    .iter()
    .map(|value| density_byte(density_multiplier(*value).min(most)))
    .collect()
}

pub(crate) fn terrain_seed(map: &HeightMap) -> u64 {
  let step = (map.heights.len() / 65_536).max(1);

  map
    .heights
    .iter()
    .step_by(step)
    .fold(map.heights.len() as u64, |seed, height| {
      crate::maths::hash_u64(seed ^ height.to_bits() as u64)
    })
}

fn terrain_height_range(map: &HeightMap) -> (f32, f32) {
  let mut low = f32::MAX;
  let mut high = f32::MIN;

  for (height, missing) in map.heights.iter().zip(&map.no_data) {
    if !missing {
      low = low.min(*height);
      high = high.max(*height);
    }
  }

  if low > high {
    (0.0, 0.0)
  } else {
    (low, high)
  }
}

/// Shader index for a debug view (see `clipmap_render.wgsl`).
#[cfg(target_arch = "wasm32")]
fn debug_view_index(view: DebugView) -> u32 {
  match view {
    DebugView::None | DebugView::Lod | DebugView::Flow | DebugView::NoData => 0,
    DebugView::Height => 1,
    DebugView::Slope => 2,
    DebugView::Normals => 3,
    DebugView::Materials => 4,
    DebugView::Biomes => 5,
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::render::pipelines::{PipelineKind, PipelineSlots};
  use vista_types::{FractalTerrainOptions, NoiseKind, NoiseOptions, VistaEngineOptions};

  #[test]
  fn a_frame_copies_its_options_without_the_lists_it_never_reads() {
    let mut water = vista_types::WaterOptions::default();
    water.rivers.inflow = vista_types::RiverInflows::List(vec![vista_types::RiverInflow {
      position: [10.0, 20.0],
      discharge_cubic_metres_per_second: 30.0,
    }]);
    water.foam = 0.3;
    let mut copied = frame_water(&water);
    assert_eq!(copied.rivers.inflow, vista_types::RiverInflows::default());
    copied.rivers.inflow = water.rivers.inflow.clone();
    assert_eq!(copied, water);

    let mut flora = vista_types::FloraOptions {
      density: 0.4,
      ..vista_types::FloraOptions::default()
    };
    flora.species_rules.push(vista_types::FloraRule {
      biome: vista_types::BiomeKind::InnerForest,
      species: Vec::new(),
      density: 2.0,
    });
    let mut copied = frame_flora(&flora);
    assert!(copied.species_rules.is_empty());
    copied.species_rules = flora.species_rules.clone();
    assert_eq!(copied, flora);
  }

  #[test]
  fn weather_drives_only_the_enabled_effects() {
    let mut engine = generated_engine();
    let manual = engine.weathered_options();
    assert_eq!(manual.weather, FrameWeatherValues::default());

    engine
      .set_weather(WeatherOptions {
        enabled: true,
        state: vista_types::WeatherKind::Storm,
        transition_seconds: 0.0,
        effects: vista_types::WeatherEffects {
          water: false,
          ..Default::default()
        },
        ..Default::default()
      })
      .unwrap();

    for _ in 0..10 {
      engine.render_once().unwrap();
    }

    let stormy = engine.weathered_options();
    assert!(stormy.clouds.coverage > 0.75);
    assert!(stormy.clouds.towering > 0.5);
    assert!(stormy.clouds.rain_shafts > 0.5);
    assert_ne!(stormy.clouds.style, vista_types::CloudStyle::Off);
    assert!(stormy.weather.rain > 0.5);
    // Heavy rain is a full overcast, and a storm is heavier than full rain.
    assert!((stormy.weather.overcast - 1.0).abs() < 1e-6);
    assert!(stormy.weather.heaviness > 1.4);
    assert_eq!(stormy.water, engine.water);
    assert_eq!(
      engine.stats().weather,
      Some(vista_types::WeatherKind::Storm)
    );
    assert!(engine.weather().is_some());
  }

  #[test]
  fn custom_trees_replace_procedural_placement() {
    let mut engine = generated_engine();
    let tree = TreeInstance {
      position: [0.0, 10.0, 0.0],
      scale: 1.0,
      rotation: 0.0,
      tint: 0.5,
      species: 3,
      dryness: 0.0,
    };

    engine.set_tree_instances(Some(vec![tree; 3])).unwrap();
    assert_eq!(engine.stats().flora_instances, 3);
    assert!(engine
      .set_tree_instances(Some(vec![TreeInstance { species: 9, ..tree }]))
      .is_err());
    assert!(engine
      .set_tree_instances(Some(vec![TreeInstance {
        scale: f32::NAN,
        ..tree
      }]))
      .is_err());

    engine.set_tree_instances(None).unwrap();
    assert_ne!(engine.stats().flora_instances, 3);
  }

  #[test]
  fn replacement_textures_and_models_are_validated() {
    let mut engine = generated_engine();
    let texels = vec![0u8; (TEXTURE_LAYER_SIZE * TEXTURE_LAYER_SIZE * 4) as usize];

    assert!(engine
      .replace_texture(TextureTarget::Flora, 9, &texels)
      .is_ok());
    assert!(engine
      .replace_texture(TextureTarget::TerrainAlbedo, 10, &texels)
      .is_ok());
    // Layer 11 is scree; there is no twelfth.
    assert!(engine
      .replace_texture(TextureTarget::TerrainAlbedo, 11, &texels)
      .is_ok());
    assert!(engine
      .replace_texture(TextureTarget::TerrainAlbedo, 12, &texels)
      .is_err());
    assert!(engine
      .replace_texture(TextureTarget::TerrainNormal, 0, &texels[4..])
      .is_err());

    let positions = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 2.0, 0.0];
    let normals = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0];
    let uvs = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0];
    assert!(engine
      .set_tree_model(
        TreeSpeciesKind::Oak,
        &positions,
        &normals,
        &uvs,
        &[0, 1, 2],
        None,
        None
      )
      .is_ok());
    assert!(engine
      .set_tree_model(
        TreeSpeciesKind::Oak,
        &positions,
        &normals,
        &uvs,
        &[0, 1, 5],
        None,
        None
      )
      .is_err());
  }

  #[test]
  fn disposed_engine_rejects_use() {
    let mut engine = EngineCore::new_for_tests(VistaEngineOptions::default()).unwrap();
    engine.dispose().unwrap();

    assert!(engine.render_once().is_err());
  }

  #[test]
  fn render_stats_reflect_generated_terrain() {
    let mut engine = EngineCore::new_for_tests(VistaEngineOptions::default()).unwrap();
    let options = FractalTerrainOptions {
      size: 32,
      noise: NoiseOptions {
        kind: NoiseKind::Simplex,
        octaves: 3,
        gain: 0.5,
        lacunarity: 2.0,
        warp: Some(0.0),
      },
      ..FractalTerrainOptions::default()
    };

    futures_executor::block_on(engine.generate_fractal(options)).unwrap();
    let stats = engine.render_once().unwrap();

    assert!(stats.terrain_triangles > 0);
  }

  /// The default island of the visual checks (`scripts/visual-check`):
  /// seed 12345, 512 x 12 m.
  fn default_island() -> EngineCore {
    let mut engine = EngineCore::new_for_tests(VistaEngineOptions::default()).unwrap();
    let options = FractalTerrainOptions {
      seed: 12_345,
      size: 512,
      horizontal_scale_metres: 12.0,
      base_height_metres: None,
      noise: NoiseOptions {
        kind: NoiseKind::Ridged,
        octaves: 7,
        gain: 0.52,
        lacunarity: 2.05,
        warp: Some(0.15),
      },
      shape: Some(vista_types::TerrainShapeOptions {
        island: Some(0.35),
        ..Default::default()
      }),
      ..FractalTerrainOptions::default()
    };
    futures_executor::block_on(engine.generate_fractal(options)).unwrap();
    engine
  }

  #[test]
  fn the_lattice_keeps_the_old_counts() {
    // The grid the lattice replaced, once forests on steep slopes were
    // restored, placed 16,482 trees on the default island
    // at density 0.35, and 41,399 at 1 (on the CPU, every tree).
    let mut engine = default_island();

    for (density, old) in [(0.35, 16_482.0), (1.0, 41_399.0)] {
      let flora = FloraOptions {
        density,
        ..FloraOptions::default()
      };
      engine.set_flora(flora).unwrap();
      let count = engine.placed_trees().len() as f32;
      assert!(
        (count / old - 1.0).abs() <= 0.1,
        "{count} trees at density {density}, {old} before"
      );
      // So in the browser the far set is every tree, drawn as placed at
      // every distance, as before.
      assert_eq!(engine_far_keep(&engine), 1.0, "density {density}");
    }

    // A closed canopy is too many trees to hold: most are streamed.
    engine
      .set_flora(FloraOptions {
        density: 4.0,
        ..FloraOptions::default()
      })
      .unwrap();
    assert!(engine_far_keep(&engine) < 0.5);
  }

  fn engine_far_keep(engine: &EngineCore) -> f32 {
    let rules = crate::render::vegetation::TreeRules::new(&engine.flora, engine.tree_density());
    let mass = crate::render::vegetation::TileMass::trees(&engine.ground, &rules, &[]);
    crate::render::lattice::far_keep(mass.total())
  }

  fn generated_engine() -> EngineCore {
    let mut engine = EngineCore::new_for_tests(VistaEngineOptions::default()).unwrap();
    let options = FractalTerrainOptions {
      size: 128,
      horizontal_scale_metres: 30.0,
      vertical_scale: 1.0,
      shape: Some(vista_types::TerrainShapeOptions {
        island: Some(0.5),
        ..Default::default()
      }),
      ..FractalTerrainOptions::default()
    };
    futures_executor::block_on(engine.generate_fractal(options)).unwrap();
    engine
  }

  #[test]
  fn source_heights_match_the_map_restored_on_a_copy() {
    let mut engine = generated_engine();
    assert!(
      !engine.rivers.carved.is_empty(),
      "the test map has carved rivers"
    );
    // A glacier change on a carved sample, and one written twice: the
    // last write wins, as restoring in turn would leave it.
    let (first, _) = engine.rivers.carved[0];
    engine.glaciers = vec![(first, 123.0), (7, 4.0), (7, 5.0)];
    let terrain = engine.terrain.clone().unwrap();
    let mut expected = terrain.clone();
    restore_carving(&mut expected, &engine.rivers.carved);
    restore_glaciers(&mut expected, &engine.glaciers);

    let exported = engine
      .export_map(crate::export::MapKind::SourceHeight, None)
      .unwrap();
    let crate::export::MapData::F32(heights) = exported.data else {
      panic!("source heights are floats");
    };
    assert_eq!(heights, expected.heights);
    assert_eq!(heights[first], 123.0);
    assert_eq!(heights[7], 5.0);
  }

  #[test]
  fn biome_at_reports_biomes_inside_the_terrain_only() {
    let engine = generated_engine();

    assert!(engine.biome_at(0.0, 0.0).is_some());
    assert!(engine.biome_at(1.0e7, 0.0).is_none());
    assert!(engine.biome_at(f32::NAN, 0.0).is_none());
  }

  #[test]
  fn toggling_rivers_restores_the_original_terrain() {
    let mut engine = generated_engine();
    let mut water = WaterOptions::default();
    water.rivers.enabled = false;
    engine.set_water(water.clone()).unwrap();
    let uncarved = engine.export_heightmap().unwrap();

    water.rivers.enabled = true;
    water.rivers.min_catchment_km2 = 0.2;
    engine.set_water(water.clone()).unwrap();

    water.rivers.enabled = false;
    engine.set_water(water).unwrap();
    assert_eq!(engine.export_heightmap().unwrap(), uncarved);
  }

  /// A broad, gently sloping basin draining north to the sea, with a
  /// 30 m cliff across its upper valley: a waterfall above, meanders
  /// below.
  fn basin_engine() -> EngineCore {
    let mut engine = EngineCore::new_for_tests(VistaEngineOptions::default()).unwrap();
    let size = 256u32;
    let metadata = vista_types::TerrainMetadata {
      metres_per_sample: 40.0,
      sea_level_metres: 0.0,
      ..Default::default()
    };
    let mut heights = Vec::new();

    for y in 0..size {
      for x in 0..size {
        let cliff = if y >= 200 { 30.0 } else { 0.0 };
        heights.push(y as f32 * 0.2 - 1.5 + (x as f32 - 128.0).abs() * 0.6 + cliff);
      }
    }

    let map = HeightMap::from_values(
      size,
      size,
      heights,
      vec![false; (size * size) as usize],
      metadata,
    )
    .unwrap();
    engine.install_terrain(map, &mut |_, _| {});
    engine
  }

  #[test]
  fn toggling_rivers_restores_meanders_and_plunge_pools_exactly() {
    let mut engine = basin_engine();
    let mut water = WaterOptions::default();
    water.rivers.meanders = 1.0;
    engine.set_water(water.clone()).unwrap();

    assert!(!engine.rivers.falls.is_empty(), "no waterfall");
    let meandering = engine.rivers.reaches.iter().any(|reach| {
      reach
        .points
        .iter()
        .any(|point| (point.x - point.x.round()).abs() > 0.2)
    });
    assert!(meandering, "no meanders");
    assert!(!engine.rivers.carved.is_empty());

    water.rivers.enabled = false;
    engine.set_water(water.clone()).unwrap();
    let uncarved = engine.export_heightmap().unwrap();

    water.rivers.enabled = true;
    engine.set_water(water.clone()).unwrap();
    assert_ne!(engine.export_heightmap().unwrap(), uncarved);
    water.rivers.enabled = false;
    engine.set_water(water).unwrap();
    assert_eq!(engine.export_heightmap().unwrap(), uncarved);
  }

  /// Every placed tree stands within 0.05 m of the final ground at its
  /// position.
  fn assert_trees_on_final_ground(engine: &EngineCore) {
    let terrain = engine.terrain.as_ref().unwrap();
    let metres = terrain.metadata.metres_per_sample;
    let half_x = (terrain.metadata.width as f32 - 1.0) * metres * 0.5;
    let half_z = (terrain.metadata.height as f32 - 1.0) * metres * 0.5;
    let trees = engine.placed_trees();

    assert!(trees.len() > 100, "only {} trees", trees.len());

    for tree in trees {
      let [x, y, z] = tree.position;
      let ground = crate::render::terrain_mesh::full_detail_height(
        terrain,
        (x + half_x) / metres,
        (z + half_z) / metres,
      );
      assert!(
        (y - ground).abs() <= 0.05,
        "tree at ({x}, {z}) stands at {y}, ground {ground}"
      );
    }
  }

  #[test]
  fn trees_are_placed_after_every_height_change() {
    let mut engine = basin_engine();
    let mut flora = engine.flora.clone();
    flora.tree_line_metres = 10_000.0;
    engine.set_flora(flora).unwrap();
    let mut water = WaterOptions::default();
    engine.set_water(water.clone()).unwrap();
    assert!(!engine.rivers.carved.is_empty());
    assert_trees_on_final_ground(&engine);

    // Toggling rivers off restores the uncarved ground.
    water.rivers.enabled = false;
    engine.set_water(water.clone()).unwrap();
    assert!(engine.rivers.carved.is_empty());
    assert_trees_on_final_ground(&engine);
    water.rivers.enabled = true;
    engine.set_water(water).unwrap();
    assert_trees_on_final_ground(&engine);

    // Painted water carves the ground too.
    let mut engine = slope_engine();
    let mut flora = engine.flora.clone();
    flora.tree_line_metres = 10_000.0;
    engine.set_flora(flora).unwrap();
    engine
      .set_water_mask(Some(painted(128, |x, y| {
        if y == 64 && (30..100).contains(&x) {
          60
        } else {
          0
        }
      })))
      .unwrap();
    assert!(!engine.rivers.carved.is_empty());
    assert_trees_on_final_ground(&engine);
    engine.set_water_mask(None).unwrap();
    assert_trees_on_final_ground(&engine);
  }

  /// A plain sloping up to the east, with a sea along its west edge, and
  /// only painted rivers.
  fn slope_engine() -> EngineCore {
    let mut engine = EngineCore::new_for_tests(VistaEngineOptions::default()).unwrap();
    let size = 128u32;
    let metadata = vista_types::TerrainMetadata {
      metres_per_sample: 40.0,
      sea_level_metres: 0.0,
      ..Default::default()
    };
    let heights = (0..size * size)
      .map(|i| (i % size) as f32 * 0.8 - 6.0 + ((i / size) as f32 * 0.3).sin() * 0.2)
      .collect();
    let map = HeightMap::from_values(
      size,
      size,
      heights,
      vec![false; (size * size) as usize],
      metadata,
    )
    .unwrap();
    engine.install_terrain(map, &mut |_, _| {});
    let mut water = WaterOptions::default();
    water.rivers.enabled = false;
    engine.set_water(water).unwrap();
    engine
  }

  fn painted(size: u32, paint: impl Fn(u32, u32) -> u8) -> vista_types::WaterMask {
    vista_types::WaterMask {
      width: size,
      height: size,
      data: (0..size * size)
        .map(|i| paint(i % size, i / size))
        .collect(),
    }
  }

  #[test]
  fn water_masks_are_validated() {
    let mut engine = slope_engine();
    let mut mask = painted(64, |_, _| 0);
    mask.data.pop();
    let error = engine.set_water_mask(Some(mask)).unwrap_err().to_string();
    assert!(error.contains("4096 bytes"), "{error}");
    assert!(engine
      .set_water_mask(Some(vista_types::WaterMask {
        width: 1,
        height: 5,
        data: vec![0; 5],
      }))
      .is_err());
    // A mask of another size is resampled, with a warning.
    let warning = engine.set_water_mask(Some(painted(64, |_, _| 0))).unwrap();
    assert!(warning.unwrap().contains("resampled"));
    assert!(engine
      .set_water_mask(Some(painted(128, |_, _| 0)))
      .unwrap()
      .is_none());
  }

  #[test]
  fn a_painted_line_becomes_one_river_running_downhill() {
    let mut engine = slope_engine();
    engine
      .set_water_mask(Some(painted(128, |x, y| {
        if y == 64 && (30..100).contains(&x) {
          60
        } else {
          0
        }
      })))
      .unwrap();

    assert_eq!(engine.rivers.reaches.len(), 1);
    let points = &engine.rivers.reaches[0].points;
    assert!(
      points[0].x > points[points.len() - 1].x,
      "runs west, downhill"
    );
    assert!(points.iter().all(|p| p.width >= 28.0));

    // Beside the river it is loud; 2 km away it cannot be heard.
    let middle = points[points.len() / 2];
    let (x, z) = (middle.x * 40.0 - 2540.0, middle.y * 40.0 - 2540.0);
    let near = engine
      .water_sounds(x, middle.level + 1.0, z + 5.0)
      .river
      .unwrap();
    assert!(near.loudness > 0.5, "loudness {}", near.loudness);
    assert!(engine
      .water_sounds(x, middle.level, z + 2000.0)
      .river
      .is_none());
  }

  #[test]
  fn a_painted_blob_becomes_one_lake_and_clearing_restores_the_terrain() {
    let mut engine = slope_engine();
    let before = engine.export_heightmap().unwrap();
    engine
      .set_water_mask(Some(painted(128, |x, y| {
        let (dx, dy) = (x as f32 - 80.0, y as f32 - 60.0);
        if dx * dx + dy * dy < 100.0 {
          200
        } else {
          0
        }
      })))
      .unwrap();

    assert_eq!(engine.rivers.lakes.len(), 1);
    assert_ne!(engine.export_heightmap().unwrap(), before);

    engine.set_water_mask(None).unwrap();
    assert!(engine.rivers.lakes.is_empty());
    assert_eq!(engine.export_heightmap().unwrap(), before);
  }

  #[test]
  fn a_mask_survives_water_changes_and_is_cleared_by_new_terrain() {
    let mut engine = slope_engine();
    engine
      .set_water_mask(Some(painted(128, |x, y| {
        u8::from(y == 64 && (30..100).contains(&x)) * 60
      })))
      .unwrap();
    let mut water = WaterOptions::default();
    water.rivers.enabled = false;
    water.rivers.width_scale = 2.0;
    engine.set_water(water).unwrap();
    assert_eq!(engine.rivers.reaches.len(), 1);

    let map = engine.terrain.clone().unwrap();
    engine.install_terrain(map, &mut |_, _| {});
    assert!(engine.water_mask.is_none());
    assert!(engine.rivers.reaches.is_empty());
  }

  #[test]
  fn waterfalls_are_listed_where_their_water_lands() {
    let mut engine = basin_engine();
    engine.set_water(WaterOptions::default()).unwrap();
    let falls = engine.waterfalls();

    assert_eq!(falls.len(), engine.rivers.falls.len());
    assert!(!falls.is_empty());
    let fall = &falls[0];
    let ground = engine.surface_at(fall.position[0], fall.position[2]);
    assert!(ground.is_some());
    assert!(fall.height_metres >= 3.0 && fall.width_metres > 0.0);
    assert!(fall.discharge_cubic_metres_per_second > 0.0);
  }

  #[test]
  fn patching_the_surface_after_rivers_matches_a_full_classification() {
    for mut engine in [generated_engine(), basin_engine(), cold_engine(-20.0)] {
      engine.set_water(WaterOptions::default()).unwrap();
      let terrain = engine.terrain.as_ref().unwrap();
      let normals = crate::terrain::normals::generate_normals(terrain);
      let mut full = crate::terrain::biomes::classify_surface_with(
        terrain,
        &normals,
        Some(&engine.rivers.mask),
        &engine.rivers.riparian,
        &engine.biomes,
        &engine.soil(),
      );
      crate::terrain::biomes::apply_bed_materials(&mut full, &engine.rivers.bed);

      assert!(!engine.rivers.carved.is_empty());
      assert!(engine.rivers.riparian.iter().any(|value| *value > 0));
      assert!(engine.surface == full);
    }
  }

  #[test]
  fn trees_never_stand_inside_the_channel_mask() {
    let mut engine = generated_engine();
    engine.set_water(WaterOptions::default()).unwrap();
    let terrain = engine.terrain.as_ref().unwrap();
    let options = vista_types::FloraOptions {
      enabled: true,
      density: 1.0,
      ..vista_types::FloraOptions::default()
    };
    let trees = crate::render::flora::build_tree_instances(terrain, &engine.surface, &options, 1.0);
    let metres = terrain.metadata.metres_per_sample;
    let half = (terrain.metadata.width as f32 - 1.0) * 0.5;
    let sample = |world: f32| (world / metres + half).round() as usize;

    assert!(!trees.is_empty());
    for tree in &trees {
      let index =
        sample(tree.position[2]) * terrain.metadata.width as usize + sample(tree.position[0]);
      assert!(!engine.rivers.mask[index], "a tree at {:?}", tree.position);
    }
  }

  #[test]
  fn a_cold_climate_and_back_restores_the_terrain_exactly() {
    let mut engine = generated_engine();
    let original = engine.export_heightmap().unwrap();
    engine
      .set_biomes(BiomeOptions {
        mean_temperature_celsius: Some(-20.0),
        ..BiomeOptions::default()
      })
      .unwrap();

    assert!(!engine.glaciers.is_empty());
    assert_ne!(engine.export_heightmap().unwrap(), original);

    engine.set_biomes(BiomeOptions::default()).unwrap();
    assert_eq!(engine.export_heightmap().unwrap(), original);

    engine
      .set_biomes(BiomeOptions {
        mean_temperature_celsius: Some(-20.0),
        ..BiomeOptions::default()
      })
      .unwrap();
    engine
      .set_biomes(BiomeOptions {
        enabled: false,
        mean_temperature_celsius: Some(-20.0),
        ..BiomeOptions::default()
      })
      .unwrap();
    assert!(engine.glaciers.is_empty());
    assert_eq!(engine.export_heightmap().unwrap(), original);
  }

  fn cold_engine(celsius: f32) -> EngineCore {
    let mut engine = generated_engine();
    engine
      .set_biomes(BiomeOptions {
        mean_temperature_celsius: Some(celsius),
        ..BiomeOptions::default()
      })
      .unwrap();
    engine
  }

  #[test]
  fn cold_air_is_crisp_and_clear() {
    let mild = generated_engine().weathered_options();
    let cold = cold_engine(-20.0).weathered_options();

    assert!(cold.atmosphere.haze_distance_metres > mild.atmosphere.haze_distance_metres * 1.35);
    assert!(cold.atmosphere.mie_strength < mild.atmosphere.mie_strength * 0.75);
    assert!(cold_engine(-20.0).sea_ice_possible);
    assert!(!generated_engine().sea_ice_possible);
  }

  #[test]
  fn rain_falls_as_snow_where_the_camera_is_cold() {
    let mut engine = cold_engine(-20.0);
    engine
      .set_weather(WeatherOptions {
        enabled: true,
        state: vista_types::WeatherKind::Rain,
        transition_seconds: 0.0,
        ..Default::default()
      })
      .unwrap();
    engine.render_once().unwrap();
    let weather = engine.weathered_options().weather;

    assert_eq!(weather.rain, 0.0);
    assert!(weather.snow > 0.5);
  }

  #[test]
  fn strong_wind_over_lying_snow_blows_it_about() {
    let mut engine = cold_engine(-20.0);
    let terrain = engine.terrain.as_ref().unwrap();
    let width = terrain.metadata.width as usize;
    let metres = terrain.metadata.metres_per_sample;
    let glacier = engine
      .surface
      .iter()
      .position(|sample| sample.is_glacier())
      .unwrap();
    let x = ((glacier % width) as f32 - (width as f32 - 1.0) * 0.5) * metres;
    let z = ((glacier / width) as f32 - (terrain.metadata.height as f32 - 1.0) * 0.5) * metres;
    engine
      .set_camera(vista_types::CameraOptions {
        position: [x, 800.0, z],
        target: [x + 10.0, 790.0, z + 10.0],
        ..Default::default()
      })
      .unwrap();
    engine
      .set_weather(WeatherOptions {
        enabled: true,
        state: vista_types::WeatherKind::Storm,
        transition_seconds: 0.0,
        ..Default::default()
      })
      .unwrap();

    for _ in 0..4 {
      engine.render_once().unwrap();
    }

    let blowing = engine.weathered_options().weather.blowing_snow;
    assert!(blowing > 0.5, "blowing {blowing}");

    engine
      .set_weather(WeatherOptions {
        enabled: true,
        state: vista_types::WeatherKind::Clear,
        transition_seconds: 0.0,
        ..Default::default()
      })
      .unwrap();
    engine.render_once().unwrap();
    assert_eq!(engine.weathered_options().weather.blowing_snow, 0.0);
  }

  #[test]
  fn temperature_at_matches_the_surface_sample() {
    let mut engine = EngineCore::new_for_tests(VistaEngineOptions::default()).unwrap();
    assert_eq!(engine.celsius_at(0.0, 0.0), None);

    engine = generated_engine();
    let celsius = engine.celsius_at(0.0, 0.0).unwrap();
    let terrain = engine.terrain.as_ref().unwrap();
    let centre = (terrain.metadata.width / 2) as usize;
    let sample = engine.surface[centre * terrain.metadata.width as usize + centre];

    assert_eq!(celsius, sample.celsius());
    assert!(engine.celsius_at(1.0e7, 0.0).is_none());
    assert!(engine.celsius_at(f32::NAN, 0.0).is_none());
  }

  #[test]
  fn changing_biomes_changes_the_biome_map() {
    let mut engine = generated_engine();
    let before = engine.surface.clone();
    engine
      .set_biomes(BiomeOptions {
        temperature_bias: 1.0,
        moisture_bias: 1.0,
        ..BiomeOptions::default()
      })
      .unwrap();

    assert_ne!(before, engine.surface);
  }

  /// How many times each pipeline kind is created as the scene's needs
  /// are met, over several frames.
  fn created(engine: &EngineCore, slots: &mut PipelineSlots<()>) -> Vec<PipelineKind> {
    let (needs, _) = engine.pipeline_needs();
    let mut kinds = Vec::new();
    slots.ensure(&needs, |kind| kinds.push(kind));
    kinds
  }

  #[test]
  fn a_plain_scene_creates_no_pipelines_for_what_it_lacks() {
    let mut engine = basin_engine();
    let mut water = WaterOptions::default();
    water.rivers.enabled = false;
    engine.set_water(water).unwrap();
    engine
      .set_grass(GrassOptions {
        enabled: false,
        ..GrassOptions::default()
      })
      .unwrap();
    let kinds = created(&engine, &mut PipelineSlots::default());

    for kind in [
      PipelineKind::InlandWater,
      PipelineKind::Falls,
      PipelineKind::Clouds,
      PipelineKind::QuarterClouds,
      PipelineKind::Grass,
      PipelineKind::Present,
      PipelineKind::SeaIceOcean,
    ] {
      assert!(!kinds.contains(&kind), "{kind:?} was created");
    }

    assert!(kinds.contains(&PipelineKind::Terrain));
    assert!(kinds.contains(&PipelineKind::OpenOcean));
    // Screen reflections copy the scene; sky reflections do not.
    assert!(kinds.contains(&PipelineKind::SceneCopy));
    engine
      .set_water(WaterOptions {
        reflections: vista_types::WaterReflections::Sky,
        ..engine.water.clone()
      })
      .unwrap();
    let kinds = created(&engine, &mut PipelineSlots::default());
    assert!(!kinds.contains(&PipelineKind::SceneCopy));
  }

  #[test]
  fn a_full_scene_creates_each_pipeline_once_and_grass_when_it_appears() {
    let mut engine = basin_engine();
    engine
      .set_grass(GrassOptions {
        enabled: false,
        ..GrassOptions::default()
      })
      .unwrap();
    engine
      .set_clouds(CloudsOptions {
        style: vista_types::CloudStyle::Volumetric,
        coverage: 0.5,
        ..CloudsOptions::default()
      })
      .unwrap();
    engine
      .set_render_quality(RenderQualityOptions {
        render_scale: Some(0.75),
        ..RenderQualityOptions::default()
      })
      .unwrap();
    engine.render_once().unwrap();
    let mut slots = PipelineSlots::default();
    let mut kinds = created(&engine, &mut slots);
    kinds.extend(created(&engine, &mut slots));

    for kind in [
      PipelineKind::InlandWater,
      PipelineKind::Falls,
      PipelineKind::Clouds,
      PipelineKind::Present,
      PipelineKind::Terrain,
    ] {
      assert_eq!(kinds.iter().filter(|k| **k == kind).count(), 1, "{kind:?}");
    }

    assert!(!kinds.contains(&PipelineKind::Grass));
    engine
      .set_grass(GrassOptions {
        enabled: true,
        ..GrassOptions::default()
      })
      .unwrap();
    assert!(engine.stats.grass_instances > 0);
    // Grass streams on the GPU: its generator and cull pass come with it.
    assert_eq!(
      created(&engine, &mut slots),
      [
        PipelineKind::GrassGenerate,
        PipelineKind::GrassCull,
        PipelineKind::Grass
      ]
    );
  }

  #[test]
  fn boulder_pipelines_come_only_with_boulders_in_view() {
    let mut engine = default_island();
    let boulder_kinds = [
      PipelineKind::BoulderGenerate,
      PipelineKind::BoulderCull,
      PipelineKind::BoulderShadow,
      PipelineKind::Boulders,
    ];
    // A texel of talus: where boulders lie.
    let texel = engine
      .ground
      .banks
      .iter()
      .position(|banks| banks[3] > 0)
      .expect("the island's crags shed talus");
    let ground = &engine.ground;
    let at = [
      (texel as u32 % ground.width) as f32 * ground.texel_metres - ground.half[0],
      (texel as u32 / ground.width) as f32 * ground.texel_metres - ground.half[1],
    ];
    let height = ground.height_at(at[0], at[1]);
    let look = |engine: &mut EngineCore, from: [f32; 3]| {
      engine
        .set_camera(CameraOptions {
          position: from,
          target: [at[0], height, at[1]],
          ..CameraOptions::default()
        })
        .unwrap();
      engine.render_once().unwrap();
      engine.render_once().unwrap();
    };
    let near = [at[0] + 40.0, height + 30.0, at[1] + 40.0];
    let mut slots = PipelineSlots::default();

    // Boulders off: nothing streams, and no pipeline is made.
    engine
      .set_surface(SurfaceOptions {
        boulders: false,
        ..SurfaceOptions::default()
      })
      .unwrap();
    assert!(engine.streams.boulders.is_none());
    look(&mut engine, near);
    assert!(!created(&engine, &mut slots)
      .iter()
      .any(|kind| boulder_kinds.contains(kind)));

    // On, but the camera looks at them from 5 km away: still none.
    engine.set_surface(SurfaceOptions::default()).unwrap();
    assert!(engine.streams.boulders.is_some());
    look(
      &mut engine,
      [at[0] + 3_600.0, height + 400.0, at[1] + 3_600.0],
    );
    assert!(!engine.pipeline_needs().0.boulders);
    assert!(!created(&engine, &mut slots)
      .iter()
      .any(|kind| boulder_kinds.contains(kind)));

    // In view: the generator, the cull pass, the shadow casters and the
    // boulders themselves, once each.
    look(&mut engine, near);
    let kinds = created(&engine, &mut slots);
    assert_eq!(
      kinds
        .iter()
        .filter(|kind| boulder_kinds.contains(kind))
        .count(),
      4,
      "{kinds:?}"
    );
    assert!(created(&engine, &mut slots).is_empty());
  }

  #[test]
  fn inflows_are_reported_and_validated_against_the_map() {
    // The basin's upper valley runs off the south edge: an open edge.
    let mut engine = basin_engine();
    let auto = engine.inflows();
    assert_eq!(auto.len(), 1, "{auto:?}");
    let half = 255.0 * 40.0 * 0.5;
    assert!((auto[0].position[2] - half).abs() < 1.0, "{auto:?}");
    assert!(auto[0].discharge_cubic_metres_per_second > 1.0);

    let mut water = WaterOptions::default();
    water.rivers.inflow = vista_types::RiverInflows::List(vec![vista_types::RiverInflow {
      position: [0.0, 1200.0],
      discharge_cubic_metres_per_second: 80.0,
    }]);
    engine.set_water(water.clone()).unwrap();
    let explicit = engine.inflows();
    assert_eq!(explicit.len(), 1);
    assert_eq!(explicit[0].discharge_cubic_metres_per_second, 80.0);

    water.rivers.inflow = vista_types::RiverInflows::List(vec![vista_types::RiverInflow {
      position: [half + 50.0, 0.0],
      discharge_cubic_metres_per_second: 80.0,
    }]);
    let error = engine.set_water(water).unwrap_err().to_string();
    assert!(error.contains("off the map"), "{error}");

    let mut water = WaterOptions::default();
    water.rivers.inflow = vista_types::RiverInflows::Mode(vista_types::InflowMode::None);
    engine.set_water(water).unwrap();
    assert!(engine.inflows().is_empty());
  }

  #[test]
  fn a_cascade_is_listed_once_with_its_total_drop() {
    let mut engine = EngineCore::new_for_tests(VistaEngineOptions::default()).unwrap();
    let size = 128u32;
    let metadata = vista_types::TerrainMetadata {
      metres_per_sample: 2.0,
      sea_level_metres: 0.0,
      ..Default::default()
    };
    // Three 8 m steps 6 samples apart, in a valley draining north.
    let heights = (0..size * size)
      .map(|i| {
        let (x, y) = ((i % size) as f32, (i / size) as f32);
        let steps = [40.0, 46.0, 52.0].iter().filter(|at| y >= **at).count() as f32;
        y * 0.1 - 0.5 + (x - 64.0).abs() * 0.5 + steps * 8.0
      })
      .collect();
    let map = HeightMap::from_values(
      size,
      size,
      heights,
      vec![false; (size * size) as usize],
      metadata,
    )
    .unwrap();
    engine.install_terrain(map, &mut |_, _| {});
    let mut water = WaterOptions::default();
    water.rivers.min_catchment_km2 = 0.005;
    water.rivers.inflow = vista_types::RiverInflows::List(vec![vista_types::RiverInflow {
      position: [0.0, 120.0],
      discharge_cubic_metres_per_second: 3.0,
    }]);
    engine.set_water(water).unwrap();
    let listed: Vec<_> = engine
      .waterfalls()
      .into_iter()
      .filter(|fall| fall.discharge_cubic_metres_per_second > 1.0)
      .collect();

    assert_eq!(listed.len(), 1, "{listed:?}");
    assert!(listed[0].height_metres > 20.0, "{listed:?}");
  }

  #[test]
  fn without_weather_the_uniforms_leave_the_atmosphere_as_it_was() {
    let engine = generated_engine();
    let out = engine.weathered_options();

    // Neutral: the Mie colour white with the old phase asymmetry, and the
    // light under cloud untouched.
    assert_eq!(out.weather.air, [1.0, 1.0, 1.0, 0.76]);
    assert_eq!(out.weather.light, [1.0, 1.0, 1.0, 0.0]);
    assert_eq!(out.weather.regional[3], 0.0);
    assert_eq!(out.weather.gust[3], 0.0);
    assert_eq!(out.weather.sea, [0.0; 4]);
    // Only the cold-air rule (none in this mild climate) touches the
    // atmosphere, as before presets.
    assert_eq!(out.atmosphere.mie_strength, engine.atmosphere.mie_strength);
    assert_eq!(
      out.atmosphere.haze_distance_metres,
      engine.atmosphere.haze_distance_metres
    );
  }

  #[test]
  fn advance_weather_rejects_bad_skips() {
    let mut engine = generated_engine();

    for seconds in [-1.0, f32::NAN, f32::INFINITY, 86_401.0] {
      let error = engine.advance_weather(seconds).unwrap_err().to_string();
      assert!(error.contains("advanceWeather"), "{error}");
    }

    assert!(engine.advance_weather(0.0).is_ok());
    assert!(engine.advance_weather(86_400.0).is_ok());
  }

  #[test]
  fn weather_at_needs_a_terrain() {
    let engine = EngineCore::new_for_tests(VistaEngineOptions::default()).unwrap();
    assert!(engine.weather_at(0.0, 0.0).is_none());

    let terrain = generated_engine();
    assert!(terrain.weather_at(0.0, 0.0).is_some());
    assert!(terrain.weather_at(f32::NAN, 0.0).is_none());
  }

  #[test]
  fn skipping_ahead_wets_and_dries_the_ground() {
    let mut engine = generated_engine();
    let rain = |state| WeatherOptions {
      enabled: true,
      state,
      transition_seconds: 1.0,
      regional: false,
      ..Default::default()
    };
    engine
      .set_weather(rain(vista_types::WeatherKind::Clear))
      .unwrap();
    engine.advance_weather(10.0).unwrap();
    let dry = engine.weather_at(0.0, 0.0).unwrap();
    engine
      .set_weather(rain(vista_types::WeatherKind::Rain))
      .unwrap();
    engine.advance_weather(600.0).unwrap();
    let wet = engine.weather_at(0.0, 0.0).unwrap();
    engine
      .set_weather(rain(vista_types::WeatherKind::Clear))
      .unwrap();
    engine.advance_weather(3_600.0).unwrap();
    let dried = engine.weather_at(0.0, 0.0).unwrap();

    assert!(dry.wetness < 0.05, "{dry:?}");
    assert!(wet.wetness > 0.7 && wet.precipitation > 0.5, "{wet:?}");
    assert!(dried.wetness < wet.wetness * 0.8, "{dried:?}");
    assert_eq!(dried.precipitation, 0.0);
    assert_eq!(
      engine.weather().unwrap().to,
      vista_types::WeatherKind::Clear
    );
  }

  #[test]
  fn time_of_day_validates_and_moves_the_sun() {
    let mut engine = generated_engine();
    let bad = [
      vista_types::TimeOfDayOptions {
        hours: 25.0,
        ..Default::default()
      },
      vista_types::TimeOfDayOptions {
        latitude_degrees: 90.0,
        ..Default::default()
      },
      vista_types::TimeOfDayOptions {
        day_of_year: 0,
        ..Default::default()
      },
      vista_types::TimeOfDayOptions {
        day_of_year: 367,
        ..Default::default()
      },
      vista_types::TimeOfDayOptions {
        day_length_minutes: 0.5,
        ..Default::default()
      },
      vista_types::TimeOfDayOptions {
        hours: f32::NAN,
        ..Default::default()
      },
    ];

    for options in bad {
      let error = engine
        .set_time_of_day(options.clone())
        .unwrap_err()
        .to_string();
      assert!(error.contains("timeOfDay."), "{options:?}: {error}");
    }

    let noon = vista_types::TimeOfDayOptions {
      enabled: true,
      hours: 12.0,
      ..Default::default()
    };
    engine.set_time_of_day(noon.clone()).unwrap();
    let high = engine.time_of_day();
    engine
      .set_time_of_day(vista_types::TimeOfDayOptions {
        hours: 19.0,
        ..noon
      })
      .unwrap();
    let low = engine.time_of_day();

    assert!(high.sun_elevation_degrees > 60.0, "{high:?}");
    assert!(low.sun_elevation_degrees < 15.0, "{low:?}");
    assert!(low.sunset_hours.unwrap() > 19.0);
    // The time of day lights the scene, whatever `setSun` said.
    assert!((engine.sun_now().elevation_degrees - low.sun_elevation_degrees).abs() < 1e-4);
  }
}
