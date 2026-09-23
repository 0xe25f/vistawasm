use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use vista_types::{
  AtmosphereOptions, CloudsOptions, ErosionOptions, FloraOptions, MistOptions, ShadowOptions,
  SurfaceOptions, TextureTarget, WaterOptions,
};

use crate::errors::{VistaError, VistaResult};
use crate::render::erosion_compute::ErosionCompute;
use crate::render::flora::{FloraInstance, TreeInstance};
use crate::render::grass::GRASS_BASE_TUFT;
use crate::render::shaders;
use crate::render::shadow_math::tree_shadow_frame;
use crate::render::terrain_mesh::TerrainMeshData;
use crate::render::textures::{self, MipGenerator, MipMode, WorldTextures};
use crate::render::tree_models::{
  build_species_mesh, merge_tree_meshes, TreeMesh, TreeSpecies, SPECIES_COUNT,
};
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
  shadow_view_proj: [f32; 16],
  shadow_params: [f32; 4],
  weather: [f32; 4],
  weather2: [f32; 4],
  surface: [f32; 4],
  clouds2: [f32; 4],
  clouds3: [f32; 4],
  clouds4: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<FrameUniforms>() == 608);

/// Static world data: species bounds and tints, terrain mapping, and
/// material tints. Mirrors `WorldInfo` in `common.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct WorldInfo {
  species: [[f32; 4]; 8],
  species_tint: [[f32; 4]; 8],
  terrain: [f32; 4],
  terrain2: [f32; 4],
  material_tints: [[f32; 4]; 8],
}

const _: () = assert!(std::mem::size_of::<WorldInfo>() == 416);

/// Tree culling parameters. Mirrors `CullParams` in `tree_cull.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct CullParams {
  planes: [[f32; 4]; 6],
  camera: [f32; 4],
  params: [f32; 4],
  bounds: [[f32; 4]; 8],
  offsets: [[u32; 4]; 2],
  shadow: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<CullParams>() == 304);

/// Terrain shadow bake parameters. Mirrors `BakeParams` in
/// `terrain_shadow.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct TerrainShadowParams {
  sun: [f32; 4],
  grid: [f32; 4],
}

/// Weather values the shaders need for one frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameWeather {
  /// Rain intensity.
  pub rain: f32,
  /// Snowfall intensity.
  pub snow: f32,
  /// Ground wetness.
  pub wetness: f32,
  /// Settled snow.
  pub snow_cover: f32,
  /// Lightning flash brightness.
  pub lightning: f32,
  /// How grey and flat the sky is.
  pub overcast: f32,
  /// Wind vector (x, z) in metres per second.
  pub wind: [f32; 2],
  /// World position (x, z) of the latest lightning strike.
  pub lightning_position: [f32; 2],
}

/// Everything the renderer needs to shade one frame. The engine resolves
/// weather into these values, so the renderer never needs to know whether
/// a setting came from the host or from the weather system.
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
  /// Shadow controls.
  pub shadows: ShadowOptions,
  /// Terrain surface controls.
  pub surface: SurfaceOptions,
  /// Resolved weather.
  pub weather: FrameWeather,
  /// Lowest and highest terrain heights, for fitting the shadow map.
  pub height_range: (f32, f32),
}

// The engine validates replacement textures against these sizes without
// access to the texture module, so keep them in step.
const _: () = assert!(textures::TERRAIN_TEXTURE_SIZE == crate::engine::TEXTURE_LAYER_SIZE);
const _: () = assert!(textures::FLORA_TEXTURE_SIZE == crate::engine::TEXTURE_LAYER_SIZE);
const _: () = assert!(textures::TERRAIN_LAYERS == crate::engine::TERRAIN_TEXTURE_LAYERS);

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
  shadow_out: wgpu::Buffer,
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

/// Tree shadow map resources.
struct TreeShadowMap {
  view: wgpu::TextureView,
  resolution: u32,
}

/// Reduced-resolution cloud target, plus the bind groups that read the
/// current depth and HDR targets.
struct CloudTarget {
  view: wgpu::TextureView,
  width: u32,
  height: u32,
  cloud_bind_group: wgpu::BindGroup,
  composite_bind_group: wgpu::BindGroup,
}

/// Terrain sun-shadow texture and the inputs it was baked from.
struct TerrainShadow {
  view: wgpu::TextureView,
  texture: wgpu::Texture,
  baked_for: Option<([f32; 3], f32, u64)>,
}

/// Words in the indirect argument buffer: eight indexed mesh draws (5
/// words each), eight impostor draws (4 each), and eight shadow draws (4
/// each).
const INDIRECT_WORDS: usize = SPECIES_COUNT * 5 + SPECIES_COUNT * 4 * 2;
const IMPOSTOR_ARGS_BASE: usize = SPECIES_COUNT * 5;
const SHADOW_ARGS_BASE: usize = IMPOSTOR_ARGS_BASE + SPECIES_COUNT * 4;
const IMPOSTOR_WIDTH: u32 = 256;
const IMPOSTOR_HEIGHT: u32 = 512;
const HEIGHT_TEXTURE_MAX: u32 = 2048;
const TERRAIN_SHADOW_MAX: u32 = 1024;
const OCEAN_GRID_SAMPLES: u32 = 193;
/// Two frames in flight let the CPU record one frame while the GPU draws the
/// previous one, and keep input-to-screen latency to at most two frames.
const MAX_FRAMES_IN_FLIGHT: u32 = 2;
/// How much taller than the ordinary cloud layer storm towers grow, at
/// full `towering`.
const TOWER_STRETCH: f32 = 1.6;
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

/// Shader modules, each compiled once and shared by every pipeline that
/// uses it.
struct Modules {
  terrain: wgpu::ShaderModule,
  trees: wgpu::ShaderModule,
  grass: wgpu::ShaderModule,
  atmosphere: wgpu::ShaderModule,
  water: wgpu::ShaderModule,
  shadow: wgpu::ShaderModule,
}

/// Bind group layouts.
struct Layouts {
  world: wgpu::BindGroupLayout,
  shadow: wgpu::BindGroupLayout,
  composite: wgpu::BindGroupLayout,
  cloud: wgpu::BindGroupLayout,
  terrain_shadow: wgpu::BindGroupLayout,
}

