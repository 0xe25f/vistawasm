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
  shadows: ShadowOptions,
  surface_options: SurfaceOptions,
  /// Host-supplied trees that replace procedural placement, if any.
  custom_trees: Option<Vec<TreeInstance>>,
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
  /// Seed for springs, meanders and deltas, from the terrain's heights.
  terrain_seed: u64,
  /// The painted water mask, resampled to the terrain, if one is set.
  water_mask: Option<Vec<u8>>,
  /// Where water can be heard.
  sounds: crate::water_sounds::SoundMap,
  /// Whether any sea on the terrain is cold enough to freeze.
  sea_ice_possible: bool,
  /// Terrain materials the ground uses, one bit per material.
  terrain_materials: u32,
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
  /// Low drifting snow, 0 to 1.
  blowing_snow: f32,
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
      lens_drops: crate::lens_drops::LensDrops::new(config.weather.seed_offset),
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
      terrain_seed: 0,
      water_mask: None,
      sounds: Default::default(),
      sea_ice_possible: false,
      terrain_materials: 0,
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
      lens_drops: crate::lens_drops::LensDrops::new(config.weather.seed_offset),
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
      terrain_seed: 0,
      water_mask: None,
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
    self.state = EngineState::LoadingTerrain;
    let map = self.generate_fractal_map(&options, progress).await?;
    self.finish_gpu_work().await?;
    let handle = self.install_terrain(map, progress);
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
    self.state = EngineState::LoadingTerrain;
    let map = decode_geotiff(bytes, &options)?;
    self.finish_gpu_work().await?;
    let handle = self.install_terrain(map, &mut |_, _| {});
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
    self.finish_gpu_work().await?;
    let handle = self.install_terrain(map, &mut |_, _| {});
    self.state = EngineState::Ready;
    Ok(handle)
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
    crate::config::validate_inflows(&water.rivers.inflow)?;

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
      let camera = self.camera.options.position;
      self
        .weather
        .set_celsius(self.celsius_at(camera[0], camera[2]));
      self.weather.advance(dt);
      self.stats.weather = Some(self.weather.dominant());
    } else {
      self.stats.weather = None;
    }

    // Drops land while it rains and keep running off or evaporating after.
    let options = self.weather.options();
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
    self.water_mask = None;
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

    // The previous terrain's carving belongs to a different heightmap.
    self.rivers = RiverNetwork::default();
    self.applied_rivers = None;
    self.glaciers = Vec::new();
    self.terrain_seed = terrain_seed(&map);
    // A mask belongs to the terrain it was painted for.
    self.water_mask = None;
    self.terrain = Some(map);
    self.active_terrain_id = Some(id);

    #[cfg(target_arch = "wasm32")]
    {
      self.mesh_centre_sample = None;
      self.mesh_stream = None;
    }

    self.rebuild_world_with(progress);
    let metadata = self
      .terrain
      .as_ref()
      .map(|terrain| terrain.metadata.clone())
      .unwrap_or_default();

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

        if (mask.width, mask.height) != (width, height) {
          warning = Some(format!(
            "The water mask is {} x {} but the terrain is {width} x {height}, so it was resampled to fit.",
            mask.width, mask.height
          ));
        }

        Some(crate::terrain::water_mask::resample(&mask, width, height))
      }
    };
    self.rebuild_world();
    Ok(warning)
  }

  /// The loudest river, waterfall, lake shore and surf near a position,
  /// for hosts that play their own audio. Reads only the grid cells
  /// around the position.
  pub fn water_sounds(&self, x: f32, y: f32, z: f32) -> vista_types::WaterSounds {
    if !self.water.enabled {
      return Default::default();
    }

    let waves = &self.water.waves;
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
      self.glaciers = shape_glaciers(terrain, &self.biomes);
      self.rivers = match &wanted {
        Some(options) => {
          // Rain, snow and temperature for the hydrology, from the ground
          // before any channel is cut. This is the terrain's own surface
          // bake, done here instead of afterwards: it is patched where the
          // rivers change the ground, so it is not part of the river build.
          let (_, surface) =
            crate::render::terrain_mesh::bake_terrain_shading(terrain, &self.biomes, None);
          // With biomes switched off the ground is not shaded by climate,
          // but rain still falls: rivers follow the default climate.
          let default_climate = (!self.biomes.enabled).then(|| {
            crate::render::terrain_mesh::bake_terrain_shading(
              terrain,
              &BiomeOptions::default(),
              None,
            )
            .1
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
    } else {
      self.rivers = RiverNetwork::default();
      self.glaciers = Vec::new();
    }

    self.applied_rivers = wanted;
    self.sounds = self.terrain.as_ref().map_or(Default::default(), |terrain| {
      crate::water_sounds::SoundMap::build(terrain, &self.rivers)
    });
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
      self
        .gpu
        .upload_falls(&self.rivers.fall_vertices, &self.rivers.fall_indices);
      self.gpu.upload_wet_banks(&self.rivers.wet);
    }

    self.rebake_surface(before_rivers);
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
        let surface = match before_rivers {
          Some((mut samples, relief)) if relief == SurfaceRelief::of(terrain, &self.biomes) => {
            let touched = touched_samples(terrain, &self.rivers);
            crate::terrain::biomes::reclassify_surface(
              terrain,
              &normals,
              mask,
              &self.biomes,
              &mut samples,
              &touched,
            );
            samples
          }
          _ => crate::terrain::biomes::classify_surface(terrain, &normals, mask, &self.biomes),
        };
        self.surface = surface;
        let ocean = BiomeKind::Ocean as u8;
        self.sea_ice_possible = self
          .surface
          .iter()
          .any(|sample| sample.biome == ocean && sample.celsius() < SEA_ICE_CELSIUS);
        self.terrain_materials = terrain_materials(&self.surface);

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
        self.sea_ice_possible = false;
        self.terrain_materials = 0;
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
      Some(terrain) => crate::render::grass::build_grass_instances_by_water(
        terrain,
        surface,
        Some(&self.rivers.wet),
        &self.grass,
        density_scale,
      ),
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
      // Heavy rain (or the sleet and snow it turns to in the cold) means a
      // full deck overhead too, even between storm cells.
      out.weather.overcast = ((state.cloud_coverage - 0.6) / 0.35)
        .clamp(0.0, 1.0)
        .max(state.base_darkness * 0.9)
        .max((state.rain + state.snow).min(1.0));
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
    let clouds_on = clouds.style != vista_types::CloudStyle::Off && clouds.coverage > 0.001;
    let needs = Needs {
      terrain_materials: self.terrain_materials,
      terrain_shadows: self.terrain.is_some() && self.shadows.terrain.enabled,
      trees: self.stats.flora_instances > 0,
      tree_meshes: flora.tree_quality == vista_types::TreeQuality::Mesh,
      tree_shadows: self.shadows.trees.enabled,
      grass: self.stats.grass_instances > 0,
      clouds: clouds_on,
      cloud_noise: clouds_on || mist.style == vista_types::MistStyle::Volumetric,
      cloud_reuse: clouds.temporal && clouds.style == vista_types::CloudStyle::Volumetric,
      water: water.enabled && self.terrain.is_some(),
      sea_ice: self.sea_ice_possible || open_sea < SEA_ICE_CELSIUS,
      sea_near_freezing: false,
      inland_water: !self.rivers.vertices.is_empty(),
      falls: !self.rivers.fall_vertices.is_empty(),
      present: !self.lens_drops.is_empty() || self.stats.render_scale < 0.999,
    };
    let options = self.weather.options();
    // Weather that can reach rain brings lens drops and clouds.
    let rain_possible = options.enabled
      && (options.auto_cycle
        || matches!(
          options.state,
          vista_types::WeatherKind::Rain | vista_types::WeatherKind::Storm
        ));
    let (_, min_scale) = self.quality.render_scale_range();
    let likely = Needs {
      clouds: needs.clouds || (options.enabled && options.effects.clouds),
      cloud_noise: needs.cloud_noise || (options.enabled && options.effects.clouds),
      sea_near_freezing: open_sea < SEA_ICE_CELSIUS + 6.0 || weather.snow_cover > 0.0,
      present: needs.present
        || min_scale < 0.999
        || (rain_possible && options.lens_drops && options.effects.precipitation),
      ..needs
    };
    (needs, likely)
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
        lens_drops: self.lens_drops.packed(),
        blowing_snow: weather.blowing_snow,
      },
      sea_ice: crate::render::gpu::SeaIce {
        possible: self.sea_ice_possible || sea_level_celsius(&self.biomes) < SEA_ICE_CELSIUS,
        open_sea_unit: celsius_to_unit(sea_level_celsius(&self.biomes)).clamp(0.0, 1.0),
      },
      rivers: crate::render::gpu::RiverFrame {
        melt: melt_factor(self.celsius_at(
          self.camera.options.position[0],
          self.camera.options.position[2],
        )),
        freezing: self.rivers.freezing,
        falls: !self.rivers.falls.is_empty(),
        wet_banks: !self.rivers.wet.distance.is_empty(),
      },
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

fn terrain_seed(map: &HeightMap) -> u64 {
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
      let full = crate::terrain::biomes::classify_surface(
        terrain,
        &normals,
        Some(&engine.rivers.mask),
        &engine.biomes,
      );

      assert!(!engine.rivers.carved.is_empty());
      assert!(engine.surface == full);
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
    assert_eq!(created(&engine, &mut slots), [PipelineKind::Grass]);
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
}
