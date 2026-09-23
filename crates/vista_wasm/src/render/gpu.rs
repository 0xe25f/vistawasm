use bytemuck::{Pod, Zeroable};
use vista_types::{
  AtmosphereOptions, CloudsOptions, ErosionOptions, FloraOptions, MistOptions, WaterOptions,
};

use crate::errors::{VistaError, VistaResult};
use crate::render::erosion_compute::ErosionCompute;
use crate::render::flora::{FloraInstance, TreeInstance};
use crate::render::grass::GRASS_BASE_TUFT;
use crate::render::terrain_mesh::TerrainMeshData;
use crate::render::textures::{self, MipGenerator, MipMode, WorldTextures};
use crate::render::tree_models::{build_tree_library, SPECIES_COUNT};
use crate::render::water::{build_ocean_grid, WaterVertex, OCEAN_SNAP_METRES};
use crate::terrain::HeightMap;

/// Per-frame uniforms shared by every render shader.
///
/// The layout must stay in sync with `FrameUniforms` in `common.wgsl`, which
/// is prepended to every render shader, so there is exactly one declaration
/// to keep in step.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct FrameUniforms {
  view_proj: [f32; 16],
  camera_position: [f32; 4],
  camera_forward: [f32; 4],
  camera_right: [f32; 4],
  camera_up: [f32; 4],
  sun_direction: [f32; 4],
  atmosphere: [f32; 4],
  sky_tint: [f32; 4],
  mist_params: [f32; 4],
  mist_colour: [f32; 4],
  mist_wind: [f32; 4],
  cloud_params: [f32; 4],
  cloud_motion: [f32; 4],
  cloud_colour: [f32; 4],
  water_params: [f32; 4],
  water_shallow: [f32; 4],
  water_deep: [f32; 4],
  water_current: [f32; 4],
  wave_params: [f32; 4],
  wave_params2: [f32; 4],
  water_origin: [f32; 4],
  vegetation: [f32; 4],
  vegetation2: [f32; 4],
  viewport: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<FrameUniforms>() == 432);

/// Static world data: species bounds and tints, and the terrain height
/// texture mapping. Mirrors `WorldInfo` in `common.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct WorldInfo {
  species: [[f32; 4]; 8],
  species_tint: [[f32; 4]; 8],
  terrain: [f32; 4],
  terrain2: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<WorldInfo>() == 288);

/// Tree culling parameters. Mirrors `CullParams` in `tree_cull.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct CullParams {
  planes: [[f32; 4]; 6],
  camera: [f32; 4],
  params: [f32; 4],
  bounds: [[f32; 4]; 8],
  offsets: [[u32; 4]; 2],
}

const _: () = assert!(std::mem::size_of::<CullParams>() == 288);

/// Everything the renderer needs to shade one frame.
pub struct FrameParams {
  /// Combined view-projection matrix.
  pub view_proj: [f32; 16],
  /// Camera position in metres.
  pub camera_position: [f32; 3],
  /// Unit camera forward vector.
  pub camera_forward: [f32; 3],
  /// Unit camera right vector.
  pub camera_right: [f32; 3],
  /// Unit camera up vector.
  pub camera_up: [f32; 3],
  /// Vertical field of view.
  pub field_of_view_degrees: f32,
  /// Viewport aspect ratio.
  pub aspect_ratio: f32,
  /// Near plane distance.
  pub near_metres: f32,
  /// Far plane distance.
  pub far_metres: f32,
  /// Unit vector towards the sun.
  pub sun_direction: [f32; 3],
  /// Sun intensity.
  pub sun_intensity: f32,
  /// Atmosphere controls.
  pub atmosphere: AtmosphereOptions,
  /// Water controls.
  pub water: WaterOptions,
  /// Effective mist density (0 when mist is off).
  pub mist_density: f32,
  /// Mist noise strength (0 for flat mist).
  pub mist_noise_strength: f32,
  /// Water level for the rise-above-water term, or a far-away sentinel.
  pub mist_water_level_metres: f32,
  /// Mist controls.
  pub mist: MistOptions,
  /// Effective cloud coverage (0 when clouds are off).
  pub cloud_coverage: f32,
  /// Cloud raymarch steps (0 for painted clouds).
  pub cloud_raymarch_steps: u32,
  /// Cloud controls.
  pub clouds: CloudsOptions,
  /// Tree style: 0 billboard, 1 cross-quad, 2 mesh.
  pub tree_style: u32,
  /// Flora controls.
  pub flora: FloraOptions,
  /// Grass fade-out distance.
  pub grass_view_distance_metres: f32,
  /// Debug view index.
  pub debug_view: u32,
}

/// GPU-side terrain mesh resources for the active terrain.
struct TerrainGpu {
  vertex_buffer: wgpu::Buffer,
  index_buffer: wgpu::Buffer,
  index_count: u32,
}

/// GPU-side tree instances, culling outputs, and indirect arguments.
struct TreesGpu {
  mesh_out: wgpu::Buffer,
  impostor_out: wgpu::Buffer,
  args_buffer: wgpu::Buffer,
  cull_params_buffer: wgpu::Buffer,
  cull_bind_group: wgpu::BindGroup,
  instance_count: u32,
  offsets: [u32; SPECIES_COUNT],
  counts: [u32; SPECIES_COUNT],
  // Owned so the storage binding stays valid.
  _instance_buffer: wgpu::Buffer,
}

/// GPU-side grass instance buffer for the active terrain.
struct GrassGpu {
  instance_buffer: wgpu::Buffer,
  instance_count: u32,
}

/// An indexed mesh.
struct IndexedMesh {
  vertex_buffer: wgpu::Buffer,
  index_buffer: wgpu::Buffer,
  index_count: u32,
}

/// Words in the indirect argument buffer: eight indexed mesh draws (5
/// words each) followed by eight impostor draws (4 words each).
const INDIRECT_WORDS: usize = SPECIES_COUNT * 5 + SPECIES_COUNT * 4;
const IMPOSTOR_ARGS_BASE: usize = SPECIES_COUNT * 5;
const IMPOSTOR_WIDTH: u32 = 256;
const IMPOSTOR_HEIGHT: u32 = 512;
const HEIGHT_TEXTURE_MAX: u32 = 2048;
const OCEAN_GRID_SAMPLES: u32 = 193;
const OCEAN_FAR_REACH_METRES: f32 = 60_000.0;

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Foliage tint multipliers per species, in `TreeSpecies` order.
const SPECIES_TINTS: [[f32; 4]; 8] = [
  [1.0, 1.0, 0.95, 0.0],
  [0.78, 0.95, 0.85, 0.0],
  [0.6, 0.8, 0.76, 0.0],
  [1.12, 1.05, 0.78, 0.0],
  [0.95, 1.12, 0.85, 0.0],
  [1.0, 1.02, 0.8, 0.0],
  [1.08, 1.02, 0.72, 0.0],
  [0.95, 1.05, 0.9, 0.0],
];