/// Render and compute pipelines.
struct Pipelines {
  terrain: wgpu::RenderPipeline,
  tree_mesh: wgpu::RenderPipeline,
  tree_impostor: wgpu::RenderPipeline,
  tree_bake: wgpu::RenderPipeline,
  tree_shadow: wgpu::RenderPipeline,
  grass: wgpu::RenderPipeline,
  clouds: wgpu::RenderPipeline,
  composite: wgpu::RenderPipeline,
  water: wgpu::RenderPipeline,
  cull: wgpu::ComputePipeline,
  terrain_shadow: wgpu::ComputePipeline,
}

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
  layouts: Layouts,
  pipelines: Pipelines,
  world_buffer: wgpu::Buffer,
  world_info: WorldInfo,
  world_bind_group: wgpu::BindGroup,
  shadow_bind_group: wgpu::BindGroup,
  sampler: wgpu::Sampler,
  clamp_sampler: wgpu::Sampler,
  shadow_sampler: wgpu::Sampler,
  mips: MipGenerator,
  world_textures: WorldTextures,
  impostor_texture: wgpu::Texture,
  impostor_view: wgpu::TextureView,
  height_view: wgpu::TextureView,
  height_size: (u32, u32),
  height_version: u64,
  terrain_shadow: TerrainShadow,
  tree_shadow_map: TreeShadowMap,
  cloud_target: Option<CloudTarget>,
  tree_meshes: Vec<TreeMesh>,
  tree_mesh: IndexedMesh,
  tree_ranges: [(u32, u32, i32); SPECIES_COUNT],
  tree_bounds: [(f32, f32); SPECIES_COUNT],
  grass_base_vertex_buffer: wgpu::Buffer,
  ocean: IndexedMesh,
  rivers: Option<IndexedMesh>,
  water_visible: bool,
  uniforms: FrameUniforms,
  start_time_ms: f64,
  last_time: f32,
  // Wind-driven offsets are integrated over time rather than computed as
  // speed x time, so changing the wind (for example when the weather
  // changes) never makes clouds, mist, or currents jump.
  cloud_offset: [f32; 2],
  mist_offset: [f32; 2],
  current_offset: [f32; 2],
  cloud_evolution: f32,
  /// Frames submitted to the GPU and not yet finished. Browsers keep firing
  /// animation frames on schedule even when the GPU falls behind, so without
  /// this limit frames queue up without bound and the picture lags seconds
  /// behind the camera.
  frames_in_flight: Arc<AtomicU32>,
  /// Set when the browser reports the device lost. Work submitted to a lost
  /// device silently does nothing, so rendering stops and reports it.
  device_lost: Arc<AtomicBool>,
  erosion: ErosionCompute,
  terrain: Option<TerrainGpu>,
  trees: Option<TreesGpu>,
  grass: Option<GrassGpu>,
  width: u32,
  height: u32,
}

fn texture_entry(
  binding: u32,
  dimension: wgpu::TextureViewDimension,
  sample_type: wgpu::TextureSampleType,
  visibility: wgpu::ShaderStages,
) -> wgpu::BindGroupLayoutEntry {
  wgpu::BindGroupLayoutEntry {
    binding,
    visibility,
    ty: wgpu::BindingType::Texture {
      sample_type,
      view_dimension: dimension,
      multisampled: false,
    },
    count: None,
  }
}

fn sampler_entry(binding: u32, kind: wgpu::SamplerBindingType) -> wgpu::BindGroupLayoutEntry {
  wgpu::BindGroupLayoutEntry {
    binding,
    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
    ty: wgpu::BindingType::Sampler(kind),
    count: None,
  }
}

fn uniform_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
  wgpu::BindGroupLayoutEntry {
    binding,
    visibility,
    ty: wgpu::BindingType::Buffer {
      ty: wgpu::BufferBindingType::Uniform,
      has_dynamic_offset: false,
      min_binding_size: None,
    },
    count: None,
  }
}

fn create_layouts(device: &wgpu::Device) -> Layouts {
  use wgpu::TextureSampleType as Sample;
  use wgpu::TextureViewDimension as Dim;
  let both = wgpu::ShaderStages::VERTEX_FRAGMENT;
  let fragment = wgpu::ShaderStages::FRAGMENT;
  let filterable = Sample::Float { filterable: true };
  let unfilterable = Sample::Float { filterable: false };
  let layout = |label: &str, entries: &[wgpu::BindGroupLayoutEntry]| {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
      label: Some(label),
      entries,
    })
  };

  Layouts {
    world: layout(
      "VistaWASM world layout",
      &[
        sampler_entry(0, wgpu::SamplerBindingType::Filtering),
        texture_entry(1, Dim::D2Array, filterable, both),
        texture_entry(2, Dim::D2Array, filterable, both),
        texture_entry(3, Dim::D2Array, filterable, both),
        texture_entry(4, Dim::D2Array, filterable, both),
        texture_entry(5, Dim::D2, filterable, both),
        texture_entry(6, Dim::D3, filterable, both),
        texture_entry(7, Dim::D2, filterable, both),
        texture_entry(8, Dim::D2, unfilterable, both),
        uniform_entry(9, both),
        texture_entry(10, Dim::D2, filterable, both),
        sampler_entry(11, wgpu::SamplerBindingType::Filtering),
      ],
    ),
    shadow: layout(
      "VistaWASM shadow receiver layout",
      &[
        texture_entry(0, Dim::D2, Sample::Depth, both),
        sampler_entry(1, wgpu::SamplerBindingType::Comparison),
      ],
    ),
    composite: layout(
      "VistaWASM composite layout",
      &[
        texture_entry(0, Dim::D2, unfilterable, fragment),
        texture_entry(1, Dim::D2, Sample::Depth, fragment),
        texture_entry(2, Dim::D2, filterable, fragment),
      ],
    ),
    cloud: layout(
      "VistaWASM cloud layout",
      &[texture_entry(1, Dim::D2, Sample::Depth, fragment)],
    ),
    // Explicit, because R32Float heights are not filterable and automatic
    // layouts cannot express that.
    terrain_shadow: layout(
      "VistaWASM terrain shadow bake layout",
      &[
        texture_entry(0, Dim::D2, unfilterable, wgpu::ShaderStages::COMPUTE),
        wgpu::BindGroupLayoutEntry {
          binding: 1,
          visibility: wgpu::ShaderStages::COMPUTE,
          ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: wgpu::TextureFormat::Rgba8Unorm,
            view_dimension: Dim::D2,
          },
          count: None,
        },
        uniform_entry(2, wgpu::ShaderStages::COMPUTE),
      ],
    ),
  }
}

fn view_entry(binding: u32, view: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
  wgpu::BindGroupEntry {
    binding,
    resource: wgpu::BindingResource::TextureView(view),
  }
}

fn sampler_binding(binding: u32, sampler: &wgpu::Sampler) -> wgpu::BindGroupEntry<'_> {
  wgpu::BindGroupEntry {
    binding,
    resource: wgpu::BindingResource::Sampler(sampler),
  }
}

#[allow(clippy::too_many_arguments)]
fn create_world_bind_group(
  device: &wgpu::Device,
  layout: &wgpu::BindGroupLayout,
  sampler: &wgpu::Sampler,
  clamp_sampler: &wgpu::Sampler,
  textures: &WorldTextures,
  impostors: &wgpu::TextureView,
  height: &wgpu::TextureView,
  terrain_shadow: &wgpu::TextureView,
  world_buffer: &wgpu::Buffer,
) -> wgpu::BindGroup {
  device.create_bind_group(&wgpu::BindGroupDescriptor {
    label: Some("VistaWASM world bind group"),
    layout,
    entries: &[
      sampler_binding(0, sampler),
      view_entry(1, &textures.terrain_albedo),
      view_entry(2, &textures.terrain_normal),
      view_entry(3, &textures.flora),
      view_entry(4, impostors),
      view_entry(5, &textures.noise),
      view_entry(6, &textures.cloud),
      view_entry(7, &textures.water),
      view_entry(8, height),
      wgpu::BindGroupEntry {
        binding: 9,
        resource: world_buffer.as_entire_binding(),
      },
      view_entry(10, terrain_shadow),
      sampler_binding(11, clamp_sampler),
    ],
  })
}

fn write_layer(
  queue: &wgpu::Queue,
  texture: &wgpu::Texture,
  layer: u32,
  data: &[u8],
  bytes_per_row: u32,
  width: u32,
  height: u32,
) {
  queue.write_texture(
    wgpu::TexelCopyTextureInfo {
      texture,
      mip_level: 0,
      origin: wgpu::Origin3d {
        x: 0,
        y: 0,
        z: layer,
      },
      aspect: wgpu::TextureAspect::All,
    },
    data,
    wgpu::TexelCopyBufferLayout {
      offset: 0,
      bytes_per_row: Some(bytes_per_row),
      rows_per_image: Some(height),
    },
    wgpu::Extent3d {
      width,
      height,
      depth_or_array_layers: 1,
    },
  );
}

