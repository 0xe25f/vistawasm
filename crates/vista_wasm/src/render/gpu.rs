use std::cell::OnceCell;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use bytemuck::{Pod, Zeroable};
use vista_types::{
  AtmosphereOptions, CloudsOptions, ErosionOptions, FloraOptions, MistOptions, ShadowOptions,
  SurfaceOptions, TextureTarget, WaterOptions,
};

use crate::errors::{VistaError, VistaResult};
use crate::render::erosion_compute::ErosionCompute;
use crate::render::flora::{FloraInstance, TreeInstance};
use crate::render::grass::GRASS_BASE_TUFT;
use crate::render::pipelines::{Needs, PipelineKind, PipelineSlots};
use crate::render::shaders;
use crate::render::shadow_math::tree_shadow_frame;
use crate::render::terrain_mesh::{TerrainMeshData, TerrainVertex};
use crate::render::textures::{self, MipGenerator, MipMode, WorldTextures};
use crate::render::tree_models::{
  build_species_mesh, merge_tree_meshes, TreeMesh, TreeSpecies, SPECIES_COUNT,
};
use crate::render::water::{build_ocean_grid, WaterVertex, OCEAN_SNAP_METRES};
use crate::terrain::biomes::{celsius_to_unit, SurfaceSample, DEFAULT_SEA_LEVEL_CELSIUS};
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
  weather3: [f32; 4],
  previous_view_proj: [f32; 16],
  temporal: [f32; 4],
  distances: [f32; 4],
  fades: [f32; 4],
  output: [f32; 4],
  cold: [f32; 4],
  sea_ice: [f32; 4],
  rivers: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<FrameUniforms>() == 800);

/// Static world data: species bounds and tints, terrain mapping, and
/// material tints. Mirrors `WorldInfo` in `common.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct WorldInfo {
  species: [[f32; 4]; 8],
  species_tint: [[f32; 4]; 8],
  terrain: [f32; 4],
  terrain2: [f32; 4],
  material_tints: [[f32; 4]; vista_types::MATERIAL_COUNT],
}

const _: () = assert!(std::mem::size_of::<WorldInfo>() == 448);

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
#[derive(Clone, Debug, Default)]
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
  /// How far precipitation exceeds full intensity (1 or more).
  pub heaviness: f32,
  /// Raindrops on the lens (see [`crate::lens_drops::LensDrops::packed`]).
  pub lens_drops: Vec<[f32; 4]>,
  /// Low drifting snow, 0 to 1.
  pub blowing_snow: f32,
}

/// Where the sea may freeze.
#[derive(Clone, Copy, Debug, Default)]
pub struct SeaIce {
  /// Whether any sea, on the terrain or beyond it, is cold enough to
  /// freeze. When `false` the water shader skips sea ice entirely.
  pub possible: bool,
  /// Temperature unit ((°C + 30) / 65) of the open sea beyond the terrain.
  pub open_sea_unit: f32,
}

/// Rivers, lakes and waterfalls this frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct RiverFrame {
  /// How full snowmelt makes the rivers, 0.4 to 1.4: it scales speed and
  /// foam, and raises the surface inside the channel above 1.
  pub melt: f32,
  /// Whether any lake, river or waterfall is below 0 °C. When `false` the
  /// water shader skips all frozen-water code.
  pub freezing: bool,
  /// Whether there are waterfalls. When `false` the water shader skips
  /// all waterfall code.
  pub falls: bool,
  /// Whether there is a wet-bank field. When `false` the terrain shader
  /// skips wet banks.
  pub wet_banks: bool,
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
  /// Sea ice conditions.
  pub sea_ice: SeaIce,
  /// River, lake and waterfall conditions.
  pub rivers: RiverFrame,
  /// Lowest and highest terrain heights, for fitting the shadow map.
  pub height_range: (f32, f32),
  /// Render, detail, and cloud distances.
  pub distances: vista_types::RenderDistances,
  /// Fraction of the canvas resolution to render the scene at (0.25 to 1).
  pub render_scale: f32,
  /// Smoothed time step to animate by, in seconds.
  pub frame_seconds: f32,
  /// What the scene draws this frame.
  pub needs: Needs,
  /// What it is likely to draw soon.
  pub likely: Needs,
}

// The engine validates replacement textures against these sizes without
// access to the texture module, so keep them in step.
const _: () = assert!(textures::TERRAIN_TEXTURE_SIZE == crate::engine::TEXTURE_LAYER_SIZE);
const _: () = assert!(textures::FLORA_TEXTURE_SIZE == crate::engine::TEXTURE_LAYER_SIZE);
const _: () = assert!(textures::TERRAIN_LAYERS == crate::engine::TERRAIN_TEXTURE_LAYERS);

/// GPU-side terrain mesh resources for the active terrain.
struct TerrainGpu {
  /// Two vertex buffers: one is drawn while the next mesh is written into
  /// the other a few rows per frame, then they swap.
  vertex_buffers: [wgpu::Buffer; 2],
  front: usize,
  vertex_count: u32,
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
  /// Two cloud images: one is drawn this frame while the other, last
  /// frame's, supplies the clouds that are reused.
  views: [wgpu::TextureView; 2],
  width: u32,
  height: u32,
  /// Cloud pass bind groups; `[i]` reads image `1 - i` as the history.
  cloud_bind_groups: [wgpu::BindGroup; 2],
  /// Composite bind groups; `[i]` reads image `i`.
  composite_bind_groups: [wgpu::BindGroup; 2],
  /// Quarter-size image of the sky pixels marched this frame while clouds
  /// are reused, and the bind group its pass reads.
  quarter_view: wgpu::TextureView,
  quarter_bind_group: wgpu::BindGroup,
  /// The image drawn most recently.
  current: usize,
  /// Whether the image not being drawn holds the previous frame's clouds.
  history_valid: bool,
}

/// Timestamp slots: a begin and an end for each timed pass.
const TIMED_PASSES: u32 = 10;
const PASS_TREE_CULL: u32 = 0;
const PASS_TREE_SHADOW: u32 = 1;
const PASS_TERRAIN: u32 = 2;
const PASS_QUARTER_CLOUDS: u32 = 3;
const PASS_CLOUDS: u32 = 4;
const PASS_COMPOSITE: u32 = 5;
const PASS_WATER: u32 = 6;
const PASS_PRESENT: u32 = 7;
const PASS_TREES: u32 = 8;
const PASS_GRASS: u32 = 9;

/// GPU time per pass from timestamp queries.
/// Results are read back asynchronously, so a frame is only timed when the
/// previous reading has arrived, and rendering never waits for it.
struct GpuTimer {
  query_set: wgpu::QuerySet,
  resolve: wgpu::Buffer,
  readback: wgpu::Buffer,
  /// Whether a reading is on its way back.
  busy: Arc<AtomicBool>,
  latest: Arc<Mutex<Option<vista_types::GpuPassTimes>>>,
  /// Nanoseconds per timestamp tick.
  period: f32,
}

