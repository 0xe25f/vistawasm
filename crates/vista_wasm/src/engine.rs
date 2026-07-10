use vista_types::{
  AtmosphereOptions, CameraOptions, CloudsOptions, DebugView, DemLoadOptions, EngineState,
  FloraOptions, FractalTerrainOptions, GrassOptions, MistOptions, RawHeightmapOptions,
  RenderQualityOptions, RenderStats, SunOptions, TerrainHandle, WaterOptions,
};

use crate::camera::CameraProjector;
use crate::config::VistaEngineConfig;
use crate::dem::{decode_geotiff, decode_raw_heightmap};
use crate::errors::{VistaError, VistaResult};
#[cfg(target_arch = "wasm32")]
use crate::maths::{cross, normalise, sub};
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
  debug_view: DebugView,
  stats: RenderStats,
  render_width: u32,
  render_height: u32,
  device_pixel_ratio: f32,
  /// Cached per-sample normals for the active terrain, computed once when
  /// the terrain is installed and reused by every LOD mesh rebuild so the
  /// camera can recentre the mesh without repeating a full-heightmap pass.
  #[cfg(target_arch = "wasm32")]
  terrain_normals: Vec<vista_types::Vec3>,
  #[cfg(target_arch = "wasm32")]
  terrain_materials: Vec<crate::terrain::materials::MaterialWeights>,
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
      debug_view: DebugView::None,
      stats: RenderStats::default(),
      render_width: config.render.width,
      render_height: config.render.height,
      device_pixel_ratio: config.render.device_pixel_ratio.unwrap_or(1.0),
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
      debug_view: DebugView::None,
      stats: RenderStats::default(),
      render_width: config.render.width,
      render_height: config.render.height,
      device_pixel_ratio: config.render.device_pixel_ratio.unwrap_or(1.0),
      terrain_normals: Vec::new(),
      terrain_materials: Vec::new(),
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
  pub fn set_water(&mut self, water: WaterOptions) -> VistaResult<()> {
    self.ensure_live()?;
    self.water = water;
    self.refresh_water();
    Ok(())
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
  /// new values, mirroring `set_sun`/`set_atmosphere`.
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
      self.gpu.update_camera(
        view_proj,
        self.camera.options.position,
        camera_forward,
        camera_right,
        camera_up,
        self.camera.options.field_of_view_degrees,
        aspect_ratio,
        sun_direction,
        self.sun.intensity,
        self.atmosphere.haze_distance_metres,
        self.atmosphere.exposure,
      );
      self.gpu.set_atmosphere_params(
        self.atmosphere.rayleigh_strength,
        self.atmosphere.mie_strength,
        self.atmosphere.sky_tint,
      );

      let (mist_density, mist_noise_strength) = match self.mist.style {
        vista_types::MistStyle::Off => (0.0, 0.0),
        vista_types::MistStyle::Flat => (self.mist.density, 0.0),
        vista_types::MistStyle::Volumetric => (self.mist.density, 0.6),
      };
      // Only feed a real sea level into the shader's rise-above-water term
      // when it is actually requested; otherwise push a sentinel height
      // far from any terrain so that term always evaluates to zero.
      let effective_sea_level_metres = if self.mist.rise_above_water && self.water.enabled {
        self.water.sea_level_metres
      } else {
        self.mist.base_height_metres - 1_000_000.0
      };
      self.gpu.set_mist_params(
        mist_density,
        self.mist.base_height_metres,
        self.mist.height_falloff_metres,
        mist_noise_strength,
        self.mist.colour,
        effective_sea_level_metres,
      );

      let (cloud_coverage, cloud_raymarch_steps) = match self.clouds.style {
        vista_types::CloudStyle::Off => (0.0, 0),
        vista_types::CloudStyle::Painted => (self.clouds.coverage, 0),
        vista_types::CloudStyle::Volumetric => {
          (self.clouds.coverage, self.clouds.raymarch_steps.unwrap_or(24))
        }
      };
      self.gpu.set_cloud_params(
        cloud_coverage,
        self.clouds.speed,
        self.clouds.height_metres,
        cloud_raymarch_steps,
        self.clouds.colour,
      );

      let tree_style_cross_quad = matches!(
        self.flora.tree_quality,
        vista_types::TreeQuality::CrossQuad | vista_types::TreeQuality::Mesh
      );
      self.gpu.set_vegetation_params(
        self.flora.wind_strength,
        self.flora.species_variation,
        tree_style_cross_quad,
        self.grass.view_distance_metres,
      );

      self.gpu.render_once()?;
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

    #[cfg(target_arch = "wasm32")]
    {
      self.terrain_normals = Vec::new();
      self.terrain_materials = Vec::new();
      self.mesh_centre_sample = None;
    }

    Ok(())
  }

  fn install_terrain(&mut self, map: HeightMap) -> TerrainHandle {
    let id = self.next_terrain_id;
    self.next_terrain_id = self.next_terrain_id.saturating_add(1);
    let metadata = map.metadata.clone();

    #[cfg(target_arch = "wasm32")]
    {
      let (normals, materials) = crate::render::terrain_mesh::bake_terrain_shading(&map);
      // Centre the initial LOD mesh on the terrain itself; render_once()
      // will recentre it on the camera once rendering starts.
      let centre_sample_x = (map.metadata.width as f32 - 1.0) * 0.5;
      let centre_sample_z = (map.metadata.height as f32 - 1.0) * 0.5;
      let mesh = crate::render::terrain_mesh::build_terrain_mesh_centred(
        &map,
        &normals,
        &materials,
        centre_sample_x,
        centre_sample_z,
        crate::render::terrain_mesh::CENTRED_MESH_SAMPLES_PER_SIDE,
      );
      self.gpu.upload_terrain(&mesh);
      self.terrain_normals = normals;
      self.terrain_materials = materials;
      self.mesh_centre_sample = Some((centre_sample_x, centre_sample_z));
    }

    self.terrain = Some(map);
    self.active_terrain_id = Some(id);
    self.refresh_flora();
    self.refresh_grass();
    self.refresh_water();

    TerrainHandle { id, metadata }
  }

  /// Regenerate flora instances for the active terrain and current flora and
  /// quality settings, uploading them to the GPU on browser builds.
  fn refresh_flora(&mut self) {
    let density_scale = self.quality.flora_density_scale.unwrap_or(1.0);
    let instances = match &self.terrain {
      Some(terrain) => {
        crate::render::flora::build_flora_instances(terrain, &self.flora, density_scale)
      }
      None => Vec::new(),
    };
    self.stats.flora_instances = instances.len() as u32;

    #[cfg(target_arch = "wasm32")]
    self.gpu.upload_flora(&instances);
  }

  /// Regenerate grass tuft instances for the active terrain and current
  /// grass and quality settings, uploading them to the GPU on browser
  /// builds. Mirrors `refresh_flora`; reuses the terrain's already-baked
  /// material weights for placement when available (browser builds) so
  /// grass naturally avoids rock/snow/mud/underwater terrain without a
  /// second slope/height pass — see `render/grass.rs`.
  fn refresh_grass(&mut self) {
    let density_scale = self.quality.flora_density_scale.unwrap_or(1.0);

    #[cfg(target_arch = "wasm32")]
    let materials: Option<&[crate::terrain::materials::MaterialWeights]> =
      if self.terrain_materials.is_empty() {
        None
      } else {
        Some(&self.terrain_materials)
      };
    #[cfg(not(target_arch = "wasm32"))]
    let materials: Option<&[crate::terrain::materials::MaterialWeights]> = None;

    let instances = match &self.terrain {
      Some(terrain) => {
        crate::render::grass::build_grass_instances(terrain, materials, &self.grass, density_scale)
      }
      None => Vec::new(),
    };
    self.stats.grass_instances = instances.len() as u32;

    #[cfg(target_arch = "wasm32")]
    self.gpu.upload_grass(&instances);
  }

  /// Rebuild the water plane geometry for the active terrain footprint and
  /// current sea level, uploading it to the GPU on browser builds.
  fn refresh_water(&mut self) {
    #[cfg(target_arch = "wasm32")]
    {
      self
        .gpu
        .set_water_params(self.water.wave_scale, self.water.reflectivity);

      let plane = if self.water.enabled {
        self.terrain.as_ref().map(|terrain| {
          let half_width =
            (terrain.metadata.width as f32 - 1.0) * terrain.metadata.metres_per_sample * 0.5;
          let half_height =
            (terrain.metadata.height as f32 - 1.0) * terrain.metadata.metres_per_sample * 0.5;
          crate::render::water::build_water_plane(
            half_width,
            half_height,
            self.water.sea_level_metres,
          )
        })
      } else {
        None
      };

      self.gpu.upload_water(plane);
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
      &self.terrain_materials,
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
}