fn create_texture_2d(
  device: &wgpu::Device,
  label: &str,
  width: u32,
  height: u32,
  format: wgpu::TextureFormat,
  usage: wgpu::TextureUsages,
) -> wgpu::Texture {
  device.create_texture(&wgpu::TextureDescriptor {
    label: Some(label),
    size: wgpu::Extent3d {
      width,
      height,
      depth_or_array_layers: 1,
    },
    mip_level_count: 1,
    sample_count: 1,
    dimension: wgpu::TextureDimension::D2,
    format,
    usage,
    view_formats: &[],
  })
}

fn default_view(texture: &wgpu::Texture) -> wgpu::TextureView {
  texture.create_view(&wgpu::TextureViewDescriptor::default())
}

fn create_height_texture(
  device: &wgpu::Device,
  queue: &wgpu::Queue,
  width: u32,
  height: u32,
  data: &[f32],
) -> wgpu::TextureView {
  let texture = create_texture_2d(
    device,
    "VistaWASM terrain heights",
    width,
    height,
    wgpu::TextureFormat::R32Float,
    wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
  );
  write_layer(
    queue,
    &texture,
    0,
    bytemuck::cast_slice(data),
    width * 4,
    width,
    height,
  );
  default_view(&texture)
}

fn create_terrain_shadow(
  device: &wgpu::Device,
  queue: &wgpu::Queue,
  width: u32,
  height: u32,
) -> TerrainShadow {
  let texture = create_texture_2d(
    device,
    "VistaWASM terrain shadow",
    width,
    height,
    wgpu::TextureFormat::Rgba8Unorm,
    wgpu::TextureUsages::TEXTURE_BINDING
      | wgpu::TextureUsages::STORAGE_BINDING
      | wgpu::TextureUsages::COPY_DST,
  );
  // Fully lit until the first bake.
  let white = vec![255u8; (width * height * 4) as usize];
  write_layer(queue, &texture, 0, &white, width * 4, width, height);

  TerrainShadow {
    view: default_view(&texture),
    texture,
    baked_for: None,
  }
}

fn create_tree_shadow_map(device: &wgpu::Device, resolution: u32) -> TreeShadowMap {
  let texture = create_texture_2d(
    device,
    "VistaWASM tree shadow map",
    resolution,
    resolution,
    DEPTH_FORMAT,
    wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
  );

  TreeShadowMap {
    view: default_view(&texture),
    resolution,
  }
}

fn create_shadow_bind_group(
  device: &wgpu::Device,
  layout: &wgpu::BindGroupLayout,
  map: &TreeShadowMap,
  sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
  device.create_bind_group(&wgpu::BindGroupDescriptor {
    label: Some("VistaWASM shadow receiver bind group"),
    layout,
    entries: &[view_entry(0, &map.view), sampler_binding(1, sampler)],
  })
}

fn create_render_targets(
  device: &wgpu::Device,
  width: u32,
  height: u32,
) -> (wgpu::TextureView, wgpu::TextureView) {
  let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
  let (width, height) = (width.max(1), height.max(1));

  (
    default_view(&create_texture_2d(
      device,
      "VistaWASM depth buffer",
      width,
      height,
      DEPTH_FORMAT,
      usage,
    )),
    default_view(&create_texture_2d(
      device,
      "VistaWASM HDR scene",
      width,
      height,
      HDR_FORMAT,
      usage,
    )),
  )
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

fn indexed_mesh(
  device: &wgpu::Device,
  queue: &wgpu::Queue,
  label: &str,
  vertices: &[u8],
  indices: &[u32],
) -> IndexedMesh {
  IndexedMesh {
    vertex_buffer: buffer_with_data(device, queue, label, vertices, wgpu::BufferUsages::VERTEX),
    index_buffer: buffer_with_data(
      device,
      queue,
      label,
      bytemuck::cast_slice(indices),
      wgpu::BufferUsages::INDEX,
    ),
    index_count: indices.len() as u32,
  }
}

fn render_module(device: &wgpu::Device, label: &str, body: &str) -> wgpu::ShaderModule {
  device.create_shader_module(wgpu::ShaderModuleDescriptor {
    label: Some(label),
    source: wgpu::ShaderSource::Wgsl(shaders::render_source(body).into()),
  })
}

fn compute_pipeline(
  device: &wgpu::Device,
  label: &str,
  source: &'static str,
  entry: &str,
  layout: Option<&wgpu::PipelineLayout>,
) -> wgpu::ComputePipeline {
  let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
    label: Some(label),
    source: wgpu::ShaderSource::Wgsl(source.into()),
  });

  device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
    label: Some(label),
    layout,
    module: &module,
    entry_point: Some(entry),
    compilation_options: wgpu::PipelineCompilationOptions::default(),
    cache: None,
  })
}

struct PipelineSpec<'a> {
  label: &'a str,
  module: &'a wgpu::ShaderModule,
  vertex_entry: &'a str,
  fragment_entry: &'a str,
  buffers: &'a [Option<wgpu::VertexBufferLayout<'a>>],
  // `None` for depth-only passes.
  format: Option<wgpu::TextureFormat>,
  blend: Option<wgpu::BlendState>,
  depth: Option<(bool, wgpu::CompareFunction)>,
  cull_mode: Option<wgpu::Face>,
  depth_bias: wgpu::DepthBiasState,
}

impl<'a> PipelineSpec<'a> {
  /// An opaque, depth-tested, unculled pipeline writing the HDR target.
  fn opaque(
    label: &'a str,
    module: &'a wgpu::ShaderModule,
    entries: (&'a str, &'a str),
    buffers: &'a [Option<wgpu::VertexBufferLayout<'a>>],
  ) -> Self {
    Self {
      label,
      module,
      vertex_entry: entries.0,
      fragment_entry: entries.1,
      buffers,
      format: Some(HDR_FORMAT),
      blend: None,
      depth: Some((true, wgpu::CompareFunction::Less)),
      cull_mode: None,
      depth_bias: wgpu::DepthBiasState::default(),
    }
  }
}