/// WebGPU context owned by one VistaWASM engine.
pub struct GpuContext {
  surface: wgpu::Surface<'static>,
  device: wgpu::Device,
  queue: wgpu::Queue,
  config: wgpu::SurfaceConfiguration,
  depth_view: wgpu::TextureView,
  hdr_view: wgpu::TextureView,
  frame_bind_group: wgpu::BindGroup,
  uniform_buffer: wgpu::Buffer,
  world_layout: wgpu::BindGroupLayout,
  world_buffer: wgpu::Buffer,
  world_info: WorldInfo,
  world_bind_group: wgpu::BindGroup,
  composite_layout: wgpu::BindGroupLayout,
  composite_bind_group: wgpu::BindGroup,
  sampler: wgpu::Sampler,
  world_textures: WorldTextures,
  impostor_view: wgpu::TextureView,
  height_view: wgpu::TextureView,
  terrain_pipeline: wgpu::RenderPipeline,
  tree_mesh_pipeline: wgpu::RenderPipeline,
  tree_impostor_pipeline: wgpu::RenderPipeline,
  grass_pipeline: wgpu::RenderPipeline,
  composite_pipeline: wgpu::RenderPipeline,
  water_pipeline: wgpu::RenderPipeline,
  cull_pipeline: wgpu::ComputePipeline,
  tree_mesh: IndexedMesh,
  tree_ranges: [(u32, u32, i32); SPECIES_COUNT],
  tree_bounds: [(f32, f32); SPECIES_COUNT],
  grass_base_vertex_buffer: wgpu::Buffer,
  ocean: IndexedMesh,
  rivers: Option<IndexedMesh>,
  water_visible: bool,
  uniforms: FrameUniforms,
  start_time_ms: f64,
  erosion: ErosionCompute,
  terrain: Option<TerrainGpu>,
  trees: Option<TreesGpu>,
  grass: Option<GrassGpu>,
  width: u32,
  height: u32,
}

fn world_layout_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
  let visibility = wgpu::ShaderStages::VERTEX_FRAGMENT;
  let texture = |binding: u32, dimension: wgpu::TextureViewDimension, filterable: bool| {
    wgpu::BindGroupLayoutEntry {
      binding,
      visibility,
      ty: wgpu::BindingType::Texture {
        sample_type: wgpu::TextureSampleType::Float { filterable },
        view_dimension: dimension,
        multisampled: false,
      },
      count: None,
    }
  };

  vec![
    wgpu::BindGroupLayoutEntry {
      binding: 0,
      visibility,
      ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
      count: None,
    },
    texture(1, wgpu::TextureViewDimension::D2Array, true),
    texture(2, wgpu::TextureViewDimension::D2Array, true),
    texture(3, wgpu::TextureViewDimension::D2Array, true),
    texture(4, wgpu::TextureViewDimension::D2Array, true),
    texture(5, wgpu::TextureViewDimension::D2, true),
    texture(6, wgpu::TextureViewDimension::D3, true),
    texture(7, wgpu::TextureViewDimension::D2, true),
    texture(8, wgpu::TextureViewDimension::D2, false),
    wgpu::BindGroupLayoutEntry {
      binding: 9,
      visibility,
      ty: wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Uniform,
        has_dynamic_offset: false,
        min_binding_size: None,
      },
      count: None,
    },
  ]
}

#[allow(clippy::too_many_arguments)]
fn create_world_bind_group(
  device: &wgpu::Device,
  layout: &wgpu::BindGroupLayout,
  sampler: &wgpu::Sampler,
  textures: &WorldTextures,
  impostors: &wgpu::TextureView,
  height: &wgpu::TextureView,
  world_buffer: &wgpu::Buffer,
) -> wgpu::BindGroup {
  fn view(binding: u32, view: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
      binding,
      resource: wgpu::BindingResource::TextureView(view),
    }
  }

  device.create_bind_group(&wgpu::BindGroupDescriptor {
    label: Some("VistaWASM world bind group"),
    layout,
    entries: &[
      wgpu::BindGroupEntry {
        binding: 0,
        resource: wgpu::BindingResource::Sampler(sampler),
      },
      view(1, &textures.terrain_albedo),
      view(2, &textures.terrain_normal),
      view(3, &textures.flora),
      view(4, impostors),
      view(5, &textures.noise),
      view(6, &textures.cloud),
      view(7, &textures.water),
      view(8, height),
      wgpu::BindGroupEntry {
        binding: 9,
        resource: world_buffer.as_entire_binding(),
      },
    ],
  })
}

fn create_height_texture(
  device: &wgpu::Device,
  queue: &wgpu::Queue,
  width: u32,
  height: u32,
  data: &[f32],
) -> wgpu::TextureView {
  let texture = device.create_texture(&wgpu::TextureDescriptor {
    label: Some("VistaWASM terrain heights"),
    size: wgpu::Extent3d {
      width,
      height,
      depth_or_array_layers: 1,
    },
    mip_level_count: 1,
    sample_count: 1,
    dimension: wgpu::TextureDimension::D2,
    format: wgpu::TextureFormat::R32Float,
    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
    view_formats: &[],
  });
  queue.write_texture(
    wgpu::TexelCopyTextureInfo {
      texture: &texture,
      mip_level: 0,
      origin: wgpu::Origin3d::ZERO,
      aspect: wgpu::TextureAspect::All,
    },
    bytemuck::cast_slice(data),
    wgpu::TexelCopyBufferLayout {
      offset: 0,
      bytes_per_row: Some(width * 4),
      rows_per_image: Some(height),
    },
    wgpu::Extent3d {
      width,
      height,
      depth_or_array_layers: 1,
    },
  );
  texture.create_view(&wgpu::TextureViewDescriptor::default())
}

fn create_render_targets(
  device: &wgpu::Device,
  width: u32,
  height: u32,
) -> (wgpu::TextureView, wgpu::TextureView) {
  let size = wgpu::Extent3d {
    width: width.max(1),
    height: height.max(1),
    depth_or_array_layers: 1,
  };
  let depth = device.create_texture(&wgpu::TextureDescriptor {
    label: Some("VistaWASM depth buffer"),
    size,
    mip_level_count: 1,
    sample_count: 1,
    dimension: wgpu::TextureDimension::D2,
    format: DEPTH_FORMAT,
    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
    view_formats: &[],
  });
  let hdr = device.create_texture(&wgpu::TextureDescriptor {
    label: Some("VistaWASM HDR scene"),
    size,
    mip_level_count: 1,
    sample_count: 1,
    dimension: wgpu::TextureDimension::D2,
    format: HDR_FORMAT,
    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
    view_formats: &[],
  });

  (
    depth.create_view(&wgpu::TextureViewDescriptor::default()),
    hdr.create_view(&wgpu::TextureViewDescriptor::default()),
  )
}

