//! The weather's GPU resources: the per-terrain ground layers, the
//! regional weather texture, and the surface weather map with the compute
//! pass that steps it.
//!
//! WebGPU allows 16 sampled textures per shader stage by default, and the
//! composite and cloud passes already used all 16. The three per-terrain
//! textures of the same size (surface, banks and cover) are therefore one
//! three-layer array, which frees the two bindings the regional weather
//! texture and the surface weather map take.

use bytemuck::{Pod, Zeroable};

use super::{
  buffer_with_data, create_texture_2d, default_view, uniform_entry, view_entry, write_layer,
};

/// Layer of the ground array holding the surface texture.
pub const SURFACE_LAYER: u32 = 0;
/// Layer holding distance to water, snow cover, greening and talus.
pub const BANKS_LAYER: u32 = 1;
/// Layer holding tree cover.
pub const COVER_LAYER: u32 = 2;

/// The per-terrain surface, banks and cover textures, as one array.
pub struct GroundLayers {
  texture: wgpu::Texture,
  size: (u32, u32),
  /// All three layers, for the render shaders.
  pub array: wgpu::TextureView,
  /// Each layer alone, for the generators.
  pub surface: wgpu::TextureView,
  /// See [`BANKS_LAYER`].
  pub banks: wgpu::TextureView,
  /// See [`COVER_LAYER`].
  pub cover: wgpu::TextureView,
}

fn layer_view(texture: &wgpu::Texture, layer: u32) -> wgpu::TextureView {
  texture.create_view(&wgpu::TextureViewDescriptor {
    label: Some("VistaWASM ground layer"),
    dimension: Some(wgpu::TextureViewDimension::D2),
    base_array_layer: layer,
    array_layer_count: Some(1),
    ..Default::default()
  })
}

impl GroundLayers {
  /// Layers of `width` x `height` rgba8 texels. A layer given no data (an
  /// empty slice) is filled with its `fill` texel.
  pub fn new(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    (width, height): (u32, u32),
    layers: [(&[u8], [u8; 4]); 3],
  ) -> Self {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
      label: Some("VistaWASM ground layers"),
      size: wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 3,
      },
      mip_level_count: 1,
      sample_count: 1,
      dimension: wgpu::TextureDimension::D2,
      format: wgpu::TextureFormat::Rgba8Unorm,
      usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
      view_formats: &[],
    });
    let texels = (width * height * 4) as usize;

    for (layer, (data, fill)) in layers.iter().enumerate() {
      let filled;
      let data = if data.len() == texels {
        *data
      } else {
        filled = fill.repeat(texels / 4);
        &filled
      };
      write_layer(
        queue,
        &texture,
        layer as u32,
        data,
        width * 4,
        width,
        height,
      );
    }

    Self {
      array: texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("VistaWASM ground layers"),
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
      }),
      surface: layer_view(&texture, SURFACE_LAYER),
      banks: layer_view(&texture, BANKS_LAYER),
      cover: layer_view(&texture, COVER_LAYER),
      texture,
      size: (width, height),
    }
  }

  /// Size in texels.
  pub fn size(&self) -> (u32, u32) {
    self.size
  }

  /// Replace one layer, which must be the layers' size.
  pub fn write(&self, queue: &wgpu::Queue, layer: u32, data: &[u8]) {
    let (width, height) = self.size;
    write_layer(queue, &self.texture, layer, data, width * 4, width, height);
  }
}

/// The regional weather map's texture: 128 x 128 rgba16float, coverage,
/// precipitation, storminess and humidity.
pub fn create_regional_texture(device: &wgpu::Device) -> (wgpu::Texture, wgpu::TextureView) {
  let size = crate::weather::regional::GRID as u32;
  let texture = create_texture_2d(
    device,
    "VistaWASM regional weather",
    size,
    size,
    wgpu::TextureFormat::Rgba16Float,
    wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
  );
  let view = default_view(&texture);
  (texture, view)
}

