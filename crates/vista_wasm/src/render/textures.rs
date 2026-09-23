//! Procedural texture baking on the GPU.
//!
//! Every surface texture — eight terrain materials, bark and foliage,
//! water ripples, and 2D/3D noise for clouds and mist — is generated once
//! at engine start-up by compute shaders (`shaders/texture_gen.wgsl`) and
//! mipmapped by `shaders/mipgen.wgsl`. Nothing is downloaded, the output is
//! identical on every run, and the cost is a few milliseconds of GPU time
//! at start-up instead of megabytes of image assets.

/// Edge length of each terrain material layer.
pub const TERRAIN_TEXTURE_SIZE: u32 = 512;
/// Number of terrain material layers.
pub const TERRAIN_LAYERS: u32 = 8;
/// Edge length of each flora layer.
pub const FLORA_TEXTURE_SIZE: u32 = 512;
/// Edge length of the water ripple texture.
pub const WATER_TEXTURE_SIZE: u32 = 512;
/// Edge length of the general 2D noise texture.
pub const NOISE_TEXTURE_SIZE: u32 = 512;
/// Edge length of the 3D cloud noise volume.
pub const CLOUD_TEXTURE_SIZE: u32 = 64;

/// Mip generation mode, matching `mipgen.wgsl`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MipMode {
  /// sRGB-encoded opaque colour.
  Colour = 0,
  /// sRGB-encoded colour with alpha-tested coverage.
  Coverage = 1,
  /// Linear data such as normals and noise.
  Linear = 2,
}

/// All baked world textures.
pub struct WorldTextures {
  /// Terrain albedo (rgb) and height (a).
  pub terrain_albedo: wgpu::TextureView,
  /// Terrain detail normal (rg), occlusion (b), and roughness (a).
  pub terrain_normal: wgpu::TextureView,
  /// Bark and foliage layers.
  pub flora: wgpu::TextureView,
  /// Water ripple normal (rg), foam (b), and height (a).
  pub water: wgpu::TextureView,
  /// General-purpose tiling 2D noise.
  pub noise: wgpu::TextureView,
  /// 3D cloud noise.
  pub cloud: wgpu::TextureView,
  /// Terrain albedo texture, for replacing layers.
  pub terrain_albedo_texture: wgpu::Texture,
  /// Terrain normal texture, for replacing layers.
  pub terrain_normal_texture: wgpu::Texture,
  /// Flora texture, for replacing layers.
  pub flora_texture: wgpu::Texture,
  // Keep the remaining textures alive for as long as their views are used.
  _textures: Vec<wgpu::Texture>,
}

/// Number of mip levels for a square texture.
pub fn mip_count(size: u32) -> u32 {
  32 - size.max(1).leading_zeros()
}

fn create_texture(
  device: &wgpu::Device,
  label: &str,
  size: wgpu::Extent3d,
  dimension: wgpu::TextureDimension,
  mips: u32,
  extra_usage: wgpu::TextureUsages,
) -> wgpu::Texture {
  device.create_texture(&wgpu::TextureDescriptor {
    label: Some(label),
    size,
    mip_level_count: mips,
    sample_count: 1,
    dimension,
    format: wgpu::TextureFormat::Rgba8Unorm,
    usage: wgpu::TextureUsages::TEXTURE_BINDING
      | wgpu::TextureUsages::STORAGE_BINDING
      | extra_usage,
    view_formats: &[],
  })
}

/// Create an rgba8 2D array texture suitable for baking and mipmapping.
pub fn create_array_texture(
  device: &wgpu::Device,
  label: &str,
  width: u32,
  height: u32,
  layers: u32,
  extra_usage: wgpu::TextureUsages,
) -> wgpu::Texture {
  create_texture(
    device,
    label,
    wgpu::Extent3d {
      width,
      height,
      depth_or_array_layers: layers,
    },
    wgpu::TextureDimension::D2,
    mip_count(width.min(height)),
    extra_usage,
  )
}

/// A single-mip view of `texture` as a 2D array.
pub fn array_mip_view(texture: &wgpu::Texture, mip: u32) -> wgpu::TextureView {
  texture.create_view(&wgpu::TextureViewDescriptor {
    label: Some("VistaWASM mip view"),
    dimension: Some(wgpu::TextureViewDimension::D2Array),
    base_mip_level: mip,
    mip_level_count: Some(1),
    ..Default::default()
  })
}

