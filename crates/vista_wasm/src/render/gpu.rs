use bytemuck::{Pod, Zeroable};
use vista_types::ErosionOptions;

use crate::errors::{VistaError, VistaResult};
use crate::render::erosion_compute::ErosionCompute;
use crate::render::flora::{FloraInstance, FLORA_BASE_QUAD_CROSS};
use crate::render::grass::GRASS_BASE_TUFT;
use crate::render::terrain_mesh::TerrainMeshData;
use crate::render::water::WaterVertex;

/// Per-frame camera and lighting uniforms uploaded to the terrain shader.
///
/// The layout must stay in sync with the `FrameUniforms` struct declared in
/// `clipmap_render.wgsl`, `flora_instances.wgsl`, `grass_instances.wgsl`,
/// `water.wgsl`, and `atmosphere.wgsl`. New fields are always appended at
/// the end so existing shader struct prefixes stay compatible — see
/// `docs/environment-upgrade-plan.md` §1.2.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct FrameUniforms {
  view_proj: [f32; 16],
  camera_position: [f32; 4],
  sun_direction: [f32; 4],
  sun_colour_intensity: [f32; 4],
  fog: [f32; 4],
  water_params: [f32; 4],
  camera_forward: [f32; 4],
  camera_right: [f32; 4],
  camera_up: [f32; 4],
  camera_params: [f32; 4],
  sky_tint: [f32; 4],
  atmosphere_params: [f32; 4],
  /// x: density, y: base height (metres), z: height falloff (metres),
  /// w: noise strength (0 for the `Flat` style, > 0 for `Volumetric`).
  mist_params: [f32; 4],
  /// r, g, b, and the sea level in metres (used for the rise-above-water
  /// term, so mist can thicken near the water surface independent of
  /// `baseHeightMetres`).
  mist_colour: [f32; 4],
  /// x: coverage, y: drift speed, z: layer height (metres), w: raymarch
  /// step count as a float (0 selects the cheap `Painted` style; a
  /// validated 8..=64 selects the raymarched `Volumetric` style).
  cloud_params: [f32; 4],
  /// r, g, b, reserved.
  cloud_colour: [f32; 4],
  /// x: canopy/blade wind sway strength, y: tree canopy silhouette
  /// variety strength, z: tree rendering style flag (0 = billboard,
  /// 1 = cross-quad), w: grass view-distance fade radius (metres).
  vegetation_params: [f32; 4],
}

// Cheap compile-time regression guard: the struct must stay a whole number
// of 16-byte `vec4` chunks (WGSL's alignment expectation for this layout)
// and must only ever grow, never shrink or reorder, so every field added by
// `docs/environment-upgrade-plan.md` keeps existing shaders' struct
// prefixes valid.
const _: () = assert!(std::mem::size_of::<FrameUniforms>() == 320);
const _: () = assert!(std::mem::size_of::<FrameUniforms>() % 16 == 0);

/// GPU-side terrain mesh resources for the active terrain.
struct TerrainGpu {
  vertex_buffer: wgpu::Buffer,
  index_buffer: wgpu::Buffer,
  index_count: u32,
}

/// GPU-side flora instance buffer for the active terrain.
struct FloraGpu {
  instance_buffer: wgpu::Buffer,
  instance_count: u32,
}

/// GPU-side grass instance buffer for the active terrain.
struct GrassGpu {
  instance_buffer: wgpu::Buffer,
  instance_count: u32,
}

/// GPU-side water plane buffer for the active terrain.
struct WaterGpu {
  vertex_buffer: wgpu::Buffer,
}

/// WebGPU context owned by one VistaWASM engine.
pub struct GpuContext {
  surface: wgpu::Surface<'static>,
  device: wgpu::Device,
  queue: wgpu::Queue,
  config: wgpu::SurfaceConfiguration,
  depth_texture: wgpu::Texture,
  depth_view: wgpu::TextureView,
  atmosphere_pipeline: wgpu::RenderPipeline,
  terrain_pipeline: wgpu::RenderPipeline,
  flora_pipeline: wgpu::RenderPipeline,
  flora_base_vertex_buffer: wgpu::Buffer,
  /// Number of base-quad vertices to draw per flora instance: `6` for the
  /// `Billboard` tree style, `12` for `CrossQuad`/`Mesh` (the base vertex
  /// buffer always holds 12 vertices; this only changes the draw range).
  flora_vertices_per_instance: u32,
  grass_pipeline: wgpu::RenderPipeline,
  grass_base_vertex_buffer: wgpu::Buffer,
  water_pipeline: wgpu::RenderPipeline,
  uniform_buffer: wgpu::Buffer,
  uniform_bind_group: wgpu::BindGroup,
  uniforms: FrameUniforms,
  frame_counter: u64,
  erosion: ErosionCompute,
  terrain: Option<TerrainGpu>,
  flora: Option<FloraGpu>,
  grass: Option<GrassGpu>,
  water: Option<WaterGpu>,
  clear_colour: wgpu::Color,
}

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

