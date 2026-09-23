use vista_types::{
  AtmosphereOptions, BiomeKind, BiomeOptions, CameraOptions, CloudsOptions, DebugView,
  DemLoadOptions, EngineState, FloraOptions, FractalTerrainOptions, GrassOptions, MistOptions,
  RawHeightmapOptions, RenderQualityOptions, RenderStats, RiverOptions, SunOptions, TerrainHandle,
  WaterOptions,
};

use crate::camera::CameraProjector;
use crate::config::VistaEngineConfig;
use crate::dem::{decode_geotiff, decode_raw_heightmap};
use crate::errors::{VistaError, VistaResult};
#[cfg(target_arch = "wasm32")]
use crate::maths::{cross, normalise, sub};
use crate::render::water::{build_river_network, restore_carving, RiverNetwork};
use crate::terrain::biomes::SurfaceSample;
#[cfg(not(target_arch = "wasm32"))]
use crate::terrain::clipmap::build_clipmap_levels;
#[cfg(not(target_arch = "wasm32"))]
use crate::terrain::generate_fractal_heightmap;
use crate::terrain::HeightMap;

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
  /// Cached per-sample normals for the active terrain, computed once when
  /// the terrain is installed and reused by every LOD mesh rebuild so the
  /// camera can recentre the mesh without repeating a full-heightmap pass.
  #[cfg(target_arch = "wasm32")]
  terrain_normals: Vec<vista_types::Vec3>,
  /// Heightmap-sample coordinates the LOD mesh was last centred on. `None`
  /// until a terrain is installed.
  #[cfg(target_arch = "wasm32")]
  mesh_centre_sample: Option<(f32, f32)>,
  #[cfg(target_arch = "wasm32")]
  gpu: crate::render::gpu::GpuContext,
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
      debug_view: DebugView::None,
      stats: RenderStats::default(),
      render_width: config.render.width,
      render_height: config.render.height,
      device_pixel_ratio: config.render.device_pixel_ratio.unwrap_or(1.0),
      surface: Vec::new(),
      rivers: RiverNetwork::default(),
      applied_rivers: None,
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
    )
    .await?;

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
      debug_view: DebugView::None,
      stats: RenderStats::default(),
      render_width: config.render.width,
      render_height: config.render.height,
      device_pixel_ratio: config.render.device_pixel_ratio.unwrap_or(1.0),
      surface: Vec::new(),
      rivers: RiverNetwork::default(),
      applied_rivers: None,
      terrain_normals: Vec::new(),
      mesh_centre_sample: None,
      gpu,
    })
  }

  /// Generate deterministic fractal terrain.
  ///
  /// Browser builds run hydraulic and thermal erosion as GPU compute
  /// passes for performance; native builds (and any error recovering from
  /// a GPU erosion pass) use the CPU reference erosion in
  /// `terrain::erosion`.
  pub async fn generate_fractal(
    &mut self,
    options: FractalTerrainOptions,
  ) -> VistaResult<TerrainHandle> {
    self.ensure_live()?;
    self.state = EngineState::LoadingTerrain;
    let map = self.generate_fractal_map(&options).await?;
    let handle = self.install_terrain(map);
    self.state = EngineState::Ready;
    Ok(handle)
  }

  #[cfg(target_arch = "wasm32")]
  async fn generate_fractal_map(
    &mut self,
    options: &FractalTerrainOptions,
  ) -> VistaResult<HeightMap> {
    let mut map = crate::terrain::generate_fractal_heightmap_base(options)?;

    if let Some(erosion) = &options.erosion {
      match self
        .gpu
        .run_erosion(
          &map.heights,
          map.metadata.width,
          map.metadata.height,
          map.metadata.metres_per_sample,
          erosion,
        )
        .await
      {
        Ok(eroded) => {
          map.heights = eroded;
          crate::terrain::heightmap::update_stats(&map.heights, &map.no_data, &mut map.metadata);
        }
        Err(_) => {
          crate::terrain::erosion::apply_erosion(&mut map, erosion)?;
        }
      }
    }

    Ok(map)
  }

  #[cfg(not(target_arch = "wasm32"))]
  async fn generate_fractal_map(
    &mut self,
    options: &FractalTerrainOptions,
  ) -> VistaResult<HeightMap> {
    generate_fractal_heightmap(options)
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
  pub fn set_biomes(&mut self, biomes: BiomeOptions) -> VistaResult<()> {
    self.ensure_live()?;

    if self.biomes == biomes {
      return Ok(());
    }

    self.biomes = biomes;
    self.rebake_surface();
    Ok(())
  }

  /// Return the biome at a world-space position, or `None` when there is
  /// no terrain or the position is outside it.
  pub fn biome_at(&self, world_x: f32, world_z: f32) -> Option<BiomeKind> {
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
    self
      .surface
      .get((z * terrain.metadata.width + x) as usize)
      .map(|sample| sample.biome_kind())
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

  /// Render one frame.
  pub fn render_once(&mut self) -> VistaResult<RenderStats> {
    self.ensure_live()?;
    self.stats.frame_index = self.stats.frame_index.saturating_add(1);

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
      self.recentre_terrain_mesh_if_needed();
      let params = self.frame_params();
      self.gpu.render_once(&params)?;
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

    #[cfg(target_arch = "wasm32")]
    {
      self.terrain_normals = Vec::new();
      self.mesh_centre_sample = None;
    }

    Ok(())
  }

  fn install_terrain(&mut self, map: HeightMap) -> TerrainHandle {
    let id = self.next_terrain_id;
    self.next_terrain_id = self.next_terrain_id.saturating_add(1);

    // The previous terrain's carving belongs to a different heightmap.
    self.rivers = RiverNetwork::default();
    self.applied_rivers = None;
    self.terrain = Some(map);
    self.active_terrain_id = Some(id);

    #[cfg(target_arch = "wasm32")]
    {
      self.mesh_centre_sample = None;
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

  /// Re-extract rivers (restoring any previous carving first), then re-bake
  /// surface shading and every terrain-dependent layer.
  fn rebuild_world(&mut self) {
    let wanted = self.wanted_rivers(&self.water);

    if let Some(terrain) = self.terrain.as_mut() {
      restore_carving(terrain, &self.rivers.carved);
      self.rivers = match &wanted {
        Some(options) => build_river_network(terrain, options),
        None => RiverNetwork {
          mask: vec![false; terrain.heights.len()],
          ..RiverNetwork::default()
        },
      };
    } else {
      self.rivers = RiverNetwork::default();
    }

    self.applied_rivers = wanted;

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
          self.terrain_normals = normals;
          self.mesh_centre_sample = Some((centre_sample_x, centre_sample_z));
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

    let (mist_density, mist_noise_strength) = match self.mist.style {
      vista_types::MistStyle::Off => (0.0, 0.0),
      vista_types::MistStyle::Flat => (self.mist.density, 0.0),
      vista_types::MistStyle::Volumetric => (self.mist.density, 1.0),
    };
    // Only feed a real sea level into the shader's rise-above-water term
    // when it is actually requested; otherwise push a sentinel height far
    // from any terrain so that term always evaluates to zero.
    let mist_water_level_metres = if self.mist.rise_above_water && self.water.enabled {
      self.water.sea_level_metres
    } else {
      self.mist.base_height_metres - 1_000_000.0
    };
    let (cloud_coverage, cloud_raymarch_steps) = match self.clouds.style {
      vista_types::CloudStyle::Off => (0.0, 0),
      vista_types::CloudStyle::Painted => (self.clouds.coverage, 0),
      vista_types::CloudStyle::Volumetric => {
        // Clamped again here: this bounds a shader loop.
        (
          self.clouds.coverage,
          self.clouds.raymarch_steps.unwrap_or(32).clamp(8, 64),
        )
      }
    };
    let tree_style = match self.flora.tree_quality {
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
      atmosphere: self.atmosphere.clone(),
      water: self.water.clone(),
      mist_density,
      mist_noise_strength,
      mist_water_level_metres,
      mist: self.mist.clone(),
      cloud_coverage,
      cloud_raymarch_steps,
      clouds: self.clouds.clone(),
      tree_style,
      flora: self.flora.clone(),
      grass_view_distance_metres: self.grass.view_distance_metres,
      debug_view: debug_view_index(self.debug_view),
    }
  }

  /// Rebuild and upload the camera-centred LOD terrain mesh if the camera
  /// has drifted far enough from where the mesh is currently centred.
  ///
  /// Also refreshes `terrain_triangles` and `clipmap_levels` stats to
  /// reflect the real uploaded mesh rather than a theoretical estimate.
  #[cfg(target_arch = "wasm32")]
  fn recentre_terrain_mesh_if_needed(&mut self) {
    const RECENTRE_THRESHOLD_SAMPLES: f32 = 12.0;

    let samples_per_side = crate::render::terrain_mesh::CENTRED_MESH_SAMPLES_PER_SIDE;
    self.stats.clipmap_levels = crate::render::terrain_mesh::band_count(samples_per_side / 2);
    self.stats.terrain_triangles = (samples_per_side - 1) * (samples_per_side - 1) * 2;

    let Some(terrain) = self.terrain.as_ref() else {
      return;
    };

    let (sample_x, sample_z) = crate::render::terrain_mesh::world_to_sample_coordinates(
      terrain,
      self.camera.options.position[0],
      self.camera.options.position[2],
    );

    let needs_rebuild = match self.mesh_centre_sample {
      Some((centre_x, centre_z)) => {
        (sample_x - centre_x).abs() > RECENTRE_THRESHOLD_SAMPLES
          || (sample_z - centre_z).abs() > RECENTRE_THRESHOLD_SAMPLES
      }
      None => true,
    };

    if !needs_rebuild {
      return;
    }

    let mesh = crate::render::terrain_mesh::build_terrain_mesh_centred(
      terrain,
      &self.terrain_normals,
      &self.surface,
      sample_x,
      sample_z,
      samples_per_side,
    );
    self.gpu.upload_terrain(&mesh);
    self.mesh_centre_sample = Some((sample_x, sample_z));
  }

  fn ensure_live(&self) -> VistaResult<()> {
    if self.state == EngineState::Disposed {
      return Err(VistaError::EngineDisposed);
    }

    Ok(())
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
