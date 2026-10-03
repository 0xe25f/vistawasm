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
use crate::render::tree_growth::{Age, AGES, VARIANTS};
use crate::render::tree_models::{
  build_library, mesh_slot, SpeciesModel, TreeLibrary, TreeMesh, TreeSpecies, IMPOSTOR_CELL,
  IMPOSTOR_MIPS, MESH_SLOTS, SPECIES_COUNT,
};
use crate::render::water::{build_ocean_grid, WaterVertex, OCEAN_SNAP_METRES};
use crate::terrain::biomes::{SurfaceSample, DEFAULT_SEA_LEVEL_CELSIUS};
use crate::terrain::HeightMap;

mod weather;
pub use weather::SurfaceWeatherStep;
use weather::{GroundLayers, SurfaceWeatherMap};

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
  ground: [f32; 4],
  vegetation3: [f32; 4],
  rock: [f32; 4],
  regional: [f32; 4],
  air: [f32; 4],
  light: [f32; 4],
  gust: [f32; 4],
  weather4: [f32; 4],
  clouds5: [f32; 4],
  alto: [f32; 4],
  rivers2: [f32; 4],
  waterside: [f32; 4],
  mouths: [[f32; 4]; 16],
}

const _: () = assert!(std::mem::size_of::<FrameUniforms>() == 1248);

/// Static world data: species bounds, wind and tints, terrain mapping,
/// material tints, and the canopy layer's colour per species. Mirrors
/// `WorldInfo` in `common.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct WorldInfo {
  species: [[f32; 4]; 8],
  species_tint: [[f32; 4]; 8],
  terrain: [f32; 4],
  terrain2: [f32; 4],
  material_tints: [[f32; 4]; vista_types::MATERIAL_COUNT],
  /// x: variants per species in the impostor atlas.
  trees: [f32; 4],
  /// Per species, the canopy layer's colour, averaged from the impostors
  /// on the GPU after each bake (see `GpuContext::bake_impostors`), so it
  /// is written only up to here from the CPU.
  species_canopy: [[f32; 4]; 8],
}

const _: () = assert!(std::mem::size_of::<WorldInfo>() == 624);
/// Bytes of [`WorldInfo`] the CPU writes: all but `species_canopy`.
const WORLD_INFO_CPU_BYTES: u64 = 496;

/// Tree culling parameters. Mirrors `CullParams` in `tree_cull.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct CullParams {
  planes: [[f32; 4]; 6],
  camera: [f32; 4],
  params: [f32; 4],
  bounds: [[f32; 4]; 8],
  lod: [f32; 4],
  shadow: [f32; 4],
  stream: [u32; 4],
  thin: [f32; 4],
  budget: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<CullParams>() == 336);

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
  /// The regional weather map's corner (xy), 1 / its size (z), and 1 when
  /// it is read (w).
  pub regional: [f32; 4],
  /// Mie colour (rgb) and phase asymmetry (w).
  pub air: [f32; 4],
  /// Direct sun, shadow strength, indirect light, and how flat the sky
  /// light is under a deck.
  pub light: [f32; 4],
  /// The gust front: distance travelled, gustiness, mean wind, and 1 when
  /// on.
  pub gust: [f32; 4],
  /// 1 when the surface weather map is read, whitecaps, blown spray.
  pub sea: [f32; 4],
  /// Added to the terrain shadows' softness under cloud.
  pub shadow_softening: f32,
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
  /// Eddies and vortices in rivers, 0 to 1 (`WaterOptions::eddies`). At 0
  /// the water shader skips all eddy code.
  pub eddies: f32,
  /// Refraction and caustics in shallow water, 0 to 1
  /// (`WaterOptions::refraction`).
  pub refraction: f32,
  /// The river mouths whose plumes tint the sea (`water::plume_mouths`).
  pub mouths: [[f32; 4]; 16],
  /// The stream stones' lattice seed (`BoulderRules::stones`), which the
  /// water shader breaks its foam on.
  pub stones: u32,
}

/// The canopy layer this frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct CanopyFrame {
  /// Crown area per square metre per unit of cover share, at the tree
  /// density (`lattice::canopy_density`); 0 without trees.
  pub density: f32,
  /// Where individual trees give way to the canopy layer, in metres.
  pub distance: f32,
  /// Whether the layer is drawn.
  pub drawn: bool,
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
  /// Effective altocumulus and altostratus amounts (0 when clouds are
  /// off).
  pub alto_amounts: [f32; 2],
  /// Cloud controls.
  pub clouds: CloudsOptions,
  /// Tree style: 0 billboard, 1 cross-quad, 2 mesh.
  pub tree_style: u32,
  /// Time canopy meshes, understorey meshes and impostors as separate
  /// passes.
  pub split_tree_timing: bool,
  /// Flora controls.
  pub flora: FloraOptions,
  /// Grass fade-out distance.
  pub grass_view_distance_metres: f32,
  /// The share of an ideal meadow's ground its tufts cover near the
  /// camera (0 while grass is off), for the ground's grass sheen.
  pub grass_cover: f32,
  /// How tall meadow grass grows (see `lattice::grass_height`).
  pub grass_height: f32,
  /// Tiles to stream this frame, and the near radii.
  pub vegetation: crate::render::vegetation::StreamFrame,
  /// The canopy layer.
  pub canopy: CanopyFrame,
  /// Bare rock: the beds' rise per metre along x and z, their spacing in
  /// metres, and the angle of the sunward side in radians (see
  /// `materials.wgsl`).
  pub rock: [f32; 4],
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
  /// The terrain mesh being drawn, for grounding trees and grass (see
  /// `terrain_mesh::mesh_ground_uniform`).
  pub mesh_ground: [f32; 4],
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

impl FrameParams {
  /// Whether the cloud pass runs: low clouds, or a mid-level layer.
  fn clouds_drawn(&self) -> bool {
    self.cloud_coverage > 0.001 || self.alto_amounts[0].max(self.alto_amounts[1]) > 0.001
  }
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
  /// The far bands' triangles, which the canopy layer draws.
  canopy_index_buffer: wgpu::Buffer,
  canopy_index_count: u32,
}

/// GPU-side tree instances, culling outputs, and indirect arguments.
struct TreesGpu {
  mesh_out: wgpu::Buffer,
  impostor_out: wgpu::Buffer,
  shadow_out: wgpu::Buffer,
  args_buffer: wgpu::Buffer,
  cull_params_buffer: wgpu::Buffer,
  /// Made with the mesh lists ([`GpuContext::layout_mesh_lists`]).
  cull_bind_group: Option<wgpu::BindGroup>,
  /// The draw counts read back for the triangle budget.
  readback: DrawReadback,
  /// The static trees (the far set, or the host's), then every entry of
  /// the tile pool.
  instance_count: u32,
  /// The static trees.
  static_count: u32,
  /// Each mesh list's first slot and slots (see [`MESH_LISTS`]).
  mesh_offsets: Vec<u32>,
  mesh_capacities: Vec<u32>,
  /// The same, for the cull pass.
  lists_buffer: wgpu::Buffer,
  /// Per species, the trees that may be drawn as meshes at most.
  species_slots: [u32; SPECIES_COUNT],
  /// Entries in the tile pool.
  pool: u32,
  /// Tile counts' stand-in when nothing streams.
  no_counts: Option<wgpu::Buffer>,
  /// Species that may be drawn, one bit each.
  present: u32,
  instance_buffer: wgpu::Buffer,
  grounding: Grounding,
  /// Streamed tiles, for procedural trees.
  stream: Option<TreeStream>,
}

/// The tile pool of streamed trees and what the generator needs.
struct TreeStream {
  layout: crate::render::lattice::TileLayout,
  rules: crate::render::vegetation::TreeRules,
  /// Trees in each slot (atomic on the GPU).
  counts: wgpu::Buffer,
  jobs: wgpu::Buffer,
  params: wgpu::Buffer,
}

/// Mirrors `TreeParams` in `tree_generate.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct TreeGenerateParams {
  terrain: [f32; 4],
  terrain2: [f32; 4],
  mesh: [f32; 4],
  rules: [f32; 4],
  shape: [u32; 4],
  max_slopes: [[f32; 4]; 2],
  roots: [[f32; 4]; 2],
}

/// Mirrors `Job` in `generate_common.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct GpuJob {
  tile: [i32; 2],
  first: u32,
  capacity: u32,
  slot: u32,
  keep: f32,
  unused: [u32; 2],
}

/// Most tiles of one kind generated in a frame.
const MAX_JOBS: usize = crate::render::lattice::TILES_PER_FRAME * 2;

/// Mirrors `GrassParams` in `grass_generate.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct GrassGenerateParams {
  terrain: [f32; 4],
  terrain2: [f32; 4],
  mesh: [f32; 4],
  rules: [f32; 4],
  shape: [u32; 4],
  camera: [f32; 4],
  view: [f32; 4],
  planes: [[f32; 4]; 6],
  classes: [[u32; 4]; 8],
}

/// GPU-side grass: reeds placed on the CPU, and the tile pool of streamed
/// tufts with its culled, drawn list.
struct GrassGpu {
  reeds: Option<wgpu::Buffer>,
  reed_count: u32,
  grounding: Grounding,
  stream: Option<GrassStream>,
}

/// The tile pool of streamed tufts and what the generator and the cull
/// pass need.
struct GrassStream {
  layout: crate::render::lattice::TileLayout,
  rules: crate::render::grass::GrassRules,
  tufts: wgpu::Buffer,
  counts: wgpu::Buffer,
  drawn: wgpu::Buffer,
  args: wgpu::Buffer,
  jobs: wgpu::Buffer,
  params: wgpu::Buffer,
  grounding: Grounding,
}

/// Mirrors `BoulderParams` in `boulder_generate.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct BoulderGenerateParams {
  terrain: [f32; 4],
  terrain2: [f32; 4],
  mesh: [f32; 4],
  rules: [f32; 4],
  shape: [u32; 4],
  camera: [f32; 4],
  tall: [[f32; 4]; 2],
  planes: [[f32; 4]; 6],
}

/// Boulder draw lists: finest, middle and coarsest level per variant,
/// then one shadow list per variant (drawn at the middle level). Their
/// capacities mirror `boulder_generate.wgsl`.
const BOULDER_LISTS: usize = crate::render::boulders::VARIANTS * 4;
const BOULDER_LIST_CAPACITY: [u32; 3] = [1024, 2048, 4096];
const BOULDER_VARIANT_ENTRIES: u32 = 7168;
const BOULDER_SHADOW_CAPACITY: u32 = 2048;
const _: () = assert!(
  BOULDER_LIST_CAPACITY[0] + BOULDER_LIST_CAPACITY[1] + BOULDER_LIST_CAPACITY[2]
    == BOULDER_VARIANT_ENTRIES
);
/// Words in the boulders' indirect arguments: an indexed draw per list,
/// then the count of every boulder drawn.
const BOULDER_ARGS_WORDS: usize = BOULDER_LISTS * 5 + 1;
/// Bytes per boulder: `Boulder`'s eight floats.
const BOULDER_BYTES: u64 = 32;
const _: () =
  assert!(std::mem::size_of::<crate::render::boulders::Boulder>() as u64 == BOULDER_BYTES);

/// The boulder meshes and, when boulders are streamed, their tile pool.
struct BouldersGpu {
  vertex_buffer: wgpu::Buffer,
  index_buffer: wgpu::Buffer,
  /// Per variant and level: first index, index count, base vertex.
  ranges: [[(u32, u32, i32); crate::render::boulders::LODS]; crate::render::boulders::VARIANTS],
  stream: Option<BoulderStream>,
}

/// The tile pool of streamed boulders, the culled draw lists, and what the
/// generator and the cull pass need.
struct BoulderStream {
  layout: crate::render::lattice::TileLayout,
  rules: crate::render::boulders::BoulderRules,
  pool: wgpu::Buffer,
  counts: wgpu::Buffer,
  drawn: wgpu::Buffer,
  args: wgpu::Buffer,
  jobs: wgpu::Buffer,
  params: wgpu::Buffer,
  grounding: Grounding,
}

impl BouldersGpu {
  /// Each list's indirect draw, with no instances yet, and the count of
  /// boulders drawn at 0: what the cull pass starts from each frame.
  fn cleared_args(&self) -> [u32; BOULDER_ARGS_WORDS] {
    let mut words = [0u32; BOULDER_ARGS_WORDS];

    for list in 0..BOULDER_LISTS {
      let (variant, lod) = if list < 18 {
        (list / 3, list % 3)
      } else {
        (list - 18, 1)
      };
      let (first, count, base) = self.ranges[variant][lod];
      words[list * 5..list * 5 + 5].copy_from_slice(&[count, 0, first, base as u32, 0]);
    }

    words
  }

  /// The first drawn entry of each list.
  fn list_start(list: usize) -> u32 {
    let variants = crate::render::boulders::VARIANTS as u32;

    if list < 18 {
      let starts = [
        0,
        BOULDER_LIST_CAPACITY[0],
        BOULDER_LIST_CAPACITY[0] + BOULDER_LIST_CAPACITY[1],
      ];
      (list / 3) as u32 * BOULDER_VARIANT_ENTRIES + starts[list % 3]
    } else {
      variants * BOULDER_VARIANT_ENTRIES + (list - 18) as u32 * BOULDER_SHADOW_CAPACITY
    }
  }
}

/// Grounding parameters for one instance buffer. Mirrors `GroundParams`
/// in `grounding.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct GroundParams {
  terrain: [f32; 4],
  terrain2: [f32; 4],
  mesh: [f32; 4],
  shape: [u32; 4],
  roots: [[f32; 4]; 2],
}

const _: () = assert!(std::mem::size_of::<GroundParams>() == 96);

/// Standing one instance buffer on the drawn terrain.
struct Grounding {
  params: wgpu::Buffer,
  /// The mesh and height texture version the instances were last
  /// grounded on; they are grounded again when either changes.
  done_for: Option<([f32; 4], u64)>,
}

impl Grounding {
  fn new(device: &wgpu::Device, label: &str) -> Self {
    Self {
      params: device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: std::mem::size_of::<GroundParams>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
      }),
      done_for: None,
    }
  }
}

/// An indexed mesh.
struct IndexedMesh {
  vertex_buffer: wgpu::Buffer,
  index_buffer: wgpu::Buffer,
  index_count: u32,
}

/// The mesh slot a mesh list draws: its own, or for an understorey list
/// its species' and variant's young, full mesh.
fn list_mesh(list: usize) -> usize {
  if list < MESH_SLOTS {
    return list;
  }

  let understorey = list - MESH_SLOTS;
  mesh_slot(
    understorey / VARIANTS,
    understorey % VARIANTS,
    Age::Young as usize,
    0,
  )
}

/// Each mesh list's first slot and slots: for each species present,
/// the full and light meshes of every variant grown and age class, and
/// with streamed tiles the understorey's young meshes. Each list has
/// twice its share of the species' trees, so an uneven mix of ages still
/// fits; a full list passes trees to the lighter level, and on to
/// impostors.
fn mesh_list_layout(
  present: u32,
  species_slots: &[u32; SPECIES_COUNT],
  pool: u32,
  streamed: bool,
  variants: usize,
) -> (Vec<u32>, Vec<u32>) {
  let mut capacities = vec![0u32; MESH_LISTS];

  for species in 0..SPECIES_COUNT {
    if present & 1 << species == 0 {
      continue;
    }

    // Shrubs are never stunted.
    let ages = if species == TreeSpecies::Shrub as usize {
      AGES - 1
    } else {
      AGES
    };
    let lists = (variants * ages) as u32;

    for (lod, most) in [(0, LOD0_MESH_SLOTS), (1, LOD1_MESH_SLOTS)] {
      let total = species_slots[species].min(most);
      let share = (total * 2).div_ceil(lists).min(total);

      for variant in 0..variants {
        for age in 0..ages {
          capacities[mesh_slot(species, variant, age, lod)] = share;
        }
      }
    }

    if streamed {
      let total = pool.min(UNDERSTOREY_MESH_SLOTS);

      for variant in 0..variants {
        capacities[MESH_SLOTS + species * VARIANTS + variant] =
          (total * 3 / 2).div_ceil(variants as u32).min(total);
      }
    }
  }

  let mut running = 0;
  let offsets = capacities
    .iter()
    .map(|capacity| {
      let offset = running;
      running += capacity;
      offset
    })
    .collect();
  (offsets, capacities)
}