impl GpuContext {
  /// Create and configure WebGPU resources for a browser canvas.
  pub async fn new(
    canvas: web_sys::HtmlCanvasElement,
    width: u32,
    height: u32,
    device_pixel_ratio: f32,
  ) -> VistaResult<Self> {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = wgpu::Backends::BROWSER_WEBGPU;
    let instance = wgpu::Instance::new(descriptor);
    let surface = instance
      .create_surface(wgpu::SurfaceTarget::Canvas(canvas))
      .map_err(|_| VistaError::CanvasInvalid)?;
    let adapter = instance
      .request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: Some(&surface),
        apply_limit_buckets: true,
      })
      .await
      .map_err(|_| VistaError::WebGpuUnavailable)?;
    let (device, queue) = adapter
      .request_device(&wgpu::DeviceDescriptor {
        label: Some("VistaWASM device"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        ..Default::default()
      })
      .await
      .map_err(|_| VistaError::WebGpuDeviceRequestFailed)?;
    let pixel_width = scaled_extent(width, device_pixel_ratio);
    let pixel_height = scaled_extent(height, device_pixel_ratio);
    let config = surface
      .get_default_config(&adapter, pixel_width, pixel_height)
      .ok_or(VistaError::CanvasInvalid)?;

    surface.configure(&device, &config);

    let (depth_texture, depth_view) = create_depth_texture(&device, pixel_width, pixel_height);
    let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
      label: Some("VistaWASM frame uniforms"),
      size: std::mem::size_of::<FrameUniforms>() as u64,
      usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
      mapped_at_creation: false,
    });

    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
      label: Some("VistaWASM frame bind group layout"),
      entries: &[wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
          ty: wgpu::BufferBindingType::Uniform,
          has_dynamic_offset: false,
          min_binding_size: None,
        },
        count: None,
      }],
    });

    let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
      label: Some("VistaWASM frame bind group"),
      layout: &bind_group_layout,
      entries: &[wgpu::BindGroupEntry {
        binding: 0,
        resource: uniform_buffer.as_entire_binding(),
      }],
    });

    let terrain_pipeline = create_terrain_pipeline(&device, &bind_group_layout, config.format);
    let atmosphere_pipeline =
      create_atmosphere_pipeline(&device, &bind_group_layout, config.format);
    let flora_pipeline = create_flora_pipeline(&device, &bind_group_layout, config.format);
    let flora_base_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
      label: Some("VistaWASM flora base quad"),
      size: std::mem::size_of_val(&FLORA_BASE_QUAD_CROSS) as u64,
      usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
      mapped_at_creation: false,
    });
    queue.write_buffer(
      &flora_base_vertex_buffer,
      0,
      bytemuck::cast_slice(&FLORA_BASE_QUAD_CROSS),
    );
    let grass_pipeline = create_grass_pipeline(&device, &bind_group_layout, config.format);
    let grass_base_vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
      label: Some("VistaWASM grass base tuft"),
      size: std::mem::size_of_val(&GRASS_BASE_TUFT) as u64,
      usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
      mapped_at_creation: false,
    });
    queue.write_buffer(
      &grass_base_vertex_buffer,
      0,
      bytemuck::cast_slice(&GRASS_BASE_TUFT),
    );
    let water_pipeline = create_water_pipeline(&device, &bind_group_layout, config.format);
    let erosion = ErosionCompute::new(&device);

    Ok(Self {
      surface,
      device,
      queue,
      config,
      depth_texture,
      depth_view,
      atmosphere_pipeline,
      terrain_pipeline,
      flora_pipeline,
      flora_base_vertex_buffer,
      flora_vertices_per_instance: 6,
      grass_pipeline,
      grass_base_vertex_buffer,
      water_pipeline,
      uniform_buffer,
      uniform_bind_group,
      uniforms: FrameUniforms::zeroed(),
      frame_counter: 0,
      erosion,
      terrain: None,
      flora: None,
      grass: None,
      water: None,
      clear_colour: wgpu::Color {
        r: 0.52,
        g: 0.68,
        b: 0.82,
        a: 1.0,
      },
    })
  }

  /// Run budgeted hydraulic and thermal erosion on the GPU and return the
  /// eroded heights. See [`ErosionCompute::run`] for details.
  pub async fn run_erosion(
    &self,
    heights: &[f32],
    width: u32,
    height: u32,
    metres_per_sample: f32,
    options: &ErosionOptions,
  ) -> VistaResult<Vec<f32>> {
    self
      .erosion
      .run(
        &self.device,
        &self.queue,
        heights,
        width,
        height,
        metres_per_sample,
        options,
      )
      .await
  }

  /// Upload a CPU-baked terrain mesh, replacing any previous terrain buffers.
  pub fn upload_terrain(&mut self, mesh: &TerrainMeshData) {
    if mesh.vertices.is_empty() || mesh.indices.is_empty() {
      self.terrain = None;
      return;
    }

    let vertex_bytes: &[u8] = bytemuck::cast_slice(&mesh.vertices);
    let index_bytes: &[u8] = bytemuck::cast_slice(&mesh.indices);

    let vertex_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
      label: Some("VistaWASM terrain vertices"),
      size: vertex_bytes.len() as u64,
      usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
      mapped_at_creation: false,
    });
    self.queue.write_buffer(&vertex_buffer, 0, vertex_bytes);

    let index_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
      label: Some("VistaWASM terrain indices"),
      size: index_bytes.len() as u64,
      usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
      mapped_at_creation: false,
    });
    self.queue.write_buffer(&index_buffer, 0, index_bytes);

    self.terrain = Some(TerrainGpu {
      vertex_buffer,
      index_buffer,
      index_count: mesh.indices.len() as u32,
    });
  }

  /// Upload flora billboard instances, replacing any previous buffer.
  pub fn upload_flora(&mut self, instances: &[FloraInstance]) {
    if instances.is_empty() {
      self.flora = None;
      return;
    }

    let instance_bytes: &[u8] = bytemuck::cast_slice(instances);
    let instance_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
      label: Some("VistaWASM flora instances"),
      size: instance_bytes.len() as u64,
      usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
      mapped_at_creation: false,
    });
    self.queue.write_buffer(&instance_buffer, 0, instance_bytes);

    self.flora = Some(FloraGpu {
      instance_buffer,
      instance_count: instances.len() as u32,
    });
  }

  /// Upload grass tuft instances, replacing any previous buffer.
  pub fn upload_grass(&mut self, instances: &[FloraInstance]) {
    if instances.is_empty() {
      self.grass = None;
      return;
    }

    let instance_bytes: &[u8] = bytemuck::cast_slice(instances);
    let instance_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
      label: Some("VistaWASM grass instances"),
      size: instance_bytes.len() as u64,
      usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
      mapped_at_creation: false,
    });
    self.queue.write_buffer(&instance_buffer, 0, instance_bytes);

    self.grass = Some(GrassGpu {
      instance_buffer,
      instance_count: instances.len() as u32,
    });
  }

  /// Upload the water plane for the active terrain footprint and sea level.
  /// Pass `None` to hide the water plane.
  pub fn upload_water(&mut self, plane: Option<[WaterVertex; 6]>) {
    let Some(vertices) = plane else {
      self.water = None;
      return;
    };

    let vertex_bytes: &[u8] = bytemuck::cast_slice(&vertices);
    let vertex_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
      label: Some("VistaWASM water plane"),
      size: vertex_bytes.len() as u64,
      usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
      mapped_at_creation: false,
    });
    self.queue.write_buffer(&vertex_buffer, 0, vertex_bytes);

    self.water = Some(WaterGpu { vertex_buffer });
  }

  /// Update the per-frame camera and lighting uniforms.
  #[allow(clippy::too_many_arguments)]
  pub fn update_camera(
    &mut self,
    view_proj: [f32; 16],
    camera_position: [f32; 3],
    camera_forward: [f32; 3],
    camera_right: [f32; 3],
    camera_up: [f32; 3],
    field_of_view_degrees: f32,
    aspect_ratio: f32,
    sun_direction: [f32; 3],
    sun_intensity: f32,
    haze_distance_metres: f32,
    exposure: f32,
  ) {
    self.uniforms.view_proj = view_proj;
    self.uniforms.camera_position = [
      camera_position[0],
      camera_position[1],
      camera_position[2],
      0.0,
    ];
    self.uniforms.camera_forward = [camera_forward[0], camera_forward[1], camera_forward[2], 0.0];
    self.uniforms.camera_right = [camera_right[0], camera_right[1], camera_right[2], 0.0];
    self.uniforms.camera_up = [camera_up[0], camera_up[1], camera_up[2], 0.0];
    let tan_half_fov_y = (field_of_view_degrees.to_radians() * 0.5).tan();
    self.uniforms.camera_params = [tan_half_fov_y, aspect_ratio.max(0.001), 0.0, 0.0];
    self.uniforms.sun_direction = [sun_direction[0], sun_direction[1], sun_direction[2], 0.0];
    self.uniforms.sun_colour_intensity = [1.0, 0.98, 0.92, sun_intensity];
    self.uniforms.fog = [haze_distance_metres.max(1.0), exposure.max(0.0), 0.0, 0.0];
  }

  /// Update the water shading parameters used by the next rendered frame.
  pub fn set_water_params(&mut self, wave_scale: f32, reflectivity: f32) {
    self.uniforms.water_params[0] = wave_scale.max(0.0);
    self.uniforms.water_params[1] = reflectivity.clamp(0.0, 1.0);
  }

  /// Update the atmosphere shading parameters used by the next rendered
  /// frame.
  pub fn set_atmosphere_params(
    &mut self,
    rayleigh_strength: f32,
    mie_strength: f32,
    sky_tint: [f32; 3],
  ) {
    self.uniforms.sky_tint = [sky_tint[0], sky_tint[1], sky_tint[2], 0.0];
    self.uniforms.atmosphere_params = [rayleigh_strength.max(0.0), mie_strength.max(0.0), 0.0, 0.0];
  }

  /// Update the mist/ground-fog shading parameters used by the next
  /// rendered frame. `sea_level_metres` is threaded through so the shader
  /// can add extra mist near the water surface without needing its own
  /// copy of `WaterOptions`.
  #[allow(clippy::too_many_arguments)]
  pub fn set_mist_params(
    &mut self,
    density: f32,
    base_height_metres: f32,
    height_falloff_metres: f32,
    noise_strength: f32,
    colour: [f32; 3],
    sea_level_metres: f32,
  ) {
    self.uniforms.mist_params = [
      density.clamp(0.0, 1.0),
      base_height_metres,
      height_falloff_metres.max(0.001),
      noise_strength.max(0.0),
    ];
    self.uniforms.mist_colour = [colour[0], colour[1], colour[2], sea_level_metres];
  }

  /// Update the cloud layer shading parameters used by the next rendered
  /// frame. `raymarch_steps` of `0` selects the cheap painted-noise style;
  /// a non-zero value (already clamped to `8..=64` by `config.rs`) selects
  /// the raymarched volumetric style.
  pub fn set_cloud_params(
    &mut self,
    coverage: f32,
    speed: f32,
    height_metres: f32,
    raymarch_steps: u32,
    colour: [f32; 3],
  ) {
    self.uniforms.cloud_params = [
      coverage.clamp(0.0, 1.0),
      speed,
      height_metres,
      raymarch_steps as f32,
    ];
    self.uniforms.cloud_colour = [colour[0], colour[1], colour[2], 0.0];
  }

  /// Update the shared vegetation shading parameters (tree wind/variety/
  /// style, grass view distance) used by the next rendered frame. Also
  /// updates how many base-quad vertices are drawn per flora instance, so
  /// the `Billboard` tree style keeps drawing exactly the original 6
  /// vertices while `CrossQuad`/`Mesh` draw all 12.
  pub fn set_vegetation_params(
    &mut self,
    wind_strength: f32,
    species_variation: f32,
    tree_style_cross_quad: bool,
    grass_view_distance_metres: f32,
  ) {
    self.uniforms.vegetation_params = [
      wind_strength.clamp(0.0, 1.0),
      species_variation.clamp(0.0, 1.0),
      if tree_style_cross_quad { 1.0 } else { 0.0 },
      grass_view_distance_metres.max(0.0),
    ];
    self.flora_vertices_per_instance = if tree_style_cross_quad { 12 } else { 6 };
  }

  /// Render one frame of the active terrain, or a clear frame if no terrain
  /// has been uploaded yet.
  pub fn render_once(&mut self) -> VistaResult<()> {
    self.frame_counter = self.frame_counter.saturating_add(1);
    self.uniforms.water_params[2] = self.frame_counter as f32 / 60.0;
    self
      .queue
      .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&self.uniforms));

    let surface_texture = match self.surface.get_current_texture() {
      wgpu::CurrentSurfaceTexture::Success(texture) => texture,
      wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
      wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
        return Ok(());
      }
      wgpu::CurrentSurfaceTexture::Outdated => {
        self.surface.configure(&self.device, &self.config);
        return Ok(());
      }
      wgpu::CurrentSurfaceTexture::Lost => return Err(VistaError::WebGpuDeviceLost),
      wgpu::CurrentSurfaceTexture::Validation => return Ok(()),
    };

    let view = surface_texture
      .texture
      .create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = self
      .device
      .create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("VistaWASM frame"),
      });

    {
      let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("VistaWASM terrain pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
          view: &view,
          depth_slice: None,
          resolve_target: None,
          ops: wgpu::Operations {
            load: wgpu::LoadOp::Clear(self.clear_colour),
            store: wgpu::StoreOp::Store,
          },
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
          view: &self.depth_view,
          depth_ops: Some(wgpu::Operations {
            load: wgpu::LoadOp::Clear(1.0),
            store: wgpu::StoreOp::Store,
          }),
          stencil_ops: None,
        }),
        ..Default::default()
      });

      pass.set_pipeline(&self.atmosphere_pipeline);
      pass.set_bind_group(0, &self.uniform_bind_group, &[]);
      pass.draw(0..3, 0..1);

      if let Some(terrain) = &self.terrain {
        pass.set_pipeline(&self.terrain_pipeline);
        pass.set_bind_group(0, &self.uniform_bind_group, &[]);
        pass.set_vertex_buffer(0, terrain.vertex_buffer.slice(..));
        pass.set_index_buffer(terrain.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..terrain.index_count, 0, 0..1);
      }

      if let Some(flora) = &self.flora {
        pass.set_pipeline(&self.flora_pipeline);
        pass.set_bind_group(0, &self.uniform_bind_group, &[]);
        pass.set_vertex_buffer(0, self.flora_base_vertex_buffer.slice(..));
        pass.set_vertex_buffer(1, flora.instance_buffer.slice(..));
        pass.draw(0..self.flora_vertices_per_instance, 0..flora.instance_count);
      }

      if let Some(grass) = &self.grass {
        pass.set_pipeline(&self.grass_pipeline);
        pass.set_bind_group(0, &self.uniform_bind_group, &[]);
        pass.set_vertex_buffer(0, self.grass_base_vertex_buffer.slice(..));
        pass.set_vertex_buffer(1, grass.instance_buffer.slice(..));
        pass.draw(0..18, 0..grass.instance_count);
      }

      if let Some(water) = &self.water {
        pass.set_pipeline(&self.water_pipeline);
        pass.set_bind_group(0, &self.uniform_bind_group, &[]);
        pass.set_vertex_buffer(0, water.vertex_buffer.slice(..));
        pass.draw(0..6, 0..1);
      }
    }

    self.queue.submit(Some(encoder.finish()));
    self.queue.present(surface_texture);
    Ok(())
  }

  /// Resize the WebGPU surface and depth buffer.
  pub fn resize(&mut self, width: u32, height: u32, device_pixel_ratio: f32) -> VistaResult<()> {
    let pixel_width = scaled_extent(width, device_pixel_ratio);
    let pixel_height = scaled_extent(height, device_pixel_ratio);
    self.config.width = pixel_width;
    self.config.height = pixel_height;
    self.surface.configure(&self.device, &self.config);

    let (depth_texture, depth_view) = create_depth_texture(&self.device, pixel_width, pixel_height);
    self.depth_texture = depth_texture;
    self.depth_view = depth_view;
    Ok(())
  }
}