impl GpuTimer {
  fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
    let size = u64::from(TIMED_PASSES * 2) * 8;
    Self {
      query_set: device.create_query_set(&wgpu::QuerySetDescriptor {
        label: Some("VistaWASM pass timestamps"),
        ty: wgpu::QueryType::Timestamp,
        count: TIMED_PASSES * 2,
      }),
      resolve: device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("VistaWASM timestamp resolve"),
        size,
        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
      }),
      readback: device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("VistaWASM timestamp readback"),
        size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
      }),
      busy: Arc::new(AtomicBool::new(false)),
      latest: Arc::new(Mutex::new(None)),
      period: queue.get_timestamp_period(),
    }
  }

  fn render_writes(&self, pass: u32) -> wgpu::RenderPassTimestampWrites<'_> {
    wgpu::RenderPassTimestampWrites {
      query_set: &self.query_set,
      beginning_of_pass_write_index: Some(pass * 2),
      end_of_pass_write_index: Some(pass * 2 + 1),
    }
  }

  fn compute_writes(&self, pass: u32) -> wgpu::ComputePassTimestampWrites<'_> {
    wgpu::ComputePassTimestampWrites {
      query_set: &self.query_set,
      beginning_of_pass_write_index: Some(pass * 2),
      end_of_pass_write_index: Some(pass * 2 + 1),
    }
  }

  /// Copy this frame's timestamps out, before the command buffer ends.
  fn resolve(&self, encoder: &mut wgpu::CommandEncoder) {
    encoder.resolve_query_set(&self.query_set, 0..TIMED_PASSES * 2, &self.resolve, 0);
    encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.readback, 0, self.resolve.size());
  }

  /// Read the timestamps back once the GPU has finished. `ran` has one bit
  /// per pass that ran this frame.
  fn read_back(&self, ran: u32) {
    self.busy.store(true, Ordering::Release);
    let buffer = self.readback.clone();
    let busy = Arc::clone(&self.busy);
    let latest = Arc::clone(&self.latest);
    let period = self.period;
    self
      .readback
      .slice(..)
      .map_async(wgpu::MapMode::Read, move |result| {
        if result.is_ok() {
          if let Ok(view) = buffer.slice(..).get_mapped_range() {
            let ticks: &[u64] = bytemuck::cast_slice(&view);
            let ms = |pass: u32| -> f32 {
              if ran & (1 << pass) == 0 {
                return 0.0;
              }

              let begin = ticks[(pass * 2) as usize];
              let end = ticks[(pass * 2 + 1) as usize];
              end.saturating_sub(begin) as f32 * period / 1.0e6
            };
            let times = vista_types::GpuPassTimes {
              shadows: ms(PASS_TREE_SHADOW),
              tree_culling: ms(PASS_TREE_CULL),
              terrain: ms(PASS_TERRAIN),
              trees: ms(PASS_TREES),
              grass: ms(PASS_GRASS),
              clouds: ms(PASS_QUARTER_CLOUDS) + ms(PASS_CLOUDS),
              sky_and_fog: ms(PASS_COMPOSITE),
              water: ms(PASS_WATER),
              present: ms(PASS_PRESENT),
            };
            drop(view);

            if let Ok(mut slot) = latest.lock() {
              *slot = Some(times);
            }
          }

          buffer.unmap();
        }

        busy.store(false, Ordering::Release);
      });
  }
}

/// The finished frame, drawn off-screen so the lens-drop pass can read it,
/// with the lens drops and their screen tiles.
struct LensTarget {
  view: wgpu::TextureView,
  width: u32,
  height: u32,
  bind_group: wgpu::BindGroup,
  drops: wgpu::Buffer,
  bins: wgpu::Buffer,
}

/// A read-only storage buffer read by fragment shaders.
fn storage_buffer_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
  wgpu::BindGroupLayoutEntry {
    binding,
    visibility: wgpu::ShaderStages::FRAGMENT,
    ty: wgpu::BindingType::Buffer {
      ty: wgpu::BufferBindingType::Storage { read_only: true },
      has_dynamic_offset: false,
      min_binding_size: None,
    },
    count: None,
  }
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

/// Shader modules, each compiled the first time a pipeline needs it and
/// shared by every pipeline that uses it.
#[derive(Default)]
struct Modules {
  terrain: OnceCell<wgpu::ShaderModule>,
  trees: OnceCell<wgpu::ShaderModule>,
  grass: OnceCell<wgpu::ShaderModule>,
  atmosphere: OnceCell<wgpu::ShaderModule>,
  water: OnceCell<wgpu::ShaderModule>,
  shadow: OnceCell<wgpu::ShaderModule>,
}

/// Bind group layouts.
struct Layouts {
  frame: wgpu::BindGroupLayout,
  world: wgpu::BindGroupLayout,
  shadow: wgpu::BindGroupLayout,
  composite: wgpu::BindGroupLayout,
  cloud: wgpu::BindGroupLayout,
  cloud_quarter: wgpu::BindGroupLayout,
  terrain_shadow: wgpu::BindGroupLayout,
  lens: wgpu::BindGroupLayout,
}

/// Pipeline layouts, made once so each pipeline created later shares them.
struct PipelineLayouts {
  /// Frame, world and shadow-receiver groups.
  receivers: wgpu::PipelineLayout,
  /// Frame and world groups: the tree shadow pass cannot bind the shadow
  /// map it renders into, and the impostor bake needs no shadows.
  basic: wgpu::PipelineLayout,
  composite: wgpu::PipelineLayout,
  lens: wgpu::PipelineLayout,
  cloud: wgpu::PipelineLayout,
  cloud_quarter: wgpu::PipelineLayout,
  terrain_shadow: wgpu::PipelineLayout,
}

/// A render or compute pipeline in a [`PipelineSlots`] slot.
enum GpuPipeline {
  Render(wgpu::RenderPipeline),
  Compute(wgpu::ComputePipeline),
}

/// Pipelines created when the scene needs them.
#[derive(Default)]
struct Pipelines {
  slots: PipelineSlots<GpuPipeline>,
}

impl Pipelines {
  fn render(&self, kind: PipelineKind) -> Option<&wgpu::RenderPipeline> {
    match self.slots.get(kind)? {
      GpuPipeline::Render(pipeline) => Some(pipeline),
      GpuPipeline::Compute(_) => None,
    }
  }