/// Full view of `texture` as a 2D array.
pub fn array_view(texture: &wgpu::Texture) -> wgpu::TextureView {
  texture.create_view(&wgpu::TextureViewDescriptor {
    label: Some("VistaWASM array view"),
    dimension: Some(wgpu::TextureViewDimension::D2Array),
    ..Default::default()
  })
}

/// Builds mip chains with `mipgen.wgsl`.
pub struct MipGenerator {
  pipeline: wgpu::ComputePipeline,
  mode_buffers: [wgpu::Buffer; 3],
}

impl MipGenerator {
  /// Compile the mip pipeline.
  pub fn new(device: &wgpu::Device) -> Self {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
      label: Some("VistaWASM mip shader"),
      source: wgpu::ShaderSource::Wgsl(crate::render::shaders::MIPGEN.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
      label: Some("VistaWASM mip pipeline"),
      layout: None,
      module: &module,
      entry_point: Some("downsample"),
      compilation_options: wgpu::PipelineCompilationOptions::default(),
      cache: None,
    });
    let mode_buffers = [0u32, 1, 2].map(|mode| {
      let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("VistaWASM mip params"),
        size: 16,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: true,
      });
      if let Ok(mut range) = buffer.slice(..).get_mapped_range_mut() {
        range.copy_from_slice(bytemuck::cast_slice(&[mode, 0u32, 0, 0]));
      }
      buffer.unmap();
      buffer
    });

    Self {
      pipeline,
      mode_buffers,
    }
  }

  /// Record every mip level of an rgba8 2D (array) texture.
  pub fn generate(
    &self,
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    texture: &wgpu::Texture,
    mode: MipMode,
  ) {
    let layers = texture.depth_or_array_layers();
    let layout = self.pipeline.get_bind_group_layout(0);

    for mip in 1..texture.mip_level_count() {
      let source = array_mip_view(texture, mip - 1);
      let destination = array_mip_view(texture, mip);
      let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("VistaWASM mip bind group"),
        layout: &layout,
        entries: &[
          wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(&source),
          },
          wgpu::BindGroupEntry {
            binding: 1,
            resource: wgpu::BindingResource::TextureView(&destination),
          },
          wgpu::BindGroupEntry {
            binding: 2,
            resource: self.mode_buffers[mode as usize].as_entire_binding(),
          },
        ],
      });
      let width = (texture.width() >> mip).max(1);
      let height = (texture.height() >> mip).max(1);
      let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some("VistaWASM mip pass"),
        timestamp_writes: None,
      });
      pass.set_pipeline(&self.pipeline);
      pass.set_bind_group(0, &bind_group, &[]);
      pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), layers);
    }
  }
}

fn storage_entry(binding: u32, view: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
  wgpu::BindGroupEntry {
    binding,
    resource: wgpu::BindingResource::TextureView(view),
  }
}