fn create_depth_texture(
  device: &wgpu::Device,
  width: u32,
  height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
  let texture = device.create_texture(&wgpu::TextureDescriptor {
    label: Some("VistaWASM depth buffer"),
    size: wgpu::Extent3d {
      width: width.max(1),
      height: height.max(1),
      depth_or_array_layers: 1,
    },
    mip_level_count: 1,
    sample_count: 1,
    dimension: wgpu::TextureDimension::D2,
    format: DEPTH_FORMAT,
    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
    view_formats: &[],
  });
  let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
  (texture, view)
}

fn create_terrain_pipeline(
  device: &wgpu::Device,
  bind_group_layout: &wgpu::BindGroupLayout,
  surface_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
  let shader_source = include_str!("../shaders/clipmap_render.wgsl");
  let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
    label: Some("VistaWASM terrain shader"),
    source: wgpu::ShaderSource::Wgsl(shader_source.into()),
  });

  let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
    label: Some("VistaWASM terrain pipeline layout"),
    bind_group_layouts: &[Some(bind_group_layout)],
    immediate_size: 0,
  });

  let vertex_attributes = wgpu::vertex_attr_array![
    0 => Float32x3,
    1 => Float32x3,
    2 => Float32x4,
  ];
  let vertex_buffer_layout = wgpu::VertexBufferLayout {
    array_stride: std::mem::size_of::<crate::render::terrain_mesh::TerrainVertex>() as u64,
    step_mode: wgpu::VertexStepMode::Vertex,
    attributes: &vertex_attributes,
  };

  device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
    label: Some("VistaWASM terrain pipeline"),
    layout: Some(&pipeline_layout),
    vertex: wgpu::VertexState {
      module: &shader,
      entry_point: Some("vertex_main"),
      compilation_options: wgpu::PipelineCompilationOptions::default(),
      buffers: &[Some(vertex_buffer_layout)],
    },
    fragment: Some(wgpu::FragmentState {
      module: &shader,
      entry_point: Some("fragment_main"),
      compilation_options: wgpu::PipelineCompilationOptions::default(),
      targets: &[Some(wgpu::ColorTargetState {
        format: surface_format,
        blend: Some(wgpu::BlendState::REPLACE),
        write_mask: wgpu::ColorWrites::ALL,
      })],
    }),
    primitive: wgpu::PrimitiveState {
      topology: wgpu::PrimitiveTopology::TriangleList,
      strip_index_format: None,
      front_face: wgpu::FrontFace::Ccw,
      cull_mode: Some(wgpu::Face::Back),
      unclipped_depth: false,
      polygon_mode: wgpu::PolygonMode::Fill,
      conservative: false,
    },
    depth_stencil: Some(wgpu::DepthStencilState {
      format: DEPTH_FORMAT,
      depth_write_enabled: Some(true),
      depth_compare: Some(wgpu::CompareFunction::Less),
      stencil: wgpu::StencilState::default(),
      bias: wgpu::DepthBiasState::default(),
    }),
    multisample: wgpu::MultisampleState {
      count: 1,
      mask: !0,
      alpha_to_coverage_enabled: false,
    },
    multiview_mask: None,
    cache: None,
  })
}