  fn compute(&self, kind: PipelineKind) -> Option<&wgpu::ComputePipeline> {
    match self.slots.get(kind)? {
      GpuPipeline::Compute(pipeline) => Some(pipeline),
      GpuPipeline::Render(_) => None,
    }
  }
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
  pipeline_layouts: PipelineLayouts,
  modules: Modules,
  pipelines: Pipelines,
  /// Species whose impostors have been rendered, one bit per species.
  /// They are rendered, with the flora layers they sample, when a species
  /// first appears.
  impostors_baked: u32,
  /// Pipelines the scene is likely to need soon are created one per frame,
  /// only after the first frame.
  first_frame_presented: bool,
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
  surface_view: wgpu::TextureView,
  /// Distance to water (see `create_surface_b_texture`).
  surface_b_view: wgpu::TextureView,
  height_size: (u32, u32),
  height_version: u64,
  terrain_shadow: TerrainShadow,
  tree_shadow_map: TreeShadowMap,
  cloud_target: Option<CloudTarget>,
  lens_target: Option<LensTarget>,
  /// The previous frame's view-projection, for reusing its clouds.
  previous_view_proj: [f32; 16],
  /// Counts cloud frames, to choose which pixel of each block is marched.
  cloud_frame: u32,
  tree_meshes: Vec<TreeMesh>,
  tree_mesh: IndexedMesh,
  tree_ranges: [(u32, u32, i32); SPECIES_COUNT],
  tree_bounds: [(f32, f32); SPECIES_COUNT],
  grass_base_vertex_buffer: wgpu::Buffer,
  ocean: IndexedMesh,
  rivers: Option<IndexedMesh>,
  /// Waterfall sheets, mist and plunge pools, drawn after the rivers.
  falls: Option<IndexedMesh>,
  /// Bank strips beside the narrowest streams, drawn over the terrain.
  bank_strips: Option<IndexedMesh>,
  water_visible: bool,
  uniforms: FrameUniforms,
  last_time: f32,
  // Wind-driven offsets are integrated over time rather than computed as
  // speed x time, so changing the wind (for example when the weather
  // changes) never makes clouds, mist, or currents jump.
  cloud_offset: [f32; 2],
  /// Cirrus drifts with its own speed, not the low clouds'.
  cirrus_offset: [f32; 2],
  mist_offset: [f32; 2],
  current_offset: [f32; 2],
  /// Sea ice floes drift with the wind.
  sea_ice_offset: [f32; 2],
  cloud_evolution: f32,
  /// Frames submitted to the GPU and not yet finished. Browsers keep firing
  /// animation frames on schedule even when the GPU falls behind, so without
  /// this limit frames queue up without bound and the picture lags seconds
  /// behind the camera.
  frames_in_flight: Arc<AtomicU32>,
  /// Per-pass GPU timing, when the browser supports timestamp queries.
  timer: Option<GpuTimer>,
  /// Set when the browser reports the device lost. Work submitted to a lost
  /// device silently does nothing, so rendering stops and reports it.
  device_lost: Arc<AtomicBool>,
  /// Erosion compute pipelines, created when erosion is first requested.
  erosion: Option<ErosionCompute>,
  terrain: Option<TerrainGpu>,
  trees: Option<TreesGpu>,
  grass: Option<GrassGpu>,
  /// Internal render size in pixels: the canvas size times `render_scale`.
  width: u32,
  height: u32,
  /// Canvas (surface) size in pixels.
  canvas_width: u32,
  canvas_height: u32,
  /// Fraction of the canvas resolution the scene is rendered at; the final
  /// pass upscales it.
  render_scale: f32,
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
    frame: layout(
      "VistaWASM frame bind group layout",
      &[uniform_entry(0, wgpu::ShaderStages::VERTEX_FRAGMENT)],
    ),
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
        texture_entry(12, Dim::D2, filterable, both),
        texture_entry(13, Dim::D2, filterable, both),
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
    lens: layout(
      "VistaWASM lens layout",
      &[
        texture_entry(3, Dim::D2, filterable, fragment),
        storage_buffer_entry(6),
        storage_buffer_entry(7),
      ],
    ),
    cloud: layout(
      "VistaWASM cloud layout",
      &[
        texture_entry(1, Dim::D2, Sample::Depth, fragment),
        texture_entry(4, Dim::D2, filterable, fragment),
        texture_entry(5, Dim::D2, unfilterable, fragment),
      ],
    ),
    cloud_quarter: layout(
      "VistaWASM quarter cloud layout",
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
  surface: &wgpu::TextureView,
  surface_b: &wgpu::TextureView,
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
      view_entry(12, surface),
      view_entry(13, surface_b),
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

/// The per-terrain surface texture: r temperature unit ((°C + 30) / 65),
/// g moisture, b permanent snow (fast ice on the sea), a biome index / 255.
fn create_surface_texture(
  device: &wgpu::Device,
  queue: &wgpu::Queue,
  width: u32,
  height: u32,
  data: &[u8],
) -> wgpu::TextureView {
  let texture = create_texture_2d(
    device,
    "VistaWASM terrain surface",
    width,
    height,
    wgpu::TextureFormat::Rgba8Unorm,
    wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
  );
  write_layer(queue, &texture, 0, data, width * 4, width, height);
  default_view(&texture)
}

/// The second per-terrain surface texture, at the same resolution as the
/// first: r distance to water / 40 m; g, b and a are reserved (0).
fn create_surface_b_texture(
  device: &wgpu::Device,
  queue: &wgpu::Queue,
  width: u32,
  height: u32,
  distance: &[u8],
) -> wgpu::TextureView {
  let data: Vec<u8> = distance.iter().flat_map(|d| [*d, 0, 0, 0]).collect();
  create_surface_texture(device, queue, width, height, &data)
}

/// Surface texel for one terrain sample.
fn surface_texel(sample: &SurfaceSample) -> [u8; 4] {
  let temperature = celsius_to_unit(sample.celsius()).clamp(0.0, 1.0);

  [
    (temperature * 255.0).round() as u8,
    sample.moisture,
    sample.permanent_snow,
    sample.biome,
  ]
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
  // Values for the shader's pipeline-overridable constants.
  constants: &'a [(&'a str, f64)],
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
      constants: &[],
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
      compilation_options: wgpu::PipelineCompilationOptions {
        constants: spec.constants,
        ..Default::default()
      },
      buffers: spec.buffers,
    },
    fragment: Some(wgpu::FragmentState {
      module: spec.module,
      entry_point: Some(spec.fragment_entry),
      compilation_options: wgpu::PipelineCompilationOptions {
        constants: spec.constants,
        ..Default::default()
      },
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

const TERRAIN_ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
  0 => Float32x3,
  1 => Snorm16x2,
  2 => Uint32x3,
  3 => Unorm8x4,
  4 => Uint8x4,
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

const GRASS_INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
  2 => Float32x3,
  3 => Float32,
  4 => Float32,
  5 => Float32,
  6 => Float32,
];

const BANK_ATTRIBUTES: [wgpu::VertexAttribute; 3] = wgpu::vertex_attr_array![
  0 => Float32x3,
  1 => Float32x2,
  2 => Float32x4,
];

const WATER_ATTRIBUTES: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
  0 => Float32x3,
  1 => Float32x2,
  2 => Float32x3,
  3 => Float32x4,
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

fn flag(on: bool) -> f32 {
  if on {
    1.0
  } else {
    0.0
  }
}

fn create_pipeline_layouts(device: &wgpu::Device, layouts: &Layouts) -> PipelineLayouts {
  let pipeline_layout = |label: &str, groups: &[&wgpu::BindGroupLayout]| {
    let groups: Vec<Option<&wgpu::BindGroupLayout>> =
      groups.iter().map(|group| Some(*group)).collect();
    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
      label: Some(label),
      bind_group_layouts: &groups,
      immediate_size: 0,
    })
  };
  let frame = &layouts.frame;

  PipelineLayouts {
    receivers: pipeline_layout(
      "VistaWASM shadow receiver pipeline layout",
      &[frame, &layouts.world, &layouts.shadow],
    ),
    basic: pipeline_layout("VistaWASM basic pipeline layout", &[frame, &layouts.world]),
    composite: pipeline_layout(
      "VistaWASM composite pipeline layout",
      &[frame, &layouts.world, &layouts.shadow, &layouts.composite],
    ),
    lens: pipeline_layout(
      "VistaWASM lens pipeline layout",
      &[frame, &layouts.world, &layouts.shadow, &layouts.lens],
    ),
    cloud: pipeline_layout(
      "VistaWASM cloud pipeline layout",
      &[frame, &layouts.world, &layouts.shadow, &layouts.cloud],
    ),
    cloud_quarter: pipeline_layout(
      "VistaWASM quarter cloud pipeline layout",
      &[
        frame,
        &layouts.world,
        &layouts.shadow,
        &layouts.cloud_quarter,
      ],
    ),
    terrain_shadow: pipeline_layout(
      "VistaWASM terrain shadow pipeline layout",
      &[&layouts.terrain_shadow],
    ),
  }
}

fn water_buffers() -> [Option<wgpu::VertexBufferLayout<'static>>; 1] {
  [Some(wgpu::VertexBufferLayout {
    array_stride: std::mem::size_of::<WaterVertex>() as u64,
    step_mode: wgpu::VertexStepMode::Vertex,
    attributes: &WATER_ATTRIBUTES,
  })]
}

/// The impostor bake pipeline. It is made for each bake and dropped after
/// it.
fn create_bake_pipeline(
  device: &wgpu::Device,
  layouts: &PipelineLayouts,
  modules: &Modules,
) -> wgpu::RenderPipeline {
  let bake_buffers = [Some(tree_vertex_layout())];

  create_pipeline(
    device,
    &layouts.basic,
    PipelineSpec {
      format: Some(wgpu::TextureFormat::Rgba8Unorm),
      ..PipelineSpec::opaque(
        "VistaWASM impostor bake",
        modules.trees(device),
        ("vertex_bake", "fragment_bake"),
        &bake_buffers,
      )
    },
  )
}

impl Modules {
  fn get<'a>(
    cell: &'a OnceCell<wgpu::ShaderModule>,
    device: &wgpu::Device,
    label: &str,
    body: &str,
  ) -> &'a wgpu::ShaderModule {
    cell.get_or_init(|| render_module(device, label, body))
  }

  fn terrain(&self, device: &wgpu::Device) -> &wgpu::ShaderModule {
    Self::get(
      &self.terrain,
      device,
      "VistaWASM terrain shader",
      shaders::TERRAIN,
    )
  }

  fn trees(&self, device: &wgpu::Device) -> &wgpu::ShaderModule {
    Self::get(&self.trees, device, "VistaWASM tree shader", shaders::TREES)
  }

  fn grass(&self, device: &wgpu::Device) -> &wgpu::ShaderModule {
    Self::get(
      &self.grass,
      device,
      "VistaWASM grass shader",
      shaders::GRASS,
    )
  }

  fn atmosphere(&self, device: &wgpu::Device) -> &wgpu::ShaderModule {
    Self::get(
      &self.atmosphere,
      device,
      "VistaWASM atmosphere shader",
      shaders::ATMOSPHERE,
    )
  }

  fn water(&self, device: &wgpu::Device) -> &wgpu::ShaderModule {
    Self::get(
      &self.water,
      device,
      "VistaWASM water shader",
      shaders::WATER,
    )
  }

  fn shadow(&self, device: &wgpu::Device) -> &wgpu::ShaderModule {
    Self::get(
      &self.shadow,
      device,
      "VistaWASM shadow shader",
      shaders::SHADOW,
    )
  }
}