fn create_composite_bind_group(
  device: &wgpu::Device,
  layout: &wgpu::BindGroupLayout,
  hdr: &wgpu::TextureView,
  depth: &wgpu::TextureView,
) -> wgpu::BindGroup {
  device.create_bind_group(&wgpu::BindGroupDescriptor {
    label: Some("VistaWASM composite bind group"),
    layout,
    entries: &[
      wgpu::BindGroupEntry {
        binding: 0,
        resource: wgpu::BindingResource::TextureView(hdr),
      },
      wgpu::BindGroupEntry {
        binding: 1,
        resource: wgpu::BindingResource::TextureView(depth),
      },
    ],
  })
}

fn buffer_with_data(
  device: &wgpu::Device,
  queue: &wgpu::Queue,
  label: &str,
  data: &[u8],
  usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
  // WebGPU requires buffer sizes to be a multiple of four bytes.
  let size = (data.len() as u64).max(4).div_ceil(4) * 4;
  let buffer = device.create_buffer(&wgpu::BufferDescriptor {
    label: Some(label),
    size,
    usage: usage | wgpu::BufferUsages::COPY_DST,
    mapped_at_creation: false,
  });

  if !data.is_empty() {
    queue.write_buffer(&buffer, 0, data);
  }

  buffer
}

/// Shader source for a render shader: `common.wgsl` followed by the file.
macro_rules! render_shader {
  ($file:literal) => {
    concat!(
      include_str!("../shaders/common.wgsl"),
      "\n",
      include_str!(concat!("../shaders/", $file))
    )
  };
}

struct PipelineSpec<'a> {
  label: &'a str,
  source: &'a str,
  vertex_entry: &'a str,
  fragment_entry: &'a str,
  buffers: &'a [Option<wgpu::VertexBufferLayout<'a>>],
  format: wgpu::TextureFormat,
  blend: Option<wgpu::BlendState>,
  depth: Option<(bool, wgpu::CompareFunction)>,
  cull_mode: Option<wgpu::Face>,
}

fn create_pipeline(
  device: &wgpu::Device,
  layout: &wgpu::PipelineLayout,
  spec: PipelineSpec<'_>,
) -> wgpu::RenderPipeline {
  let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
    label: Some(spec.label),
    source: wgpu::ShaderSource::Wgsl(spec.source.into()),
  });

  device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
    label: Some(spec.label),
    layout: Some(layout),
    vertex: wgpu::VertexState {
      module: &module,
      entry_point: Some(spec.vertex_entry),
      compilation_options: wgpu::PipelineCompilationOptions::default(),
      buffers: spec.buffers,
    },
    fragment: Some(wgpu::FragmentState {
      module: &module,
      entry_point: Some(spec.fragment_entry),
      compilation_options: wgpu::PipelineCompilationOptions::default(),
      targets: &[Some(wgpu::ColorTargetState {
        format: spec.format,
        blend: spec.blend,
        write_mask: wgpu::ColorWrites::ALL,
      })],
    }),
    primitive: wgpu::PrimitiveState {
      topology: wgpu::PrimitiveTopology::TriangleList,
      strip_index_format: None,
      front_face: wgpu::FrontFace::Ccw,
      cull_mode: spec.cull_mode,
      unclipped_depth: false,
      polygon_mode: wgpu::PolygonMode::Fill,
      conservative: false,
    },
    depth_stencil: spec.depth.map(|(write, compare)| wgpu::DepthStencilState {
      format: DEPTH_FORMAT,
      depth_write_enabled: Some(write),
      depth_compare: Some(compare),
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

const TERRAIN_ATTRIBUTES: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
  0 => Float32x3,
  1 => Float32x3,
  2 => Unorm8x4,
  3 => Unorm8x4,
  4 => Unorm8x4,
  5 => Uint8x4,
];

const TREE_VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
  0 => Float32x3,
  1 => Float32x3,
  2 => Float32x2,
  3 => Float32x4,
];

const TREE_INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
  4 => Float32x3,
  5 => Float32,
  6 => Float32,
  7 => Float32,
  8 => Float32,
  9 => Float32,
];

const GRASS_BASE_ATTRIBUTES: [wgpu::VertexAttribute; 2] = wgpu::vertex_attr_array![
  0 => Float32x2,
  1 => Float32x2,
];

const GRASS_INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
  2 => Float32x3,
  3 => Float32,
  4 => Float32,
  5 => Float32,
];

const WATER_ATTRIBUTES: [wgpu::VertexAttribute; 3] = wgpu::vertex_attr_array![
  0 => Float32x3,
  1 => Float32x2,
  2 => Float32x3,
];

fn tree_vertex_layout() -> wgpu::VertexBufferLayout<'static> {
  wgpu::VertexBufferLayout {
    array_stride: std::mem::size_of::<crate::render::tree_models::TreeVertex>() as u64,
    step_mode: wgpu::VertexStepMode::Vertex,
    attributes: &TREE_VERTEX_ATTRIBUTES,
  }
}

fn tree_instance_layout() -> wgpu::VertexBufferLayout<'static> {
  wgpu::VertexBufferLayout {
    array_stride: std::mem::size_of::<TreeInstance>() as u64,
    step_mode: wgpu::VertexStepMode::Instance,
    attributes: &TREE_INSTANCE_ATTRIBUTES,
  }
}

/// Extract normalised frustum planes (`ax + by + cz + d >= 0` inside) from
/// a column-major view-projection matrix with a 0..1 depth range.
fn frustum_planes(m: &[f32; 16]) -> [[f32; 4]; 6] {
  let row = |i: usize| [m[i], m[4 + i], m[8 + i], m[12 + i]];
  let (r0, r1, r2, r3) = (row(0), row(1), row(2), row(3));
  let combine = |a: [f32; 4], b: [f32; 4], sign: f32| {
    [
      a[0] + sign * b[0],
      a[1] + sign * b[1],
      a[2] + sign * b[2],
      a[3] + sign * b[3],
    ]
  };
  let planes = [
    combine(r3, r0, 1.0),
    combine(r3, r0, -1.0),
    combine(r3, r1, 1.0),
    combine(r3, r1, -1.0),
    r2,
    combine(r3, r2, -1.0),
  ];

  planes.map(|plane| {
    let length = (plane[0] * plane[0] + plane[1] * plane[1] + plane[2] * plane[2])
      .sqrt()
      .max(1e-6);
    [
      plane[0] / length,
      plane[1] / length,
      plane[2] / length,
      plane[3] / length,
    ]
  })
}