fn create_flora_pipeline(
  device: &wgpu::Device,
  bind_group_layout: &wgpu::BindGroupLayout,
  surface_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
  let shader_source = include_str!("../shaders/flora_instances.wgsl");
  let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
    label: Some("VistaWASM flora shader"),
    source: wgpu::ShaderSource::Wgsl(shader_source.into()),
  });

  let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
    label: Some("VistaWASM flora pipeline layout"),
    bind_group_layouts: &[Some(bind_group_layout)],
    immediate_size: 0,
  });

  let base_attributes = wgpu::vertex_attr_array![
    0 => Float32x2,
    1 => Float32x2,
  ];
  let base_buffer_layout = wgpu::VertexBufferLayout {
    array_stride: std::mem::size_of::<crate::render::flora::FloraVertex>() as u64,
    step_mode: wgpu::VertexStepMode::Vertex,
    attributes: &base_attributes,
  };

  let instance_attributes = wgpu::vertex_attr_array![
    2 => Float32x3,
    3 => Float32,
    4 => Float32,
  ];
  let instance_buffer_layout = wgpu::VertexBufferLayout {
    array_stride: std::mem::size_of::<crate::render::flora::FloraInstance>() as u64,
    step_mode: wgpu::VertexStepMode::Instance,
    attributes: &instance_attributes,
  };

  device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
    label: Some("VistaWASM flora pipeline"),
    layout: Some(&pipeline_layout),
    vertex: wgpu::VertexState {
      module: &shader,
      entry_point: Some("vertex_main"),
      compilation_options: wgpu::PipelineCompilationOptions::default(),
      buffers: &[Some(base_buffer_layout), Some(instance_buffer_layout)],
    },
    fragment: Some(wgpu::FragmentState {
      module: &shader,
      entry_point: Some("fragment_main"),
      compilation_options: wgpu::PipelineCompilationOptions::default(),
      targets: &[Some(wgpu::ColorTargetState {
        format: surface_format,
        blend: None,
        write_mask: wgpu::ColorWrites::ALL,
      })],
    }),
    primitive: wgpu::PrimitiveState {
      topology: wgpu::PrimitiveTopology::TriangleList,
      strip_index_format: None,
      front_face: wgpu::FrontFace::Ccw,
      cull_mode: None,
      unclipped_depth: false,
      polygon_mode: wgpu::PolygonMode::Fill,
      conservative: false,
    },
    depth_stencil: Some(wgpu::DepthStencilState {
      format: DEPTH_FORMAT,
      depth_write_enabled: Some(true),
      depth_compare: Some(wgpu::CompareFunction::Less),
      stencil: wgpu::StencilState::default(),
      bias: wgpu::DepthBiasState::default(),
    }),
    multisample: wgpu::MultisampleState {
      count: 1,
      mask: !0,
      alpha_to_coverage_enabled: false,
    },
    multiview_mask: None,
    cache: None,
  })
}