/// Bake every world texture and record the work on `queue`.
pub fn bake_world_textures(
  device: &wgpu::Device,
  queue: &wgpu::Queue,
  mips: &MipGenerator,
) -> WorldTextures {
  let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
    label: Some("VistaWASM texture generation shader"),
    source: wgpu::ShaderSource::Wgsl(crate::render::shaders::TEXTURE_GEN.into()),
  });
  let pipeline = |entry: &str| {
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
      label: Some("VistaWASM texture generation pipeline"),
      layout: None,
      module: &module,
      entry_point: Some(entry),
      compilation_options: wgpu::PipelineCompilationOptions::default(),
      cache: None,
    })
  };

  let terrain_albedo = create_array_texture(
    device,
    "VistaWASM terrain albedo",
    TERRAIN_TEXTURE_SIZE,
    TERRAIN_TEXTURE_SIZE,
    TERRAIN_LAYERS,
    wgpu::TextureUsages::empty(),
  );
  let terrain_normal = create_array_texture(
    device,
    "VistaWASM terrain normals",
    TERRAIN_TEXTURE_SIZE,
    TERRAIN_TEXTURE_SIZE,
    TERRAIN_LAYERS,
    wgpu::TextureUsages::empty(),
  );
  let flora = create_array_texture(
    device,
    "VistaWASM flora textures",
    FLORA_TEXTURE_SIZE,
    FLORA_TEXTURE_SIZE,
    crate::render::tree_models::layers::COUNT,
    wgpu::TextureUsages::empty(),
  );
  let water = create_array_texture(
    device,
    "VistaWASM water texture",
    WATER_TEXTURE_SIZE,
    WATER_TEXTURE_SIZE,
    1,
    wgpu::TextureUsages::empty(),
  );
  let noise = create_array_texture(
    device,
    "VistaWASM noise texture",
    NOISE_TEXTURE_SIZE,
    NOISE_TEXTURE_SIZE,
    1,
    wgpu::TextureUsages::empty(),
  );
  let cloud = create_texture(
    device,
    "VistaWASM cloud noise",
    wgpu::Extent3d {
      width: CLOUD_TEXTURE_SIZE,
      height: CLOUD_TEXTURE_SIZE,
      depth_or_array_layers: CLOUD_TEXTURE_SIZE,
    },
    wgpu::TextureDimension::D3,
    1,
    wgpu::TextureUsages::empty(),
  );

  let terrain_albedo_mip0 = array_mip_view(&terrain_albedo, 0);
  let terrain_normal_mip0 = array_mip_view(&terrain_normal, 0);
  let flora_mip0 = array_mip_view(&flora, 0);
  let single_mip0 = |texture: &wgpu::Texture| {
    texture.create_view(&wgpu::TextureViewDescriptor {
      label: Some("VistaWASM mip 0"),
      dimension: Some(wgpu::TextureViewDimension::D2),
      base_mip_level: 0,
      mip_level_count: Some(1),
      ..Default::default()
    })
  };
  let water_mip0 = single_mip0(&water);
  let noise_mip0 = single_mip0(&noise);
  let cloud_view = cloud.create_view(&wgpu::TextureViewDescriptor::default());

  let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
    label: Some("VistaWASM texture bake"),
  });

  let jobs: [(&str, Vec<wgpu::BindGroupEntry<'_>>, [u32; 3]); 5] = [
    (
      "gen_terrain",
      vec![
        storage_entry(0, &terrain_albedo_mip0),
        storage_entry(1, &terrain_normal_mip0),
      ],
      [
        TERRAIN_TEXTURE_SIZE / 8,
        TERRAIN_TEXTURE_SIZE / 8,
        TERRAIN_LAYERS,
      ],
    ),
    (
      "gen_flora",
      vec![storage_entry(2, &flora_mip0)],
      [
        FLORA_TEXTURE_SIZE / 8,
        FLORA_TEXTURE_SIZE / 8,
        crate::render::tree_models::layers::COUNT,
      ],
    ),
    (
      "gen_water",
      vec![storage_entry(3, &water_mip0)],
      [WATER_TEXTURE_SIZE / 8, WATER_TEXTURE_SIZE / 8, 1],
    ),
    (
      "gen_noise",
      vec![storage_entry(4, &noise_mip0)],
      [NOISE_TEXTURE_SIZE / 8, NOISE_TEXTURE_SIZE / 8, 1],
    ),
    (
      "gen_cloud",
      vec![storage_entry(5, &cloud_view)],
      [
        CLOUD_TEXTURE_SIZE / 4,
        CLOUD_TEXTURE_SIZE / 4,
        CLOUD_TEXTURE_SIZE / 4,
      ],
    ),
  ];

  for (entry, entries, groups) in jobs.iter() {
    let pipeline = pipeline(entry);
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
      label: Some("VistaWASM texture generation bind group"),
      layout: &pipeline.get_bind_group_layout(0),
      entries,
    });
    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
      label: Some("VistaWASM texture generation pass"),
      timestamp_writes: None,
    });
    pass.set_pipeline(&pipeline);
    pass.set_bind_group(0, &bind_group, &[]);
    pass.dispatch_workgroups(groups[0], groups[1], groups[2]);
  }

  drop(jobs);
  mips.generate(device, &mut encoder, &terrain_albedo, MipMode::Colour);
  mips.generate(device, &mut encoder, &terrain_normal, MipMode::Linear);
  mips.generate(device, &mut encoder, &flora, MipMode::Coverage);
  mips.generate(device, &mut encoder, &water, MipMode::Linear);
  mips.generate(device, &mut encoder, &noise, MipMode::Linear);
  queue.submit(Some(encoder.finish()));

  let view_2d = |texture: &wgpu::Texture| {
    texture.create_view(&wgpu::TextureViewDescriptor {
      label: Some("VistaWASM 2D view"),
      dimension: Some(wgpu::TextureViewDimension::D2),
      ..Default::default()
    })
  };

  WorldTextures {
    terrain_albedo: array_view(&terrain_albedo),
    terrain_normal: array_view(&terrain_normal),
    flora: array_view(&flora),
    water: view_2d(&water),
    noise: view_2d(&noise),
    cloud: cloud_view,
    terrain_albedo_texture: terrain_albedo,
    terrain_normal_texture: terrain_normal,
    flora_texture: flora,
    _textures: vec![water, noise, cloud],
  }
}