/// Times the GPU work between two points in the queue with timestamp
/// queries, adding the milliseconds to a total once they come back.
struct Stopwatch {
  query_set: wgpu::QuerySet,
  resolve: wgpu::Buffer,
  readback: wgpu::Buffer,
  period: f32,
}

impl Stopwatch {
  /// Start timing: `None` without timestamp queries.
  fn start(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
    if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
      return None;
    }

    let watch = Self {
      query_set: device.create_query_set(&wgpu::QuerySetDescriptor {
        label: Some("VistaWASM bake timestamps"),
        ty: wgpu::QueryType::Timestamp,
        count: 2,
      }),
      resolve: device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("VistaWASM bake timestamp resolve"),
        size: 16,
        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
      }),
      readback: device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("VistaWASM bake timestamp readback"),
        size: 16,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
      }),
      period: queue.get_timestamp_period(),
    };
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
      label: Some("VistaWASM bake start"),
    });
    watch.mark(&mut encoder, 0);
    queue.submit(Some(encoder.finish()));
    Some(watch)
  }

  fn mark(&self, encoder: &mut wgpu::CommandEncoder, index: u32) {
    encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
      label: Some("VistaWASM bake timestamp"),
      timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
        query_set: &self.query_set,
        beginning_of_pass_write_index: Some(index),
        end_of_pass_write_index: None,
      }),
    });
  }

  /// Stop timing at the end of `encoder`'s work, before it is submitted.
  fn stop(self, encoder: &mut wgpu::CommandEncoder, total: Arc<Mutex<f64>>) -> impl FnOnce() {
    self.mark(encoder, 1);
    encoder.resolve_query_set(&self.query_set, 0..2, &self.resolve, 0);
    encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.readback, 0, 16);
    // Mapped once submitted.
    move || {
      let readback = self.readback.clone();
      let period = self.period;
      self
        .readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
          if result.is_err() {
            return;
          }

          if let Ok(range) = readback.slice(..).get_mapped_range() {
            let ticks: &[u64] = bytemuck::cast_slice(&range);
            let millis = ticks[1].saturating_sub(ticks[0]) as f64 * f64::from(period) / 1e6;

            if let Ok(mut total) = total.lock() {
              *total += millis;
            }
          }

          readback.unmap();
        });
    }
  }
}

/// Growing the trees' variants. The first variant of every species grows
/// before the first frame; the rest grow after it, one mesh a frame, and
/// join the library together once every species has them.
struct TreeGrowth {
  /// Variants per species wanted (`FloraOptions::variantsPerSpecies`).
  wanted: usize,
  /// Variants per species in the library now.
  ready: usize,
  /// The models growing, and the next species, variant and age class.
  growing: Option<(Vec<SpeciesModel>, [usize; 3])>,
  /// Milliseconds spent growing: before the first frame, and after it.
  millis: [f64; 2],
  /// Milliseconds the GPU took to bake the impostors, in all.
  bake_millis: Arc<Mutex<f64>>,
}

/// Milliseconds since the page loaded, from the monotonic page clock:
/// unlike `Date.now()`, it does not jump when the system clock is set.
pub fn now_millis() -> f64 {
  web_sys::window()
    .and_then(|window| window.performance())
    .map_or(0.0, |performance| performance.now())
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
const TIMED_PASSES: u32 = 15;
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
const PASS_GENERATION: u32 = 10;
/// With split tree timing, `PASS_TREES` times the canopy meshes alone.
const PASS_UNDERSTOREY: u32 = 11;
const PASS_TREE_IMPOSTORS: u32 = 12;
const PASS_BOULDERS: u32 = 13;
const PASS_SURFACE_WEATHER: u32 = 14;

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
            let split = ran & (1 << PASS_TREE_IMPOSTORS) != 0;
            let times = vista_types::GpuPassTimes {
              shadows: ms(PASS_TREE_SHADOW),
              tree_culling: ms(PASS_TREE_CULL),
              terrain: ms(PASS_TERRAIN),
              trees: ms(PASS_TREES) + ms(PASS_UNDERSTOREY) + ms(PASS_TREE_IMPOSTORS),
              tree_meshes: if split { ms(PASS_TREES) } else { 0.0 },
              understorey: ms(PASS_UNDERSTOREY),
              tree_impostors: ms(PASS_TREE_IMPOSTORS),
              grass: ms(PASS_GRASS),
              clouds: ms(PASS_QUARTER_CLOUDS) + ms(PASS_CLOUDS),
              sky_and_fog: ms(PASS_COMPOSITE),
              water: ms(PASS_WATER),
              present: ms(PASS_PRESENT),
              generation: ms(PASS_GENERATION),
              boulders: ms(PASS_BOULDERS),
              surface_weather: ms(PASS_SURFACE_WEATHER),
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

/// The trees' draw arguments, read back a frame or more late for the
/// triangle budget. A frame is copied only when the previous copy has
/// arrived, so rendering never waits for it.
struct DrawReadback {
  buffer: wgpu::Buffer,
  busy: Arc<AtomicBool>,
  latest: Arc<Mutex<Option<Vec<u32>>>>,
}

impl DrawReadback {
  fn new(device: &wgpu::Device) -> Self {
    Self {
      buffer: device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("VistaWASM tree draw readback"),
        size: (INDIRECT_WORDS * 4) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
      }),
      busy: Arc::new(AtomicBool::new(false)),
      latest: Arc::new(Mutex::new(None)),
    }
  }

  /// Copy this frame's draw arguments, unless a copy is on its way back.
  fn copy(&self, encoder: &mut wgpu::CommandEncoder, args: &wgpu::Buffer) -> bool {
    if self.busy.load(Ordering::Acquire) {
      return false;
    }

    encoder.copy_buffer_to_buffer(args, 0, &self.buffer, 0, self.buffer.size());
    true
  }

  /// Read the copy back once the GPU has finished.
  fn read_back(&self) {
    self.busy.store(true, Ordering::Release);
    let buffer = self.buffer.clone();
    let busy = Arc::clone(&self.busy);
    let latest = Arc::clone(&self.latest);
    self
      .buffer
      .slice(..)
      .map_async(wgpu::MapMode::Read, move |result| {
        if result.is_ok() {
          if let Ok(view) = buffer.slice(..).get_mapped_range() {
            let words: Vec<u32> = bytemuck::cast_slice(&view).to_vec();
            drop(view);

            if let Ok(mut slot) = latest.lock() {
              *slot = Some(words);
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

/// Mesh lists: canopy trees per mesh slot (species, variant, age class
/// and level of detail, `tree_models::mesh_slot`), then understorey
/// saplings per species and variant.
const MESH_LISTS: usize = MESH_SLOTS + SPECIES_COUNT * VARIANTS;
/// Words in the indirect argument buffer: an indexed mesh draw per list
/// (5 words each), then one impostor draw and one shadow draw (4 each).
/// The triangle budget reads them all back.
const INDIRECT_WORDS: usize = MESH_LISTS * 5 + 4 * 2;
const IMPOSTOR_ARGS_BASE: usize = MESH_LISTS * 5;
const SHADOW_ARGS_BASE: usize = IMPOSTOR_ARGS_BASE + 4;
/// Mesh slots per species for understorey saplings: they are meshes only
/// within 15 m.
const UNDERSTOREY_MESH_SLOTS: u32 = 4_096;
/// Full meshes are drawn within this distance, lighter ones beyond.
const LOD0_METRES: f32 = 50.0;
/// Trees are impostors beyond this distance, or the mesh distance when
/// less.
const IMPOSTOR_METRES: f32 = 150.0;
/// Most full meshes per species: more than stand within 50 m.
const LOD0_MESH_SLOTS: u32 = 2_048;
/// Most light meshes per species: more than stand within 150 m, less
/// than the triangle budget allows.
const LOD1_MESH_SLOTS: u32 = 12_288;
/// Beyond this distance, understorey saplings are impostor cards.
const UNDERSTOREY_CARD_METRES: f32 = 15.0;
/// Drawn tufts within 15 m of the camera, which are two crossed quads:
/// every lattice point within 15 m, with room to spare. The single cards
/// beyond follow them in the drawn list.
const GRASS_NEAR_SLOTS: u32 = 8_192;
/// Bytes per drawn tuft: `FloraInstance`'s seven floats.
const DRAWN_TUFT_BYTES: u64 = 28;
const _: () = assert!(std::mem::size_of::<FloraInstance>() as u64 == DRAWN_TUFT_BYTES);
/// Floats per drawn tree: `TreeInstance`'s eight, the species word
/// holding species plus fade / 2, and the extra crown width.
const DRAWN_TREE_FLOATS: u64 = 9;
/// Most mesh slots per species: more would already cost far more than
/// the triangle budget allows, and the rest draw as impostors.
const STREAMED_MESH_SLOTS: u32 = 24_576;
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
  /// The grass generator and cull pass (compute).
  grass_generate: OnceCell<wgpu::ShaderModule>,
  boulders: OnceCell<wgpu::ShaderModule>,
  /// The boulder generator and cull pass (compute).
  boulder_generate: OnceCell<wgpu::ShaderModule>,
}

/// Bind group layouts.
struct Layouts {
  frame: wgpu::BindGroupLayout,
  world: wgpu::BindGroupLayout,
  /// The grass density mask the terrain's grass sheen reads.
  grass_mask: wgpu::BindGroupLayout,
  shadow: wgpu::BindGroupLayout,
  composite: wgpu::BindGroupLayout,
  cloud: wgpu::BindGroupLayout,
  cloud_quarter: wgpu::BindGroupLayout,
  terrain_shadow: wgpu::BindGroupLayout,
  surface_weather: wgpu::BindGroupLayout,
  lens: wgpu::BindGroupLayout,
  /// The scene copy that water reflects.
  reflection: wgpu::BindGroupLayout,
}

/// Pipeline layouts, made once so each pipeline created later shares them.
struct PipelineLayouts {
  /// Frame, world and shadow-receiver groups.
  receivers: wgpu::PipelineLayout,
  /// Shadow receivers plus the grass density mask.
  terrain: wgpu::PipelineLayout,
  /// Frame and world groups: the tree shadow pass cannot bind the shadow
  /// map it renders into, and the impostor bake needs no shadows.
  basic: wgpu::PipelineLayout,
  composite: wgpu::PipelineLayout,
  lens: wgpu::PipelineLayout,
  cloud: wgpu::PipelineLayout,
  cloud_quarter: wgpu::PipelineLayout,
  terrain_shadow: wgpu::PipelineLayout,
  surface_weather: wgpu::PipelineLayout,
  /// Shadow receivers plus the scene copy they reflect.
  water: wgpu::PipelineLayout,
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
  /// Variants whose impostors have been rendered, one bit per species
  /// and variant (`species * VARIANTS + variant`). They are rendered, with
  /// the flora layers they sample, when a species first appears.
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
  /// The surface texture, distance to water, snow cover and bankside
  /// greening (see `GroundData::banks`), and tree cover
  /// (`flora::bake_cover`).
  ground_layers: GroundLayers,
  /// The regional weather map.
  regional_texture: wgpu::Texture,
  regional_view: wgpu::TextureView,
  /// Wet ground, puddles and snow depth.
  surface_weather: SurfaceWeatherMap,
  /// The grass texture (`grass::bake_grass`), read only by the grass
  /// generator.
  grass_view: Option<wgpu::TextureView>,
  /// The grass density mask (`GroundData::grass_mask`), read by the grass
  /// generator and the terrain; 1 x 1 and neutral without one.
  grass_mask_view: wgpu::TextureView,
  /// The terrain's bind group for `grass_mask_view` and the channel
  /// field.
  grass_mask_group: wgpu::BindGroup,
  /// The channel distance field's atlas, and its slot words.
  channel_field: (wgpu::TextureView, wgpu::Buffer),
  /// Whether `grass_mask_view` is the neutral one.
  grass_mask_neutral: bool,
  /// The drawn channels, binned for the generators.
  channel_bins: Option<wgpu::Buffer>,
  /// 1 / metres per height texel, as the CPU mirror multiplies by it.
  texel_inverse: f32,
  height_size: (u32, u32),
  height_version: u64,
  terrain_shadow: TerrainShadow,
  tree_shadow_map: TreeShadowMap,
  cloud_target: Option<CloudTarget>,
  lens_target: Option<LensTarget>,
  /// The half-resolution scene copy water reflects, and its bind group.
  reflection: Option<(wgpu::TextureView, wgpu::BindGroup)>,
  /// The previous frame's view-projection, for reusing its clouds.
  previous_view_proj: [f32; 16],
  /// Counts cloud frames, to choose which pixel of each block is marched.
  cloud_frame: u32,
  /// Each species' model: grown, or the host's.
  tree_models: Vec<SpeciesModel>,
  /// Variants still to grow after the first frame.
  tree_growth: TreeGrowth,
  tree_mesh: IndexedMesh,
  /// `(first_index, index_count, base_vertex)` per mesh slot.
  tree_ranges: Vec<(u32, u32, i32)>,
  tree_bounds: [(f32, f32); SPECIES_COUNT],
  /// Root radius per species, for grounding.
  tree_roots: [f32; SPECIES_COUNT],
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
  /// So does the mid-level layer, along its own, veered wind.
  alto_offset: [f32; 2],
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
  /// GPU errors no error scope caught, and why the device was lost, for
  /// the wrapper's `gpuError` and `deviceLost` events. The browser reports
  /// them outside any call, so they wait here until the wrapper asks.
  events: Arc<Mutex<GpuEvents>>,
  /// The device's buffer limits, which large buffers are checked against.
  limits: crate::render::gpu_limits::BufferLimits,
  /// Erosion compute pipelines, created when erosion is first requested.
  erosion: Option<ErosionCompute>,
  terrain: Option<TerrainGpu>,
  trees: Option<TreesGpu>,
  grass: Option<GrassGpu>,
  boulders: BouldersGpu,
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
        texture_entry(12, Dim::D2Array, filterable, both),
        texture_entry(13, Dim::D2, filterable, both),
        texture_entry(14, Dim::D2, filterable, both),
      ],
    ),
    // Its own group, bound for the terrain alone: the composite and cloud
    // passes use every sampled texture a stage may have. The grass mask,
    // and the channel distance field's atlas and slots
    // (`render/channel_field.rs`).
    grass_mask: layout(
      "VistaWASM terrain extras layout",
      &[
        texture_entry(0, Dim::D2, filterable, fragment),
        texture_entry(1, Dim::D2, unfilterable, fragment),
        storage_buffer_entry(2),
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
    reflection: layout(
      "VistaWASM reflection layout",
      &[texture_entry(0, Dim::D2, filterable, fragment)],
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
    surface_weather: weather::surface_weather_layout(device),
  }
}

fn view_entry(binding: u32, view: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
  wgpu::BindGroupEntry {
    binding,
    resource: wgpu::BindingResource::TextureView(view),
  }
}

fn buffer_entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
  wgpu::BindGroupEntry {
    binding,
    resource: buffer.as_entire_binding(),
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
  ground: &wgpu::TextureView,
  regional: &wgpu::TextureView,
  surface_weather: &wgpu::TextureView,
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
      view_entry(12, ground),
      view_entry(13, regional),
      view_entry(14, surface_weather),
    ],
  })
}

/// The grass density mask, one byte a texel, with the terrain's bind
/// group for it.
/// The channel distance field's atlas and slot words (see
/// `ChannelField::atlas_image` and `slot_words`).
fn create_channel_field(
  device: &wgpu::Device,
  queue: &wgpu::Queue,
  field: &crate::render::channel_field::ChannelField,
) -> (wgpu::TextureView, wgpu::Buffer) {
  use crate::render::channel_field::{ATLAS_TILES_PER_ROW, FIELD_TILE};
  let (image, height) = field.atlas_image();
  let width = ATLAS_TILES_PER_ROW * FIELD_TILE;
  let texture = create_texture_2d(
    device,
    "VistaWASM channel field atlas",
    width,
    height,
    wgpu::TextureFormat::Rg8Unorm,
    wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
  );
  write_layer(queue, &texture, 0, &image, width * 2, width, height);
  let slots = buffer_with_data(
    device,
    queue,
    "VistaWASM channel field slots",
    bytemuck::cast_slice(&field.slot_words()),
    wgpu::BufferUsages::STORAGE,
  );
  (default_view(&texture), slots)
}

/// The terrain's own bind group: the grass mask and the channel field.
fn create_terrain_extras(
  device: &wgpu::Device,
  layout: &wgpu::BindGroupLayout,
  mask: &wgpu::TextureView,
  (atlas, slots): (&wgpu::TextureView, &wgpu::Buffer),
) -> wgpu::BindGroup {
  device.create_bind_group(&wgpu::BindGroupDescriptor {
    label: Some("VistaWASM terrain extras bind group"),
    layout,
    entries: &[
      view_entry(0, mask),
      view_entry(1, atlas),
      buffer_entry(2, slots),
    ],
  })
}

fn create_grass_mask(
  device: &wgpu::Device,
  queue: &wgpu::Queue,
  (width, height, data): (u32, u32, &[u8]),
) -> wgpu::TextureView {
  let texture = create_texture_2d(
    device,
    "VistaWASM grass mask",
    width,
    height,
    wgpu::TextureFormat::R8Unorm,
    wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
  );
  write_layer(queue, &texture, 0, data, width, width, height);
  default_view(&texture)
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

/// Most uncaptured GPU errors kept until the wrapper collects them.
const MAX_PENDING_GPU_ERRORS: usize = 8;

/// See [`GpuContext::take_events`].
#[derive(Default)]
struct GpuEvents {
  errors: Vec<String>,
  lost: Option<String>,
}

/// What a GPU error says, without the formatting of its source chain,
/// which the browser's message already covers.
fn describe(error: wgpu::Error) -> String {
  match error {
    wgpu::Error::OutOfMemory { .. } => "the GPU ran out of memory.".to_string(),
    wgpu::Error::Validation { description, .. } | wgpu::Error::Internal { description, .. } => {
      description
    }
  }
}

/// Push an out-of-memory scope, then a validation scope inside it.
fn push_error_scopes(device: &wgpu::Device) -> [wgpu::ErrorScopeGuard; 2] {
  [
    device.push_error_scope(wgpu::ErrorFilter::OutOfMemory),
    device.push_error_scope(wgpu::ErrorFilter::Validation),
  ]
}

/// Pop the scopes in the reverse order, and return the first error.
async fn pop_error_scopes(scopes: [wgpu::ErrorScopeGuard; 2]) -> Option<String> {
  let [out_of_memory, validation] = scopes;
  let validation = validation.pop().await;
  let out_of_memory = out_of_memory.pop().await;
  validation.or(out_of_memory).map(describe)
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
  source: std::borrow::Cow<'static, str>,
  entry: &str,
  layout: Option<&wgpu::PipelineLayout>,
) -> wgpu::ComputePipeline {
  let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
    label: Some(label),
    source: wgpu::ShaderSource::Wgsl(source),
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

const TREE_VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
  0 => Float32x3,
  1 => Snorm16x4,
  2 => Float32x2,
  3 => Float32x4,
  11 => Float32x3,
];

const TREE_INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 7] = wgpu::vertex_attr_array![
  4 => Float32x3,
  5 => Float32,
  6 => Float32,
  7 => Float32,
  8 => Float32,
  9 => Float32,
  10 => Float32,
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

const BOULDER_VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 2] = wgpu::vertex_attr_array![
  0 => Float32x3,
  1 => Float32x3,
];

const BOULDER_INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 2] = wgpu::vertex_attr_array![
  2 => Float32x4,
  3 => Float32x4,
];

const BANK_ATTRIBUTES: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
  0 => Float32x3,
  1 => Float32x2,
  2 => Float32x4,
  3 => Float32x2,
];

const WATER_ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
  0 => Float32x3,
  1 => Float32x2,
  2 => Float32x4,
  3 => Float32x4,
  4 => Float32,
];