fn create_grass_pipeline(
  device: &wgpu::Device,
  bind_group_layout: &wgpu::BindGroupLayout,
  surface_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
  let shader_source = include_str!("../shaders/grass_instances.wgsl");
  let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
    label: Some("VistaWASM grass shader"),
    source: wgpu::ShaderSource::Wgsl(shader_source.into()),
  });

  let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
    label: Some("VistaWASM grass pipeline layout"),
    bind_group_layouts: &[Some(bind_group_layout)],
    immediate_size: 0,
  });

  // Grass reuses flora's vertex/instance shapes exactly (a local
  // offset/uv base quad plus a position/scale/tint instance) — only the
  // shader and pipeline blend/depth state differ.
  let base_attributes = wgpu::vertex_attr_array![
    0 => Float32x2,
    1 => Float32x2,
  ];
  let base_buffer_layout = wgpu::VertexBufferLayout {
    array_stride: std::mem::size_of::<crate::render::flora::FloraVertex>() as u64,
    step_mode: wgpu::VertexStepMode::Vertex,
    attributes: &base_attributes,
  };

  let instance_attributes = wgpu::vertex_attr_array![
    2 => Float32x3,
    3 => Float32,
    4 => Float32,
  ];
  let instance_buffer_layout = wgpu::VertexBufferLayout {
    array_stride: std::mem::size_of::<crate::render::flora::FloraInstance>() as u64,
    step_mode: wgpu::VertexStepMode::Instance,
    attributes: &instance_attributes,
  };

  device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
    label: Some("VistaWASM grass pipeline"),
    layout: Some(&pipeline_layout),
    vertex: wgpu::VertexState {
      module: &shader,
      entry_point: Some("vertex_main"),
      compilation_options: wgpu::PipelineCompilationOptions::default(),
      buffers: &[Some(base_buffer_layout), Some(instance_buffer_layout)],
    },
    fragment: Some(wgpu::FragmentState {
      module: &shader,
      entry_point: Some("fragment_main"),
      compilation_options: wgpu::PipelineCompilationOptions::default(),
      targets: &[Some(wgpu::ColorTargetState {
        format: surface_format,
        // Alpha-blended (like water) so the distance fade-out at
        // `grassViewDistanceMetres` reads as a smooth fade rather than a
        // hard pop; depth writes stay off for the same reason blended
        // water's do, avoiding depth-fight artefacts between overlapping
        // blended tufts.
        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
        write_mask: wgpu::ColorWrites::ALL,
      })],
    }),
    primitive: wgpu::PrimitiveState {
      topology: wgpu::PrimitiveTopology::TriangleList,
      strip_index_format: None,
      front_face: wgpu::FrontFace::Ccw,
      cull_mode: None,
      unclipped_depth: false,
      polygon_mode: wgpu::PolygonMode::Fill,
      conservative: false,
    },
    depth_stencil: Some(wgpu::DepthStencilState {
      format: DEPTH_FORMAT,
      depth_write_enabled: Some(false),
      depth_compare: Some(wgpu::CompareFunction::Less),
      stencil: wgpu::StencilState::default(),
      bias: wgpu::DepthBiasState::default(),
    }),
    multisample: wgpu::MultisampleState {
      count: 1,
      mask: !0,
      alpha_to_coverage_enabled: false,
    },
    multiview_mask: None,
    cache: None,
  })
}