/// Write rows of the regional weather map.
pub fn write_regional(
  queue: &wgpu::Queue,
  texture: &wgpu::Texture,
  upload: &crate::weather::regional::GridUpload<'_>,
) {
  let width = crate::weather::regional::GRID as u32;
  queue.write_texture(
    wgpu::TexelCopyTextureInfo {
      texture,
      mip_level: 0,
      origin: wgpu::Origin3d {
        x: 0,
        y: upload.first_row,
        z: 0,
      },
      aspect: wgpu::TextureAspect::All,
    },
    bytemuck::cast_slice(upload.data),
    wgpu::TexelCopyBufferLayout {
      offset: 0,
      bytes_per_row: Some(width * 8),
      rows_per_image: Some(upload.rows),
    },
    wgpu::Extent3d {
      width,
      height: upload.rows,
      depth_or_array_layers: 1,
    },
  );
}

/// One step of the surface weather, as the engine asks for it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SurfaceWeatherStep {
  /// Weather seconds to advance.
  pub dt: f32,
  /// Settle every texel to this state first, instead of stepping.
  pub settle: Option<crate::weather::surface::SurfaceCell>,
  /// Sunlight reaching the ground, 0 to 1.
  pub sun: f32,
  /// Mean wind in m/s.
  pub wind: f32,
  /// The weather's temperature offset in °C.
  pub celsius_offset: f32,
  /// Precipitation everywhere, when there is no regional map.
  pub precipitation: f32,
  /// The preset's mean precipitation: settling wets each place in
  /// proportion to its own share of it.
  pub mean_precipitation: f32,
  /// The regional map's size in metres, when it is read.
  pub regional: Option<f32>,
  /// Crown area per square metre per unit of cover share.
  pub canopy: f32,
}

/// Mirrors `Params` in `surface_weather.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct SurfaceWeatherParams {
  step: [f32; 4],
  rain: [f32; 4],
  settle: [f32; 4],
  grid: [f32; 4],
  extent: [f32; 4],
}

/// The surface weather map: wetness, puddle water, snow depth and each
/// hollow's fill, at the terrain shadow's resolution. The pass reads
/// `current` and writes `next`, which is then copied back, so the render
/// shaders always read `current`.
pub struct SurfaceWeatherMap {
  current: wgpu::Texture,
  /// What the render shaders sample.
  pub view: wgpu::TextureView,
  next: wgpu::Texture,
  next_view: wgpu::TextureView,
  size: (u32, u32),
  /// Runs so far, which vary the rounding.
  runs: u32,
  /// A step waiting for the next frame.
  pub pending: Option<SurfaceWeatherStep>,
  /// The pass's parameters, rewritten each step.
  params: wgpu::Buffer,
  /// The pass's bind group, and the height, ground and regional views it
  /// binds: it is made again only when one of them changes.
  bound: Option<(wgpu::BindGroup, [wgpu::TextureView; 3])>,
}