fn create_pipeline(
  device: &wgpu::Device,
  layout: &wgpu::PipelineLayout,
  spec: PipelineSpec<'_>,
) -> wgpu::RenderPipeline {
  let targets = [spec.format.map(|format| wgpu::ColorTargetState {
    format,
    blend: spec.blend,
    write_mask: wgpu::ColorWrites::ALL,
  })];

  device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
    label: Some(spec.label),
    layout: Some(layout),
    vertex: wgpu::VertexState {
      module: spec.module,
      entry_point: Some(spec.vertex_entry),
      compilation_options: wgpu::PipelineCompilationOptions::default(),
      buffers: spec.buffers,
    },
    fragment: Some(wgpu::FragmentState {
      module: spec.module,
      entry_point: Some(spec.fragment_entry),
      compilation_options: wgpu::PipelineCompilationOptions::default(),
      targets: if spec.format.is_some() { &targets } else { &[] },
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
      bias: spec.depth_bias,
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

fn flag(on: bool) -> f32 {
  if on {
    1.0
  } else {
    0.0
  }
}

fn create_pipelines(
  device: &wgpu::Device,
  frame_layout: &wgpu::BindGroupLayout,
  layouts: &Layouts,
  modules: &Modules,
  surface_format: wgpu::TextureFormat,
) -> Pipelines {
  let pipeline_layout = |label: &str, groups: &[&wgpu::BindGroupLayout]| {
    let groups: Vec<Option<&wgpu::BindGroupLayout>> =
      groups.iter().map(|group| Some(*group)).collect();
    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
      label: Some(label),
      bind_group_layouts: &groups,
      immediate_size: 0,
    })
  };
  let receivers = pipeline_layout(
    "VistaWASM shadow receiver pipeline layout",
    &[frame_layout, &layouts.world, &layouts.shadow],
  );
  // The shadow pass cannot bind the shadow map it renders into, and the
  // impostor bake needs no shadows, so both use the basic layout.
  let basic = pipeline_layout(
    "VistaWASM basic pipeline layout",
    &[frame_layout, &layouts.world],
  );
  let composite = pipeline_layout(
    "VistaWASM composite pipeline layout",
    &[
      frame_layout,
      &layouts.world,
      &layouts.shadow,
      &layouts.composite,
    ],
  );
  let cloud = pipeline_layout(
    "VistaWASM cloud pipeline layout",
    &[
      frame_layout,
      &layouts.world,
      &layouts.shadow,
      &layouts.cloud,
    ],
  );
  let terrain_shadow_layout = pipeline_layout(
    "VistaWASM terrain shadow pipeline layout",
    &[&layouts.terrain_shadow],
  );
  let tree_buffers = [Some(tree_vertex_layout()), Some(tree_instance_layout())];
  let instance_buffers = [Some(tree_instance_layout())];
  let bake_buffers = [Some(tree_vertex_layout())];
  let terrain_buffers = [Some(wgpu::VertexBufferLayout {
    array_stride: std::mem::size_of::<crate::render::terrain_mesh::TerrainVertex>() as u64,
    step_mode: wgpu::VertexStepMode::Vertex,
    attributes: &TERRAIN_ATTRIBUTES,
  })];
  let grass_buffers = [
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
  ];
  let water_buffers = [Some(wgpu::VertexBufferLayout {
    array_stride: std::mem::size_of::<WaterVertex>() as u64,
    step_mode: wgpu::VertexStepMode::Vertex,
    attributes: &WATER_ATTRIBUTES,
  })];
  let main = ("vertex_main", "fragment_main");

  Pipelines {
    terrain: create_pipeline(
      device,
      &receivers,
      PipelineSpec {
        cull_mode: Some(wgpu::Face::Back),
        ..PipelineSpec::opaque(
          "VistaWASM terrain",
          &modules.terrain,
          main,
          &terrain_buffers,
        )
      },
    ),
    tree_mesh: create_pipeline(
      device,
      &receivers,
      PipelineSpec::opaque(
        "VistaWASM tree meshes",
        &modules.trees,
        ("vertex_mesh", "fragment_mesh"),
        &tree_buffers,
      ),
    ),
    tree_impostor: create_pipeline(
      device,
      &receivers,
      PipelineSpec::opaque(
        "VistaWASM tree impostors",
        &modules.trees,
        ("vertex_impostor", "fragment_impostor"),
        &instance_buffers,
      ),
    ),
    tree_bake: create_pipeline(
      device,
      &basic,
      PipelineSpec {
        format: Some(wgpu::TextureFormat::Rgba8Unorm),
        ..PipelineSpec::opaque(
          "VistaWASM impostor bake",
          &modules.trees,
          ("vertex_bake", "fragment_bake"),
          &bake_buffers,
        )
      },
    ),
    tree_shadow: create_pipeline(
      device,
      &basic,
      PipelineSpec {
        format: None,
        depth_bias: wgpu::DepthBiasState {
          constant: 2,
          slope_scale: 2.5,
          clamp: 0.0,
        },
        ..PipelineSpec::opaque(
          "VistaWASM tree shadows",
          &modules.shadow,
          main,
          &instance_buffers,
        )
      },
    ),
    grass: create_pipeline(
      device,
      &receivers,
      PipelineSpec::opaque("VistaWASM grass", &modules.grass, main, &grass_buffers),
    ),
    clouds: create_pipeline(
      device,
      &cloud,
      PipelineSpec {
        depth: None,
        ..PipelineSpec::opaque(
          "VistaWASM clouds",
          &modules.atmosphere,
          ("vertex_main", "cloud_main"),
          &[],
        )
      },
    ),
    composite: create_pipeline(
      device,
      &composite,
      PipelineSpec {
        format: Some(surface_format),
        depth: None,
        ..PipelineSpec::opaque("VistaWASM composite", &modules.atmosphere, main, &[])
      },
    ),
    water: create_pipeline(
      device,
      &receivers,
      PipelineSpec {
        format: Some(surface_format),
        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
        depth: Some((false, wgpu::CompareFunction::Less)),
        ..PipelineSpec::opaque("VistaWASM water", &modules.water, main, &water_buffers)
      },
    ),
    cull: compute_pipeline(
      device,
      "VistaWASM tree cull",
      shaders::TREE_CULL,
      "cull_main",
      None,
    ),
    terrain_shadow: compute_pipeline(
      device,
      "VistaWASM terrain shadow bake",
      shaders::TERRAIN_SHADOW,
      "bake",
      Some(&terrain_shadow_layout),
    ),
  }
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
    tree_shadow_resolution: u32,
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
    let device_lost = Arc::new(AtomicBool::new(false));
    let lost_flag = Arc::clone(&device_lost);
    device.set_device_lost_callback(move |_reason, _message| {
      lost_flag.store(true, Ordering::Release);
    });
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
      entries: &[uniform_entry(0, wgpu::ShaderStages::VERTEX_FRAGMENT)],
    });
    let frame_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
      label: Some("VistaWASM frame bind group"),
      layout: &frame_layout,
      entries: &[wgpu::BindGroupEntry {
        binding: 0,
        resource: uniform_buffer.as_entire_binding(),
      }],
    });
    let layouts = create_layouts(&device);
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
    let clamp_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
      label: Some("VistaWASM linear clamp sampler"),
      mag_filter: wgpu::FilterMode::Linear,
      min_filter: wgpu::FilterMode::Linear,
      ..Default::default()
    });
    let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
      label: Some("VistaWASM shadow comparison sampler"),
      mag_filter: wgpu::FilterMode::Linear,
      min_filter: wgpu::FilterMode::Linear,
      compare: Some(wgpu::CompareFunction::LessEqual),
      ..Default::default()
    });

    // Each module is compiled once; the tree module serves the bake, mesh,
    // and impostor pipelines, and the atmosphere module serves both the
    // cloud and composite passes.
    let modules = Modules {
      terrain: render_module(&device, "VistaWASM terrain shader", shaders::TERRAIN),
      trees: render_module(&device, "VistaWASM tree shader", shaders::TREES),
      grass: render_module(&device, "VistaWASM grass shader", shaders::GRASS),
      atmosphere: render_module(&device, "VistaWASM atmosphere shader", shaders::ATMOSPHERE),
      water: render_module(&device, "VistaWASM water shader", shaders::WATER),
      shadow: render_module(&device, "VistaWASM shadow shader", shaders::SHADOW),
    };
    let pipelines = create_pipelines(&device, &frame_layout, &layouts, &modules, config.format);
    let mips = MipGenerator::new(&device);
    let world_textures = textures::bake_world_textures(&device, &queue, &mips);
    let tree_meshes: Vec<TreeMesh> = TreeSpecies::ALL
      .iter()
      .map(|species| build_species_mesh(*species))
      .collect();
    let library = merge_tree_meshes(&tree_meshes);
    let tree_mesh = indexed_mesh(
      &device,
      &queue,
      "VistaWASM tree meshes",
      bytemuck::cast_slice(&library.vertices),
      &library.indices,
    );
    let mut world_info = WorldInfo::zeroed();

    for (slot, (height, radius)) in library.bounds.iter().enumerate() {
      world_info.species[slot] = [*height, *radius, 0.0, 0.0];
      world_info.species_tint[slot] = SPECIES_TINTS[slot];
      world_info.material_tints[slot] = [1.0, 1.0, 1.0, 0.0];
    }

    let world_buffer = buffer_with_data(
      &device,
      &queue,
      "VistaWASM world info",
      bytemuck::bytes_of(&world_info),
      wgpu::BufferUsages::UNIFORM,
    );
    let height_view = create_height_texture(&device, &queue, 1, 1, &[-100_000.0]);
    let terrain_shadow = create_terrain_shadow(&device, &queue, 1, 1);
    let tree_shadow_map = create_tree_shadow_map(&device, tree_shadow_resolution);
    let shadow_bind_group =
      create_shadow_bind_group(&device, &layouts.shadow, &tree_shadow_map, &shadow_sampler);
    let impostor_texture = textures::create_array_texture(
      &device,
      "VistaWASM tree impostors",
      IMPOSTOR_WIDTH,
      IMPOSTOR_HEIGHT,
      SPECIES_COUNT as u32,
      wgpu::TextureUsages::RENDER_ATTACHMENT,
    );
    let impostor_view = textures::array_view(&impostor_texture);
    let world_bind_group = create_world_bind_group(
      &device,
      &layouts.world,
      &sampler,
      &clamp_sampler,
      &world_textures,
      &impostor_view,
      &height_view,
      &terrain_shadow.view,
      &world_buffer,
    );
    let grass_base_vertex_buffer = buffer_with_data(
      &device,
      &queue,
      "VistaWASM grass base tuft",
      bytemuck::cast_slice(&GRASS_BASE_TUFT),
      wgpu::BufferUsages::VERTEX,
    );
    let (ocean_vertices, ocean_indices) =
      build_ocean_grid(OCEAN_GRID_SAMPLES, OCEAN_FAR_REACH_METRES);
    let ocean = indexed_mesh(
      &device,
      &queue,
      "VistaWASM ocean grid",
      bytemuck::cast_slice(&ocean_vertices),
      &ocean_indices,
    );
    let erosion = ErosionCompute::new(&device);
    let mut uniforms = FrameUniforms::zeroed();
    uniforms.camera_up[3] = flag(!config.format.is_srgb());

    let mut context = Self {
      surface,
      device,
      queue,
      config,
      depth_view,
      hdr_view,
      frame_bind_group,
      uniform_buffer,
      layouts,
      pipelines,
      world_buffer,
      world_info,
      world_bind_group,
      shadow_bind_group,
      sampler,
      clamp_sampler,
      shadow_sampler,
      mips,
      world_textures,
      impostor_texture,
      impostor_view,
      height_view,
      height_size: (1, 1),
      height_version: 0,
      terrain_shadow,
      tree_shadow_map,
      cloud_target: None,
      tree_meshes,
      tree_mesh,
      tree_ranges: library.ranges,
      tree_bounds: library.bounds,
      grass_base_vertex_buffer,
      ocean,
      rivers: None,
      water_visible: false,
      uniforms,
      start_time_ms: now_ms(),
      last_time: 0.0,
      cloud_offset: [0.0; 2],
      mist_offset: [0.0; 2],
      current_offset: [0.0; 2],
      frames_in_flight: Arc::new(AtomicU32::new(0)),
      device_lost,
      cloud_evolution: 0.0,
      erosion,
      terrain: None,
      trees: None,
      grass: None,
      width: pixel_width,
      height: pixel_height,
    };
    context.bake_impostors();
    Ok(context)
  }

  fn rebuild_world_bind_group(&mut self) {
    self.world_bind_group = create_world_bind_group(
      &self.device,
      &self.layouts.world,
      &self.sampler,
      &self.clamp_sampler,
      &self.world_textures,
      &self.impostor_view,
      &self.height_view,
      &self.terrain_shadow.view,
      &self.world_buffer,
    );
  }

  fn write_world_info(&self) {
    self
      .queue
      .write_buffer(&self.world_buffer, 0, bytemuck::bytes_of(&self.world_info));
  }

  /// Render every species into the impostor texture array, then mipmap it.
  fn bake_impostors(&mut self) {
    // The impostor texture is the render target here, so the bake binds a
    // placeholder in its slot.
    let placeholder = textures::create_array_texture(
      &self.device,
      "VistaWASM impostor placeholder",
      1,
      1,
      1,
      wgpu::TextureUsages::empty(),
    );
    let placeholder_view = textures::array_view(&placeholder);
    let bake_bind_group = create_world_bind_group(
      &self.device,
      &self.layouts.world,
      &self.sampler,
      &self.clamp_sampler,
      &self.world_textures,
      &placeholder_view,
      &self.height_view,
      &self.terrain_shadow.view,
      &self.world_buffer,
    );
    let depth_view = default_view(&create_texture_2d(
      &self.device,
      "VistaWASM impostor depth",
      IMPOSTOR_WIDTH,
      IMPOSTOR_HEIGHT,
      DEPTH_FORMAT,
      wgpu::TextureUsages::RENDER_ATTACHMENT,
    ));
    let mut encoder = self
      .device
      .create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("VistaWASM impostor bake"),
      });

    for (slot, (first_index, index_count, base_vertex)) in self.tree_ranges.iter().enumerate() {
      let layer_view = self
        .impostor_texture
        .create_view(&wgpu::TextureViewDescriptor {
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
          view: &depth_view,
          depth_ops: Some(wgpu::Operations {
            load: wgpu::LoadOp::Clear(1.0),
            store: wgpu::StoreOp::Discard,
          }),
          stencil_ops: None,
        }),
        ..Default::default()
      });
      pass.set_pipeline(&self.pipelines.tree_bake);
      pass.set_bind_group(0, &self.frame_bind_group, &[]);
      pass.set_bind_group(1, &bake_bind_group, &[]);
      pass.set_vertex_buffer(0, self.tree_mesh.vertex_buffer.slice(..));
      pass.set_index_buffer(
        self.tree_mesh.index_buffer.slice(..),
        wgpu::IndexFormat::Uint32,
      );
      pass.draw_indexed(
        *first_index..*first_index + *index_count,
        *base_vertex,
        slot as u32..slot as u32 + 1,
      );
    }

    self.mips.generate(
      &self.device,
      &mut encoder,
      &self.impostor_texture,
      MipMode::Coverage,
    );
    self.queue.submit(Some(encoder.finish()));
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

  /// Replace one species' model (`None` restores the procedural model),
  /// then rebuild the merged tree buffers and re-bake the impostors, so
  /// meshes, impostors, and shadows all use the new model.
  pub fn set_tree_model(&mut self, species: usize, mesh: Option<TreeMesh>) {
    if species >= SPECIES_COUNT {
      return;
    }

    self.tree_meshes[species] =
      mesh.unwrap_or_else(|| build_species_mesh(TreeSpecies::ALL[species]));
    let library = merge_tree_meshes(&self.tree_meshes);
    self.tree_mesh = indexed_mesh(
      &self.device,
      &self.queue,
      "VistaWASM tree meshes",
      bytemuck::cast_slice(&library.vertices),
      &library.indices,
    );
    self.tree_ranges = library.ranges;
    self.tree_bounds = library.bounds;

    for (slot, (height, radius)) in library.bounds.iter().enumerate() {
      self.world_info.species[slot] = [*height, *radius, 0.0, 0.0];
    }

    self.write_world_info();
    self.bake_impostors();
  }

  /// Replace one layer of a baked texture array with host-supplied RGBA8
  /// texels (`size() x size()`, already validated by the engine), then
  /// rebuild its mips. Flora changes also re-bake the impostors.
  pub fn replace_texture_layer(&mut self, target: TextureTarget, layer: u32, rgba: &[u8]) {
    let size = crate::engine::TEXTURE_LAYER_SIZE;
    let (texture, mode) = match target {
      TextureTarget::TerrainAlbedo => {
        (&self.world_textures.terrain_albedo_texture, MipMode::Colour)
      }
      TextureTarget::TerrainNormal => {
        (&self.world_textures.terrain_normal_texture, MipMode::Linear)
      }
      TextureTarget::Flora => (&self.world_textures.flora_texture, MipMode::Coverage),
    };
    write_layer(&self.queue, texture, layer, rgba, size * 4, size, size);
    let mut encoder = self
      .device
      .create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("VistaWASM texture replacement"),
      });
    self
      .mips
      .generate(&self.device, &mut encoder, texture, mode);
    self.queue.submit(Some(encoder.finish()));

    if target == TextureTarget::Flora {
      self.bake_impostors();
    }
  }

  /// Regenerate every procedural texture, discarding replaced layers, and
  /// re-bake the impostors.
  pub fn reset_textures(&mut self) {
    self.world_textures = textures::bake_world_textures(&self.device, &self.queue, &self.mips);
    self.rebuild_world_bind_group();
    self.bake_impostors();
  }

  /// Upload a CPU-baked terrain mesh, replacing any previous terrain buffers.
  pub fn upload_terrain(&mut self, mesh: &TerrainMeshData) {
    if mesh.vertices.is_empty() || mesh.indices.is_empty() {
      self.terrain = None;
      return;
    }

    let mesh = indexed_mesh(
      &self.device,
      &self.queue,
      "VistaWASM terrain",
      bytemuck::cast_slice(&mesh.vertices),
      &mesh.indices,
    );
    self.terrain = Some(TerrainGpu {
      vertex_buffer: mesh.vertex_buffer,
      index_buffer: mesh.index_buffer,
      index_count: mesh.index_count,
    });
  }

  /// Upload the terrain heights used for water depth, shorelines, terrain
  /// shadows, and the sea bed beyond the terrain. Large terrain is
  /// downsampled.
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
    self.height_size = (texture_width, texture_height);
    self.height_version = self.height_version.wrapping_add(1);

    // The shadow texture keeps the height texture's aspect ratio, capped
    // at TERRAIN_SHADOW_MAX texels on the long side.
    let longest = texture_width.max(texture_height);
    let shadow_scale = (TERRAIN_SHADOW_MAX as f32 / longest as f32).min(1.0);
    let shadow_width = ((texture_width as f32 * shadow_scale).round() as u32).max(1);
    let shadow_height = ((texture_height as f32 * shadow_scale).round() as u32).max(1);
    self.terrain_shadow =
      create_terrain_shadow(&self.device, &self.queue, shadow_width, shadow_height);
    self.world_info.terrain = [
      (width as f32 - 1.0) * metres_per_sample * 0.5,
      (height as f32 - 1.0) * metres_per_sample * 0.5,
      metres_per_sample * stride as f32,
      metres_per_sample * stride as f32,
    ];
    self.world_info.terrain2 = [texture_width as f32, texture_height as f32, 1.0, 0.0];
    self.write_world_info();
    self.rebuild_world_bind_group();
  }

  /// Upload tree instances, replacing any previous trees. Instances are
  /// culled and sorted into per-species level-of-detail and shadow lists on
  /// the GPU every frame.
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
    let shadow_out = output("VistaWASM shadow-casting trees");
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
    let buffers = [
      &cull_params_buffer,
      &instance_buffer,
      &mesh_out,
      &impostor_out,
      &args_buffer,
      &shadow_out,
    ];
    let entries: Vec<wgpu::BindGroupEntry> = buffers
      .iter()
      .enumerate()
      .map(|(binding, buffer)| wgpu::BindGroupEntry {
        binding: binding as u32,
        resource: buffer.as_entire_binding(),
      })
      .collect();
    let cull_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
      label: Some("VistaWASM tree cull bind group"),
      layout: &self.pipelines.cull.get_bind_group_layout(0),
      entries: &entries,
    });

    self.trees = Some(TreesGpu {
      mesh_out,
      impostor_out,
      shadow_out,
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

    self.rivers = Some(indexed_mesh(
      &self.device,
      &self.queue,
      "VistaWASM rivers",
      bytemuck::cast_slice(vertices),
      indices,
    ));
  }

  /// Show or hide all water.
  pub fn set_water_visible(&mut self, visible: bool) {
    self.water_visible = visible;
  }

  /// Update the terrain material colour multipliers.
  pub fn set_material_tints(&mut self, tints: &[[f32; 3]; 8]) {
    for (slot, tint) in tints.iter().enumerate() {
      self.world_info.material_tints[slot] = [tint[0], tint[1], tint[2], 0.0];
    }

    self.write_world_info();
  }

  fn update_uniforms(&mut self, params: &FrameParams, time: f32, dt: f32) {
    // Integrate wind-driven motion. The wind carries clouds and currents
    // downwind, so their noise lookups move upwind.
    let clouds = &params.clouds;
    let cloud_wind = direction_from_degrees(clouds.wind_direction_degrees);
    let cloud_speed = clouds.speed.max(0.0) * 15.0 * dt;
    self.cloud_offset[0] -= cloud_wind[0] * cloud_speed;
    self.cloud_offset[1] -= cloud_wind[1] * cloud_speed;
    self.cloud_evolution += clouds.evolution.clamp(0.0, 1.0) * 14.0 * dt;
    let mist = &params.mist;
    let mist_wind = direction_from_degrees(mist.wind_direction_degrees);
    let mist_speed = mist.wind_speed_metres_per_second.max(0.0) * dt;
    self.mist_offset[0] += mist_wind[0] * mist_speed;
    self.mist_offset[1] += mist_wind[1] * mist_speed;
    let water = &params.water;
    let current = direction_from_degrees(water.current_direction_degrees);
    let current_speed = water.current_speed.max(0.0) * dt;
    self.current_offset[0] -= current[0] * current_speed;
    self.current_offset[1] -= current[1] * current_speed;

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
      self.mist_offset[0],
      self.mist_offset[1],
      mist.sun_scattering.clamp(0.0, 1.0),
      ((mist.seed_offset % 997) as f32 * 0.618_034).fract(),
    ];

    let seed = (clouds.seed_offset % 10_007) as f32;
    u.clouds2 = [
      if params.cloud_coverage > 0.0 {
        clouds.cirrus.clamp(0.0, 1.0)
      } else {
        0.0
      },
      clouds
        .cirrus_height_metres
        .max(clouds.height_metres + clouds.thickness_metres),
      cloud_wind[0],
      cloud_wind[1],
    ];
    // Storm towers rise well above the ordinary cloud layer, so the slab is
    // stretched to hold them; the shader keeps ordinary clouds at their
    // own height inside it.
    let towering = clouds.towering.clamp(0.0, 1.0);
    u.cloud_params = [
      params.cloud_coverage.clamp(0.0, 1.0),
      clouds.height_metres,
      clouds.thickness_metres.max(1.0) * (1.0 + towering * TOWER_STRETCH),
      params.cloud_raymarch_steps as f32,
    ];
    u.clouds3 = [
      clouds.stratiform.clamp(0.0, 1.0),
      towering,
      clouds.base_darkness.clamp(0.0, 1.0),
      clouds.ragged_base.clamp(0.0, 1.0),
    ];
    u.clouds4 = [
      if params.cloud_coverage > 0.0 {
        clouds.rain_shafts.clamp(0.0, 1.0)
      } else {
        0.0
      },
      params.weather.lightning_position[0],
      params.weather.lightning_position[1],
      1.0 + towering * TOWER_STRETCH,
    ];
    u.cloud_motion = [
      self.cloud_offset[0] + seed * 173.0,
      self.cloud_offset[1] + seed * 311.0,
      self.cloud_evolution,
      clouds.density.clamp(0.0, 1.0),
    ];
    u.cloud_colour = [
      clouds.colour[0],
      clouds.colour[1],
      clouds.colour[2],
      flag(clouds.cast_shadows && params.shadows.clouds.enabled),
    ];
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
      self.current_offset[0],
      self.current_offset[1],
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
      flag(waves.enabled),
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

    let shadows = &params.shadows;
    let tree_shadows =
      shadows.trees.enabled && self.trees.is_some() && params.sun_direction[1] > 0.0;
    u.shadow_params = [
      if tree_shadows {
        shadows.trees.strength.clamp(0.0, 1.0)
      } else {
        0.0
      },
      0.5 + shadows.trees.softness.clamp(0.0, 1.0) * 2.5,
      if shadows.terrain.enabled {
        shadows.terrain.strength.clamp(0.0, 1.0)
      } else {
        0.0
      },
      shadows.clouds.strength.clamp(0.0, 1.0),
    ];
    let weather = &params.weather;
    u.weather = [
      weather.rain,
      weather.snow,
      weather.wetness,
      weather.snow_cover,
    ];
    u.weather2 = [
      weather.lightning,
      weather.overcast,
      weather.wind[0],
      weather.wind[1],
    ];
    let surface = &params.surface;
    u.surface = [
      flag(surface.textures),
      flag(surface.detail_normals),
      surface.texture_scale.clamp(0.05, 20.0),
      0.0,
    ];
  }

  /// Re-bake terrain self-shadowing when the sun, softness, or terrain has
  /// changed since the last bake. A slowly moving sun only re-bakes every
  /// few tenths of a degree.
  fn bake_terrain_shadow_if_needed(
    &mut self,
    params: &FrameParams,
    encoder: &mut wgpu::CommandEncoder,
  ) {
    if !params.shadows.terrain.enabled || self.world_info.terrain2[2] < 0.5 {
      return;
    }

    let sun = params.sun_direction;
    let softness = params.shadows.terrain.softness.clamp(0.0, 1.0);

    if let Some((baked_sun, baked_softness, version)) = self.terrain_shadow.baked_for {
      let moved = (0..3).any(|i| (baked_sun[i] - sun[i]).abs() > 0.005);

      if !moved && (baked_softness - softness).abs() < 0.001 && version == self.height_version {
        return;
      }
    }

    let out_width = self.terrain_shadow.texture.width();
    let out_height = self.terrain_shadow.texture.height();
    let bake_params = TerrainShadowParams {
      sun: [sun[0], sun[1], sun[2], softness],
      grid: [
        self.world_info.terrain[2],
        self.height_size.0 as f32 / out_width.max(1) as f32,
        self.height_size.0 as f32,
        self.height_size.1 as f32,
      ],
    };
    let params_buffer = buffer_with_data(
      &self.device,
      &self.queue,
      "VistaWASM terrain shadow parameters",
      bytemuck::bytes_of(&bake_params),
      wgpu::BufferUsages::UNIFORM,
    );
    let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
      label: Some("VistaWASM terrain shadow bind group"),
      layout: &self.layouts.terrain_shadow,
      entries: &[
        view_entry(0, &self.height_view),
        view_entry(1, &self.terrain_shadow.view),
        wgpu::BindGroupEntry {
          binding: 2,
          resource: params_buffer.as_entire_binding(),
        },
      ],
    });
    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
      label: Some("VistaWASM terrain shadow pass"),
      timestamp_writes: None,
    });
    pass.set_pipeline(&self.pipelines.terrain_shadow);
    pass.set_bind_group(0, &bind_group, &[]);
    pass.dispatch_workgroups(out_width.div_ceil(8), out_height.div_ceil(8), 1);
    self.terrain_shadow.baked_for = Some((sun, softness, self.height_version));
  }

  /// Make sure the reduced-resolution cloud target matches the canvas and
  /// the requested scale.
  fn ensure_cloud_target(&mut self, scale: f32) {
    let scale = scale.clamp(0.25, 1.0);
    let width = ((self.width as f32 * scale).round() as u32).max(1);
    let height = ((self.height as f32 * scale).round() as u32).max(1);

    if self
      .cloud_target
      .as_ref()
      .is_some_and(|target| target.width == width && target.height == height)
    {
      return;
    }

    let view = default_view(&create_texture_2d(
      &self.device,
      "VistaWASM cloud target",
      width,
      height,
      HDR_FORMAT,
      wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
    ));
    let cloud_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
      label: Some("VistaWASM cloud bind group"),
      layout: &self.layouts.cloud,
      entries: &[view_entry(1, &self.depth_view)],
    });
    let composite_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
      label: Some("VistaWASM composite bind group"),
      layout: &self.layouts.composite,
      entries: &[
        view_entry(0, &self.hdr_view),
        view_entry(1, &self.depth_view),
        view_entry(2, &view),
      ],
    });

    self.cloud_target = Some(CloudTarget {
      view,
      width,
      height,
      cloud_bind_group,
      composite_bind_group,
    });
  }

  /// Render one frame.
  pub fn render_once(&mut self, params: &FrameParams) -> VistaResult<()> {
    if self.device_lost.load(Ordering::Acquire) {
      return Err(VistaError::WebGpuDeviceLost);
    }

    let time = ((now_ms() - self.start_time_ms) / 1000.0) as f32;
    let dt = (time - self.last_time).clamp(0.0, 0.25);
    self.last_time = time;

    if params.shadows.trees.resolution != self.tree_shadow_map.resolution {
      self.tree_shadow_map = create_tree_shadow_map(&self.device, params.shadows.trees.resolution);
      self.shadow_bind_group = create_shadow_bind_group(
        &self.device,
        &self.layouts.shadow,
        &self.tree_shadow_map,
        &self.shadow_sampler,
      );
    }

    self.ensure_cloud_target(params.clouds.resolution_scale);
    self.update_uniforms(params, time, dt);
    let shadow_frame = tree_shadow_frame(
      params.camera_position,
      params.camera_forward,
      params.sun_direction,
      params.shadows.trees.distance_metres,
      self.tree_shadow_map.resolution,
      params.height_range,
    );
    self.uniforms.shadow_view_proj = shadow_frame.view_proj;
    let tree_shadows = self.uniforms.shadow_params[0] > 0.0;
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
      // The square shadow map covers a circle of radius x sqrt(2) at its
      // corners; casters just outside it still reach into it.
      cull.shadow = [
        shadow_frame.centre[0],
        shadow_frame.centre[1],
        shadow_frame.radius * 1.42,
        flag(tree_shadows),
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
        args[SHADOW_ARGS_BASE + slot * 4] = 6;
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

    let view = default_view(&surface_texture.texture);
    let mut encoder = self
      .device
      .create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("VistaWASM frame"),
      });
    self.bake_terrain_shadow_if_needed(params, &mut encoder);

    if let Some(trees) = &self.trees {
      let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some("VistaWASM tree cull pass"),
        timestamp_writes: None,
      });
      pass.set_pipeline(&self.pipelines.cull);
      pass.set_bind_group(0, &trees.cull_bind_group, &[]);
      pass.dispatch_workgroups(trees.instance_count.div_ceil(64), 1, 1);
    }

    let stride = std::mem::size_of::<TreeInstance>() as u64;

    // Tree shadow map: one sun-facing impostor quad per caster.
    if let (Some(trees), true) = (&self.trees, tree_shadows) {
      let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("VistaWASM tree shadow pass"),
        color_attachments: &[],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
          view: &self.tree_shadow_map.view,
          depth_ops: Some(wgpu::Operations {
            load: wgpu::LoadOp::Clear(1.0),
            store: wgpu::StoreOp::Store,
          }),
          stencil_ops: None,
        }),
        ..Default::default()
      });
      pass.set_pipeline(&self.pipelines.tree_shadow);
      pass.set_bind_group(0, &self.frame_bind_group, &[]);
      pass.set_bind_group(1, &self.world_bind_group, &[]);

      for slot in 0..SPECIES_COUNT {
        if trees.counts[slot] == 0 {
          continue;
        }

        pass.set_vertex_buffer(
          0,
          trees
            .shadow_out
            .slice(trees.offsets[slot] as u64 * stride..),
        );
        pass.draw_indirect(
          &trees.args_buffer,
          ((SHADOW_ARGS_BASE + slot * 4) * 4) as u64,
        );
      }
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
      pass.set_bind_group(2, &self.shadow_bind_group, &[]);

      if let Some(terrain) = &self.terrain {
        pass.set_pipeline(&self.pipelines.terrain);
        pass.set_vertex_buffer(0, terrain.vertex_buffer.slice(..));
        pass.set_index_buffer(terrain.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..terrain.index_count, 0, 0..1);
      }

      if let Some(trees) = &self.trees {
        if params.tree_style == 2 {
          pass.set_pipeline(&self.pipelines.tree_mesh);
          pass.set_vertex_buffer(0, self.tree_mesh.vertex_buffer.slice(..));
          pass.set_index_buffer(
            self.tree_mesh.index_buffer.slice(..),
            wgpu::IndexFormat::Uint32,
          );

          for slot in 0..SPECIES_COUNT {
            if trees.counts[slot] == 0 {
              continue;
            }

            pass.set_vertex_buffer(
              1,
              trees.mesh_out.slice(trees.offsets[slot] as u64 * stride..),
            );
            pass.draw_indexed_indirect(&trees.args_buffer, (slot * 5 * 4) as u64);
          }
        }

        pass.set_pipeline(&self.pipelines.tree_impostor);

        for slot in 0..SPECIES_COUNT {
          if trees.counts[slot] == 0 {
            continue;
          }

          pass.set_vertex_buffer(
            0,
            trees
              .impostor_out
              .slice(trees.offsets[slot] as u64 * stride..),
          );
          pass.draw_indirect(
            &trees.args_buffer,
            ((IMPOSTOR_ARGS_BASE + slot * 4) * 4) as u64,
          );
        }
      }

      if let Some(grass) = &self.grass {
        pass.set_pipeline(&self.pipelines.grass);
        pass.set_vertex_buffer(0, self.grass_base_vertex_buffer.slice(..));
        pass.set_vertex_buffer(1, grass.instance_buffer.slice(..));
        pass.draw(0..18, 0..grass.instance_count);
      }
    }

    let Some(cloud_target) = &self.cloud_target else {
      return Ok(());
    };

    // Clouds at reduced resolution; the composite upsamples them. Skipped
    // entirely when there are no clouds.
    if params.cloud_coverage > 0.001 {
      let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("VistaWASM cloud pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
          view: &cloud_target.view,
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
      pass.set_pipeline(&self.pipelines.clouds);
      pass.set_bind_group(0, &self.frame_bind_group, &[]);
      pass.set_bind_group(1, &self.world_bind_group, &[]);
      pass.set_bind_group(2, &self.shadow_bind_group, &[]);
      pass.set_bind_group(3, &cloud_target.cloud_bind_group, &[]);
      pass.draw(0..3, 0..1);
    }

    // Sky, clouds, fog, precipitation, and tone mapping onto the canvas.
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
      pass.set_pipeline(&self.pipelines.composite);
      pass.set_bind_group(0, &self.frame_bind_group, &[]);
      pass.set_bind_group(1, &self.world_bind_group, &[]);
      pass.set_bind_group(2, &self.shadow_bind_group, &[]);
      pass.set_bind_group(3, &cloud_target.composite_bind_group, &[]);
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
      pass.set_pipeline(&self.pipelines.water);
      pass.set_bind_group(0, &self.frame_bind_group, &[]);
      pass.set_bind_group(1, &self.world_bind_group, &[]);
      pass.set_bind_group(2, &self.shadow_bind_group, &[]);

      for mesh in std::iter::once(&self.ocean).chain(self.rivers.iter()) {
        pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
        pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..mesh.index_count, 0, 0..1);
      }
    }

    self.queue.submit(Some(encoder.finish()));
    self.frames_in_flight.fetch_add(1, Ordering::AcqRel);
    let frames_in_flight = Arc::clone(&self.frames_in_flight);
    self.queue.on_submitted_work_done(move || {
      frames_in_flight.fetch_sub(1, Ordering::AcqRel);
    });
    self.queue.present(surface_texture);
    Ok(())
  }

  /// Whether the GPU is still drawing earlier frames. The engine skips a
  /// frame rather than queue another one behind them.
  pub fn is_busy(&self) -> bool {
    self.frames_in_flight.load(Ordering::Acquire) >= MAX_FRAMES_IN_FLIGHT
  }

  /// Resize the WebGPU surface and render targets.
  pub fn resize(&mut self, width: u32, height: u32, device_pixel_ratio: f32) -> VistaResult<()> {
    let pixel_width = scaled_extent(width, device_pixel_ratio);
    let pixel_height = scaled_extent(height, device_pixel_ratio);
    self.config.width = pixel_width;
    self.config.height = pixel_height;
    self.surface.configure(&self.device, &self.config);

    let (depth_view, hdr_view) = create_render_targets(&self.device, pixel_width, pixel_height);
    self.depth_view = depth_view;
    self.hdr_view = hdr_view;
    self.width = pixel_width;
    self.height = pixel_height;
    // The cloud and composite bind groups read the old targets, so rebuild
    // them on the next frame.
    self.cloud_target = None;
    Ok(())
  }
}

fn scaled_extent(value: u32, device_pixel_ratio: f32) -> u32 {
  ((value as f32 * device_pixel_ratio).round() as u32).max(1)
}