fn direction_from_degrees(degrees: f32) -> [f32; 2] {
  let radians = degrees.to_radians();
  [radians.sin(), radians.cos()]
}

fn now_ms() -> f64 {
  js_sys::Date::now()
}

impl GpuContext {
  /// Create and configure WebGPU resources for a browser canvas, bake the
  /// procedural textures, model the tree species, and render their
  /// impostors.
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

    let (depth_view, hdr_view) = create_render_targets(&device, pixel_width, pixel_height);
    let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
      label: Some("VistaWASM frame uniforms"),
      size: std::mem::size_of::<FrameUniforms>() as u64,
      usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
      mapped_at_creation: false,
    });

    let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
    let frame_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
      label: Some("VistaWASM frame bind group"),
      layout: &frame_layout,
      entries: &[wgpu::BindGroupEntry {
        binding: 0,
        resource: uniform_buffer.as_entire_binding(),
      }],
    });
    let world_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
      label: Some("VistaWASM world bind group layout"),
      entries: &world_layout_entries(),
    });
    let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
      label: Some("VistaWASM composite bind group layout"),
      entries: &[
        wgpu::BindGroupLayoutEntry {
          binding: 0,
          visibility: wgpu::ShaderStages::FRAGMENT,
          ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
          },
          count: None,
        },
        wgpu::BindGroupLayoutEntry {
          binding: 1,
          visibility: wgpu::ShaderStages::FRAGMENT,
          ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Depth,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
          },
          count: None,
        },
      ],
    });
    let composite_bind_group =
      create_composite_bind_group(&device, &composite_layout, &hdr_view, &depth_view);

    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
      label: Some("VistaWASM linear repeat sampler"),
      address_mode_u: wgpu::AddressMode::Repeat,
      address_mode_v: wgpu::AddressMode::Repeat,
      address_mode_w: wgpu::AddressMode::Repeat,
      mag_filter: wgpu::FilterMode::Linear,
      min_filter: wgpu::FilterMode::Linear,
      mipmap_filter: wgpu::MipmapFilterMode::Linear,
      anisotropy_clamp: 8,
      ..Default::default()
    });

    // Procedural textures and tree models.
    let mips = MipGenerator::new(&device);
    let world_textures = textures::bake_world_textures(&device, &queue, &mips);
    let library = build_tree_library();
    let tree_mesh = IndexedMesh {
      vertex_buffer: buffer_with_data(
        &device,
        &queue,
        "VistaWASM tree vertices",
        bytemuck::cast_slice(&library.vertices),
        wgpu::BufferUsages::VERTEX,
      ),
      index_buffer: buffer_with_data(
        &device,
        &queue,
        "VistaWASM tree indices",
        bytemuck::cast_slice(&library.indices),
        wgpu::BufferUsages::INDEX,
      ),
      index_count: library.indices.len() as u32,
    };

    let mut world_info = WorldInfo::zeroed();

    for (slot, (height, radius)) in library.bounds.iter().enumerate() {
      world_info.species[slot] = [*height, *radius, 0.0, 0.0];
      world_info.species_tint[slot] = SPECIES_TINTS[slot];
    }

    let world_buffer = buffer_with_data(
      &device,
      &queue,
      "VistaWASM world info",
      bytemuck::bytes_of(&world_info),
      wgpu::BufferUsages::UNIFORM,
    );
    let height_view = create_height_texture(&device, &queue, 1, 1, &[-100_000.0]);

    let render_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
      label: Some("VistaWASM render pipeline layout"),
      bind_group_layouts: &[Some(&frame_layout), Some(&world_layout)],
      immediate_size: 0,
    });
    let composite_pipeline_layout =
      device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("VistaWASM composite pipeline layout"),
        bind_group_layouts: &[
          Some(&frame_layout),
          Some(&world_layout),
          Some(&composite_layout),
        ],
        immediate_size: 0,
      });

    // Bake impostors of every species with the real tree meshes.
    let impostor_texture = textures::create_array_texture(
      &device,
      "VistaWASM tree impostors",
      IMPOSTOR_WIDTH,
      IMPOSTOR_HEIGHT,
      SPECIES_COUNT as u32,
      wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    {
      let placeholder = textures::create_array_texture(
        &device,
        "VistaWASM impostor placeholder",
        1,
        1,
        1,
        wgpu::TextureUsages::empty(),
      );
      let placeholder_view = textures::array_view(&placeholder);
      let bake_bind_group = create_world_bind_group(
        &device,
        &world_layout,
        &sampler,
        &world_textures,
        &placeholder_view,
        &height_view,
        &world_buffer,
      );
      let bake_pipeline = create_pipeline(
        &device,
        &render_layout,
        PipelineSpec {
          label: "VistaWASM impostor bake pipeline",
          source: render_shader!("trees.wgsl"),
          vertex_entry: "vertex_bake",
          fragment_entry: "fragment_bake",
          buffers: &[Some(tree_vertex_layout())],
          format: wgpu::TextureFormat::Rgba8Unorm,
          blend: None,
          depth: Some((true, wgpu::CompareFunction::Less)),
          cull_mode: None,
        },
      );
      let bake_depth = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("VistaWASM impostor depth"),
        size: wgpu::Extent3d {
          width: IMPOSTOR_WIDTH,
          height: IMPOSTOR_HEIGHT,
          depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
      });
      let bake_depth_view = bake_depth.create_view(&wgpu::TextureViewDescriptor::default());
      let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("VistaWASM impostor bake"),
      });

      for (slot, (first_index, index_count, base_vertex)) in library.ranges.iter().enumerate() {
        let layer_view = impostor_texture.create_view(&wgpu::TextureViewDescriptor {
          label: Some("VistaWASM impostor layer"),
          dimension: Some(wgpu::TextureViewDimension::D2),
          base_mip_level: 0,
          mip_level_count: Some(1),
          base_array_layer: slot as u32,
          array_layer_count: Some(1),
          ..Default::default()
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
          label: Some("VistaWASM impostor bake pass"),
          color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &layer_view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
              load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
              store: wgpu::StoreOp::Store,
            },
          })],
          depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: &bake_depth_view,
            depth_ops: Some(wgpu::Operations {
              load: wgpu::LoadOp::Clear(1.0),
              store: wgpu::StoreOp::Discard,
            }),
            stencil_ops: None,
          }),
          ..Default::default()
        });
        pass.set_pipeline(&bake_pipeline);
        pass.set_bind_group(0, &frame_bind_group, &[]);
        pass.set_bind_group(1, &bake_bind_group, &[]);
        pass.set_vertex_buffer(0, tree_mesh.vertex_buffer.slice(..));
        pass.set_index_buffer(tree_mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(
          *first_index..*first_index + *index_count,
          *base_vertex,
          slot as u32..slot as u32 + 1,
        );
      }

      mips.generate(&device, &mut encoder, &impostor_texture, MipMode::Coverage);
      queue.submit(Some(encoder.finish()));
    }
    let impostor_view = textures::array_view(&impostor_texture);

    let world_bind_group = create_world_bind_group(
      &device,
      &world_layout,
      &sampler,
      &world_textures,
      &impostor_view,
      &height_view,
      &world_buffer,
    );

    let terrain_pipeline = create_pipeline(
      &device,
      &render_layout,
      PipelineSpec {
        label: "VistaWASM terrain pipeline",
        source: render_shader!("clipmap_render.wgsl"),
        vertex_entry: "vertex_main",
        fragment_entry: "fragment_main",
        buffers: &[Some(wgpu::VertexBufferLayout {
          array_stride: std::mem::size_of::<crate::render::terrain_mesh::TerrainVertex>() as u64,
          step_mode: wgpu::VertexStepMode::Vertex,
          attributes: &TERRAIN_ATTRIBUTES,
        })],
        format: HDR_FORMAT,
        blend: None,
        depth: Some((true, wgpu::CompareFunction::Less)),
        cull_mode: Some(wgpu::Face::Back),
      },
    );
    let tree_mesh_pipeline = create_pipeline(
      &device,
      &render_layout,
      PipelineSpec {
        label: "VistaWASM tree mesh pipeline",
        source: render_shader!("trees.wgsl"),
        vertex_entry: "vertex_mesh",
        fragment_entry: "fragment_mesh",
        buffers: &[Some(tree_vertex_layout()), Some(tree_instance_layout())],
        format: HDR_FORMAT,
        blend: None,
        depth: Some((true, wgpu::CompareFunction::Less)),
        cull_mode: None,
      },
    );
    let tree_impostor_pipeline = create_pipeline(
      &device,
      &render_layout,
      PipelineSpec {
        label: "VistaWASM tree impostor pipeline",
        source: render_shader!("trees.wgsl"),
        vertex_entry: "vertex_impostor",
        fragment_entry: "fragment_impostor",
        buffers: &[Some(tree_instance_layout())],
        format: HDR_FORMAT,
        blend: None,
        depth: Some((true, wgpu::CompareFunction::Less)),
        cull_mode: None,
      },
    );
    let grass_pipeline = create_pipeline(
      &device,
      &render_layout,
      PipelineSpec {
        label: "VistaWASM grass pipeline",
        source: render_shader!("grass_instances.wgsl"),
        vertex_entry: "vertex_main",
        fragment_entry: "fragment_main",
        buffers: &[
          Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<crate::render::flora::FloraVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &GRASS_BASE_ATTRIBUTES,
          }),
          Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<FloraInstance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &GRASS_INSTANCE_ATTRIBUTES,
          }),
        ],
        format: HDR_FORMAT,
        blend: None,
        depth: Some((true, wgpu::CompareFunction::Less)),
        cull_mode: None,
      },
    );
    let composite_pipeline = create_pipeline(
      &device,
      &composite_pipeline_layout,
      PipelineSpec {
        label: "VistaWASM composite pipeline",
        source: render_shader!("atmosphere.wgsl"),
        vertex_entry: "vertex_main",
        fragment_entry: "fragment_main",
        buffers: &[],
        format: config.format,
        blend: None,
        depth: None,
        cull_mode: None,
      },
    );
    let water_pipeline = create_pipeline(
      &device,
      &render_layout,
      PipelineSpec {
        label: "VistaWASM water pipeline",
        source: render_shader!("water.wgsl"),
        vertex_entry: "vertex_main",
        fragment_entry: "fragment_main",
        buffers: &[Some(wgpu::VertexBufferLayout {
          array_stride: std::mem::size_of::<WaterVertex>() as u64,
          step_mode: wgpu::VertexStepMode::Vertex,
          attributes: &WATER_ATTRIBUTES,
        })],
        format: config.format,
        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
        depth: Some((false, wgpu::CompareFunction::Less)),
        cull_mode: None,
      },
    );
    let cull_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
      label: Some("VistaWASM tree cull shader"),
      source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/tree_cull.wgsl").into()),
    });
    let cull_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
      label: Some("VistaWASM tree cull pipeline"),
      layout: None,
      module: &cull_module,
      entry_point: Some("cull_main"),
      compilation_options: wgpu::PipelineCompilationOptions::default(),
      cache: None,
    });

    let grass_base_vertex_buffer = buffer_with_data(
      &device,
      &queue,
      "VistaWASM grass base tuft",
      bytemuck::cast_slice(&GRASS_BASE_TUFT),
      wgpu::BufferUsages::VERTEX,
    );
    let (ocean_vertices, ocean_indices) =
      build_ocean_grid(OCEAN_GRID_SAMPLES, OCEAN_FAR_REACH_METRES);
    let ocean = IndexedMesh {
      vertex_buffer: buffer_with_data(
        &device,
        &queue,
        "VistaWASM ocean grid",
        bytemuck::cast_slice(&ocean_vertices),
        wgpu::BufferUsages::VERTEX,
      ),
      index_buffer: buffer_with_data(
        &device,
        &queue,
        "VistaWASM ocean indices",
        bytemuck::cast_slice(&ocean_indices),
        wgpu::BufferUsages::INDEX,
      ),
      index_count: ocean_indices.len() as u32,
    };
    let erosion = ErosionCompute::new(&device);
    let mut uniforms = FrameUniforms::zeroed();
    uniforms.camera_up[3] = if config.format.is_srgb() { 0.0 } else { 1.0 };

    Ok(Self {
      surface,
      device,
      queue,
      config,
      depth_view,
      hdr_view,
      frame_bind_group,
      uniform_buffer,
      world_layout,
      world_buffer,
      world_info,
      world_bind_group,
      composite_layout,
      composite_bind_group,
      sampler,
      world_textures,
      impostor_view,
      height_view,
      terrain_pipeline,
      tree_mesh_pipeline,
      tree_impostor_pipeline,
      grass_pipeline,
      composite_pipeline,
      water_pipeline,
      cull_pipeline,
      tree_mesh,
      tree_ranges: library.ranges,
      tree_bounds: library.bounds,
      grass_base_vertex_buffer,
      ocean,
      rivers: None,
      water_visible: false,
      uniforms,
      start_time_ms: now_ms(),
      erosion,
      terrain: None,
      trees: None,
      grass: None,
      width: pixel_width,
      height: pixel_height,
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

    self.terrain = Some(TerrainGpu {
      vertex_buffer: buffer_with_data(
        &self.device,
        &self.queue,
        "VistaWASM terrain vertices",
        bytemuck::cast_slice(&mesh.vertices),
        wgpu::BufferUsages::VERTEX,
      ),
      index_buffer: buffer_with_data(
        &self.device,
        &self.queue,
        "VistaWASM terrain indices",
        bytemuck::cast_slice(&mesh.indices),
        wgpu::BufferUsages::INDEX,
      ),
      index_count: mesh.indices.len() as u32,
    });
  }

  /// Upload the terrain heights used for water depth, shorelines, and the
  /// sea bed beyond the terrain. Large terrain is downsampled.
  pub fn upload_heightmap(&mut self, map: &HeightMap) {
    let width = map.metadata.width;
    let height = map.metadata.height;

    if width == 0 || height == 0 {
      return;
    }

    let stride = (width.max(height).saturating_sub(1) / (HEIGHT_TEXTURE_MAX - 1)).max(1);
    let texture_width = (width - 1) / stride + 1;
    let texture_height = (height - 1) / stride + 1;
    let sea = map.metadata.sea_level_metres;
    let mut data = Vec::with_capacity((texture_width * texture_height) as usize);

    for ty in 0..texture_height {
      for tx in 0..texture_width {
        let index = ((ty * stride) * width + tx * stride) as usize;
        data.push(if map.no_data[index] {
          sea - 50.0
        } else {
          map.heights[index]
        });
      }
    }

    let metres_per_sample = map.metadata.metres_per_sample.max(0.001);
    self.height_view = create_height_texture(
      &self.device,
      &self.queue,
      texture_width,
      texture_height,
      &data,
    );
    self.world_info.terrain = [
      (width as f32 - 1.0) * metres_per_sample * 0.5,
      (height as f32 - 1.0) * metres_per_sample * 0.5,
      metres_per_sample * stride as f32,
      metres_per_sample * stride as f32,
    ];
    self.world_info.terrain2 = [texture_width as f32, texture_height as f32, 1.0, 0.0];
    self
      .queue
      .write_buffer(&self.world_buffer, 0, bytemuck::bytes_of(&self.world_info));
    self.world_bind_group = create_world_bind_group(
      &self.device,
      &self.world_layout,
      &self.sampler,
      &self.world_textures,
      &self.impostor_view,
      &self.height_view,
      &self.world_buffer,
    );
  }

  /// Upload tree instances, replacing any previous trees. Instances are
  /// culled and sorted into per-species level-of-detail lists on the GPU
  /// every frame.
  pub fn upload_trees(&mut self, instances: &[TreeInstance]) {
    if instances.is_empty() {
      self.trees = None;
      return;
    }

    let mut counts = [0u32; SPECIES_COUNT];

    for instance in instances {
      counts[(instance.species as usize).min(SPECIES_COUNT - 1)] += 1;
    }

    let mut offsets = [0u32; SPECIES_COUNT];
    let mut running = 0;

    for (slot, count) in counts.iter().enumerate() {
      offsets[slot] = running;
      running += count;
    }

    let instance_bytes: &[u8] = bytemuck::cast_slice(instances);
    let instance_buffer = buffer_with_data(
      &self.device,
      &self.queue,
      "VistaWASM tree instances",
      instance_bytes,
      wgpu::BufferUsages::STORAGE,
    );
    let output = |label: &str| {
      self.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: instance_bytes.len() as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::VERTEX,
        mapped_at_creation: false,
      })
    };
    let mesh_out = output("VistaWASM visible tree meshes");
    let impostor_out = output("VistaWASM visible tree impostors");
    let args_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
      label: Some("VistaWASM tree indirect arguments"),
      size: (INDIRECT_WORDS * 4) as u64,
      usage: wgpu::BufferUsages::STORAGE
        | wgpu::BufferUsages::INDIRECT
        | wgpu::BufferUsages::COPY_DST,
      mapped_at_creation: false,
    });
    let cull_params_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
      label: Some("VistaWASM tree cull parameters"),
      size: std::mem::size_of::<CullParams>() as u64,
      usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
      mapped_at_creation: false,
    });
    let cull_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
      label: Some("VistaWASM tree cull bind group"),
      layout: &self.cull_pipeline.get_bind_group_layout(0),
      entries: &[
        wgpu::BindGroupEntry {
          binding: 0,
          resource: cull_params_buffer.as_entire_binding(),
        },
        wgpu::BindGroupEntry {
          binding: 1,
          resource: instance_buffer.as_entire_binding(),
        },
        wgpu::BindGroupEntry {
          binding: 2,
          resource: mesh_out.as_entire_binding(),
        },
        wgpu::BindGroupEntry {
          binding: 3,
          resource: impostor_out.as_entire_binding(),
        },
        wgpu::BindGroupEntry {
          binding: 4,
          resource: args_buffer.as_entire_binding(),
        },
      ],
    });

    self.trees = Some(TreesGpu {
      mesh_out,
      impostor_out,
      args_buffer,
      cull_params_buffer,
      cull_bind_group,
      instance_count: instances.len() as u32,
      offsets,
      counts,
      _instance_buffer: instance_buffer,
    });
  }

  /// Upload grass tuft instances, replacing any previous buffer.
  pub fn upload_grass(&mut self, instances: &[FloraInstance]) {
    if instances.is_empty() {
      self.grass = None;
      return;
    }

    self.grass = Some(GrassGpu {
      instance_buffer: buffer_with_data(
        &self.device,
        &self.queue,
        "VistaWASM grass instances",
        bytemuck::cast_slice(instances),
        wgpu::BufferUsages::VERTEX,
      ),
      instance_count: instances.len() as u32,
    });
  }

  /// Upload river and lake geometry, replacing any previous buffers.
  pub fn upload_rivers(&mut self, vertices: &[WaterVertex], indices: &[u32]) {
    if vertices.is_empty() || indices.is_empty() {
      self.rivers = None;
      return;
    }

    self.rivers = Some(IndexedMesh {
      vertex_buffer: buffer_with_data(
        &self.device,
        &self.queue,
        "VistaWASM river vertices",
        bytemuck::cast_slice(vertices),
        wgpu::BufferUsages::VERTEX,
      ),
      index_buffer: buffer_with_data(
        &self.device,
        &self.queue,
        "VistaWASM river indices",
        bytemuck::cast_slice(indices),
        wgpu::BufferUsages::INDEX,
      ),
      index_count: indices.len() as u32,
    });
  }

  /// Show or hide all water.
  pub fn set_water_visible(&mut self, visible: bool) {
    self.water_visible = visible;
  }

  fn update_uniforms(&mut self, params: &FrameParams, time: f32) {
    let u = &mut self.uniforms;
    let apply_gamma = u.camera_up[3];
    let p = params.camera_position;
    let tan_half_fov_y = (params.field_of_view_degrees.to_radians() * 0.5).tan();

    u.view_proj = params.view_proj;
    u.camera_position = [p[0], p[1], p[2], time];
    u.camera_forward = [
      params.camera_forward[0],
      params.camera_forward[1],
      params.camera_forward[2],
      tan_half_fov_y,
    ];
    u.camera_right = [
      params.camera_right[0],
      params.camera_right[1],
      params.camera_right[2],
      params.aspect_ratio.max(0.001),
    ];
    u.camera_up = [
      params.camera_up[0],
      params.camera_up[1],
      params.camera_up[2],
      apply_gamma,
    ];
    u.sun_direction = [
      params.sun_direction[0],
      params.sun_direction[1],
      params.sun_direction[2],
      params.sun_intensity.max(0.0),
    ];

    let atmosphere = &params.atmosphere;
    u.atmosphere = [
      atmosphere.rayleigh_strength.max(0.0),
      atmosphere.mie_strength.max(0.0),
      atmosphere.haze_distance_metres.max(1.0),
      atmosphere.exposure.max(0.0),
    ];
    u.sky_tint = [
      atmosphere.sky_tint[0],
      atmosphere.sky_tint[1],
      atmosphere.sky_tint[2],
      params.debug_view as f32,
    ];

    let mist = &params.mist;
    let mist_wind = direction_from_degrees(mist.wind_direction_degrees);
    let mist_drift = mist.wind_speed_metres_per_second.max(0.0) * time;
    u.mist_params = [
      params.mist_density.clamp(0.0, 1.0),
      mist.base_height_metres,
      mist.height_falloff_metres.max(1.0),
      params.mist_noise_strength.max(0.0),
    ];
    u.mist_colour = [
      mist.colour[0],
      mist.colour[1],
      mist.colour[2],
      params.mist_water_level_metres,
    ];
    u.mist_wind = [
      mist_wind[0] * mist_drift,
      mist_wind[1] * mist_drift,
      mist.sun_scattering.clamp(0.0, 1.0),
      ((mist.seed_offset % 997) as f32 * 0.618_034).fract(),
    ];

    let clouds = &params.clouds;
    let cloud_wind = direction_from_degrees(clouds.wind_direction_degrees);
    let cloud_drift = clouds.speed * 15.0 * time;
    let seed = (clouds.seed_offset % 10_007) as f32;
    u.cloud_params = [
      params.cloud_coverage.clamp(0.0, 1.0),
      clouds.height_metres,
      clouds.thickness_metres.max(1.0),
      params.cloud_raymarch_steps as f32,
    ];
    // The wind carries clouds downwind, so the noise lookup moves upwind.
    u.cloud_motion = [
      -cloud_wind[0] * cloud_drift + seed * 173.0,
      -cloud_wind[1] * cloud_drift + seed * 311.0,
      clouds.evolution.clamp(0.0, 1.0) * time * 14.0,
      clouds.density.clamp(0.0, 1.0),
    ];
    u.cloud_colour = [
      clouds.colour[0],
      clouds.colour[1],
      clouds.colour[2],
      if clouds.cast_shadows { 1.0 } else { 0.0 },
    ];

    let water = &params.water;
    let current = direction_from_degrees(water.current_direction_degrees);
    let current_drift = water.current_speed.max(0.0) * time;
    u.water_params = [
      water.wave_scale.max(0.0),
      water.reflectivity.clamp(0.0, 1.0),
      water.clarity_metres.max(0.1),
      water.foam.clamp(0.0, 1.0),
    ];
    u.water_shallow = [
      water.shallow_colour[0],
      water.shallow_colour[1],
      water.shallow_colour[2],
      water.sea_level_metres,
    ];
    u.water_deep = [
      water.deep_colour[0],
      water.deep_colour[1],
      water.deep_colour[2],
      params.near_metres.max(0.001),
    ];
    u.water_current = [
      -current[0] * current_drift,
      -current[1] * current_drift,
      water.current_speed.max(0.0),
      params.far_metres.max(1.0),
    ];
    let waves = &water.waves;
    u.wave_params = [
      waves.amplitude_metres.clamp(0.0, 30.0),
      waves.wavelength_metres.clamp(0.5, 2_000.0),
      waves.direction_degrees.to_radians(),
      waves.steepness.clamp(0.0, 1.0),
    ];
    u.wave_params2 = [
      waves.speed.max(0.0),
      waves.directional_spread.clamp(0.0, 1.0),
      if waves.enabled { 1.0 } else { 0.0 },
      0.0,
    ];
    u.water_origin = [
      (p[0] / OCEAN_SNAP_METRES).round() * OCEAN_SNAP_METRES,
      (p[2] / OCEAN_SNAP_METRES).round() * OCEAN_SNAP_METRES,
      0.0,
      0.0,
    ];

    let flora = &params.flora;
    u.vegetation = [
      flora.wind_strength.clamp(0.0, 1.0),
      flora.species_variation.clamp(0.0, 1.0),
      params.tree_style as f32,
      params.grass_view_distance_metres.max(0.0),
    ];
    u.vegetation2 = [flora.mesh_distance_metres.max(1.0), 0.0, 0.0, 0.0];
    let width = self.width.max(1) as f32;
    let height = self.height.max(1) as f32;
    u.viewport = [width, height, 1.0 / width, 1.0 / height];
  }

  /// Render one frame.
  pub fn render_once(&mut self, params: &FrameParams) -> VistaResult<()> {
    let time = ((now_ms() - self.start_time_ms) / 1000.0) as f32;
    self.update_uniforms(params, time);
    self
      .queue
      .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&self.uniforms));

    let impostor_vertices = if params.tree_style == 1 { 12 } else { 6 };

    if let Some(trees) = &self.trees {
      let tan_half_fov_y = (params.field_of_view_degrees.to_radians() * 0.5)
        .tan()
        .max(0.0001);
      let mut cull = CullParams::zeroed();
      cull.planes = frustum_planes(&params.view_proj);
      cull.camera = [
        params.camera_position[0],
        params.camera_position[1],
        params.camera_position[2],
        params.flora.mesh_distance_metres.max(1.0),
      ];
      cull.params = [
        params.far_metres.min(40_000.0),
        params.tree_style as f32,
        self.height as f32 / (2.0 * tan_half_fov_y),
        trees.instance_count as f32,
      ];

      for (slot, (height, radius)) in self.tree_bounds.iter().enumerate() {
        cull.bounds[slot] = [*height, *radius, 0.0, 0.0];
        cull.offsets[slot / 4][slot % 4] = trees.offsets[slot];
      }

      self
        .queue
        .write_buffer(&trees.cull_params_buffer, 0, bytemuck::bytes_of(&cull));

      let mut args = [0u32; INDIRECT_WORDS];

      for (slot, (first_index, index_count, base_vertex)) in self.tree_ranges.iter().enumerate() {
        args[slot * 5] = *index_count;
        args[slot * 5 + 2] = *first_index;
        args[slot * 5 + 3] = *base_vertex as u32;
        args[IMPOSTOR_ARGS_BASE + slot * 4] = impostor_vertices;
      }

      self
        .queue
        .write_buffer(&trees.args_buffer, 0, bytemuck::cast_slice(&args));
    }

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

    if let Some(trees) = &self.trees {
      let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some("VistaWASM tree cull pass"),
        timestamp_writes: None,
      });
      pass.set_pipeline(&self.cull_pipeline);
      pass.set_bind_group(0, &trees.cull_bind_group, &[]);
      pass.dispatch_workgroups(trees.instance_count.div_ceil(64), 1, 1);
    }

    // Opaque geometry into the HDR target.
    {
      let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("VistaWASM opaque pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
          view: &self.hdr_view,
          depth_slice: None,
          resolve_target: None,
          ops: wgpu::Operations {
            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
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
      pass.set_bind_group(0, &self.frame_bind_group, &[]);
      pass.set_bind_group(1, &self.world_bind_group, &[]);

      if let Some(terrain) = &self.terrain {
        pass.set_pipeline(&self.terrain_pipeline);
        pass.set_vertex_buffer(0, terrain.vertex_buffer.slice(..));
        pass.set_index_buffer(terrain.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..terrain.index_count, 0, 0..1);
      }

      if let Some(trees) = &self.trees {
        let stride = std::mem::size_of::<TreeInstance>() as u64;

        if params.tree_style == 2 {
          pass.set_pipeline(&self.tree_mesh_pipeline);
          pass.set_vertex_buffer(0, self.tree_mesh.vertex_buffer.slice(..));
          pass.set_index_buffer(
            self.tree_mesh.index_buffer.slice(..),
            wgpu::IndexFormat::Uint32,
          );

          for slot in 0..SPECIES_COUNT {
            if trees.counts[slot] == 0 {
              continue;
            }

            let start = trees.offsets[slot] as u64 * stride;
            pass.set_vertex_buffer(1, trees.mesh_out.slice(start..));
            pass.draw_indexed_indirect(&trees.args_buffer, (slot * 5 * 4) as u64);
          }
        }

        pass.set_pipeline(&self.tree_impostor_pipeline);

        for slot in 0..SPECIES_COUNT {
          if trees.counts[slot] == 0 {
            continue;
          }

          let start = trees.offsets[slot] as u64 * stride;
          pass.set_vertex_buffer(0, trees.impostor_out.slice(start..));
          pass.draw_indirect(
            &trees.args_buffer,
            ((IMPOSTOR_ARGS_BASE + slot * 4) * 4) as u64,
          );
        }
      }

      if let Some(grass) = &self.grass {
        pass.set_pipeline(&self.grass_pipeline);
        pass.set_vertex_buffer(0, self.grass_base_vertex_buffer.slice(..));
        pass.set_vertex_buffer(1, grass.instance_buffer.slice(..));
        pass.draw(0..18, 0..grass.instance_count);
      }
    }

    // Sky, clouds, fog, and tone mapping onto the canvas.
    {
      let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("VistaWASM composite pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
          view: &view,
          depth_slice: None,
          resolve_target: None,
          ops: wgpu::Operations {
            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
            store: wgpu::StoreOp::Store,
          },
        })],
        depth_stencil_attachment: None,
        ..Default::default()
      });
      pass.set_pipeline(&self.composite_pipeline);
      pass.set_bind_group(0, &self.frame_bind_group, &[]);
      pass.set_bind_group(1, &self.world_bind_group, &[]);
      pass.set_bind_group(2, &self.composite_bind_group, &[]);
      pass.draw(0..3, 0..1);
    }

    // Transparent water on top, depth-tested against the opaque scene.
    if self.water_visible {
      let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("VistaWASM water pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
          view: &view,
          depth_slice: None,
          resolve_target: None,
          ops: wgpu::Operations {
            load: wgpu::LoadOp::Load,
            store: wgpu::StoreOp::Store,
          },
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
          view: &self.depth_view,
          depth_ops: Some(wgpu::Operations {
            load: wgpu::LoadOp::Load,
            store: wgpu::StoreOp::Store,
          }),
          stencil_ops: None,
        }),
        ..Default::default()
      });
      pass.set_pipeline(&self.water_pipeline);
      pass.set_bind_group(0, &self.frame_bind_group, &[]);
      pass.set_bind_group(1, &self.world_bind_group, &[]);

      for mesh in std::iter::once(&self.ocean).chain(self.rivers.iter()) {
        pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
        pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..mesh.index_count, 0, 0..1);
      }
    }

    self.queue.submit(Some(encoder.finish()));
    self.queue.present(surface_texture);
    Ok(())
  }

  /// Resize the WebGPU surface and render targets.
  pub fn resize(&mut self, width: u32, height: u32, device_pixel_ratio: f32) -> VistaResult<()> {
    let pixel_width = scaled_extent(width, device_pixel_ratio);
    let pixel_height = scaled_extent(height, device_pixel_ratio);
    self.config.width = pixel_width;
    self.config.height = pixel_height;
    self.surface.configure(&self.device, &self.config);

    let (depth_view, hdr_view) = create_render_targets(&self.device, pixel_width, pixel_height);
    self.composite_bind_group =
      create_composite_bind_group(&self.device, &self.composite_layout, &hdr_view, &depth_view);
    self.depth_view = depth_view;
    self.hdr_view = hdr_view;
    self.width = pixel_width;
    self.height = pixel_height;
    Ok(())
  }
}

fn scaled_extent(value: u32, device_pixel_ratio: f32) -> u32 {
  ((value as f32 * device_pixel_ratio).round() as u32).max(1)
}