/// Create one pipeline. The ocean, inland water and waterfall pipelines
/// share the water module and differ only in override constants, and the
/// atmosphere module serves the cloud, composite and present passes.
fn create_pipeline_of(
  kind: PipelineKind,
  device: &wgpu::Device,
  layouts: &PipelineLayouts,
  modules: &Modules,
  surface_format: wgpu::TextureFormat,
) -> GpuPipeline {
  let tree_buffers = [Some(tree_vertex_layout()), Some(tree_instance_layout())];
  let instance_buffers = [Some(tree_instance_layout())];
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
  let water_buffers = water_buffers();
  let main = ("vertex_main", "fragment_main");
  // Water draws over the finished frame, blended and depth-tested.
  let water = |label, constants, entries| {
    create_pipeline(
      device,
      &layouts.receivers,
      PipelineSpec {
        format: Some(surface_format),
        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
        depth: Some((false, wgpu::CompareFunction::Less)),
        constants,
        ..PipelineSpec::opaque(label, modules.water(device), entries, &water_buffers)
      },
    )
  };
  let render = |pipeline| GpuPipeline::Render(pipeline);

  match kind {
    PipelineKind::TerrainShadow => GpuPipeline::Compute(compute_pipeline(
      device,
      "VistaWASM terrain shadow bake",
      shaders::TERRAIN_SHADOW,
      "bake",
      Some(&layouts.terrain_shadow),
    )),
    PipelineKind::TreeCull => GpuPipeline::Compute(compute_pipeline(
      device,
      "VistaWASM tree cull",
      shaders::TREE_CULL,
      "cull_main",
      None,
    )),
    PipelineKind::TreeShadow => render(create_pipeline(
      device,
      &layouts.basic,
      PipelineSpec {
        format: None,
        depth_bias: wgpu::DepthBiasState {
          constant: 2,
          slope_scale: 2.5,
          clamp: 0.0,
        },
        ..PipelineSpec::opaque(
          "VistaWASM tree shadows",
          modules.shadow(device),
          main,
          &instance_buffers,
        )
      },
    )),
    PipelineKind::Terrain => render(create_pipeline(
      device,
      &layouts.receivers,
      PipelineSpec {
        cull_mode: Some(wgpu::Face::Back),
        ..PipelineSpec::opaque(
          "VistaWASM terrain",
          modules.terrain(device),
          main,
          &terrain_buffers,
        )
      },
    )),
    // Blended over the terrain it lies on, pulled towards the camera so it
    // wins the depth test against the ground it follows.
    PipelineKind::BankStrips => render(create_pipeline(
      device,
      &layouts.receivers,
      PipelineSpec {
        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
        depth: Some((false, wgpu::CompareFunction::LessEqual)),
        depth_bias: wgpu::DepthBiasState {
          constant: -4,
          slope_scale: -1.0,
          clamp: 0.0,
        },
        ..PipelineSpec::opaque(
          "VistaWASM bank strips",
          modules.terrain(device),
          ("vertex_bank", "fragment_bank"),
          &[Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<crate::render::water::BankVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &BANK_ATTRIBUTES,
          })],
        )
      },
    )),
    PipelineKind::TreeMesh => render(create_pipeline(
      device,
      &layouts.receivers,
      PipelineSpec::opaque(
        "VistaWASM tree meshes",
        modules.trees(device),
        ("vertex_mesh", "fragment_mesh"),
        &tree_buffers,
      ),
    )),
    PipelineKind::TreeImpostor => render(create_pipeline(
      device,
      &layouts.receivers,
      PipelineSpec::opaque(
        "VistaWASM tree impostors",
        modules.trees(device),
        ("vertex_impostor", "fragment_impostor"),
        &instance_buffers,
      ),
    )),
    PipelineKind::Grass => render(create_pipeline(
      device,
      &layouts.receivers,
      PipelineSpec::opaque(
        "VistaWASM grass",
        modules.grass(device),
        main,
        &grass_buffers,
      ),
    )),
    PipelineKind::QuarterClouds => render(create_pipeline(
      device,
      &layouts.cloud_quarter,
      PipelineSpec {
        depth: None,
        ..PipelineSpec::opaque(
          "VistaWASM quarter clouds",
          modules.atmosphere(device),
          ("vertex_main", "cloud_quarter_main"),
          &[],
        )
      },
    )),
    PipelineKind::Clouds => render(create_pipeline(
      device,
      &layouts.cloud,
      PipelineSpec {
        depth: None,
        ..PipelineSpec::opaque(
          "VistaWASM clouds",
          modules.atmosphere(device),
          ("vertex_main", "cloud_main"),
          &[],
        )
      },
    )),
    PipelineKind::Composite => render(create_pipeline(
      device,
      &layouts.composite,
      PipelineSpec {
        format: Some(surface_format),
        depth: None,
        ..PipelineSpec::opaque("VistaWASM composite", modules.atmosphere(device), main, &[])
      },
    )),
    PipelineKind::SeaIceOcean => render(water("VistaWASM water", &[], main)),
    // The same water without sea ice, drawn whenever no sea can freeze, so
    // mild maps pay nothing for it.
    PipelineKind::OpenOcean => render(water("VistaWASM open water", &[("SEA_ICE", 0.0)], main)),
    // Rivers and lakes without the ocean's waves and sea ice, and the
    // ocean without them.
    PipelineKind::InlandWater => render(water(
      "VistaWASM inland water",
      &[("SEA_ICE", 0.0), ("INLAND", 1.0)],
      main,
    )),
    PipelineKind::Falls => render(water(
      "VistaWASM waterfalls",
      &[("SEA_ICE", 0.0), ("INLAND", 1.0)],
      ("vertex_main", "fragment_fall"),
    )),
    PipelineKind::Present => render(create_pipeline(
      device,
      &layouts.lens,
      PipelineSpec {
        format: Some(surface_format),
        depth: None,
        ..PipelineSpec::opaque(
          "VistaWASM present",
          modules.atmosphere(device),
          ("vertex_main", "present_main"),
          &[],
        )
      },
    )),
  }
}