fn tree_vertex_layout() -> wgpu::VertexBufferLayout<'static> {
  wgpu::VertexBufferLayout {
    array_stride: std::mem::size_of::<crate::render::tree_models::TreeVertex>() as u64,
    step_mode: wgpu::VertexStepMode::Vertex,
    attributes: &TREE_VERTEX_ATTRIBUTES,
  }
}

/// A drawn tree from the cull pass: see [`DRAWN_TREE_FLOATS`].
fn tree_instance_layout() -> wgpu::VertexBufferLayout<'static> {
  wgpu::VertexBufferLayout {
    array_stride: DRAWN_TREE_FLOATS * 4,
    step_mode: wgpu::VertexStepMode::Instance,
    attributes: &TREE_INSTANCE_ATTRIBUTES,
  }
}

fn direction_from_degrees(degrees: f32) -> [f32; 2] {
  let radians = degrees.to_radians();
  [radians.sin(), radians.cos()]
}

/// The mid-level layer's drift direction: the cloud wind veered 20
/// degrees, as winds veer with height. `alto_wind` in `atmosphere.wgsl`
/// turns the cloud wind the same way.
fn alto_wind_direction(cloud_wind_degrees: f32) -> [f32; 2] {
  direction_from_degrees(cloud_wind_degrees + 20.0)
}

