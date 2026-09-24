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
#[cfg(target_arch = "wasm32")]
use crate::maths::{cross, normalise, sub};
use crate::render::flora::TreeInstance;
use crate::render::tree_models::{layers, mesh_from_arrays, TreeMesh};
use crate::render::water::{build_river_network, restore_carving, RiverNetwork};
use crate::terrain::biomes::SurfaceSample;
#[cfg(not(target_arch = "wasm32"))]
use crate::terrain::clipmap::build_clipmap_levels;
use crate::terrain::fractal::Progress;
#[cfg(not(target_arch = "wasm32"))]
use crate::terrain::generate_fractal_heightmap_with_progress;
use crate::terrain::glaciers::{restore_glaciers, shape_glaciers};
use crate::terrain::HeightMap;
use crate::weather::WeatherSystem;

/// Edge length, in texels, of every replaceable texture layer.
pub const TEXTURE_LAYER_SIZE: u32 = 512;
/// Number of terrain material texture layers.
pub const TERRAIN_TEXTURE_LAYERS: u32 = 10;
/// Largest custom tree instance list accepted by `set_tree_instances`.
pub const MAX_CUSTOM_TREES: usize = 1_000_000;

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
  shadows: ShadowOptions,
  surface_options: SurfaceOptions,
  /// Host-supplied trees that replace procedural placement, if any.
  custom_trees: Option<Vec<TreeInstance>>,
  /// Lowest and highest terrain heights, for fitting the shadow map.
  height_range: (f32, f32),
  /// Time of the previous frame in milliseconds, for the weather clock.
  last_frame_ms: Option<f64>,
  frame_clock: crate::pacing::FrameClock,
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
/// the enabled effects.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
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
      weather: WeatherSystem::new(config.weather),
      shadows: config.shadows,
      surface_options: config.surface,
      custom_trees: None,
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
      weather: WeatherSystem::new(config.weather),
      shadows: config.shadows,
      surface_options: config.surface,
      custom_trees: None,
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
  /// `"detail"`, `"erosion"` (when erosion is requested) and
  /// `"finishing"` (conditioning the map and building rivers, flora and
  /// the terrain mesh).
  pub async fn generate_fractal_with_progress(
    &mut self,
    options: FractalTerrainOptions,
    progress: Progress<'_>,
  ) -> VistaResult<TerrainHandle> {
    self.ensure_live()?;
    self.state = EngineState::LoadingTerrain;
    let map = self.generate_fractal_map(&options, progress).await?;
    let handle = self.install_terrain(map);
    progress("finishing", 1.0);
    self.state = EngineState::Ready;
    Ok(handle)
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
    crate::terrain::finish_fractal_heightmap(&mut map);
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
    self.state = EngineState::LoadingTerrain;
    let map = decode_geotiff(bytes, &options)?;
    let handle = self.install_terrain(map);
    self.state = EngineState::Ready;
    Ok(handle)
  }

  /// Decode a raw heightmap from bytes.
  pub async fn load_raw_heightmap(
    &mut self,
    bytes: &[u8],
    options: RawHeightmapOptions,
  ) -> VistaResult<TerrainHandle> {
    self.ensure_live()?;
    self.state = EngineState::LoadingTerrain;
    let map = decode_raw_heightmap(bytes, &options)?;
    let handle = self.install_terrain(map);
    self.state = EngineState::Ready;
    Ok(handle)
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
    self.sun = sun;
    Ok(())
  }

  /// Replace atmosphere controls.
  pub fn set_atmosphere(&mut self, atmosphere: AtmosphereOptions) -> VistaResult<()> {
    self.ensure_live()?;
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
    self.flora = flora;
    self.refresh_flora();
    Ok(())
  }

  /// Replace grass controls.
  pub fn set_grass(&mut self, grass: GrassOptions) -> VistaResult<()> {
    self.ensure_live()?;
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
    self.clouds = clouds;
    Ok(())
  }

  /// Replace mist/ground-fog controls. Like clouds, mist has no
  /// terrain-dependent placement.
  pub fn set_mist(&mut self, mist: MistOptions) -> VistaResult<()> {
    self.ensure_live()?;
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

  /// Replace shadow controls.
  pub fn set_shadows(&mut self, shadows: ShadowOptions) -> VistaResult<()> {
    self.ensure_live()?;
    self.shadows = shadows;
    Ok(())
  }

  /// Replace terrain surface controls.
  pub fn set_surface(&mut self, surface: SurfaceOptions) -> VistaResult<()> {
    self.ensure_live()?;

    #[cfg(target_arch = "wasm32")]
    self.gpu.set_material_tints(&surface.material_tints);

    self.surface_options = surface;
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

  #[cfg_attr(not(target_arch = "wasm32"), allow(unused_variables))]
  fn install_tree_model(&mut self, species: TreeSpeciesKind, mesh: Option<TreeMesh>) {
    #[cfg(target_arch = "wasm32")]
    self.gpu.set_tree_model(species.index(), mesh);
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

  /// Render one frame.
  pub fn render_once(&mut self) -> VistaResult<RenderStats> {
    self.ensure_live()?;

    // While the GPU is still drawing earlier frames, skip this one instead
    // of queueing it. The unchanged `frame_index` tells the caller that
    // nothing was drawn.
    #[cfg(target_arch = "wasm32")]
    if self.gpu.is_busy() {
      return Ok(self.stats.clone());
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

    if self.weather.options().enabled {
      self.weather.advance(dt);
      self.stats.weather = Some(self.weather.dominant());
    } else {
      self.stats.weather = None;
    }

    // Native builds have no GPU mesh to measure, so report a theoretical
    // estimate based on the configured clipmap level budget. Browser
    // builds report the real uploaded mesh below instead.
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(terrain) = &self.terrain {
      let levels = build_clipmap_levels(
        terrain.metadata.width,
        terrain.metadata.metres_per_sample,
        self.quality.max_clipmap_levels.unwrap_or(7),
      );
      self.stats.clipmap_levels = levels.len() as u32;
      self.stats.terrain_triangles = levels.iter().map(|level| level.index_count / 3).sum();
    }

    #[cfg(target_arch = "wasm32")]
    {
      self.recentre_terrain_mesh_if_needed(dt);
      let params = self.frame_params();
      self.gpu.render_once(&params)?;
      self.stats.gpu_pass_times_ms = self.gpu.pass_times();
      self.stats.gpu_frame_time_ms = self.stats.gpu_pass_times_ms.map(|times| {
        times.shadows
          + times.tree_culling
          + times.terrain
          + times.trees
          + times.grass
          + times.clouds
          + times.sky_and_fog
          + times.water
          + times.present
      });
    }

    Ok(self.stats.clone())
  }

  /// Resize the renderer.
  pub fn resize(
    &mut self,
    width: u32,
    height: u32,
    device_pixel_ratio: Option<f32>,
  ) -> VistaResult<()> {
    self.ensure_live()?;

    if width == 0 || height == 0 {
      return Err(VistaError::options(
        "resize width and height must be at least 1.",
      ));
    }

    let ratio = device_pixel_ratio.unwrap_or(self.device_pixel_ratio);

    if !ratio.is_finite() || ratio <= 0.0 || ratio > 8.0 {
      return Err(VistaError::options(
        "resize devicePixelRatio must be a finite value in the range 0 to 8.",
      ));
    }

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

    #[cfg(target_arch = "wasm32")]
    {
      self.terrain_normals = Vec::new();
      self.mesh_centre_sample = None;
      self.mesh_stream = None;
    }

    Ok(())
  }

  fn install_terrain(&mut self, map: HeightMap) -> TerrainHandle {
    let id = self.next_terrain_id;
    self.next_terrain_id = self.next_terrain_id.saturating_add(1);

    // The previous terrain's carving belongs to a different heightmap.
    self.rivers = RiverNetwork::default();
    self.applied_rivers = None;
    self.glaciers = Vec::new();
    self.terrain = Some(map);
    self.active_terrain_id = Some(id);

    #[cfg(target_arch = "wasm32")]
    {
      self.mesh_centre_sample = None;
      self.mesh_stream = None;
    }

    self.rebuild_world();
    let metadata = self
      .terrain
      .as_ref()
      .map(|terrain| terrain.metadata.clone())
      .unwrap_or_default();

    TerrainHandle { id, metadata }
  }

  /// River options that should currently be carved, if any.
  fn wanted_rivers(&self, water: &WaterOptions) -> Option<RiverOptions> {
    if water.enabled && water.rivers.enabled {
      Some(water.rivers.clone())
    } else {
      None
    }
  }

  /// Re-shape glaciers and re-extract rivers (restoring any previous
  /// shaping and carving first), then re-bake surface shading and every
  /// terrain-dependent layer.
  fn rebuild_world(&mut self) {
    let wanted = self.wanted_rivers(&self.water);

    if let Some(terrain) = self.terrain.as_mut() {
      // Undo in the reverse order of shaping: rivers were carved into the
      // glacier surface.
      restore_carving(terrain, &self.rivers.carved);
      restore_glaciers(terrain, &self.glaciers);
      self.glaciers = shape_glaciers(terrain, &self.biomes);
      self.rivers = match &wanted {
        Some(options) => build_river_network(terrain, options),
        None => RiverNetwork {
          mask: vec![false; terrain.heights.len()],
          ..RiverNetwork::default()
        },
      };
    } else {
      self.rivers = RiverNetwork::default();
      self.glaciers = Vec::new();
    }

    self.applied_rivers = wanted;
    self.height_range = self
      .terrain
      .as_ref()
      .map_or((0.0, 0.0), terrain_height_range);

    #[cfg(target_arch = "wasm32")]
    if let Some(terrain) = self.terrain.as_ref() {
      self.gpu.upload_heightmap(terrain);
      self
        .gpu
        .upload_rivers(&self.rivers.vertices, &self.rivers.indices);
    }

    self.rebake_surface();
  }

  /// Re-bake normals and biome/surface data, rebuild the terrain mesh, and
  /// refresh trees, grass, and water.
  fn rebake_surface(&mut self) {
    match self.terrain.as_ref() {
      Some(terrain) => {
        let mask = if self.rivers.mask.len() == terrain.heights.len() {
          Some(self.rivers.mask.as_slice())
        } else {
          None
        };
        let (normals, surface) =
          crate::render::terrain_mesh::bake_terrain_shading(terrain, &self.biomes, mask);
        self.surface = surface;

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
          self.gpu.upload_surface(terrain, &self.surface);
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
      }
    }

    self.refresh_flora();
    self.refresh_grass();
    self.refresh_water();
  }

  /// Regenerate tree instances for the active terrain and current flora and
  /// quality settings, uploading them to the GPU on browser builds.
  fn refresh_flora(&mut self) {
    if let Some(custom) = &self.custom_trees {
      self.stats.flora_instances = custom.len() as u32;

      #[cfg(target_arch = "wasm32")]
      self.gpu.upload_trees(custom);

      return;
    }

    let density_scale = self.quality.flora_density_scale.unwrap_or(1.0);
    let instances = match &self.terrain {
      Some(terrain) => crate::render::flora::build_tree_instances(
        terrain,
        &self.surface,
        &self.flora,
        density_scale,
      ),
      None => Vec::new(),
    };
    self.stats.flora_instances = instances.len() as u32;

    #[cfg(target_arch = "wasm32")]
    self.gpu.upload_trees(&instances);
  }

  /// Regenerate grass tuft instances for the active terrain and current
  /// grass and quality settings, uploading them to the GPU on browser
  /// builds. Mirrors `refresh_flora`; reuses the terrain's baked surface
  /// samples so grass follows the biome map.
  fn refresh_grass(&mut self) {
    let density_scale = self.quality.flora_density_scale.unwrap_or(1.0);
    let surface = if self.surface.is_empty() {
      None
    } else {
      Some(self.surface.as_slice())
    };
    let instances = match &self.terrain {
      Some(terrain) => {
        crate::render::grass::build_grass_instances(terrain, surface, &self.grass, density_scale)
      }
      None => Vec::new(),
    };
    self.stats.grass_instances = instances.len() as u32;

    #[cfg(target_arch = "wasm32")]
    self.gpu.upload_grass(&instances);
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
      water: self.water.clone(),
      mist: self.mist.clone(),
      clouds: self.clouds.clone(),
      flora: self.flora.clone(),
      weather: FrameWeatherValues::default(),
    };
    let options = self.weather.options();

    if !options.enabled {
      return out;
    }

    let state = self.weather.state();
    let effects = &options.effects;
    let wind = state.wind_speed_metres_per_second;

    if effects.clouds {
      // Weather without clouds would look wrong, so switch them on.
      if out.clouds.style == vista_types::CloudStyle::Off {
        out.clouds.style = vista_types::CloudStyle::Volumetric;
      }

      out.clouds.coverage = state.cloud_coverage;
      out.clouds.density = state.cloud_density;
      out.clouds.thickness_metres *= self.weather.cloud_thickness_scale();
      out.clouds.stratiform = state.stratiform;
      out.clouds.towering = state.towering;
      out.clouds.base_darkness = state.base_darkness;
      out.clouds.ragged_base = state.ragged_base;
      out.clouds.rain_shafts = state.rain_shafts;
      // The sky greys with full cover and darkens further under
      // rain-laden cloud. A near-complete deck (rain's 97 % cover) is a full
      // overcast: no direct sun, no sharp shadows, no glint on the water.
      // Heavy rain means a full deck overhead too, even between storm cells.
      out.weather.overcast = ((state.cloud_coverage - 0.6) / 0.35)
        .clamp(0.0, 1.0)
        .max(state.base_darkness * 0.9)
        .max(state.rain);
    }

    if effects.mist {
      if out.mist.style == vista_types::MistStyle::Off && state.mist_density > 0.01 {
        out.mist.style = vista_types::MistStyle::Volumetric;
      }

      out.mist.density = state.mist_density;
      out.atmosphere.haze_distance_metres *= self.weather.haze_scale();
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
    }

    if effects.water {
      let sea_state = (0.35 + wind / 7.0).min(3.5);
      let waves = &mut out.water.waves;
      waves.amplitude_metres = (waves.amplitude_metres * sea_state).min(30.0);
      waves.steepness = (waves.steepness * (0.7 + wind / 25.0)).min(1.0);
      waves.direction_degrees = state.wind_direction_degrees;
      out.water.foam = out.water.foam.max((wind - 6.0) / 14.0).clamp(0.0, 1.0);
      out.water.current_direction_degrees = state.wind_direction_degrees;
      out.water.current_speed *= 0.6 + wind / 12.0;
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
    }

    if effects.lightning {
      out.weather.lightning = state.lightning;
      let offset = self.weather.lightning_offset();
      let camera = self.camera.options.position;
      out.weather.lightning_position = [camera[0] + offset[0], camera[2] + offset[1]];
    }

    out
  }

  /// Collect every per-frame shading parameter for the GPU.
  #[cfg(target_arch = "wasm32")]
  fn frame_params(&self) -> crate::render::gpu::FrameParams {
    let view_proj =
      crate::maths::mat4_multiply(self.camera.projection_matrix, self.camera.view_matrix);
    let sun_direction =
      crate::maths::sun_direction_vector(self.sun.azimuth_degrees, self.sun.elevation_degrees);
    let camera_forward = normalise(sub(
      self.camera.options.target,
      self.camera.options.position,
    ));
    let camera_right = normalise(cross(camera_forward, [0.0, 1.0, 0.0]));
    let camera_up = cross(camera_right, camera_forward);
    let aspect_ratio = self.render_width as f32 / self.render_height.max(1) as f32;
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
    let tree_style = match flora.tree_quality {
      vista_types::TreeQuality::Billboard => 0,
      vista_types::TreeQuality::CrossQuad => 1,
      vista_types::TreeQuality::Mesh => 2,
    };

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
      clouds,
      tree_style,
      flora,
      grass_view_distance_metres: self.grass.view_distance_metres,
      debug_view: debug_view_index(self.debug_view),
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
        lens_drops: weather.lens_drops,
      },
      height_range: self.height_range,
      render_scale: self.stats.render_scale,
      frame_seconds: self.frame_seconds,
      distances: self.quality.distances(),
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
    let now = js_sys::Date::now();
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
      "setTreeInstances accepts at most {MAX_CUSTOM_TREES} trees."
    )));
  }

  for (index, tree) in trees.iter().enumerate() {
    let finite = tree.position.iter().all(|value| value.is_finite())
      && tree.scale.is_finite()
      && tree.rotation.is_finite()
      && tree.tint.is_finite()
      && tree.dryness.is_finite();

    if !finite || tree.scale <= 0.0 || tree.scale > 20.0 {
      return Err(VistaError::options(format!(
        "tree {index} must have finite values and a scale between 0 and 20."
      )));
    }

    if tree.species as usize >= TreeSpeciesKind::ALL.len() {
      return Err(VistaError::options(format!(
        "tree {index} has species {}, but species must be 0 to 7.",
        tree.species
      )));
    }
  }

  Ok(())
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
  use vista_types::{FractalTerrainOptions, NoiseKind, NoiseOptions, VistaEngineOptions};

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
      .replace_texture(TextureTarget::TerrainAlbedo, 9, &texels)
      .is_ok());
    assert!(engine
      .replace_texture(TextureTarget::TerrainAlbedo, 10, &texels)
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
}