fn create_atmosphere_pipeline(
  device: &wgpu::Device,
  bind_group_layout: &wgpu::BindGroupLayout,
  surface_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
  let shader_source = include_str!("../shaders/atmosphere.wgsl");
  let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
    label: Some("VistaWASM atmosphere shader"),
    source: wgpu::ShaderSource::Wgsl(shader_source.into()),
  });

  let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
    label: Some("VistaWASM atmosphere pipeline layout"),
    bind_group_layouts: &[Some(bind_group_layout)],
    immediate_size: 0,
  });

  device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
    label: Some("VistaWASM atmosphere pipeline"),
    layout: Some(&pipeline_layout),
    vertex: wgpu::VertexState {
      module: &shader,
      entry_point: Some("vertex_main"),
      compilation_options: wgpu::PipelineCompilationOptions::default(),
      buffers: &[],
    },
    fragment: Some(wgpu::FragmentState {
      module: &shader,
      entry_point: Some("fragment_main"),
      compilation_options: wgpu::PipelineCompilationOptions::default(),
      targets: &[Some(wgpu::ColorTargetState {
        format: surface_format,
        blend: Some(wgpu::BlendState::REPLACE),
        write_mask: wgpu::ColorWrites::ALL,
      })],
    }),
    primitive: wgpu::PrimitiveState {
      topology: wgpu::PrimitiveTopology::TriangleList,
      strip_index_format: None,
      front_face: wgpu::FrontFace::Ccw,
      cull_mode: None,
      unclipped_depth: false,
      polygon_mode: wgpu::PolygonMode::Fill,
      conservative: false,
    },
    depth_stencil: Some(wgpu::DepthStencilState {
      format: DEPTH_FORMAT,
      depth_write_enabled: Some(false),
      depth_compare: Some(wgpu::CompareFunction::Less),
      stencil: wgpu::StencilState::default(),
      bias: wgpu::DepthBiasState::default(),
    }),
    multisample: wgpu::MultisampleState {
      count: 1,
      mask: !0,
      alpha_to_coverage_enabled: false,
    },
    multiview_mask: None,
    cache: None,
  })
}