impl GpuContext {
  /// Create and configure WebGPU resources for a browser canvas, bake the
  /// procedural textures and model the tree species. Pipelines, the tree
  /// impostors and the 3D cloud noise are made when a scene first needs
  /// them (see [`Self::ensure_pipelines`]).
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
        // Timestamps feed the per-pass profiler when the browser offers
        // them; everything else works without.
        required_features: adapter.features() & wgpu::Features::TIMESTAMP_QUERY,
        required_limits: wgpu::Limits::default(),
        ..Default::default()
      })
      .await
      .map_err(|_| VistaError::WebGpuDeviceRequestFailed)?;
    let timer = device
      .features()
      .contains(wgpu::Features::TIMESTAMP_QUERY)
      .then(|| GpuTimer::new(&device, &queue));
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
    let layouts = create_layouts(&device);
    let frame_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
      label: Some("VistaWASM frame bind group"),
      layout: &layouts.frame,
      entries: &[wgpu::BindGroupEntry {
        binding: 0,
        resource: uniform_buffer.as_entire_binding(),
      }],
    });
    let pipeline_layouts = create_pipeline_layouts(&device, &layouts);
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
    }

    for tint in &mut world_info.material_tints {
      *tint = [1.0, 1.0, 1.0, 0.0];
    }

    let world_buffer = buffer_with_data(
      &device,
      &queue,
      "VistaWASM world info",
      bytemuck::bytes_of(&world_info),
      wgpu::BufferUsages::UNIFORM,
    );
    let height_view = create_height_texture(&device, &queue, 1, 1, &[-100_000.0]);
    let surface_view = create_surface_texture(
      &device,
      &queue,
      1,
      1,
      &surface_texel(&SurfaceSample {
        celsius_hundredths: (DEFAULT_SEA_LEVEL_CELSIUS * 100.0) as i16,
        ..SurfaceSample::default()
      }),
    );
    let surface_b_view = create_surface_b_texture(&device, &queue, 1, 1, &[255]);
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
      &surface_view,
      &surface_b_view,
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
    let mut uniforms = FrameUniforms::zeroed();
    uniforms.camera_up[3] = flag(!config.format.is_srgb());

    let context = Self {
      surface,
      device,
      queue,
      config,
      depth_view,
      hdr_view,
      frame_bind_group,
      uniform_buffer,
      layouts,
      pipeline_layouts,
      modules: Modules::default(),
      pipelines: Pipelines::default(),
      impostors_baked: 0,
      first_frame_presented: false,
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
      surface_view,
      surface_b_view,
      height_size: (1, 1),
      height_version: 0,
      terrain_shadow,
      tree_shadow_map,
      cloud_target: None,
      lens_target: None,
      previous_view_proj: [0.0; 16],
      cloud_frame: 0,
      tree_meshes,
      tree_mesh,
      tree_ranges: library.ranges,
      tree_bounds: library.bounds,
      grass_base_vertex_buffer,
      ocean,
      rivers: None,
      falls: None,
      bank_strips: None,
      water_visible: false,
      uniforms,
      last_time: 0.0,
      cloud_offset: [0.0; 2],
      cirrus_offset: [0.0; 2],
      mist_offset: [0.0; 2],
      current_offset: [0.0; 2],
      sea_ice_offset: [0.0; 2],
      frames_in_flight: Arc::new(AtomicU32::new(0)),
      timer,
      device_lost,
      cloud_evolution: 0.0,
      erosion: None,
      terrain: None,
      trees: None,
      grass: None,
      width: pixel_width,
      height: pixel_height,
      canvas_width: pixel_width,
      canvas_height: pixel_height,
      render_scale: 1.0,
    };
    Ok(context)
  }

  /// Create every pipeline the scene needs that does not exist yet, in the
  /// order the frame draws, and bake the terrain materials, tree species
  /// (flora layers and impostors) and cloud noise the first time they are
  /// needed. Cheap when nothing is missing.
  pub fn ensure_pipelines(&mut self, needs: &Needs) {
    let Self {
      device,
      pipeline_layouts,
      modules,
      pipelines,
      config,
      ..
    } = self;
    pipelines.slots.ensure(needs, |kind| {
      create_pipeline_of(kind, device, pipeline_layouts, modules, config.format)
    });

    self.world_textures.bake_terrain(
      &self.device,
      &self.queue,
      &self.mips,
      needs.terrain_materials,
    );

    // The cloud noise replaces its placeholder, so the bind group changes.
    if needs.cloud_noise && !self.world_textures.cloud_baked {
      self
        .world_textures
        .bake_cloud_noise(&self.device, &self.queue);
      self.rebuild_world_bind_group();
    }

    if needs.trees {
      let present = self.trees.as_ref().map_or(0, |trees| {
        (0..SPECIES_COUNT)
          .filter(|slot| trees.counts[*slot] > 0)
          .fold(0, |mask, slot| mask | 1 << slot)
      });
      let missing = present & !self.impostors_baked;

      if missing != 0 {
        self.bake_impostors(missing);
      }
    }
  }

  /// Create at most one pipeline the scene is likely to need soon, once
  /// the first frame has been presented.
  fn warm_up(&mut self, likely: &Needs) {
    if !self.first_frame_presented {
      self.first_frame_presented = true;
      return;
    }

    let Self {
      device,
      pipeline_layouts,
      modules,
      pipelines,
      config,
      ..
    } = self;
    pipelines.slots.warm_one(likely, |kind| {
      create_pipeline_of(kind, device, pipeline_layouts, modules, config.format)
    });
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
      &self.surface_view,
      &self.surface_b_view,
      &self.world_buffer,
    );
  }

  fn write_world_info(&self) {
    self
      .queue
      .write_buffer(&self.world_buffer, 0, bytemuck::bytes_of(&self.world_info));
  }

  /// Render the species in `species` (one bit each) into the impostor
  /// texture array, baking the flora layers they sample first, then
  /// mipmap it.
  fn bake_impostors(&mut self, species: u32) {
    let layers = self
      .tree_meshes
      .iter()
      .enumerate()
      .filter(|(slot, _)| species & 1 << slot != 0)
      .fold(0, |mask, (_, mesh)| mask | mesh.flora_layers());
    self
      .world_textures
      .bake_flora(&self.device, &self.queue, &self.mips, layers);

    self.impostors_baked |= species;
    let pipeline = create_bake_pipeline(&self.device, &self.pipeline_layouts, &self.modules);
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
      &self.surface_view,
      &self.surface_b_view,
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
      if species & 1 << slot == 0 {
        continue;
      }

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
      pass.set_pipeline(&pipeline);
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

  /// Erode `map` on the GPU and return the eroded heights. See
  /// [`ErosionCompute::run`] for details.
  pub async fn run_erosion(
    &mut self,
    map: &crate::terrain::HeightMap,
    options: &ErosionOptions,
    landform: &crate::terrain::landforms::Landform,
    progress: crate::terrain::fractal::Progress<'_>,
  ) -> VistaResult<Vec<f32>> {
    let device = &self.device;
    let erosion = self
      .erosion
      .get_or_insert_with(|| ErosionCompute::new(device));
    erosion
      .run(device, &self.queue, map, options, landform, progress)
      .await
  }

  /// Wait until the GPU has finished the work submitted so far. Called
  /// before the first large upload of a new terrain, so the page keeps
  /// running (and receiving progress events) while the browser compiles
  /// start-up work, instead of freezing inside the upload.
  pub async fn finish_submitted_work(&self) -> VistaResult<()> {
    crate::render::erosion_compute::work_done(&self.queue).await
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

    if self.impostors_baked != 0 {
      self.bake_impostors(self.impostors_baked);
    }
  }

  /// Replace one layer of a baked texture array with host-supplied RGBA8
  /// texels (`size() x size()`, already validated by the engine), then
  /// rebuild its mips. Flora changes also re-bake the impostors.
  pub fn replace_texture_layer(&mut self, target: TextureTarget, layer: u32, rgba: &[u8]) {
    let size = crate::engine::TEXTURE_LAYER_SIZE;

    // Both terrain arrays are baked together, so a layer is baked before
    // either is replaced, and never baked over later.
    if target != TextureTarget::Flora {
      self
        .world_textures
        .bake_terrain(&self.device, &self.queue, &self.mips, 1 << layer);
    }

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

    // A replaced flora layer is never baked over later.
    if target == TextureTarget::Flora {
      self.world_textures.flora_baked |= 1 << layer;

      if self.impostors_baked != 0 {
        self.bake_impostors(self.impostors_baked);
      }
    }
  }

  /// Regenerate every procedural texture, discarding replaced layers, and
  /// re-bake the impostors if they have been baked.
  pub fn reset_textures(&mut self) {
    let terrain = self.world_textures.terrain_baked;
    let cloud = self.world_textures.cloud_baked;
    self.world_textures = textures::bake_world_textures(&self.device, &self.queue, &self.mips);
    self
      .world_textures
      .bake_terrain(&self.device, &self.queue, &self.mips, terrain);

    if cloud {
      self
        .world_textures
        .bake_cloud_noise(&self.device, &self.queue);
    }

    self.rebuild_world_bind_group();

    // Re-baking the impostors bakes their flora layers again too.
    if self.impostors_baked != 0 {
      self.bake_impostors(self.impostors_baked);
    }
  }

  /// Upload a CPU-baked terrain mesh, replacing any previous terrain buffers.
  pub fn upload_terrain(&mut self, mesh: &TerrainMeshData) {
    if mesh.vertices.is_empty() || mesh.indices.is_empty() {
      self.terrain = None;
      return;
    }

    let vertex_bytes: &[u8] = bytemuck::cast_slice(&mesh.vertices);
    let reusable = self.terrain.as_ref().is_some_and(|terrain| {
      terrain.vertex_count as usize == mesh.vertices.len()
        && terrain.index_count as usize == mesh.indices.len()
    });

    // The camera-centred mesh always has the same size, so a new terrain
    // or recolouring reuses the buffers (and the unchanged indices).
    if reusable {
      if let Some(terrain) = &self.terrain {
        self
          .queue
          .write_buffer(&terrain.vertex_buffers[terrain.front], 0, vertex_bytes);
        self.queue.write_buffer(
          &terrain.index_buffer,
          0,
          bytemuck::cast_slice(&mesh.indices),
        );
      }

      return;
    }

    let usage = wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST;
    let front = buffer_with_data(
      &self.device,
      &self.queue,
      "VistaWASM terrain",
      vertex_bytes,
      usage,
    );
    let back = self.device.create_buffer(&wgpu::BufferDescriptor {
      label: Some("VistaWASM terrain (next)"),
      size: vertex_bytes.len() as u64,
      usage,
      mapped_at_creation: false,
    });
    let index_buffer = buffer_with_data(
      &self.device,
      &self.queue,
      "VistaWASM terrain indices",
      bytemuck::cast_slice(&mesh.indices),
      wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
    );
    self.terrain = Some(TerrainGpu {
      vertex_buffers: [front, back],
      front: 0,
      vertex_count: mesh.vertices.len() as u32,
      index_buffer,
      index_count: mesh.indices.len() as u32,
    });
  }

  /// Write vertices of the next terrain mesh, starting at `first_vertex`,
  /// into the buffer that is not being drawn. Returns `false` when there is
  /// no terrain or the vertices would not fit.
  pub fn write_next_terrain_vertices(
    &mut self,
    first_vertex: u32,
    vertices: &[TerrainVertex],
  ) -> bool {
    let Some(terrain) = &self.terrain else {
      return false;
    };

    if first_vertex as usize + vertices.len() > terrain.vertex_count as usize {
      return false;
    }

    let offset = first_vertex as u64 * std::mem::size_of::<TerrainVertex>() as u64;
    self.queue.write_buffer(
      &terrain.vertex_buffers[1 - terrain.front],
      offset,
      bytemuck::cast_slice(vertices),
    );
    true
  }

  /// Start drawing the terrain mesh written by
  /// [`Self::write_next_terrain_vertices`].
  pub fn show_next_terrain(&mut self) {
    if let Some(terrain) = &mut self.terrain {
      terrain.front = 1 - terrain.front;
    }
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
    self.world_info.terrain2 = [texture_width as f32, texture_height as f32, 1.0, sea];
    self.write_world_info();
    self.rebuild_world_bind_group();
  }

  /// Upload the per-terrain surface texture (temperature, moisture,
  /// permanent snow, biome) at the same resolution as the height texture.
  pub fn upload_surface(&mut self, map: &HeightMap, surface: &[SurfaceSample]) {
    let width = map.metadata.width;
    let height = map.metadata.height;

    if width == 0 || height == 0 || surface.len() != map.heights.len() {
      return;
    }

    let stride = (width.max(height).saturating_sub(1) / (HEIGHT_TEXTURE_MAX - 1)).max(1);
    let texture_width = (width - 1) / stride + 1;
    let texture_height = (height - 1) / stride + 1;
    let mut data = Vec::with_capacity((texture_width * texture_height * 4) as usize);

    for ty in 0..texture_height {
      for tx in 0..texture_width {
        let index = ((ty * stride) * width + tx * stride) as usize;
        data.extend_from_slice(&surface_texel(&surface[index]));
      }
    }

    self.surface_view = create_surface_texture(
      &self.device,
      &self.queue,
      texture_width,
      texture_height,
      &data,
    );
    self.rebuild_world_bind_group();
  }

  /// Upload the distance-to-water field, or clear it with an empty one.
  pub fn upload_wet_banks(&mut self, wet: &crate::render::water::WetBanks) {
    self.surface_b_view = if wet.distance.is_empty() {
      create_surface_b_texture(&self.device, &self.queue, 1, 1, &[255])
    } else {
      create_surface_b_texture(
        &self.device,
        &self.queue,
        wet.width,
        wet.height,
        &wet.distance,
      )
    };
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
    // The cull pipeline's layout is implicit, so the pipeline comes first.
    self.ensure_pipelines(&Needs {
      trees: true,
      ..Needs::default()
    });
    let Some(cull) = self.pipelines.compute(PipelineKind::TreeCull) else {
      self.trees = None;
      return;
    };
    let cull_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
      label: Some("VistaWASM tree cull bind group"),
      layout: &cull.get_bind_group_layout(0),
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

  /// Upload waterfall geometry, replacing any previous buffers.
  pub fn upload_falls(&mut self, vertices: &[WaterVertex], indices: &[u32]) {
    self.falls = (!vertices.is_empty() && !indices.is_empty()).then(|| {
      indexed_mesh(
        &self.device,
        &self.queue,
        "VistaWASM waterfalls",
        bytemuck::cast_slice(vertices),
        indices,
      )
    });
  }

  /// Upload bank strip geometry, replacing any previous buffers.
  pub fn upload_bank_strips(
    &mut self,
    vertices: &[crate::render::water::BankVertex],
    indices: &[u32],
  ) {
    self.bank_strips = (!vertices.is_empty() && !indices.is_empty()).then(|| {
      indexed_mesh(
        &self.device,
        &self.queue,
        "VistaWASM bank strips",
        bytemuck::cast_slice(vertices),
        indices,
      )
    });
  }

  /// Show or hide all water.
  pub fn set_water_visible(&mut self, visible: bool) {
    self.water_visible = visible;
  }

  /// Update the terrain material colour multipliers.
  pub fn set_material_tints(&mut self, tints: &[[f32; 3]; vista_types::MATERIAL_COUNT]) {
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
    let cirrus_speed = clouds.cirrus_speed.max(0.0) * 15.0 * dt;
    self.cirrus_offset[0] -= cloud_wind[0] * cirrus_speed;
    self.cirrus_offset[1] -= cloud_wind[1] * cirrus_speed;
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
    // Pack ice drifts at about 2 % of the wind speed.
    self.sea_ice_offset[0] -= params.weather.wind[0] * 0.02 * dt;
    self.sea_ice_offset[1] -= params.weather.wind[1] * 0.02 * dt;

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
      flag(self.water_visible),
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
    u.output = [
      self.canvas_width.max(1) as f32,
      self.canvas_height.max(1) as f32,
      self.render_scale,
      0.0,
    ];

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
    u.distances = [
      params.distances.render_metres,
      params.distances.detail_metres,
      params.distances.cloud_metres,
      0.0,
    ];
    u.fades = [
      params.distances.render_fade_metres,
      params.distances.cloud_fade_metres,
      0.0,
      0.0,
    ];
    u.weather3 = [
      self.cirrus_offset[0],
      self.cirrus_offset[1],
      flag(!weather.lens_drops.is_empty()),
      weather.heaviness.max(1.0),
    ];
    u.cold = [weather.blowing_snow.clamp(0.0, 1.0), 0.0, 0.0, 0.0];
    u.sea_ice = [
      flag(params.sea_ice.possible),
      params.sea_ice.open_sea_unit.clamp(0.0, 1.0),
      self.sea_ice_offset[0],
      self.sea_ice_offset[1],
    ];
    let rivers = &params.rivers;
    u.rivers = [
      rivers.melt.clamp(0.4, 1.4),
      flag(rivers.freezing),
      flag(rivers.falls),
      flag(rivers.wet_banks),
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

    let Some(pipeline) = self.pipelines.compute(PipelineKind::TerrainShadow) else {
      return;
    };

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
    pass.set_pipeline(pipeline);
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

    let image = || {
      default_view(&create_texture_2d(
        &self.device,
        "VistaWASM cloud target",
        width,
        height,
        HDR_FORMAT,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
      ))
    };
    let views = [image(), image()];
    let quarter_view = default_view(&create_texture_2d(
      &self.device,
      "VistaWASM quarter cloud target",
      width.div_ceil(2),
      height.div_ceil(2),
      HDR_FORMAT,
      wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
    ));
    let quarter_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
      label: Some("VistaWASM quarter cloud bind group"),
      layout: &self.layouts.cloud_quarter,
      entries: &[view_entry(1, &self.depth_view)],
    });
    let cloud_bind_group = |history: &wgpu::TextureView| {
      self.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("VistaWASM cloud bind group"),
        layout: &self.layouts.cloud,
        entries: &[
          view_entry(1, &self.depth_view),
          view_entry(4, history),
          view_entry(5, &quarter_view),
        ],
      })
    };
    let composite_bind_group = |clouds: &wgpu::TextureView| {
      self.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("VistaWASM composite bind group"),
        layout: &self.layouts.composite,
        entries: &[
          view_entry(0, &self.hdr_view),
          view_entry(1, &self.depth_view),
          view_entry(2, clouds),
        ],
      })
    };
    let cloud_bind_groups = [cloud_bind_group(&views[1]), cloud_bind_group(&views[0])];
    let composite_bind_groups = [
      composite_bind_group(&views[0]),
      composite_bind_group(&views[1]),
    ];

    self.cloud_target = Some(CloudTarget {
      views,
      width,
      height,
      cloud_bind_groups,
      composite_bind_groups,
      quarter_view,
      quarter_bind_group,
      current: 0,
      history_valid: false,
    });
  }

  /// Make sure the off-screen target for the lens-drop pass matches the
  /// canvas.
  fn ensure_lens_target(&mut self) {
    if self
      .lens_target
      .as_ref()
      .is_some_and(|target| target.width == self.width && target.height == self.height)
    {
      return;
    }

    let view = default_view(&create_texture_2d(
      &self.device,
      "VistaWASM lens source",
      self.width,
      self.height,
      self.config.format,
      wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
    ));
    let buffer = |label, words: usize| {
      self.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: (words * 4) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
      })
    };
    let drops = buffer(
      "VistaWASM lens drops",
      crate::lens_drops::MAX_LENS_DROPS * 4,
    );
    let bins = buffer("VistaWASM lens tiles", crate::lens_drops::BIN_WORDS);
    let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
      label: Some("VistaWASM lens bind group"),
      layout: &self.layouts.lens,
      entries: &[
        wgpu::BindGroupEntry {
          binding: 3,
          resource: wgpu::BindingResource::TextureView(&view),
        },
        wgpu::BindGroupEntry {
          binding: 6,
          resource: drops.as_entire_binding(),
        },
        wgpu::BindGroupEntry {
          binding: 7,
          resource: bins.as_entire_binding(),
        },
      ],
    });
    self.lens_target = Some(LensTarget {
      view,
      width: self.width,
      height: self.height,
      bind_group,
      drops,
      bins,
    });
  }

  /// Render one frame.
  pub fn render_once(&mut self, params: &FrameParams) -> VistaResult<()> {
    if self.device_lost.load(Ordering::Acquire) {
      return Err(VistaError::WebGpuDeviceLost);
    }

    // Animation runs on the engine's smoothed time step, so a late frame
    // does not make wind, water, and clouds jump.
    let dt = params.frame_seconds.clamp(0.0, 0.25);
    let time = self.last_time + dt;
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

    self.set_render_scale(params.render_scale);
    self.ensure_pipelines(&params.needs);
    self.ensure_cloud_target(params.clouds.resolution_scale);
    // When the scene is rendered below the canvas resolution, or lens drops
    // refract it, the frame is drawn off-screen first and a final pass
    // upscales it (adding the drops); otherwise it goes straight to the
    // canvas with no extra pass.
    let drops = &params.weather.lens_drops;
    let present = !drops.is_empty() || self.render_scale < 0.999;

    if present {
      self.ensure_lens_target();
    }

    // The drops and the tiles they cover, written only while there are
    // drops; the shader skips them otherwise.
    if let (Some(target), false) = (&self.lens_target, drops.is_empty()) {
      let bins = crate::lens_drops::bin(drops, self.canvas_width, self.canvas_height);
      self
        .queue
        .write_buffer(&target.drops, 0, bytemuck::cast_slice(drops));
      self
        .queue
        .write_buffer(&target.bins, 0, bytemuck::cast_slice(&bins));
    }
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

    // Reusing distant clouds: only for volumetric clouds seen from well
    // outside their layer (from inside it clouds are close and shift too
    // much between frames), and only when the other cloud image holds the
    // previous frame's clouds.
    let clouds_drawn = params.cloud_coverage > 0.001;
    let camera_y = params.camera_position[1];
    let base = params.clouds.height_metres;
    let top = base
      + params.clouds.thickness_metres.max(1.0)
        * (1.0 + params.clouds.towering.clamp(0.0, 1.0) * TOWER_STRETCH);
    let outside_layer = camera_y < base - 300.0 || camera_y > top + 300.0;

    if let Some(target) = &mut self.cloud_target {
      if clouds_drawn {
        target.current = 1 - target.current;
      }

      let reuse = params.clouds.temporal
        && clouds_drawn
        && params.cloud_raymarch_steps > 0
        && outside_layer
        && target.history_valid;
      self.uniforms.temporal = [
        flag(reuse),
        (self.cloud_frame % 4) as f32,
        target.width as f32,
        target.height as f32,
      ];
      target.history_valid = clouds_drawn;
    }

    self.uniforms.previous_view_proj = self.previous_view_proj;
    self.previous_view_proj = params.view_proj;
    self.cloud_frame = self.cloud_frame.wrapping_add(1);
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
        params
          .far_metres
          .min(40_000.0)
          .min(params.distances.render_metres),
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

    let canvas_view = default_view(&surface_texture.texture);
    let view = match (&self.lens_target, present) {
      (Some(target), true) => target.view.clone(),
      _ => canvas_view.clone(),
    };
    let mut encoder = self
      .device
      .create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("VistaWASM frame"),
      });
    self.bake_terrain_shadow_if_needed(params, &mut encoder);

    // Time this frame's passes unless the previous reading is still on its
    // way back.
    let timing = self
      .timer
      .as_ref()
      .filter(|timer| !timer.busy.load(Ordering::Acquire));
    let mut ran = 0u32;

    if let (Some(trees), Some(cull)) = (&self.trees, self.pipelines.compute(PipelineKind::TreeCull))
    {
      let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some("VistaWASM tree cull pass"),
        timestamp_writes: timing.map(|timer| timer.compute_writes(PASS_TREE_CULL)),
      });
      ran |= 1 << PASS_TREE_CULL;
      pass.set_pipeline(cull);
      pass.set_bind_group(0, &trees.cull_bind_group, &[]);
      pass.dispatch_workgroups(trees.instance_count.div_ceil(64), 1, 1);
    }

    let stride = std::mem::size_of::<TreeInstance>() as u64;

    // Tree shadow map: one sun-facing impostor quad per caster.
    if let (Some(trees), true, Some(pipeline)) = (
      &self.trees,
      tree_shadows,
      self.pipelines.render(PipelineKind::TreeShadow),
    ) {
      let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("VistaWASM tree shadow pass"),
        timestamp_writes: timing.map(|timer| timer.render_writes(PASS_TREE_SHADOW)),
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
      ran |= 1 << PASS_TREE_SHADOW;
      pass.set_pipeline(pipeline);
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
    // Terrain, trees, and grass draw into the same targets in three passes
    // so the profiler can time each; the terrain pass clears them.
    {
      let mut pass = self.begin_opaque_pass(
        &mut encoder,
        "VistaWASM terrain pass",
        timing.map(|timer| timer.render_writes(PASS_TERRAIN)),
        true,
      );
      ran |= 1 << PASS_TERRAIN;

      if let (Some(terrain), Some(pipeline)) =
        (&self.terrain, self.pipelines.render(PipelineKind::Terrain))
      {
        pass.set_pipeline(pipeline);
        pass.set_vertex_buffer(0, terrain.vertex_buffers[terrain.front].slice(..));
        pass.set_index_buffer(terrain.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..terrain.index_count, 0, 0..1);
      }

      if let (Some(strips), Some(pipeline), true) = (
        &self.bank_strips,
        self.pipelines.render(PipelineKind::BankStrips),
        self.water_visible,
      ) {
        pass.set_pipeline(pipeline);
        pass.set_vertex_buffer(0, strips.vertex_buffer.slice(..));
        pass.set_index_buffer(strips.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..strips.index_count, 0, 0..1);
      }
    }

    if let Some(trees) = &self.trees {
      let mut pass = self.begin_opaque_pass(
        &mut encoder,
        "VistaWASM tree pass",
        timing.map(|timer| timer.render_writes(PASS_TREES)),
        false,
      );
      ran |= 1 << PASS_TREES;

      if let (2, Some(pipeline)) = (
        params.tree_style,
        self.pipelines.render(PipelineKind::TreeMesh),
      ) {
        pass.set_pipeline(pipeline);
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

      if let Some(pipeline) = self.pipelines.render(PipelineKind::TreeImpostor) {
        pass.set_pipeline(pipeline);
      }

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

    if let (Some(grass), Some(pipeline)) = (&self.grass, self.pipelines.render(PipelineKind::Grass))
    {
      let mut pass = self.begin_opaque_pass(
        &mut encoder,
        "VistaWASM grass pass",
        timing.map(|timer| timer.render_writes(PASS_GRASS)),
        false,
      );
      ran |= 1 << PASS_GRASS;
      pass.set_pipeline(pipeline);
      pass.set_vertex_buffer(0, self.grass_base_vertex_buffer.slice(..));
      pass.set_vertex_buffer(1, grass.instance_buffer.slice(..));
      pass.draw(0..18, 0..grass.instance_count);
    }

    let Some(cloud_target) = &self.cloud_target else {
      return Ok(());
    };

    // Clouds at reduced resolution; the composite upsamples them. Skipped
    // entirely when there are no clouds.
    if let (true, Some(pipeline)) = (
      params.cloud_coverage > 0.001 && self.uniforms.temporal[0] > 0.5,
      self.pipelines.render(PipelineKind::QuarterClouds),
    ) {
      let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("VistaWASM quarter cloud pass"),
        timestamp_writes: timing.map(|timer| timer.render_writes(PASS_QUARTER_CLOUDS)),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
          view: &cloud_target.quarter_view,
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
      ran |= 1 << PASS_QUARTER_CLOUDS;
      pass.set_pipeline(pipeline);
      pass.set_bind_group(0, &self.frame_bind_group, &[]);
      pass.set_bind_group(1, &self.world_bind_group, &[]);
      pass.set_bind_group(2, &self.shadow_bind_group, &[]);
      pass.set_bind_group(3, &cloud_target.quarter_bind_group, &[]);
      pass.draw(0..3, 0..1);
    }

    if let (true, Some(pipeline)) = (
      params.cloud_coverage > 0.001,
      self.pipelines.render(PipelineKind::Clouds),
    ) {
      let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("VistaWASM cloud pass"),
        timestamp_writes: timing.map(|timer| timer.render_writes(PASS_CLOUDS)),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
          view: &cloud_target.views[cloud_target.current],
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
      ran |= 1 << PASS_CLOUDS;
      pass.set_pipeline(pipeline);
      pass.set_bind_group(0, &self.frame_bind_group, &[]);
      pass.set_bind_group(1, &self.world_bind_group, &[]);
      pass.set_bind_group(2, &self.shadow_bind_group, &[]);
      pass.set_bind_group(
        3,
        &cloud_target.cloud_bind_groups[cloud_target.current],
        &[],
      );
      pass.draw(0..3, 0..1);
    }

    // Sky, clouds, fog, precipitation, and tone mapping onto the canvas.
    if let Some(pipeline) = self.pipelines.render(PipelineKind::Composite) {
      let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("VistaWASM composite pass"),
        timestamp_writes: timing.map(|timer| timer.render_writes(PASS_COMPOSITE)),
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
      ran |= 1 << PASS_COMPOSITE;
      pass.set_pipeline(pipeline);
      pass.set_bind_group(0, &self.frame_bind_group, &[]);
      pass.set_bind_group(1, &self.world_bind_group, &[]);
      pass.set_bind_group(2, &self.shadow_bind_group, &[]);
      pass.set_bind_group(
        3,
        &cloud_target.composite_bind_groups[cloud_target.current],
        &[],
      );
      pass.draw(0..3, 0..1);
    }

    // Transparent water on top, depth-tested against the opaque scene.
    if self.water_visible {
      let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("VistaWASM water pass"),
        timestamp_writes: timing.map(|timer| timer.render_writes(PASS_WATER)),
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
      ran |= 1 << PASS_WATER;
      pass.set_bind_group(0, &self.frame_bind_group, &[]);
      pass.set_bind_group(1, &self.world_bind_group, &[]);
      pass.set_bind_group(2, &self.shadow_bind_group, &[]);
      let ocean = if self.uniforms.sea_ice[0] > 0.5 {
        PipelineKind::SeaIceOcean
      } else {
        PipelineKind::OpenOcean
      };
      let meshes = [
        (ocean, Some(&self.ocean)),
        (PipelineKind::InlandWater, self.rivers.as_ref()),
        (PipelineKind::Falls, self.falls.as_ref()),
      ];

      for (kind, mesh) in meshes {
        let (Some(mesh), Some(pipeline)) = (mesh, self.pipelines.render(kind)) else {
          continue;
        };

        pass.set_pipeline(pipeline);
        pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
        pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..mesh.index_count, 0, 0..1);
      }
    }

    if let (Some(target), true, Some(pipeline)) = (
      &self.lens_target,
      present,
      self.pipelines.render(PipelineKind::Present),
    ) {
      let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("VistaWASM present pass"),
        timestamp_writes: timing.map(|timer| timer.render_writes(PASS_PRESENT)),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
          view: &canvas_view,
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
      ran |= 1 << PASS_PRESENT;
      pass.set_pipeline(pipeline);
      pass.set_bind_group(0, &self.frame_bind_group, &[]);
      pass.set_bind_group(1, &self.world_bind_group, &[]);
      pass.set_bind_group(2, &self.shadow_bind_group, &[]);
      pass.set_bind_group(3, &target.bind_group, &[]);
      pass.draw(0..3, 0..1);
    }

    if let Some(timer) = timing {
      timer.resolve(&mut encoder);
    }

    self.queue.submit(Some(encoder.finish()));

    if let Some(timer) = timing {
      timer.read_back(ran);
    }

    self.frames_in_flight.fetch_add(1, Ordering::AcqRel);
    let frames_in_flight = Arc::clone(&self.frames_in_flight);
    self.queue.on_submitted_work_done(move || {
      frames_in_flight.fetch_sub(1, Ordering::AcqRel);
    });
    self.queue.present(surface_texture);
    self.warm_up(&params.likely);
    Ok(())
  }

  /// Begin a pass drawing opaque geometry into the HDR and depth targets,
  /// clearing them first when `clear` is set.
  fn begin_opaque_pass<'encoder>(
    &self,
    encoder: &'encoder mut wgpu::CommandEncoder,
    label: &str,
    timestamp_writes: Option<wgpu::RenderPassTimestampWrites<'_>>,
    clear: bool,
  ) -> wgpu::RenderPass<'encoder> {
    let (colour_load, depth_load) = if clear {
      (
        wgpu::LoadOp::Clear(wgpu::Color::BLACK),
        wgpu::LoadOp::Clear(1.0),
      )
    } else {
      (wgpu::LoadOp::Load, wgpu::LoadOp::Load)
    };
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
      label: Some(label),
      timestamp_writes,
      color_attachments: &[Some(wgpu::RenderPassColorAttachment {
        view: &self.hdr_view,
        depth_slice: None,
        resolve_target: None,
        ops: wgpu::Operations {
          load: colour_load,
          store: wgpu::StoreOp::Store,
        },
      })],
      depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
        view: &self.depth_view,
        depth_ops: Some(wgpu::Operations {
          load: depth_load,
          store: wgpu::StoreOp::Store,
        }),
        stencil_ops: None,
      }),
      ..Default::default()
    });
    pass.set_bind_group(0, &self.frame_bind_group, &[]);
    pass.set_bind_group(1, &self.world_bind_group, &[]);
    pass.set_bind_group(2, &self.shadow_bind_group, &[]);
    pass.forget_lifetime().into()
  }

  /// The latest per-pass GPU timings, if the browser supports them.
  pub fn pass_times(&self) -> Option<vista_types::GpuPassTimes> {
    self
      .timer
      .as_ref()?
      .latest
      .lock()
      .ok()
      .and_then(|slot| *slot)
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
    self.canvas_width = pixel_width;
    self.canvas_height = pixel_height;
    self.create_scaled_targets();
    Ok(())
  }

  /// Render the scene at `scale` (0.25 to 1) of the canvas resolution; the
  /// final pass upscales and sharpens it.
  pub fn set_render_scale(&mut self, scale: f32) {
    let scale = scale.clamp(0.25, 1.0);

    if (scale - self.render_scale).abs() < 0.001 {
      return;
    }

    self.render_scale = scale;
    self.create_scaled_targets();
  }

  /// The fraction of the canvas resolution currently rendered.
  pub fn render_scale(&self) -> f32 {
    self.render_scale
  }

  /// (Re)create the depth, HDR, cloud, and final-pass targets at the
  /// internal render size.
  fn create_scaled_targets(&mut self) {
    self.width = scaled_extent(self.canvas_width, self.render_scale);
    self.height = scaled_extent(self.canvas_height, self.render_scale);
    let (depth_view, hdr_view) = create_render_targets(&self.device, self.width, self.height);
    self.depth_view = depth_view;
    self.hdr_view = hdr_view;
    // The cloud, composite, and final-pass bind groups read the old
    // targets, so rebuild them on the next frame.
    self.cloud_target = None;
    self.lens_target = None;
  }
}

fn scaled_extent(value: u32, device_pixel_ratio: f32) -> u32 {
  ((value as f32 * device_pixel_ratio).round() as u32).max(1)
}