/// Height of the mid-level layer: the requested height, kept 300 m above
/// the top of the ordinary cloud layer and, where there is room, 500 m
/// below the cirrus. Storm towers may rise through it.
fn alto_height(clouds: &CloudsOptions) -> f32 {
  let low = clouds.height_metres + clouds.thickness_metres.max(1.0) + 300.0;
  let high = clouds
    .cirrus_height_metres
    .max(clouds.height_metres + clouds.thickness_metres)
    - 500.0;
  clouds.alto_height_metres.clamp(low, high.max(low))
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
    terrain: pipeline_layout(
      "VistaWASM terrain pipeline layout",
      &[frame, &layouts.world, &layouts.shadow, &layouts.grass_mask],
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
    surface_weather: pipeline_layout(
      "VistaWASM surface weather pipeline layout",
      &[&layouts.surface_weather],
    ),
    water: pipeline_layout(
      "VistaWASM water pipeline layout",
      &[frame, &layouts.world, &layouts.shadow, &layouts.reflection],
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

/// The impostor bake pipeline, writing colour and normals. It is made
/// for each bake and dropped after it.
fn create_bake_pipeline(
  device: &wgpu::Device,
  layouts: &PipelineLayouts,
  modules: &Modules,
) -> wgpu::RenderPipeline {
  let bake_buffers = [Some(tree_vertex_layout())];
  let module = modules.trees(device);
  let target = Some(wgpu::ColorTargetState {
    format: wgpu::TextureFormat::Rgba8Unorm,
    blend: None,
    write_mask: wgpu::ColorWrites::ALL,
  });

  device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
    label: Some("VistaWASM impostor bake"),
    layout: Some(&layouts.basic),
    vertex: wgpu::VertexState {
      module,
      entry_point: Some("vertex_bake"),
      compilation_options: Default::default(),
      buffers: &bake_buffers,
    },
    fragment: Some(wgpu::FragmentState {
      module,
      entry_point: Some("fragment_bake"),
      compilation_options: Default::default(),
      targets: &[target.clone(), target],
    }),
    primitive: wgpu::PrimitiveState::default(),
    depth_stencil: Some(wgpu::DepthStencilState {
      format: DEPTH_FORMAT,
      depth_write_enabled: Some(true),
      depth_compare: Some(wgpu::CompareFunction::Less),
      stencil: wgpu::StencilState::default(),
      bias: wgpu::DepthBiasState::default(),
    }),
    multisample: wgpu::MultisampleState::default(),
    multiview_mask: None,
    cache: None,
  })
}

/// The impostor atlas: a colour and a normal layer for each species and
/// variant (`variants` a species), each holding 3 x 3 views.
fn create_impostor_texture(device: &wgpu::Device, variants: usize) -> wgpu::Texture {
  textures::create_texture(
    device,
    "VistaWASM tree impostors",
    wgpu::Extent3d {
      width: IMPOSTOR_CELL[0] * 3,
      height: IMPOSTOR_CELL[1] * 3,
      depth_or_array_layers: (SPECIES_COUNT * variants * 2) as u32,
    },
    wgpu::TextureDimension::D2,
    IMPOSTOR_MIPS,
    wgpu::TextureUsages::RENDER_ATTACHMENT,
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

  fn boulders(&self, device: &wgpu::Device) -> &wgpu::ShaderModule {
    Self::get(
      &self.boulders,
      device,
      "VistaWASM boulder shader",
      shaders::BOULDERS,
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
}

/// A compute pipeline from a module two passes share, compiled once:
/// `cull_main` for the cull pass, else `generate_main`.
fn shared_compute(
  device: &wgpu::Device,
  cell: &OnceCell<wgpu::ShaderModule>,
  body: &'static str,
  cull: bool,
) -> GpuPipeline {
  let module = cell.get_or_init(|| {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
      label: Some("VistaWASM generator"),
      source: wgpu::ShaderSource::Wgsl(shaders::compute_source(body)),
    })
  });
  let entry = if cull { "cull_main" } else { "generate_main" };
  GpuPipeline::Compute(
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
      label: Some(entry),
      layout: None,
      module,
      entry_point: Some(entry),
      compilation_options: wgpu::PipelineCompilationOptions::default(),
      cache: None,
    }),
  )
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
  let boulder_buffers = [
    Some(wgpu::VertexBufferLayout {
      array_stride: std::mem::size_of::<crate::render::boulders::BoulderVertex>() as u64,
      step_mode: wgpu::VertexStepMode::Vertex,
      attributes: &BOULDER_VERTEX_ATTRIBUTES,
    }),
    Some(wgpu::VertexBufferLayout {
      array_stride: BOULDER_BYTES,
      step_mode: wgpu::VertexStepMode::Instance,
      attributes: &BOULDER_INSTANCE_ATTRIBUTES,
    }),
  ];
  let water_buffers = water_buffers();
  let main = ("vertex_main", "fragment_main");
  // Water draws over the finished frame, blended and depth-tested.
  let water = |label, constants, entries| {
    create_pipeline(
      device,
      &layouts.water,
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
  // Shadow casters: depth only, into the tree shadow map.
  let caster = |spec: PipelineSpec| {
    create_pipeline(
      device,
      &layouts.basic,
      PipelineSpec {
        format: None,
        depth_bias: wgpu::DepthBiasState {
          constant: 2,
          slope_scale: 2.5,
          clamp: 0.0,
        },
        ..spec
      },
    )
  };

  match kind {
    PipelineKind::TerrainShadow => GpuPipeline::Compute(compute_pipeline(
      device,
      "VistaWASM terrain shadow bake",
      shaders::TERRAIN_SHADOW.into(),
      "bake",
      Some(&layouts.terrain_shadow),
    )),
    PipelineKind::SurfaceWeather => GpuPipeline::Compute(compute_pipeline(
      device,
      "VistaWASM surface weather",
      shaders::SURFACE_WEATHER.into(),
      "step_main",
      Some(&layouts.surface_weather),
    )),
    PipelineKind::Grounding => GpuPipeline::Compute(compute_pipeline(
      device,
      "VistaWASM grounding",
      shaders::compute_source(shaders::GROUNDING),
      "ground_main",
      None,
    )),
    PipelineKind::TreeCull => GpuPipeline::Compute(compute_pipeline(
      device,
      "VistaWASM tree cull",
      shaders::compute_source(shaders::TREE_CULL),
      "cull_main",
      None,
    )),
    PipelineKind::TreeGenerate => GpuPipeline::Compute(compute_pipeline(
      device,
      "VistaWASM tree generator",
      shaders::compute_source(shaders::TREE_GENERATE),
      "generate_main",
      None,
    )),
    // Generating and culling grass share one module, and so do
    // generating and culling boulders.
    PipelineKind::GrassGenerate | PipelineKind::GrassCull => shared_compute(
      device,
      &modules.grass_generate,
      shaders::GRASS_GENERATE,
      kind == PipelineKind::GrassCull,
    ),
    PipelineKind::BoulderGenerate | PipelineKind::BoulderCull => shared_compute(
      device,
      &modules.boulder_generate,
      shaders::BOULDER_GENERATE,
      kind == PipelineKind::BoulderCull,
    ),
    PipelineKind::Boulders => render(create_pipeline(
      device,
      &layouts.receivers,
      PipelineSpec::opaque(
        "VistaWASM boulders",
        modules.boulders(device),
        main,
        &boulder_buffers,
      ),
    )),
    PipelineKind::BoulderShadow => render(caster(PipelineSpec::opaque(
      "VistaWASM boulder shadows",
      modules.boulders(device),
      ("shadow_main", "shadow_fragment"),
      &boulder_buffers,
    ))),
    PipelineKind::Canopy => render(create_pipeline(
      device,
      &layouts.receivers,
      PipelineSpec {
        cull_mode: Some(wgpu::Face::Back),
        ..PipelineSpec::opaque(
          "VistaWASM canopy",
          modules.terrain(device),
          ("canopy_vertex_main", "canopy_fragment_main"),
          &terrain_buffers,
        )
      },
    )),
    PipelineKind::TreeShadow => render(caster(PipelineSpec::opaque(
      "VistaWASM tree shadows",
      modules.trees(device),
      ("vertex_shadow", "fragment_shadow"),
      &instance_buffers,
    ))),
    PipelineKind::Terrain => render(create_pipeline(
      device,
      &layouts.terrain,
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
    // wins the depth test against the ground it follows. It writes depth:
    // its faces and lips stand in front of the water and plants drawn
    // after it.
    PipelineKind::BankStrips => render(create_pipeline(
      device,
      &layouts.terrain,
      PipelineSpec {
        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
        depth: Some((true, wgpu::CompareFunction::LessEqual)),
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
    // Full and light meshes share the module, varied by `LIGHT`.
    PipelineKind::TreeMesh | PipelineKind::TreeMeshLight => {
      let light = kind == PipelineKind::TreeMeshLight;
      render(create_pipeline(
        device,
        &layouts.receivers,
        PipelineSpec {
          constants: if light { &[("LIGHT", 1.0)] } else { &[] },
          ..PipelineSpec::opaque(
            if light {
              "VistaWASM light tree meshes"
            } else {
              "VistaWASM tree meshes"
            },
            modules.trees(device),
            ("vertex_mesh", "fragment_mesh"),
            &tree_buffers,
          )
        },
      ))
    }
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
    PipelineKind::SceneCopy => render(create_pipeline(
      device,
      &layouts.composite,
      PipelineSpec {
        format: Some(HDR_FORMAT),
        depth: None,
        ..PipelineSpec::opaque(
          "VistaWASM scene copy",
          modules.atmosphere(device),
          ("vertex_main", "scene_copy_main"),
          &[],
        )
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
    tree_variants: usize,
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
    // The defaults allow 256 MiB buffers and 128 MiB storage bindings,
    // whatever the GPU can do; GPU erosion of a large map needs more.
    let offered = adapter.limits();
    let limits =
      crate::render::gpu_limits::BufferLimits::requested(crate::render::gpu_limits::BufferLimits {
        max_buffer_size: offered.max_buffer_size,
        max_storage_binding: offered.max_storage_buffer_binding_size,
      });
    let (device, queue) = adapter
      .request_device(&wgpu::DeviceDescriptor {
        label: Some("VistaWASM device"),
        // Timestamps feed the per-pass profiler when the browser offers
        // them; everything else works without.
        required_features: adapter.features() & wgpu::Features::TIMESTAMP_QUERY,
        required_limits: wgpu::Limits {
          max_buffer_size: limits.max_buffer_size,
          max_storage_buffer_binding_size: limits.max_storage_binding,
          ..wgpu::Limits::default()
        },
        ..Default::default()
      })
      .await
      .map_err(|_| VistaError::WebGpuDeviceRequestFailed)?;
    let timer = device
      .features()
      .contains(wgpu::Features::TIMESTAMP_QUERY)
      .then(|| GpuTimer::new(&device, &queue));
    let device_lost = Arc::new(AtomicBool::new(false));
    let events = Arc::new(Mutex::new(GpuEvents::default()));
    let lost_flag = Arc::clone(&device_lost);
    let lost_events = Arc::clone(&events);
    device.set_device_lost_callback(move |reason, message| {
      lost_flag.store(true, Ordering::Release);

      if let Ok(mut events) = lost_events.lock() {
        events.lost = Some(match reason {
          wgpu::DeviceLostReason::Destroyed => format!("the device was destroyed: {message}"),
          _ => format!("the browser lost the device: {message}"),
        });
      }
    });
    let error_events = Arc::clone(&events);
    device.on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
      if let Ok(mut events) = error_events.lock() {
        // A broken pipeline can raise the same error every frame; a
        // handful is enough to say what is wrong.
        if events.errors.len() < MAX_PENDING_GPU_ERRORS {
          events.errors.push(describe(error));
        }
      }
    }));
    // Errors creating the engine's resources reject `create`, instead of
    // reaching only the console.
    let scopes = push_error_scopes(&device);
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
    // Growth is sequential, so it runs here, in WASM: the first variant
    // of each species now, the rest after the first frame.
    let started = now_millis();
    let tree_models: Vec<SpeciesModel> = TreeSpecies::ALL
      .iter()
      .map(|species| SpeciesModel::grow(*species, 1))
      .collect();
    let library = build_library(&tree_models);
    let tree_growth = TreeGrowth {
      wanted: tree_variants.clamp(1, VARIANTS),
      ready: 1,
      growing: None,
      millis: [now_millis() - started, 0.0],
      bake_millis: Arc::new(Mutex::new(0.0)),
    };
    let tree_mesh = indexed_mesh(
      &device,
      &queue,
      "VistaWASM tree meshes",
      bytemuck::cast_slice(&library.vertices),
      &library.indices,
    );
    let mut world_info = WorldInfo::zeroed();

    for (slot, (height, radius)) in library.bounds.iter().enumerate() {
      let (bend, flutter) = crate::render::tree_growth::species_wind(TreeSpecies::ALL[slot]);
      world_info.species[slot] = [*height, *radius, bend, flutter];
      world_info.species_tint[slot] = SPECIES_TINTS[slot];
      // Until the impostors are baked, the canopy takes a plain green.
      let tint = SPECIES_TINTS[slot];
      world_info.species_canopy[slot] = [0.039 * tint[0], 0.065 * tint[1], 0.023 * tint[2], 0.0];
    }

    world_info.trees = [tree_growth.wanted as f32, 0.0, 0.0, 0.0];

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
    let ground_layers = GroundLayers::new(
      &device,
      &queue,
      (1, 1),
      [
        (
          &crate::render::vegetation::surface_texel(&SurfaceSample {
            celsius_hundredths: (DEFAULT_SEA_LEVEL_CELSIUS * 100.0) as i16,
            ..SurfaceSample::default()
          }),
          [0; 4],
        ),
        (&[255, 0, 0, 0], [0; 4]),
        (&[0; 4], [0; 4]),
      ],
    );
    let (regional_texture, regional_view) = weather::create_regional_texture(&device);
    let surface_weather = SurfaceWeatherMap::new(&device, &queue, 1, 1);
    let grass_mask_view = create_grass_mask(&device, &queue, (1, 1, &[128]));
    let channel_field = create_channel_field(&device, &queue, &Default::default());
    let grass_mask_group = create_terrain_extras(
      &device,
      &layouts.grass_mask,
      &grass_mask_view,
      (&channel_field.0, &channel_field.1),
    );
    let terrain_shadow = create_terrain_shadow(&device, &queue, 1, 1);
    let tree_shadow_map = create_tree_shadow_map(&device, tree_shadow_resolution);
    let shadow_bind_group =
      create_shadow_bind_group(&device, &layouts.shadow, &tree_shadow_map, &shadow_sampler);
    let impostor_texture = create_impostor_texture(&device, tree_growth.wanted);
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
      &ground_layers.array,
      &regional_view,
      &surface_weather.view,
      &world_buffer,
    );
    let grass_base_vertex_buffer = buffer_with_data(
      &device,
      &queue,
      "VistaWASM grass base tuft",
      bytemuck::cast_slice(&GRASS_BASE_TUFT),
      wgpu::BufferUsages::VERTEX,
    );
    let meshes = crate::render::boulders::BoulderMeshes::build();
    let boulders = BouldersGpu {
      vertex_buffer: buffer_with_data(
        &device,
        &queue,
        "VistaWASM boulder meshes",
        bytemuck::cast_slice(&meshes.vertices),
        wgpu::BufferUsages::VERTEX,
      ),
      index_buffer: buffer_with_data(
        &device,
        &queue,
        "VistaWASM boulder mesh indices",
        bytemuck::cast_slice(&meshes.indices),
        wgpu::BufferUsages::INDEX,
      ),
      ranges: meshes.ranges,
      stream: None,
    };
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
      ground_layers,
      regional_texture,
      regional_view,
      surface_weather,
      grass_view: None,
      grass_mask_view,
      grass_mask_group,
      channel_field,
      grass_mask_neutral: true,
      channel_bins: None,
      texel_inverse: 1.0,
      height_size: (1, 1),
      height_version: 0,
      terrain_shadow,
      tree_shadow_map,
      cloud_target: None,
      lens_target: None,
      reflection: None,
      previous_view_proj: [0.0; 16],
      cloud_frame: 0,
      tree_models,
      tree_growth,
      tree_mesh,
      tree_ranges: library.ranges,
      tree_bounds: library.bounds,
      tree_roots: library.roots,
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
      alto_offset: [0.0; 2],
      mist_offset: [0.0; 2],
      current_offset: [0.0; 2],
      sea_ice_offset: [0.0; 2],
      frames_in_flight: Arc::new(AtomicU32::new(0)),
      timer,
      device_lost,
      events,
      limits,
      cloud_evolution: 0.0,
      erosion: None,
      terrain: None,
      trees: None,
      grass: None,
      boulders,
      width: pixel_width,
      height: pixel_height,
      canvas_width: pixel_width,
      canvas_height: pixel_height,
      render_scale: 1.0,
    };

    if let Some(error) = pop_error_scopes(scopes).await {
      return Err(VistaError::GpuError(error));
    }

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

    // Ferns and undergrowth are drawn with the grass.
    if needs.grass {
      use crate::render::tree_models::layers;
      self.world_textures.bake_flora(
        &self.device,
        &self.queue,
        &self.mips,
        1 << layers::FERN as u32 | 1 << layers::UNDERGROWTH as u32,
      );
    }

    if needs.trees {
      let missing = self.impostors_wanted() & !self.impostors_baked;

      if missing != 0 {
        self.bake_impostors(missing);
      }
    }
  }

  /// The variants whose impostors the scene needs, one bit per species
  /// and variant: every variant grown of each species present.
  fn impostors_wanted(&self) -> u32 {
    let present = self.trees.as_ref().map_or(0, |trees| trees.present);
    let variants = (1u32 << self.tree_growth.ready) - 1;
    (0..SPECIES_COUNT)
      .filter(|species| present & 1 << species != 0)
      .fold(0, |mask, species| mask | variants << (species * VARIANTS))
  }

  /// Variants whose impostors are baked for every species present: the
  /// cull pass shows the first of them in place of the others.
  fn impostor_variants(&self) -> usize {
    let present = self.trees.as_ref().map_or(0, |trees| trees.present);
    (0..self.tree_growth.ready)
      .take_while(|variant| {
        (0..SPECIES_COUNT)
          .filter(|species| present & 1 << species != 0)
          .all(|species| self.impostors_baked & 1 << (species * VARIANTS + variant) != 0)
      })
      .count()
      .max(1)
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
      &self.ground_layers.array,
      &self.regional_view,
      &self.surface_weather.view,
      &self.world_buffer,
    );
  }

  /// Write the world info, but for the canopy colours the GPU averages
  /// from the impostors.
  fn write_world_info(&self) {
    let bytes = bytemuck::bytes_of(&self.world_info);
    self.queue.write_buffer(
      &self.world_buffer,
      0,
      &bytes[..WORLD_INFO_CPU_BYTES as usize],
    );
  }

  /// Render the variants in `mask` (one bit per species and variant) into
  /// the impostor atlas, baking the flora layers they sample first: nine
  /// views each, with their normals. Then mipmap it, keeping each view's
  /// leaf coverage, and average each species' views into the canopy
  /// layer's colour.
  fn bake_impostors(&mut self, mask: u32) {
    let variants = self.tree_growth.wanted;
    let layers = (0..SPECIES_COUNT)
      .filter(|species| mask >> (species * VARIANTS) & 0xf != 0)
      .fold(0, |layers, species| {
        layers | self.tree_models[species].flora_layers()
      });
    // The leaf textures and the impostors are timed together.
    let stopwatch = Stopwatch::start(&self.device, &self.queue);
    let started = now_millis();
    self
      .world_textures
      .bake_flora(&self.device, &self.queue, &self.mips, layers);

    self.impostors_baked |= mask;
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
      &self.ground_layers.array,
      &self.regional_view,
      &self.surface_weather.view,
      &self.world_buffer,
    );
    let depth_view = default_view(&create_texture_2d(
      &self.device,
      "VistaWASM impostor depth",
      IMPOSTOR_CELL[0] * 3,
      IMPOSTOR_CELL[1] * 3,
      DEPTH_FORMAT,
      wgpu::TextureUsages::RENDER_ATTACHMENT,
    ));
    let mut encoder = self
      .device
      .create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("VistaWASM impostor bake"),
      });
    let layer_view = |layer: usize| {
      self
        .impostor_texture
        .create_view(&wgpu::TextureViewDescriptor {
          label: Some("VistaWASM impostor layer"),
          dimension: Some(wgpu::TextureViewDimension::D2),
          base_mip_level: 0,
          mip_level_count: Some(1),
          base_array_layer: layer as u32,
          array_layer_count: Some(1),
          ..Default::default()
        })
    };

    for species in 0..SPECIES_COUNT {
      for variant in 0..variants {
        if mask & 1 << (species * VARIANTS + variant) == 0 {
          continue;
        }

        let layer = (species * variants + variant) * 2;
        let (colour, normals) = (layer_view(layer), layer_view(layer + 1));
        let attachment = |view, clear| {
          Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
              load: wgpu::LoadOp::Clear(clear),
              store: wgpu::StoreOp::Store,
            },
          })
        };
        let flat = wgpu::Color {
          r: 0.5,
          g: 0.5,
          b: 1.0,
          a: 0.0,
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
          label: Some("VistaWASM impostor bake pass"),
          color_attachments: &[
            attachment(&colour, wgpu::Color::TRANSPARENT),
            attachment(&normals, flat),
          ],
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
        let (first_index, index_count, base_vertex) =
          self.tree_ranges[mesh_slot(species, variant, Age::Mature as usize, 1)];

        // Each view into its cell of the 3 x 3 grid.
        for view in 0..9u32 {
          let [width, height] = IMPOSTOR_CELL;
          pass.set_viewport(
            (view % 3 * width) as f32,
            (view / 3 * height) as f32,
            width as f32,
            height as f32,
            0.0,
            1.0,
          );
          let instance = species as u32 * 16 + view;
          pass.draw_indexed(
            first_index..first_index + index_count,
            base_vertex,
            instance..instance + 1,
          );
        }
      }
    }

    // Only the layers just baked.
    let baked_layers = (0..SPECIES_COUNT * variants)
      .filter(|slot| mask & 1 << (slot / variants * VARIANTS + slot % variants) != 0)
      .fold(0u64, |layers, slot| layers | 3 << (slot * 2));
    self.mips.generate_layers(
      &self.device,
      &mut encoder,
      &self.impostor_texture,
      MipMode::Impostors,
      baked_layers,
    );
    // The canopy layer's colour per species, into the world info.
    let canopy = self.device.create_buffer(&wgpu::BufferDescriptor {
      label: Some("VistaWASM canopy colours"),
      size: (SPECIES_COUNT * 16) as u64,
      usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
      mapped_at_creation: false,
    });
    self.mips.average_canopy(
      &self.device,
      &mut encoder,
      &self.impostor_texture,
      3,
      variants as u32,
      &canopy,
    );
    // Only species with baked impostors: the others keep their colour.
    for species in 0..SPECIES_COUNT {
      if self.impostors_baked & 1 << (species * VARIANTS) != 0 {
        encoder.copy_buffer_to_buffer(
          &canopy,
          (species * 16) as u64,
          &self.world_buffer,
          WORLD_INFO_CPU_BYTES + (species * 16) as u64,
          16,
        );
      }
    }

    let millis = Arc::clone(&self.tree_growth.bake_millis);

    // Without timestamps, the wall-clock time until the GPU is done,
    // which includes any work queued before.
    match stopwatch {
      Some(watch) => {
        let read = watch.stop(&mut encoder, millis);
        self.queue.submit(Some(encoder.finish()));
        read();
      }
      None => {
        self.queue.submit(Some(encoder.finish()));
        self.queue.on_submitted_work_done(move || {
          if let Ok(mut total) = millis.lock() {
            *total += now_millis() - started;
          }
        });
      }
    }
  }

  /// Merge the species models into the library and upload it, with its
  /// bounds, wind and roots.
  fn rebuild_tree_library(&mut self) {
    let library: TreeLibrary = build_library(&self.tree_models);
    self.tree_mesh = indexed_mesh(
      &self.device,
      &self.queue,
      "VistaWASM tree meshes",
      bytemuck::cast_slice(&library.vertices),
      &library.indices,
    );
    self.tree_ranges = library.ranges;
    self.tree_bounds = library.bounds;
    self.tree_roots = library.roots;

    for (slot, (height, radius)) in library.bounds.iter().enumerate() {
      self.world_info.species[slot][0] = *height;
      self.world_info.species[slot][1] = *radius;
    }

    self.write_world_info();
  }

  /// Grow one more mesh of the variants still to grow, once the first
  /// frame is up; when every species has them all, they join the library,
  /// the mesh lists make room for them, and their impostors are baked.
  fn grow_step(&mut self) {
    let growth = &mut self.tree_growth;

    if growth.ready >= growth.wanted {
      return;
    }

    let started = now_millis();
    let ready = growth.ready;
    let (models, [species, variant, age]) = growth
      .growing
      .get_or_insert_with(|| (self.tree_models.clone(), [0, ready, 0]));
    let (species_now, variant_now, age_now) = (*species, *variant, *age);

    if let SpeciesModel::Grown {
      meshes, variants, ..
    } = &mut models[species_now]
    {
      let kind = TreeSpecies::ALL[species_now];
      let age = Age::ALL[age_now];
      meshes.push(if kind == TreeSpecies::Shrub && age == Age::Krummholz {
        [TreeMesh::default(), TreeMesh::default()]
      } else {
        crate::render::tree_growth::grow_meshes(kind, variant_now, age)
      });

      if age_now + 1 == AGES {
        *variants = variant_now + 1;
      }
    }

    // Species, then age, then variant, so each species gets its variants
    // in order.
    let next = if age_now + 1 < AGES {
      [species_now, variant_now, age_now + 1]
    } else if variant_now + 1 < growth.wanted {
      [species_now, variant_now + 1, 0]
    } else {
      [species_now + 1, growth.ready, 0]
    };
    growth.millis[1] += now_millis() - started;

    if next[0] < SPECIES_COUNT {
      if let Some((_, cursor)) = &mut growth.growing {
        *cursor = next;
      }

      return;
    }

    let Some((models, _)) = growth.growing.take() else {
      return;
    };
    growth.ready = growth.wanted;

    // A custom model set meanwhile stays.
    for (slot, model) in models.into_iter().enumerate() {
      if matches!(self.tree_models[slot], SpeciesModel::Grown { .. }) {
        self.tree_models[slot] = model;
      }
    }

    self.rebuild_tree_library();
    self.layout_mesh_lists();
  }

  /// Root radius per species at scale 1, in metres, as trees are
  /// grounded.
  pub fn tree_roots(&self) -> [f32; SPECIES_COUNT] {
    self.tree_roots
  }

  /// Change how many variants each species grows (1 to 4). Fewer take
  /// effect at once; more grow after the frames that follow. The impostor
  /// atlas is sized for them, so its impostors are baked again.
  pub fn set_tree_variants(&mut self, variants: usize) {
    let variants = variants.clamp(1, VARIANTS);

    if variants == self.tree_growth.wanted {
      return;
    }

    self.tree_growth.wanted = variants;
    self.tree_growth.growing = None;
    self.tree_growth.ready = self.tree_growth.ready.min(variants);

    for model in &mut self.tree_models {
      if let SpeciesModel::Grown {
        variants: grown,
        meshes,
        ..
      } = model
      {
        if *grown > variants {
          *grown = variants;
          meshes.truncate(variants * AGES);
        }
      }
    }

    self.world_info.trees[0] = variants as f32;
    self.impostor_texture = create_impostor_texture(&self.device, variants);
    self.impostor_view = textures::array_view(&self.impostor_texture);
    self.rebuild_world_bind_group();
    self.impostors_baked = 0;
    self.rebuild_tree_library();
    self.layout_mesh_lists();
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
    crate::render::gpu_limits::check_erosion(map.metadata.width, &self.limits)?;
    let device = &self.device;
    let scopes = push_error_scopes(device);
    let erosion = self
      .erosion
      .get_or_insert_with(|| ErosionCompute::new(device));
    let eroded = erosion
      .run(device, &self.queue, map, options, landform, progress)
      .await;
    // The scopes are popped whatever happened, so none is left open.
    let error = pop_error_scopes(scopes).await;
    let eroded = eroded?;
    error.map_or(Ok(eroded), |error| Err(VistaError::GpuError(error)))
  }

  /// Capture validation and out-of-memory errors from here until the
  /// scopes are popped with [`Self::end_error_scopes_later`]: around resource
  /// creation and each terrain's GPU work, never per frame.
  pub fn begin_error_scopes(&self) -> [wgpu::ErrorScopeGuard; 2] {
    push_error_scopes(&self.device)
  }

  /// Pop `scopes` now, but report what they caught later, as a
  /// `"gpuError"` event, instead of waiting for it: the answer comes only
  /// once the GPU has caught up with the work before it, which held a
  /// terrain call back by a second under software rendering.
  pub fn end_error_scopes_later(&self, scopes: [wgpu::ErrorScopeGuard; 2]) {
    // Popped here, innermost first, so no scope pushed meanwhile can
    // come between them.
    let [out_of_memory, validation] = scopes;
    let validation = validation.pop();
    let out_of_memory = out_of_memory.pop();
    let events = Arc::clone(&self.events);

    wasm_bindgen_futures::spawn_local(async move {
      let error = validation.await.or(out_of_memory.await);

      if let (Some(error), Ok(mut events)) = (error, events.lock()) {
        if events.errors.len() < MAX_PENDING_GPU_ERRORS {
          events.errors.push(describe(error));
        }
      }
    });
  }

  /// GPU errors no scope caught since the last call, and why the device
  /// was lost if it was (reported once).
  pub fn take_events(&self) -> (Vec<String>, Option<String>) {
    self.events.lock().map_or((Vec::new(), None), |mut events| {
      (std::mem::take(&mut events.errors), events.lost.take())
    })
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
  /// meshes, impostors, and shadows all use the new model. A custom model
  /// stands for every variant, age class and level of detail.
  pub fn set_tree_model(&mut self, species: usize, mesh: Option<TreeMesh>) {
    if species >= SPECIES_COUNT {
      return;
    }

    self.tree_models[species] = match mesh {
      Some(mesh) => SpeciesModel::Custom(mesh),
      None => SpeciesModel::grow(TreeSpecies::ALL[species], self.tree_growth.ready),
    };

    // Growth under way carries on without this species.
    if let Some((models, _)) = &mut self.tree_growth.growing {
      models[species] = self.tree_models[species].clone();
    }

    self.rebuild_tree_library();
    let variants = (1u32 << self.tree_growth.wanted) - 1;
    let species_bits = variants << (species * VARIANTS);

    if self.impostors_baked & species_bits != 0 {
      self.impostors_baked &= !species_bits;
      self.bake_impostors(self.impostors_wanted() & species_bits);
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
      TextureTarget::Flora => (&self.world_textures.flora_texture, MipMode::Flora),
    };
    write_layer(&self.queue, texture, layer, rgba, size * 4, size, size);

    // A replaced flora layer is never baked over later, and a replaced
    // leaf layer's normal map turns flat, before the mips are rebuilt.
    if target == TextureTarget::Flora {
      self.world_textures.replaced_flora(&self.queue, layer);
    }

    let texture = match target {
      TextureTarget::TerrainAlbedo => &self.world_textures.terrain_albedo_texture,
      TextureTarget::TerrainNormal => &self.world_textures.terrain_normal_texture,
      TextureTarget::Flora => &self.world_textures.flora_texture,
    };
    let mut encoder = self
      .device
      .create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("VistaWASM texture replacement"),
      });
    self
      .mips
      .generate(&self.device, &mut encoder, texture, mode);
    self.queue.submit(Some(encoder.finish()));

    if target == TextureTarget::Flora && self.impostors_baked != 0 {
      self.bake_impostors(self.impostors_baked);
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
    let canopy = crate::render::terrain_mesh::canopy_indices(
      crate::render::terrain_mesh::CENTRED_MESH_SAMPLES_PER_SIDE,
      2,
    );
    let canopy_index_buffer = buffer_with_data(
      &self.device,
      &self.queue,
      "VistaWASM canopy indices",
      bytemuck::cast_slice(&canopy),
      wgpu::BufferUsages::INDEX,
    );
    self.terrain = Some(TerrainGpu {
      vertex_buffers: [front, back],
      front: 0,
      vertex_count: mesh.vertices.len() as u32,
      index_buffer,
      index_count: mesh.indices.len() as u32,
      canopy_index_buffer,
      canopy_index_count: canopy.len() as u32,
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
  /// shadows, the sea bed beyond the terrain, grounding and the
  /// generators: `ground`'s heights, read every few samples on large maps.
  pub fn upload_heightmap(
    &mut self,
    map: &HeightMap,
    ground: &crate::render::vegetation::GroundData,
  ) {
    let (texture_width, texture_height) = (ground.width, ground.height);

    if texture_width == 0 || texture_height == 0 {
      return;
    }

    self.height_view = create_height_texture(
      &self.device,
      &self.queue,
      texture_width,
      texture_height,
      &ground.heights,
    );
    self.height_size = (texture_width, texture_height);
    self.height_version = self.height_version.wrapping_add(1);
    self.texel_inverse = 1.0 / ground.texel_metres;

    // The shadow texture keeps the height texture's aspect ratio, capped
    // at TERRAIN_SHADOW_MAX texels on the long side.
    let longest = texture_width.max(texture_height);
    let shadow_scale = (TERRAIN_SHADOW_MAX as f32 / longest as f32).min(1.0);
    let shadow_width = ((texture_width as f32 * shadow_scale).round() as u32).max(1);
    let shadow_height = ((texture_height as f32 * shadow_scale).round() as u32).max(1);
    self.terrain_shadow =
      create_terrain_shadow(&self.device, &self.queue, shadow_width, shadow_height);
    // The surface weather map shares the terrain shadow's resolution. A
    // rebuild of the same terrain keeps its puddles.
    if self.surface_weather.size() != (shadow_width, shadow_height) {
      self.surface_weather =
        SurfaceWeatherMap::new(&self.device, &self.queue, shadow_width, shadow_height);
    }
    self.world_info.terrain = [
      ground.half[0],
      ground.half[1],
      ground.texel_metres,
      ground.texel_metres,
    ];
    self.world_info.terrain2 = [
      texture_width as f32,
      texture_height as f32,
      1.0,
      map.metadata.sea_level_metres,
    ];
    self.write_world_info();
    self.rebuild_world_bind_group();
  }

  /// Upload the per-terrain surface textures (`ground`'s surface and
  /// banks: temperature, moisture, permanent snow and biome, then distance
  /// to water, snow and ice cover and bankside greening) and the channel
  /// bins the generators keep out of the water.
  pub fn upload_surface(&mut self, ground: &crate::render::vegetation::GroundData, bins: &[u32]) {
    let texels = (ground.width * ground.height) as usize;

    if texels == 0 || ground.surface.len() != texels {
      return;
    }

    self.ground_layers = GroundLayers::new(
      &self.device,
      &self.queue,
      (ground.width, ground.height),
      [
        (bytemuck::cast_slice(&ground.surface), [0; 4]),
        (bytemuck::cast_slice(&ground.banks), [255, 0, 0, 0]),
        (bytemuck::cast_slice(&ground.cover), [0; 4]),
      ],
    );
    self.channel_bins = Some(buffer_with_data(
      &self.device,
      &self.queue,
      "VistaWASM channel bins",
      bytemuck::cast_slice(bins),
      wgpu::BufferUsages::STORAGE,
    ));
    self.rebuild_world_bind_group();
  }

  /// Upload rows of the regional weather map.
  pub fn upload_regional_weather(&self, upload: crate::weather::regional::GridUpload<'_>) {
    weather::write_regional(&self.queue, &self.regional_texture, &upload);
  }

  /// Step the surface weather map: with the next frame, or at once with
  /// `now` (when skipping ahead, a step per chunk of weather time). A step
  /// at once waits until the pass's pipeline exists.
  pub fn step_surface_weather(&mut self, step: &SurfaceWeatherStep, now: bool) {
    self.surface_weather.queue(step.clone());

    if !now || self.world_info.terrain2[2] < 0.5 {
      return;
    }

    // Skipping ahead can come before any frame has needed the pass.
    self.ensure_pipelines(&Needs {
      surface_weather: true,
      ..Needs::default()
    });
    let Some(pipeline) = self.pipelines.compute(PipelineKind::SurfaceWeather) else {
      return;
    };
    let Some(step) = self.surface_weather.pending.take() else {
      return;
    };
    let mut encoder = self
      .device
      .create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("VistaWASM surface weather"),
      });
    let inputs = weather::SurfaceWeatherInputs {
      queue: &self.queue,
      heights: &self.height_view,
      height_size: self.height_size,
      texel_metres: self.world_info.terrain[2],
      ground: &self.ground_layers.array,
      regional: &self.regional_view,
      half: [self.world_info.terrain[0], self.world_info.terrain[1]],
    };
    self.surface_weather.record(
      &self.device,
      &mut encoder,
      pipeline,
      &self.layouts.surface_weather,
      &step,
      inputs,
      None,
    );
    self.queue.submit([encoder.finish()]);
  }

  /// Upload the cover texture (`flora::bake_cover`).
  pub fn upload_cover(&mut self, ground: &crate::render::vegetation::GroundData) {
    if ground.cover.len() != (ground.width * ground.height) as usize || ground.cover.is_empty() {
      return;
    }

    if self.ground_layers.size() == (ground.width, ground.height) {
      self.ground_layers.write(
        &self.queue,
        weather::COVER_LAYER,
        bytemuck::cast_slice(&ground.cover),
      );
      return;
    }

    let surface = if ground.surface.len() == ground.cover.len() {
      bytemuck::cast_slice(&ground.surface)
    } else {
      &[][..]
    };
    let banks = if ground.banks.len() == ground.cover.len() {
      bytemuck::cast_slice(&ground.banks)
    } else {
      &[][..]
    };
    self.ground_layers = GroundLayers::new(
      &self.device,
      &self.queue,
      (ground.width, ground.height),
      [
        (surface, [0; 4]),
        (banks, [255, 0, 0, 0]),
        (bytemuck::cast_slice(&ground.cover), [0; 4]),
      ],
    );
    self.rebuild_world_bind_group();
  }

  /// A storage buffer of `bytes` bytes (at least 4) that shaders and
  /// copies may write.
  fn storage_buffer(&self, label: &str, bytes: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
    self.device.create_buffer(&wgpu::BufferDescriptor {
      label: Some(label),
      size: bytes.max(4).div_ceil(4) * 4,
      usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | usage,
      mapped_at_creation: false,
    })
  }

  /// Upload the static trees (the far set, or the host's), replacing any
  /// previous trees, and with `stream` the tile pool the generator fills
  /// with the rest of the procedural trees. Trees are culled and sorted
  /// into level-of-detail and shadow lists on the GPU every frame.
  pub fn upload_trees(
    &mut self,
    instances: &[TreeInstance],
    stream: Option<(
      &crate::render::lattice::TileLayout,
      crate::render::vegetation::TreeRules,
      u32,
    )>,
  ) {
    let pool = stream.map_or(0, |(layout, _, _)| layout.instance_count());

    if instances.is_empty() && pool == 0 {
      self.trees = None;
      return;
    }

    let static_count = instances.len() as u32;
    let total = static_count + pool;
    let mut counts = [0u32; SPECIES_COUNT];

    for instance in instances {
      counts[(instance.species_index() as usize).min(SPECIES_COUNT - 1)] += 1;
    }

    // Streamed trees may be of any species the cover texture names.
    let present = counts
      .iter()
      .enumerate()
      .filter(|(_, count)| **count > 0)
      .fold(0u32, |mask, (slot, _)| mask | 1 << slot)
      | stream.map_or(0, |(_, _, species)| species);
    let species_slots = counts
      .map(|count| (count + if stream.is_some() { pool } else { 0 }).min(STREAMED_MESH_SLOTS));

    let tree_bytes = std::mem::size_of::<TreeInstance>() as u64;
    let instance_buffer = self.storage_buffer(
      "VistaWASM tree instances",
      u64::from(total) * tree_bytes,
      wgpu::BufferUsages::empty(),
    );

    if !instances.is_empty() {
      self
        .queue
        .write_buffer(&instance_buffer, 0, bytemuck::cast_slice(instances));
    }

    let drawn = |label: &str, slots: u32| {
      self.storage_buffer(
        label,
        u64::from(slots.max(1)) * DRAWN_TREE_FLOATS * 4,
        wgpu::BufferUsages::VERTEX,
      )
    };
    let impostor_out = drawn("VistaWASM visible tree impostors", total);
    let shadow_out = drawn("VistaWASM shadow-casting trees", total);
    let args_buffer = self.storage_buffer(
      "VistaWASM tree indirect arguments",
      (INDIRECT_WORDS * 4) as u64,
      wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_SRC,
    );
    let cull_params_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
      label: Some("VistaWASM tree cull parameters"),
      size: std::mem::size_of::<CullParams>() as u64,
      usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
      mapped_at_creation: false,
    });
    let stream = stream.map(|(layout, rules, _)| TreeStream {
      layout: layout.clone(),
      rules,
      counts: self.storage_buffer(
        "VistaWASM tree tile counts",
        u64::from(layout.slot_count()) * 4,
        wgpu::BufferUsages::empty(),
      ),
      jobs: self.storage_buffer(
        "VistaWASM tree tile jobs",
        (MAX_JOBS * std::mem::size_of::<GpuJob>()) as u64,
        wgpu::BufferUsages::empty(),
      ),
      params: self.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("VistaWASM tree generator parameters"),
        size: std::mem::size_of::<TreeGenerateParams>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
      }),
    });
    // Without a pool every tree is static, so the counts are never read.
    let no_counts = stream
      .is_none()
      .then(|| self.storage_buffer("VistaWASM no tile counts", 4, wgpu::BufferUsages::empty()));
    // The cull pipeline's layout is implicit, so the pipeline comes first.
    self.ensure_pipelines(&Needs {
      trees: true,
      ..Needs::default()
    });

    if self.pipelines.compute(PipelineKind::TreeCull).is_none() {
      self.trees = None;
      return;
    }

    let placeholder = self.storage_buffer(
      "VistaWASM tree list placeholder",
      16,
      wgpu::BufferUsages::VERTEX,
    );
    self.trees = Some(TreesGpu {
      mesh_out: placeholder.clone(),
      readback: DrawReadback::new(&self.device),
      impostor_out,
      shadow_out,
      args_buffer,
      cull_bind_group: None,
      cull_params_buffer,
      instance_count: total,
      static_count,
      mesh_offsets: vec![0; MESH_LISTS],
      mesh_capacities: vec![0; MESH_LISTS],
      lists_buffer: placeholder,
      species_slots,
      pool,
      no_counts,
      present,
      instance_buffer,
      grounding: Grounding::new(&self.device, "VistaWASM tree grounding parameters"),
      stream,
    });
    self.layout_mesh_lists();
  }

  /// Size the mesh lists for the species present and the variants grown,
  /// and bind them for the cull pass.
  fn layout_mesh_lists(&mut self) {
    let Some(trees) = &self.trees else {
      return;
    };
    let (offsets, capacities) = mesh_list_layout(
      trees.present,
      &trees.species_slots,
      trees.pool,
      trees.stream.is_some(),
      self.tree_growth.ready,
    );
    let running: u32 = capacities.iter().sum();
    let mesh_out = self.storage_buffer(
      "VistaWASM visible tree meshes",
      u64::from(running.max(1)) * DRAWN_TREE_FLOATS * 4,
      wgpu::BufferUsages::VERTEX,
    );
    let table: Vec<u32> = offsets
      .iter()
      .zip(&capacities)
      .flat_map(|(offset, capacity)| [*offset, *capacity])
      .collect();
    let lists_buffer = buffer_with_data(
      &self.device,
      &self.queue,
      "VistaWASM tree mesh lists",
      bytemuck::cast_slice(&table),
      wgpu::BufferUsages::STORAGE,
    );
    let Some(cull) = self.pipelines.compute(PipelineKind::TreeCull) else {
      return;
    };
    let counts = trees
      .stream
      .as_ref()
      .map(|stream| &stream.counts)
      .or(trees.no_counts.as_ref());
    let Some(counts) = counts else {
      return;
    };
    let buffers = [
      &trees.cull_params_buffer,
      &trees.instance_buffer,
      &mesh_out,
      &trees.impostor_out,
      &trees.args_buffer,
      &trees.shadow_out,
      counts,
      &lists_buffer,
    ];
    let entries: Vec<wgpu::BindGroupEntry> = buffers
      .iter()
      .enumerate()
      .map(|(binding, buffer)| wgpu::BindGroupEntry {
        binding: binding as u32,
        resource: buffer.as_entire_binding(),
      })
      .collect();
    let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
      label: Some("VistaWASM tree cull bind group"),
      layout: &cull.get_bind_group_layout(0),
      entries: &entries,
    });

    if let Some(trees) = &mut self.trees {
      trees.mesh_out = mesh_out;
      trees.lists_buffer = lists_buffer;
      trees.mesh_offsets = offsets;
      trees.mesh_capacities = capacities;
      trees.cull_bind_group = Some(bind_group);
    }
  }

  /// Upload the grass density mask, or the neutral one when `ground` has
  /// none. Called when the grass is rebuilt, never per frame; a neutral
  /// mask already in place is kept.
  pub fn upload_grass_mask(&mut self, ground: &crate::render::vegetation::GroundData) {
    let neutral = ground.grass_mask.is_empty();

    if neutral && self.grass_mask_neutral {
      return;
    }

    let mask = if neutral {
      (1, 1, &[128][..])
    } else {
      (ground.width, ground.height, &ground.grass_mask[..])
    };
    self.grass_mask_view = create_grass_mask(&self.device, &self.queue, mask);
    self.grass_mask_neutral = neutral;
    self.rebuild_terrain_extras();
  }

  /// Upload the channel distance field (`RiverNetwork::field`).
  pub fn upload_channel_field(&mut self, field: &crate::render::channel_field::ChannelField) {
    self.channel_field = create_channel_field(&self.device, &self.queue, field);
    self.rebuild_terrain_extras();
  }

  fn rebuild_terrain_extras(&mut self) {
    self.grass_mask_group = create_terrain_extras(
      &self.device,
      &self.layouts.grass_mask,
      &self.grass_mask_view,
      (&self.channel_field.0, &self.channel_field.1),
    );
  }

  /// Upload the grass texture, the reeds and, with `stream`, the tile
  /// pool the generator fills with tufts, replacing any previous grass.
  pub fn upload_grass(
    &mut self,
    ground: &crate::render::vegetation::GroundData,
    reeds: &[FloraInstance],
    stream: Option<(
      &crate::render::lattice::TileLayout,
      crate::render::grass::GrassRules,
    )>,
  ) {
    if ground.grass.len() == (ground.width * ground.height) as usize && !ground.grass.is_empty() {
      self.grass_view = Some(create_surface_texture(
        &self.device,
        &self.queue,
        ground.width,
        ground.height,
        bytemuck::cast_slice(&ground.grass),
      ));
    }

    if reeds.is_empty() && stream.is_none() {
      self.grass = None;
      return;
    }

    let floats = |count: u32| u64::from(count) * std::mem::size_of::<FloraInstance>() as u64;
    let stream = stream.map(|(layout, rules)| {
      let entries = layout.instance_count();
      GrassStream {
        layout: layout.clone(),
        rules,
        tufts: self.storage_buffer(
          "VistaWASM grass tiles",
          floats(entries),
          wgpu::BufferUsages::empty(),
        ),
        counts: self.storage_buffer(
          "VistaWASM grass tile counts",
          u64::from(layout.slot_count()) * 4,
          wgpu::BufferUsages::empty(),
        ),
        drawn: self.storage_buffer(
          "VistaWASM drawn grass",
          floats(entries + GRASS_NEAR_SLOTS),
          wgpu::BufferUsages::VERTEX,
        ),
        args: self.storage_buffer(
          "VistaWASM grass indirect arguments",
          32,
          wgpu::BufferUsages::INDIRECT,
        ),
        jobs: self.storage_buffer(
          "VistaWASM grass tile jobs",
          (MAX_JOBS * std::mem::size_of::<GpuJob>()) as u64,
          wgpu::BufferUsages::empty(),
        ),
        params: self.device.create_buffer(&wgpu::BufferDescriptor {
          label: Some("VistaWASM grass generator parameters"),
          size: std::mem::size_of::<GrassGenerateParams>() as u64,
          usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
          mapped_at_creation: false,
        }),
        grounding: Grounding::new(&self.device, "VistaWASM grass tile grounding parameters"),
      }
    });

    self.grass = Some(GrassGpu {
      reeds: (!reeds.is_empty()).then(|| {
        buffer_with_data(
          &self.device,
          &self.queue,
          "VistaWASM reeds",
          bytemuck::cast_slice(reeds),
          wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::STORAGE,
        )
      }),
      reed_count: reeds.len() as u32,
      grounding: Grounding::new(&self.device, "VistaWASM reed grounding parameters"),
      stream,
    });
  }

  /// Create the tile pool the generator fills with boulders, for `stream`,
  /// or drop it when there are none.
  pub fn upload_boulders(
    &mut self,
    stream: Option<(
      &crate::render::lattice::TileLayout,
      crate::render::boulders::BoulderRules,
    )>,
  ) {
    let lists = crate::render::boulders::VARIANTS as u64
      * u64::from(BOULDER_VARIANT_ENTRIES + BOULDER_SHADOW_CAPACITY);
    self.boulders.stream = stream.map(|(layout, rules)| BoulderStream {
      layout: layout.clone(),
      rules,
      pool: self.storage_buffer(
        "VistaWASM boulder tiles",
        u64::from(layout.instance_count()) * BOULDER_BYTES,
        wgpu::BufferUsages::empty(),
      ),
      counts: self.storage_buffer(
        "VistaWASM boulder tile counts",
        u64::from(layout.slot_count()) * 4,
        wgpu::BufferUsages::empty(),
      ),
      drawn: self.storage_buffer(
        "VistaWASM drawn boulders",
        lists * BOULDER_BYTES,
        wgpu::BufferUsages::VERTEX,
      ),
      args: self.storage_buffer(
        "VistaWASM boulder indirect arguments",
        (BOULDER_ARGS_WORDS * 4) as u64,
        wgpu::BufferUsages::INDIRECT,
      ),
      jobs: self.storage_buffer(
        "VistaWASM boulder tile jobs",
        (MAX_JOBS * std::mem::size_of::<GpuJob>()) as u64,
        wgpu::BufferUsages::empty(),
      ),
      params: self.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("VistaWASM boulder generator parameters"),
        size: std::mem::size_of::<BoulderGenerateParams>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
      }),
      grounding: Grounding::new(&self.device, "VistaWASM boulder grounding parameters"),
    });
  }

  /// Stand trees, reeds and streamed tufts on the terrain mesh drawn this
  /// frame, when it or the heights have changed since they were last
  /// grounded: the static trees and every entry of the tile pools, near
  /// the camera by construction. Each is grounded once, not every frame
  /// and not once per vertex; tiles generated later are grounded as they
  /// are generated.
  /// Returns which of the trees, the reeds, the tufts and the boulders
  /// were grounded, for [`Self::grounded`].
  fn ground_instances(&self, pass: &mut wgpu::ComputePass<'_>, mesh: [f32; 4]) -> [bool; 4] {
    let mut done = [false; 4];
    let Some(pipeline) = self.pipelines.compute(PipelineKind::Grounding) else {
      return done;
    };
    let now = (mesh, self.height_version);
    let mut roots = [[0.0; 4]; 2];

    for (slot, root) in self.tree_roots.iter().enumerate() {
      roots[slot / 4][slot % 4] = *root;
    }

    // Trees are 8 floats with a species and flags; tufts and reeds 7;
    // boulders 8, sunk by their fifth.
    let grass = self.grass.as_ref();
    let jobs = [
      self.trees.as_ref().map(|trees| {
        (
          &trees.grounding,
          &trees.instance_buffer,
          trees.instance_count,
          8,
          1,
        )
      }),
      grass.and_then(|grass| {
        grass
          .reeds
          .as_ref()
          .map(|reeds| (&grass.grounding, reeds, grass.reed_count, 7, 0))
      }),
      grass.and_then(|grass| {
        grass.stream.as_ref().map(|stream| {
          (
            &stream.grounding,
            &stream.tufts,
            stream.layout.instance_count(),
            7,
            0,
          )
        })
      }),
      self.boulders.stream.as_ref().map(|stream| {
        (
          &stream.grounding,
          &stream.pool,
          stream.layout.instance_count(),
          8,
          2,
        )
      }),
    ];

    for (slot, job) in jobs.into_iter().enumerate() {
      let Some((grounding, buffer, count, floats, trees)) = job else {
        continue;
      };

      if grounding.done_for == Some(now) || count == 0 {
        continue;
      }

      let ground = GroundParams {
        terrain: self.world_info.terrain,
        terrain2: self.world_info.terrain2,
        mesh,
        shape: [count, floats, trees, 0],
        roots,
      };
      self
        .queue
        .write_buffer(&grounding.params, 0, bytemuck::bytes_of(&ground));
      let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("VistaWASM grounding bind group"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
          wgpu::BindGroupEntry {
            binding: 0,
            resource: grounding.params.as_entire_binding(),
          },
          wgpu::BindGroupEntry {
            binding: 1,
            resource: buffer.as_entire_binding(),
          },
          wgpu::BindGroupEntry {
            binding: 2,
            resource: wgpu::BindingResource::TextureView(&self.height_view),
          },
        ],
      });
      pass.set_pipeline(pipeline);
      pass.set_bind_group(0, &bind_group, &[]);
      pass.dispatch_workgroups(count.div_ceil(64), 1, 1);
      done[slot] = true;
    }

    done
  }

  /// Record that what `done` names stands on the mesh `mesh`.
  fn grounded(&mut self, done: [bool; 4], mesh: [f32; 4]) {
    let now = (mesh, self.height_version);

    if let (true, Some(trees)) = (done[0], self.trees.as_mut()) {
      trees.grounding.done_for = Some(now);
    }

    if let Some(grass) = self.grass.as_mut() {
      if done[1] {
        grass.grounding.done_for = Some(now);
      }

      if let (true, Some(stream)) = (done[2], grass.stream.as_mut()) {
        stream.grounding.done_for = Some(now);
      }
    }

    if let (true, Some(stream)) = (done[3], self.boulders.stream.as_mut()) {
      stream.grounding.done_for = Some(now);
    }
  }

  /// Whether the tree, grass and boulder generators can run this frame:
  /// their pipelines are built and the ground they read is uploaded.
  /// Pipelines are built in the background, so for the first frames they
  /// may not be; a tile handed out then would be recorded as filled while
  /// nothing filled it, and stay empty until the camera moved away.
  pub fn generators_ready(&self) -> [bool; 3] {
    let ground = self.channel_bins.is_some();
    let ready = |kind| ground && self.pipelines.compute(kind).is_some();
    [
      ready(PipelineKind::TreeGenerate),
      ready(PipelineKind::GrassGenerate) && self.grass_view.is_some(),
      ready(PipelineKind::BoulderGenerate),
    ]
  }

  /// The generator jobs for `changes`, and zero counts for every slot
  /// they empty or refill.
  fn jobs_of(
    &self,
    layout: &crate::render::lattice::TileLayout,
    counts: &wgpu::Buffer,
    changes: &crate::render::lattice::TileChanges,
  ) -> Vec<GpuJob> {
    for slot in changes
      .freed
      .iter()
      .chain(changes.jobs.iter().map(|job| &job.slot))
    {
      self
        .queue
        .write_buffer(counts, u64::from(*slot) * 4, bytemuck::bytes_of(&0u32));
    }

    changes
      .jobs
      .iter()
      .take(MAX_JOBS)
      .map(|job| {
        let (first, capacity) = layout.slot_range(job.slot);
        GpuJob {
          tile: job.tile,
          first,
          capacity,
          slot: job.slot,
          keep: job.keep,
          unused: [0; 2],
        }
      })
      .collect()
  }

  /// The generators' shared bindings: heights, surface, banks, cover,
  /// channel bins and jobs.
  fn generator_entries<'a>(
    &'a self,
    jobs: &'a wgpu::Buffer,
  ) -> Option<Vec<wgpu::BindGroupEntry<'a>>> {
    let bins = self.channel_bins.as_ref()?;
    Some(vec![
      view_entry(1, &self.height_view),
      view_entry(2, &self.ground_layers.surface),
      view_entry(3, &self.ground_layers.banks),
      view_entry(4, &self.ground_layers.cover),
      wgpu::BindGroupEntry {
        binding: 5,
        resource: bins.as_entire_binding(),
      },
      wgpu::BindGroupEntry {
        binding: 6,
        resource: jobs.as_entire_binding(),
      },
    ])
  }

  /// Fill this frame's boulder tiles and cull the boulders into their
  /// draw lists.
  fn stream_boulders(
    &self,
    pass: &mut wgpu::ComputePass<'_>,
    params: &FrameParams,
    planes: &[[f32; 4]; 6],
  ) {
    let Some(stream) = self.boulders.stream.as_ref() else {
      return;
    };
    let frame = &params.vegetation;
    let camera = params.camera_position;
    let mut tall = [[0.0; 4]; 2];

    for (variant, height) in stream.rules.heights.iter().enumerate() {
      tall[variant / 4][variant % 4] = *height;
    }

    let generate = BoulderGenerateParams {
      terrain: self.world_info.terrain,
      terrain2: self.world_info.terrain2,
      mesh: params.mesh_ground,
      rules: [
        self.texel_inverse,
        frame.boulder_metres.max(1.0),
        crate::render::boulders::BOULDER_SHADOW_METRES,
        self.height as f32
          / (2.0
            * (params.field_of_view_degrees.to_radians() * 0.5)
              .tan()
              .max(1e-4)),
      ],
      shape: [
        stream.rules.seed,
        stream.layout.instance_count(),
        stream
          .layout
          .classes
          .first()
          .map_or(1, |class| class.capacity),
        stream.rules.stones,
      ],
      camera: [camera[0], camera[1], camera[2], 0.0],
      tall,
      planes: *planes,
    };
    self
      .queue
      .write_buffer(&stream.params, 0, bytemuck::bytes_of(&generate));
    let bind = |pipeline: &wgpu::ComputePipeline, entries: &[wgpu::BindGroupEntry]| {
      self.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("VistaWASM boulder bind group"),
        layout: &pipeline.get_bind_group_layout(0),
        entries,
      })
    };
    let buffer = buffer_entry;

    if let Some(pipeline) = self.pipelines.compute(PipelineKind::BoulderGenerate) {
      let jobs = self.jobs_of(&stream.layout, &stream.counts, &frame.boulders);

      if let (false, Some(mut entries)) = (jobs.is_empty(), self.generator_entries(&stream.jobs)) {
        self
          .queue
          .write_buffer(&stream.jobs, 0, bytemuck::cast_slice(&jobs));
        // Boulders read neither the surface nor the cover texture, so the
        // pipeline's layout has no place for them.
        entries.retain(|entry| entry.binding != 2 && entry.binding != 4);
        entries.extend([
          buffer(0, &stream.params),
          buffer(7, &stream.pool),
          buffer(8, &stream.counts),
        ]);
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &bind(pipeline, &entries), &[]);
        pass.dispatch_workgroups(2, 2, jobs.len() as u32);
      }
    }

    if let Some(pipeline) = self.pipelines.compute(PipelineKind::BoulderCull) {
      self.queue.write_buffer(
        &stream.args,
        0,
        bytemuck::cast_slice(&self.boulders.cleared_args()),
      );
      let entries = [
        buffer(0, &stream.params),
        buffer(7, &stream.pool),
        buffer(8, &stream.counts),
        buffer(10, &stream.drawn),
        buffer(11, &stream.args),
      ];
      pass.set_pipeline(pipeline);
      pass.set_bind_group(0, &bind(pipeline, &entries), &[]);
      pass.dispatch_workgroups(stream.layout.instance_count().div_ceil(64), 1, 1);
    }
  }

  /// Draw the culled boulders of lists `lists` (one indexed indirect draw
  /// each) with `pipeline`.
  fn draw_boulders(
    &self,
    pass: &mut wgpu::RenderPass<'_>,
    pipeline: &wgpu::RenderPipeline,
    lists: std::ops::Range<usize>,
  ) {
    let Some(stream) = &self.boulders.stream else {
      return;
    };
    pass.set_pipeline(pipeline);
    pass.set_vertex_buffer(0, self.boulders.vertex_buffer.slice(..));
    pass.set_index_buffer(
      self.boulders.index_buffer.slice(..),
      wgpu::IndexFormat::Uint16,
    );

    for list in lists {
      pass.set_vertex_buffer(
        1,
        stream
          .drawn
          .slice(u64::from(BouldersGpu::list_start(list)) * BOULDER_BYTES..),
      );
      pass.draw_indexed_indirect(&stream.args, (list * 5 * 4) as u64);
    }
  }

  /// Fill this frame's tree and grass tiles, and cull the streamed grass.
  fn stream_vegetation(
    &self,
    pass: &mut wgpu::ComputePass<'_>,
    params: &FrameParams,
    planes: &[[f32; 4]; 6],
  ) {
    let frame = &params.vegetation;
    let mapping = [
      self.world_info.terrain,
      self.world_info.terrain2,
      params.mesh_ground,
    ];

    if let (Some(stream), Some(pipeline)) = (
      self.trees.as_ref().and_then(|trees| trees.stream.as_ref()),
      self.pipelines.compute(PipelineKind::TreeGenerate),
    ) {
      let jobs = self.jobs_of(&stream.layout, &stream.counts, &frame.trees);

      if let (false, Some(mut entries)) = (jobs.is_empty(), self.generator_entries(&stream.jobs)) {
        let trees = self.trees.as_ref();
        let mut max_slopes = [[0.0; 4]; 2];
        let mut roots = [[0.0; 4]; 2];

        for slot in 0..SPECIES_COUNT {
          max_slopes[slot / 4][slot % 4] = stream.rules.max_slopes[slot];
          roots[slot / 4][slot % 4] = self.tree_roots[slot];
        }

        let generate = TreeGenerateParams {
          terrain: mapping[0],
          terrain2: mapping[1],
          mesh: mapping[2],
          rules: [
            self.texel_inverse,
            stream.rules.p_scale,
            stream.rules.variation,
            stream.rules.far_keep,
          ],
          shape: [
            stream.rules.seed,
            stream.rules.understorey.to_bits(),
            stream.rules.boulders.unwrap_or(0),
            u32::from(stream.rules.boulders.is_some()),
          ],
          max_slopes,
          roots,
        };
        // Job slots are within the pool, which follows the static trees.
        let offset = trees.map_or(0, |trees| trees.static_count);
        let jobs: Vec<GpuJob> = jobs
          .into_iter()
          .map(|job| GpuJob {
            first: job.first + offset,
            ..job
          })
          .collect();
        self
          .queue
          .write_buffer(&stream.params, 0, bytemuck::bytes_of(&generate));
        self
          .queue
          .write_buffer(&stream.jobs, 0, bytemuck::cast_slice(&jobs));
        entries.extend([
          wgpu::BindGroupEntry {
            binding: 0,
            resource: stream.params.as_entire_binding(),
          },
          wgpu::BindGroupEntry {
            binding: 7,
            resource: trees.map_or(stream.counts.as_entire_binding(), |trees| {
              trees.instance_buffer.as_entire_binding()
            }),
          },
          wgpu::BindGroupEntry {
            binding: 8,
            resource: stream.counts.as_entire_binding(),
          },
        ]);
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
          label: Some("VistaWASM tree generator bind group"),
          layout: &pipeline.get_bind_group_layout(0),
          entries: &entries,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(3, 3, jobs.len() as u32);
      }
    }

    let Some(stream) = self.grass.as_ref().and_then(|grass| grass.stream.as_ref()) else {
      return;
    };
    let mut classes = [[0u32; 4]; 8];

    for (slot, class) in stream.layout.classes.iter().take(8).enumerate() {
      classes[slot] = [
        class.first_slot,
        class.first_instance,
        class.capacity,
        class.slots,
      ];
    }

    let camera = params.camera_position;
    let generate = GrassGenerateParams {
      terrain: mapping[0],
      terrain2: mapping[1],
      mesh: mapping[2],
      rules: [
        self.texel_inverse,
        stream.rules.probability,
        stream.rules.canopy,
        stream.rules.height,
      ],
      shape: [
        stream.rules.seed,
        stream.layout.classes.len().min(8) as u32,
        stream.layout.instance_count(),
        stream.rules.boulders.unwrap_or(0),
      ],
      camera: [camera[0], camera[1], camera[2], frame.grass_radius],
      view: [
        params.grass_view_distance_metres,
        flag(stream.rules.boulders.is_some()),
        flag(!self.grass_mask_neutral),
        frame.grass_handover,
      ],
      planes: *planes,
      classes,
    };
    self
      .queue
      .write_buffer(&stream.params, 0, bytemuck::bytes_of(&generate));
    let grass_texture = self.grass_view.as_ref();

    if let (Some(pipeline), Some(grass_texture)) = (
      self.pipelines.compute(PipelineKind::GrassGenerate),
      grass_texture,
    ) {
      let jobs = self.jobs_of(&stream.layout, &stream.counts, &frame.grass);

      if let (false, Some(mut entries)) = (jobs.is_empty(), self.generator_entries(&stream.jobs)) {
        self
          .queue
          .write_buffer(&stream.jobs, 0, bytemuck::cast_slice(&jobs));
        entries.extend([
          wgpu::BindGroupEntry {
            binding: 0,
            resource: stream.params.as_entire_binding(),
          },
          wgpu::BindGroupEntry {
            binding: 7,
            resource: stream.tufts.as_entire_binding(),
          },
          wgpu::BindGroupEntry {
            binding: 8,
            resource: stream.counts.as_entire_binding(),
          },
          view_entry(9, grass_texture),
          view_entry(12, &self.grass_mask_view),
          sampler_binding(13, &self.clamp_sampler),
        ]);
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
          label: Some("VistaWASM grass generator bind group"),
          layout: &pipeline.get_bind_group_layout(0),
          entries: &entries,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(6, 6, jobs.len() as u32);
      }
    }

    if let Some(pipeline) = self.pipelines.compute(PipelineKind::GrassCull) {
      self.queue.write_buffer(
        &stream.args,
        0,
        bytemuck::cast_slice(&[12u32, 0, 0, 0, 6, 0, 0, 0]),
      );
      let buffers = [
        (0, &stream.params),
        (7, &stream.tufts),
        (8, &stream.counts),
        (10, &stream.drawn),
        (11, &stream.args),
      ];
      let entries: Vec<wgpu::BindGroupEntry> = buffers
        .iter()
        .map(|(binding, buffer)| wgpu::BindGroupEntry {
          binding: *binding,
          resource: buffer.as_entire_binding(),
        })
        .collect();
      let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("VistaWASM grass cull bind group"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &entries,
      });
      pass.set_pipeline(pipeline);
      pass.set_bind_group(0, &bind_group, &[]);
      pass.dispatch_workgroups(stream.layout.instance_count().div_ceil(64), 1, 1);
    }
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
    let alto_wind = alto_wind_direction(clouds.wind_direction_degrees);
    let alto_speed = clouds.alto_speed.clamp(0.0, 4.0) * 15.0 * dt;
    self.alto_offset[0] -= alto_wind[0] * alto_speed;
    self.alto_offset[1] -= alto_wind[1] * alto_speed;
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
    u.clouds5 = [
      clouds.base_variation.clamp(0.0, 0.2),
      clouds.base_lumpiness.clamp(0.0, 1.0),
      params.alto_amounts[0].clamp(0.0, 1.0),
      params.alto_amounts[1].clamp(0.0, 1.0),
    ];
    let alto_height = alto_height(clouds);
    u.alto = [self.alto_offset[0], self.alto_offset[1], alto_height, 0.0];
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
      flag(water.reflections == vista_types::WaterReflections::Screen),
    ];

    let flora = &params.flora;
    u.vegetation = [
      flora.wind_strength.clamp(0.0, 1.0),
      flora.species_variation.clamp(0.0, 1.0),
      params.tree_style as f32,
      params.grass_view_distance_metres.max(0.0),
    ];
    u.vegetation2 = [
      flora.mesh_distance_metres.max(1.0),
      params.grass_cover,
      params.vegetation.grass_radius,
      params.grass_height,
    ];
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
    // Boulders in view cast into the same map as the trees.
    let tree_shadows = shadows.trees.enabled
      && (self.trees.is_some() || (params.needs.boulders && self.boulders.stream.is_some()))
      && params.sun_direction[1] > 0.0;
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
    // The stones' seed in two halves, whole numbers a float holds
    // exactly, and whether boulder meshes draw them.
    u.rivers2 = [
      rivers.eddies.clamp(0.0, 1.0),
      flag(self.boulders.stream.is_some()),
      (rivers.stones & 0xffff) as f32,
      (rivers.stones >> 16) as f32,
    ];
    // Where riparian scrub thins out with distance, as the tree cull
    // thins it, so the terrain's far tint takes over exactly there.
    let far_keep = self
      .trees
      .as_ref()
      .and_then(|trees| trees.stream.as_ref())
      .map_or(1.0, |stream| stream.rules.far_keep);
    u.waterside = [
      if self.trees.is_some() {
        params.vegetation.tree_radius.max(1.0)
      } else {
        0.0
      },
      far_keep,
      if params.canopy.drawn {
        params.canopy.distance.max(1.0)
      } else {
        1.0e30
      },
      params.rivers.refraction.clamp(0.0, 1.0),
    ];
    u.mouths = params.rivers.mouths;
    u.ground = params.mesh_ground;
    u.vegetation3 = [
      params.canopy.density,
      params.canopy.distance.max(1.0),
      params.shadows.trees.distance_metres,
      flag(params.canopy.drawn),
    ];
    u.rock = params.rock;
    u.regional = weather.regional;
    u.air = weather.air;
    u.light = weather.light;
    u.gust = weather.gust;
    u.weather4 = weather.sea;
    // Under cloud, shadows soften towards nothing.
    u.shadow_params[0] *= weather.light[1];
    u.shadow_params[2] *= weather.light[1];
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
    // Cloud widens the penumbra, in tenths so a slowly thickening deck
    // re-bakes only now and then.
    let softening = (params.weather.shadow_softening * 10.0).round() / 10.0;
    let softness = (params.shadows.terrain.softness + softening).clamp(0.0, 1.0);

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

  /// Make sure the half-resolution scene copy that water reflects exists
  /// at the internal render size.
  fn ensure_reflection_target(&mut self) {
    if self.reflection.is_some() {
      return;
    }

    let view = default_view(&create_texture_2d(
      &self.device,
      "VistaWASM scene copy",
      self.width.div_ceil(2),
      self.height.div_ceil(2),
      HDR_FORMAT,
      wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
    ));
    let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
      label: Some("VistaWASM reflection bind group"),
      layout: &self.layouts.reflection,
      entries: &[view_entry(0, &view)],
    });
    self.reflection = Some((view, bind_group));
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
    self.ensure_reflection_target();
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
    let clouds_drawn = params.clouds_drawn();
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
      cull.planes = crate::maths::frustum_planes(&params.view_proj);
      // Not `clamp`: `min` then `max` keeps a NaN reach finite.
      #[allow(clippy::manual_clamp)]
      let mesh_reach = params
        .vegetation
        .mesh_metres
        .map_or(params.flora.mesh_distance_metres, |reach| {
          reach.min(params.flora.mesh_distance_metres)
        })
        .min(IMPOSTOR_METRES)
        .max(1.0);
      cull.camera = [
        params.camera_position[0],
        params.camera_position[1],
        params.camera_position[2],
        mesh_reach,
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
      }

      // Full meshes near, lighter ones to the mesh distance (a third of
      // it at least), then impostors.
      cull.lod = [
        LOD0_METRES.min(cull.camera[3] / 3.0),
        0.0,
        self.tree_growth.ready as f32,
        self.impostor_variants() as f32,
      ];

      cull.budget = [
        params.vegetation.shadow_metres,
        0.0,
        UNDERSTOREY_CARD_METRES,
        params.vegetation.max_shadow_casters as f32,
      ];

      cull.stream = [
        trees.static_count,
        trees
          .stream
          .as_ref()
          .map_or(1, |stream| stream.layout.classes[0].capacity),
        0,
        0,
      ];
      // Without streamed tiles the far set is the whole forest: nothing
      // thins, and with no canopy layer drawn no tree gives way to it.
      cull.thin = [
        params.vegetation.tree_radius.max(1.0),
        trees
          .stream
          .as_ref()
          .map_or(1.0, |stream| stream.rules.far_keep),
        if params.canopy.drawn {
          params.canopy.distance.max(1.0)
        } else {
          f32::MAX
        },
        trees
          .stream
          .as_ref()
          .map_or(1.0, |stream| stream.rules.understorey),
      ];

      self
        .queue
        .write_buffer(&trees.cull_params_buffer, 0, bytemuck::bytes_of(&cull));

      let mut args = [0u32; INDIRECT_WORDS];

      for list in 0..MESH_LISTS {
        let (first_index, index_count, base_vertex) = self.tree_ranges[list_mesh(list)];
        args[list * 5] = index_count;
        args[list * 5 + 2] = first_index;
        args[list * 5 + 3] = base_vertex as u32;
      }

      args[IMPOSTOR_ARGS_BASE] = impostor_vertices;
      args[SHADOW_ARGS_BASE] = 6;

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

    if let Some(step) = self.surface_weather.pending.take() {
      if let Some(pipeline) = self.pipelines.compute(PipelineKind::SurfaceWeather) {
        let inputs = weather::SurfaceWeatherInputs {
          queue: &self.queue,
          heights: &self.height_view,
          height_size: self.height_size,
          texel_metres: self.world_info.terrain[2],
          ground: &self.ground_layers.array,
          regional: &self.regional_view,
          half: [self.world_info.terrain[0], self.world_info.terrain[1]],
        };
        self.surface_weather.record(
          &self.device,
          &mut encoder,
          pipeline,
          &self.layouts.surface_weather,
          &step,
          inputs,
          timing.map(|timer| timer.compute_writes(PASS_SURFACE_WEATHER)),
        );
        ran |= 1 << PASS_SURFACE_WEATHER;
      } else {
        self.surface_weather.pending = Some(step);
      }
    }

    // Grounding (when the mesh has recentred), filling this frame's tiles
    // and culling the streamed grass, in one timed pass.
    let mut grounded = [false; 4];

    if self.pipelines.compute(PipelineKind::Grounding).is_some()
      && (self.trees.is_some() || self.grass.is_some() || self.boulders.stream.is_some())
    {
      let planes = crate::maths::frustum_planes(&params.view_proj);
      let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some("VistaWASM vegetation pass"),
        timestamp_writes: timing.map(|timer| timer.compute_writes(PASS_GENERATION)),
      });
      ran |= 1 << PASS_GENERATION;
      grounded = self.ground_instances(&mut pass, params.mesh_ground);
      self.stream_vegetation(&mut pass, params, &planes);
      self.stream_boulders(&mut pass, params, &planes);
    }

    let mut read_draws = false;

    if let (Some(trees), Some(cull)) = (&self.trees, self.pipelines.compute(PipelineKind::TreeCull))
    {
      let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some("VistaWASM tree cull pass"),
        timestamp_writes: timing.map(|timer| timer.compute_writes(PASS_TREE_CULL)),
      });
      ran |= 1 << PASS_TREE_CULL;
      pass.set_pipeline(cull);
      pass.set_bind_group(0, trees.cull_bind_group.as_ref(), &[]);
      pass.dispatch_workgroups(trees.instance_count.div_ceil(64), 1, 1);
      drop(pass);
      read_draws = trees.readback.copy(&mut encoder, &trees.args_buffer);
    }

    let stride = DRAWN_TREE_FLOATS * 4;

    // Tree shadow map: one sun-facing impostor quad per tree, and the
    // boulders within the shadow distance.
    let boulder_shadows = self
      .pipelines
      .render(PipelineKind::BoulderShadow)
      .filter(|_| self.boulders.stream.is_some());
    let tree_casters = self
      .trees
      .as_ref()
      .zip(self.pipelines.render(PipelineKind::TreeShadow));

    if tree_shadows && (tree_casters.is_some() || boulder_shadows.is_some()) {
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
      pass.set_bind_group(0, &self.frame_bind_group, &[]);
      pass.set_bind_group(1, &self.world_bind_group, &[]);

      if let Some((trees, pipeline)) = tree_casters {
        pass.set_pipeline(pipeline);
        pass.set_vertex_buffer(0, trees.shadow_out.slice(..));
        pass.draw_indirect(&trees.args_buffer, (SHADOW_ARGS_BASE * 4) as u64);
      }

      if let Some(pipeline) = boulder_shadows {
        self.draw_boulders(&mut pass, pipeline, 18..BOULDER_LISTS);
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
        pass.set_bind_group(3, &self.grass_mask_group, &[]);
        pass.set_vertex_buffer(0, terrain.vertex_buffers[terrain.front].slice(..));
        pass.set_index_buffer(terrain.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..terrain.index_count, 0, 0..1);

        // Drawn with the terrain, before the scene copy, so water
        // reflects distant forests too.
        if let (true, Some(canopy)) = (
          params.canopy.drawn && self.trees.is_some(),
          self.pipelines.render(PipelineKind::Canopy),
        ) {
          pass.set_pipeline(canopy);
          pass.set_index_buffer(
            terrain.canopy_index_buffer.slice(..),
            wgpu::IndexFormat::Uint32,
          );
          pass.draw_indexed(0..terrain.canopy_index_count, 0, 0..1);
        }
      }

      if let (Some(strips), Some(pipeline), true) = (
        &self.bank_strips,
        self.pipelines.render(PipelineKind::BankStrips),
        self.water_visible,
      ) {
        pass.set_pipeline(pipeline);
        // Banks read the channel field, as the terrain does.
        pass.set_bind_group(3, &self.grass_mask_group, &[]);
        pass.set_vertex_buffer(0, strips.vertex_buffer.slice(..));
        pass.set_index_buffer(strips.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..strips.index_count, 0, 0..1);
      }
    }

    // Boulders, before the trees and grass so what they hide fails the
    // depth test early.
    if let (Some(_), Some(pipeline)) = (
      &self.boulders.stream,
      self.pipelines.render(PipelineKind::Boulders),
    ) {
      let mut pass = self.begin_opaque_pass(
        &mut encoder,
        "VistaWASM boulder pass",
        timing.map(|timer| timer.render_writes(PASS_BOULDERS)),
        false,
      );
      ran |= 1 << PASS_BOULDERS;
      self.draw_boulders(&mut pass, pipeline, 0..18);
    }

    // Canopy meshes, understorey meshes, then impostors: one pass, or
    // three with split timing so each is timed.
    if let Some(trees) = &self.trees {
      let split = params.split_tree_timing;
      let meshes = match (
        params.tree_style,
        self.pipelines.render(PipelineKind::TreeMesh),
      ) {
        (2, Some(pipeline)) => Some(pipeline),
        _ => None,
      };
      let mut pass = self.begin_opaque_pass(
        &mut encoder,
        "VistaWASM tree pass",
        timing.map(|timer| timer.render_writes(PASS_TREES)),
        false,
      );
      ran |= 1 << PASS_TREES;

      // Full meshes, light meshes (their own pipeline once it exists),
      // then the understorey's young meshes.
      let light = self
        .pipelines
        .render(PipelineKind::TreeMeshLight)
        .or(meshes);
      let groups = [
        (meshes, 0, PASS_TREES),
        (light, 1, PASS_TREES),
        (meshes, 2, PASS_UNDERSTOREY),
      ];

      for (pipeline, group, timed) in groups {
        let Some(pipeline) = pipeline else {
          break;
        };

        if split && group == 2 {
          drop(pass);
          pass = self.begin_opaque_pass(
            &mut encoder,
            "VistaWASM understorey pass",
            timing.map(|timer| timer.render_writes(timed)),
            false,
          );
          ran |= 1 << timed;
        }

        pass.set_pipeline(pipeline);
        pass.set_vertex_buffer(0, self.tree_mesh.vertex_buffer.slice(..));
        pass.set_index_buffer(
          self.tree_mesh.index_buffer.slice(..),
          wgpu::IndexFormat::Uint32,
        );
        let lists = if group == 2 {
          MESH_SLOTS..MESH_LISTS
        } else {
          0..MESH_SLOTS
        };

        for list in lists {
          if (group < 2 && list % 2 != group) || trees.mesh_capacities[list] == 0 {
            continue;
          }

          pass.set_vertex_buffer(
            1,
            trees
              .mesh_out
              .slice(u64::from(trees.mesh_offsets[list]) * stride..),
          );
          pass.draw_indexed_indirect(&trees.args_buffer, (list * 5 * 4) as u64);
        }
      }

      if let Some(pipeline) = self.pipelines.render(PipelineKind::TreeImpostor) {
        if split {
          drop(pass);
          pass = self.begin_opaque_pass(
            &mut encoder,
            "VistaWASM tree impostor pass",
            timing.map(|timer| timer.render_writes(PASS_TREE_IMPOSTORS)),
            false,
          );
          ran |= 1 << PASS_TREE_IMPOSTORS;
        }

        pass.set_pipeline(pipeline);
        pass.set_vertex_buffer(0, trees.impostor_out.slice(..));
        pass.draw_indirect(&trees.args_buffer, (IMPOSTOR_ARGS_BASE * 4) as u64);
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

      if let Some(reeds) = &grass.reeds {
        pass.set_vertex_buffer(1, reeds.slice(..));
        pass.draw(0..18, 0..grass.reed_count);
      }

      // Near tufts are two crossed quads, the rest single cards.
      if let Some(stream) = &grass.stream {
        pass.set_vertex_buffer(1, stream.drawn.slice(..));
        pass.draw_indirect(&stream.args, 0);
        pass.set_vertex_buffer(
          1,
          stream
            .drawn
            .slice(u64::from(GRASS_NEAR_SLOTS) * DRAWN_TUFT_BYTES..),
        );
        pass.draw_indirect(&stream.args, 16);
      }
    }

    let Some(cloud_target) = &self.cloud_target else {
      return Ok(());
    };

    // Clouds at reduced resolution; the composite upsamples them. Skipped
    // entirely when there are no clouds.
    if let (true, Some(pipeline)) = (
      params.clouds_drawn() && self.uniforms.temporal[0] > 0.5,
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
      params.clouds_drawn(),
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

    // A half-resolution copy of the opaque scene and its depth, for water
    // to reflect. Water is not in it, so water never reflects water. Its
    // time counts towards the water pass.
    let copy = match (self.water_visible, &self.reflection) {
      (true, Some((copy_view, _))) => {
        self
          .pipelines
          .render(PipelineKind::SceneCopy)
          .map(|pipeline| {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
              label: Some("VistaWASM scene copy pass"),
              timestamp_writes: timing.map(|timer| wgpu::RenderPassTimestampWrites {
                end_of_pass_write_index: None,
                ..timer.render_writes(PASS_WATER)
              }),
              color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: copy_view,
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
          })
      }
      _ => None,
    };

    // Transparent water on top, depth-tested against the opaque scene.
    if let (true, Some((_, reflection))) = (self.water_visible, &self.reflection) {
      let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("VistaWASM water pass"),
        timestamp_writes: timing.map(|timer| wgpu::RenderPassTimestampWrites {
          beginning_of_pass_write_index: copy.is_none().then_some(PASS_WATER * 2),
          ..timer.render_writes(PASS_WATER)
        }),
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
      pass.set_bind_group(3, reflection, &[]);
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

    if let (true, Some(trees)) = (read_draws, &self.trees) {
      trees.readback.read_back();
    }

    self.grounded(grounded, params.mesh_ground);

    self.frames_in_flight.fetch_add(1, Ordering::AcqRel);
    let frames_in_flight = Arc::clone(&self.frames_in_flight);
    self.queue.on_submitted_work_done(move || {
      frames_in_flight.fetch_sub(1, Ordering::AcqRel);
    });
    self.queue.present(surface_texture);
    self.warm_up(&params.likely);

    // The rest of the trees' variants grow after the first frame.
    if self.first_frame_presented {
      self.grow_step();
    }
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
    pass.forget_lifetime()
  }

  /// Milliseconds spent growing trees (before and after the first frame)
  /// and baking their impostors.
  pub fn tree_timings(&self) -> ([f32; 2], f32) {
    let bake = self
      .tree_growth
      .bake_millis
      .lock()
      .map_or(0.0, |millis| *millis);
    (
      self.tree_growth.millis.map(|millis| millis as f32),
      bake as f32,
    )
  }

  /// What the trees drew in the frame read back last, once: `None` until
  /// the next reading arrives, or without trees.
  pub fn tree_draw(&self, tree_style: u32) -> Option<crate::render::vegetation::TreeDraw> {
    let trees = self.trees.as_ref()?;
    let words = trees.readback.latest.lock().ok()?.take()?;
    let mut draw = crate::render::vegetation::TreeDraw::default();

    for list in 0..MESH_LISTS {
      let triangles = self.tree_ranges[list_mesh(list)].1 / 3;
      let count = words[list * 5 + 1].min(trees.mesh_capacities[list]);
      draw.meshes += count;
      draw.mesh_triangles += u64::from(count) * u64::from(triangles);
    }

    let impostor_triangles = if tree_style == 1 { 4 } else { 2 };
    draw.impostor_triangles = u64::from(words[IMPOSTOR_ARGS_BASE + 1]) * impostor_triangles;
    draw.shadow_casters = words[SHADOW_ARGS_BASE + 1];
    Some(draw)
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
    self.reflection = None;
  }
}

/// Device pixels for `value` CSS pixels, within the texture size the
/// device allows, so a large display at a high pixel ratio draws slightly
/// softer instead of failing to configure the surface.
fn scaled_extent(value: u32, device_pixel_ratio: f32) -> u32 {
  ((value as f32 * device_pixel_ratio).round() as u32).clamp(1, crate::config::MAX_RENDER_SIZE)
}