fn create_water_pipeline(
  device: &wgpu::Device,
  bind_group_layout: &wgpu::BindGroupLayout,
  surface_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
  let shader_source = include_str!("../shaders/water.wgsl");
  let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
    label: Some("VistaWASM water shader"),
    source: wgpu::ShaderSource::Wgsl(shader_source.into()),
  });

  let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
    label: Some("VistaWASM water pipeline layout"),
    bind_group_layouts: &[Some(bind_group_layout)],
    immediate_size: 0,
  });

  let attributes = wgpu::vertex_attr_array![
    0 => Float32x3,
  ];
  let buffer_layout = wgpu::VertexBufferLayout {
    array_stride: std::mem::size_of::<crate::render::water::WaterVertex>() as u64,
    step_mode: wgpu::VertexStepMode::Vertex,
    attributes: &attributes,
  };

  device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
    label: Some("VistaWASM water pipeline"),
    layout: Some(&pipeline_layout),
    vertex: wgpu::VertexState {
      module: &shader,
      entry_point: Some("vertex_main"),
      compilation_options: wgpu::PipelineCompilationOptions::default(),
      buffers: &[Some(buffer_layout)],
    },
    fragment: Some(wgpu::FragmentState {
      module: &shader,
      entry_point: Some("fragment_main"),
      compilation_options: wgpu::PipelineCompilationOptions::default(),
      targets: &[Some(wgpu::ColorTargetState {
        format: surface_format,
        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
        write_mask: wgpu::ColorWrites::ALL,
      })],
    }),
    primitive: wgpu::PrimitiveState {
      topology: wgpu::PrimitiveTopology::TriangleList,
      strip_index_format: None,
      front_face: wgpu::FrontFace::Ccw,
      cull_mode: None,
      unclipped_depth: false,
      polygon_mode: wgpu::PolygonMode::Fill,
      conservative: false,
    },
    depth_stencil: Some(wgpu::DepthStencilState {
      format: DEPTH_FORMAT,
      depth_write_enabled: Some(false),
      depth_compare: Some(wgpu::CompareFunction::Less),
      stencil: wgpu::StencilState::default(),
      bias: wgpu::DepthBiasState::default(),
    }),
    multisample: wgpu::MultisampleState {
      count: 1,
      mask: !0,
      alpha_to_coverage_enabled: false,
    },
    multiview_mask: None,
    cache: None,
  })
}

fn scaled_extent(value: u32, device_pixel_ratio: f32) -> u32 {
  ((value as f32 * device_pixel_ratio).round() as u32).max(1)
}