impl SurfaceWeatherMap {
  /// A dry map of `width` x `height` texels.
  pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, width: u32, height: u32) -> Self {
    let current = create_texture_2d(
      device,
      "VistaWASM surface weather",
      width,
      height,
      wgpu::TextureFormat::Rgba8Unorm,
      wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
    );
    let next = create_texture_2d(
      device,
      "VistaWASM surface weather step",
      width,
      height,
      wgpu::TextureFormat::Rgba8Unorm,
      wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
    );
    write_layer(
      queue,
      &current,
      0,
      &vec![0u8; (width * height * 4) as usize],
      width * 4,
      width,
      height,
    );
    let params = buffer_with_data(
      device,
      queue,
      "VistaWASM surface weather parameters",
      bytemuck::bytes_of(&SurfaceWeatherParams::zeroed()),
      wgpu::BufferUsages::UNIFORM,
    );
    Self {
      view: default_view(&current),
      next_view: default_view(&next),
      current,
      next,
      size: (width, height),
      runs: 0,
      pending: None,
      params,
      bound: None,
    }
  }

  /// Size in texels.
  pub fn size(&self) -> (u32, u32) {
    self.size
  }

  /// Queue a step for the next frame, merging it with any still waiting.
  pub fn queue(&mut self, step: SurfaceWeatherStep) {
    self.pending = Some(match self.pending.take() {
      Some(waiting) => SurfaceWeatherStep {
        dt: waiting.dt + step.dt,
        settle: step.settle.or(waiting.settle),
        ..step
      },
      None => step,
    });
  }

  /// Record one step into `encoder`: the pass, then the copy back.
  #[allow(clippy::too_many_arguments)]
  pub fn record(
    &mut self,
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    pipeline: &wgpu::ComputePipeline,
    layout: &wgpu::BindGroupLayout,
    step: &SurfaceWeatherStep,
    inputs: SurfaceWeatherInputs<'_>,
    timestamp_writes: Option<wgpu::ComputePassTimestampWrites<'_>>,
  ) {
    self.runs = self.runs.wrapping_add(1);
    let settle = step.settle.unwrap_or_default();
    let (width, height) = self.size;
    let (height_width, height_height) = inputs.height_size;
    let params = SurfaceWeatherParams {
      step: [step.dt, step.sun, step.wind, step.celsius_offset],
      rain: [
        step.precipitation,
        if step.regional.is_some() { 1.0 } else { 0.0 },
        1.0 / step.regional.unwrap_or(1.0).max(1.0),
        step.canopy,
      ],
      settle: [
        if step.settle.is_some() { 1.0 } else { 0.0 },
        settle.wetness,
        settle.puddles,
        settle.snow,
      ],
      grid: [
        height_width as f32 / width.max(1) as f32,
        height_height as f32 / height.max(1) as f32,
        inputs.texel_metres,
        (self.runs % 65_536) as f32,
      ],
      extent: [inputs.half[0], inputs.half[1], step.mean_precipitation, 0.0],
    };
    inputs
      .queue
      .write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
    let views = [inputs.heights, inputs.ground, inputs.regional];
    let current = self
      .bound
      .as_ref()
      .is_some_and(|(_, bound)| bound.iter().zip(views).all(|(a, b)| a == b));

    if !current {
      let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("VistaWASM surface weather bind group"),
        layout,
        entries: &[
          view_entry(0, &self.view),
          view_entry(1, &self.next_view),
          view_entry(2, inputs.heights),
          view_entry(3, inputs.ground),
          view_entry(4, inputs.regional),
          wgpu::BindGroupEntry {
            binding: 5,
            resource: self.params.as_entire_binding(),
          },
        ],
      });
      self.bound = Some((bind_group, views.map(Clone::clone)));
    }

    let Some((bind_group, _)) = &self.bound else {
      return;
    };

    {
      let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some("VistaWASM surface weather pass"),
        timestamp_writes,
      });
      pass.set_pipeline(pipeline);
      pass.set_bind_group(0, bind_group, &[]);
      pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), 1);
    }

    encoder.copy_texture_to_texture(
      self.next.as_image_copy(),
      self.current.as_image_copy(),
      wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
      },
    );
  }
}

/// What the surface weather pass reads besides its own state.
pub struct SurfaceWeatherInputs<'a> {
  /// For the parameter buffer.
  pub queue: &'a wgpu::Queue,
  /// The height texture and its size.
  pub heights: &'a wgpu::TextureView,
  /// Its size in texels.
  pub height_size: (u32, u32),
  /// Metres per height texel.
  pub texel_metres: f32,
  /// The ground layers.
  pub ground: &'a wgpu::TextureView,
  /// The regional weather map.
  pub regional: &'a wgpu::TextureView,
  /// Terrain half extents in metres.
  pub half: [f32; 2],
}

/// The pass's bind group layout.
pub fn surface_weather_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
  use wgpu::TextureViewDimension as Dim;
  let compute = wgpu::ShaderStages::COMPUTE;
  let texture = |binding, dimension| wgpu::BindGroupLayoutEntry {
    binding,
    visibility: compute,
    ty: wgpu::BindingType::Texture {
      sample_type: wgpu::TextureSampleType::Float { filterable: false },
      view_dimension: dimension,
      multisampled: false,
    },
    count: None,
  };
  device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
    label: Some("VistaWASM surface weather layout"),
    entries: &[
      texture(0, Dim::D2),
      wgpu::BindGroupLayoutEntry {
        binding: 1,
        visibility: compute,
        ty: wgpu::BindingType::StorageTexture {
          access: wgpu::StorageTextureAccess::WriteOnly,
          format: wgpu::TextureFormat::Rgba8Unorm,
          view_dimension: Dim::D2,
        },
        count: None,
      },
      texture(2, Dim::D2),
      texture(3, Dim::D2Array),
      texture(4, Dim::D2),
      uniform_entry(5, compute),
    ],
  })
}
